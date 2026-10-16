//! Lawfully authored schema-23 records and encoder-generated synthetic pixels only.
use lytronaut_service::{AppState, SecurityConfig, api::Api, router_with_routes, uploads};
use lytronaut_storage::{Catalog, DatabaseConfig, Limits, Store};
use serde_json::{Value, json};
use sqlx::{
    ConnectOptions, Connection, PgConnection, Row, SqliteConnection, any::AnyConnectOptions,
    postgres::PgConnectOptions, sqlite::SqliteConnectOptions,
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
    str::FromStr,
};
fn synthetic_focus(jpeg: &[u8]) -> Vec<u8> {
    use sha1::{Digest, Sha1};
    let identity = |b: &[u8]| format!("sha1-{:x}", Sha1::digest(b));
    let directory = serde_json::to_vec(&json!({"version":{"major":1,"minor":0},"picture":{"recipe":{"orientation":8,"unknown":"preserve"},"accelerationArray":[{"type":"com.lytro.acceleration.refocusStack","vendorContent":{"imageArray":[{"representation":"jpeg","width":2,"height":1,"lambda":-1,"imageRef":identity(jpeg)},{"representation":"jpeg","width":2,"height":1,"lambda":1,"imageRef":identity(jpeg)}]}}]}})).unwrap();
    let mut bytes = b"\x89LFP\r\n\x1a\n\0\0\0\x01\0\0\0\0".to_vec();
    for (directory, payload) in [(true, directory.as_slice()), (false, jpeg)] {
        bytes.extend_from_slice(if directory {
            b"\x89LFM\r\n\x1a\n"
        } else {
            b"\x89LFC\r\n\x1a\n"
        });
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(&(payload.len() as u32).to_be_bytes());
        bytes.extend_from_slice(identity(payload).as_bytes());
        bytes.extend_from_slice(&[0; 35]);
        bytes.extend_from_slice(payload);
        while !bytes.len().is_multiple_of(16) {
            bytes.push(0);
        }
    }
    bytes
}
struct Server {
    root: PathBuf,
    store: Option<Store>,
    catalog: Option<Catalog>,
    database: DatabaseConfig,
    postgres: Option<(PgConnection, String)>,
    port: u16,
    task: Option<tokio::task::JoinHandle<()>>,
    api: Option<Api>,
}
impl Drop for Server {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort()
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Server {
    async fn new() -> Self {
        let mut b = [0; 16];
        getrandom::fill(&mut b).unwrap();
        let root =
            std::env::temp_dir().join(format!("lytronaut-api-test-{:x}", u128::from_le_bytes(b)));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let mut postgres = None;
        let database = match std::env::var("LYTRONAUT_TEST_BACKEND").as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("sqlite") => {
                DatabaseConfig::Sqlite(root.join("catalog.db"))
            }
            Ok("postgres") => {
                let url = std::env::var("LYTRONAUT_TEST_POSTGRES_URL")
                    .expect("PostgreSQL API tests require the explicit disposable test URL");
                let options = PgConnectOptions::from_str(&url)
                    .expect("invalid test PostgreSQL configuration")
                    .disable_statement_logging();
                let mut admin = PgConnection::connect_with(&options)
                    .await
                    .expect("test PostgreSQL connection failed");
                let restricted: bool = sqlx::query_scalar("SELECT NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication) FROM pg_roles WHERE rolname=current_user")
                    .fetch_one(&mut admin).await.unwrap();
                assert!(
                    restricted,
                    "parity tests require a restricted application role"
                );
                let public_create: bool = sqlx::query_scalar(
                    "SELECT has_schema_privilege(current_user,'public','CREATE')",
                )
                .fetch_one(&mut admin)
                .await
                .unwrap();
                assert!(
                    !public_create,
                    "test role must not create objects in public"
                );
                let schema = format!("lytronaut_api_{:x}", u128::from_le_bytes(b));
                sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA \"{schema}\"")))
                    .execute(&mut admin)
                    .await
                    .unwrap();
                let mut selected = AnyConnectOptions::from_str(&url)
                    .expect("invalid test database URL")
                    .database_url;
                selected
                    .query_pairs_mut()
                    .append_pair("options", &format!("-c search_path={schema}"));
                postgres = Some((admin, schema));
                DatabaseConfig::Postgres(selected.to_string())
            }
            _ => panic!("LYTRONAUT_TEST_BACKEND must be sqlite or postgres"),
        };
        let mut s = Self {
            root,
            database,
            postgres,
            catalog: None,
            store: None,
            port: 0,
            task: None,
            api: None,
        };
        s.start().await;
        s
    }
    async fn start(&mut self) {
        self.assert_public_sentinel().await;
        let catalog = Catalog::open(self.database.clone()).await.unwrap();
        if let Some((_, schema)) = &self.postgres {
            // Hold all four leases together: checking one repeatedly could miss
            // a connection whose session options selected the wrong schema.
            let mut connections = Vec::new();
            for _ in 0..4 {
                let mut connection = catalog.pool().acquire().await.unwrap();
                let actual: String = sqlx::query_scalar("SELECT current_schema()::text")
                    .fetch_one(&mut *connection)
                    .await
                    .unwrap();
                assert_eq!(&actual, schema);
                connections.push(connection);
            }
            drop(connections);
            self.assert_public_sentinel().await;
            assert!(!self.root.join("catalog.db").exists());
        }
        let store = Store::open(
            catalog.clone(),
            &self.root.join("originals"),
            &self.root.join("staging"),
            Limits {
                upload_bytes: 4 * 1024 * 1024,
                file_bytes: 2 * 1024 * 1024,
                encoded_bytes: 4 * 1024 * 1024,
                metadata_bytes: 64 * 1024,
                files: 32,
                free_floor: 0,
                reserved_bytes: 2 * 1024 * 1024 * 1024,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let api = Api::open(catalog.clone(), store.clone()).await.unwrap();
        self.assert_public_sentinel().await;
        let app = router_with_routes(
            AppState {
                catalog: catalog.clone(),
                security: SecurityConfig::new("http://127.0.0.1:3000").unwrap(),
            },
            uploads::routes(store.clone()).merge(api.clone().routes()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        self.port = listener.local_addr().unwrap().port();
        self.task = Some(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
        self.store = Some(store);
        self.catalog = Some(catalog);
        self.api = Some(api);
    }
    async fn restart(&mut self) {
        let t = self.task.take().unwrap();
        t.abort();
        let _ = t.await;
        self.api.take().unwrap().shutdown().await.unwrap();
        self.store.take();
        self.catalog.take().unwrap().pool().close().await;
        self.start().await;
    }
    async fn assert_public_sentinel(&self) {
        if let Some((_, schema)) = &self.postgres {
            let options =
                PgConnectOptions::from_str(&std::env::var("LYTRONAUT_TEST_POSTGRES_URL").unwrap())
                    .unwrap()
                    .disable_statement_logging();
            let mut connection = PgConnection::connect_with(&options).await.unwrap();
            let intact: bool = sqlx::query_scalar("SELECT count(*)=1 AND min(marker)='synthetic-persistence-parity' FROM public.lytronaut_parity_sentinel")
                .fetch_one(&mut connection).await.unwrap();
            assert!(intact, "public sentinel must remain unchanged");
            let leaked: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname IN ('intake_jobs','application_job_order','catalog_versions','export_jobs')")
                .fetch_one(&mut connection).await.unwrap();
            assert_eq!(
                leaked, 0,
                "application objects must remain in the owned schema"
            );
            assert!(schema.starts_with("lytronaut_api_"));
            connection.close().await.unwrap();
        }
    }
    async fn close(mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
            let _ = task.await;
        }
        self.api.take().unwrap().shutdown().await.unwrap();
        self.store.take();
        self.catalog.take().unwrap().pool().close().await;
        self.assert_public_sentinel().await;
        if let Some((mut admin, schema)) = self.postgres.take() {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP SCHEMA \"{schema}\" CASCADE"
            )))
            .execute(&mut admin)
            .await
            .unwrap();
            admin.close().await.unwrap();
        }
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        headers: &str,
        body: &[u8],
    ) -> (u16, String, Vec<u8>) {
        let port = self.port;
        let mut request=format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:3000\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}\r\n",body.len()).into_bytes();
        request.extend(body);
        tokio::task::spawn_blocking(move || {
            let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(30)))
                .unwrap();
            socket.write_all(&request).unwrap();
            let mut reply = Vec::new();
            match socket.read_to_end(&mut reply) {
                Ok(_) => {}
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset && !reply.is_empty() => {}
                Err(e) => panic!("HTTP read failed: {e}"),
            };
            let pos = reply.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
            let head = String::from_utf8(reply[..pos].to_vec()).unwrap();
            let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
            (status, head, reply[pos + 4..].to_vec())
        })
        .await
        .unwrap()
    }
    async fn get(&self, path: &str, headers: &str) -> Value {
        let r = self.request("GET", path, headers, b"").await;
        assert_eq!(r.0, 200, "{path}: {}", String::from_utf8_lossy(&r.2));
        serde_json::from_slice(&r.2).unwrap()
    }
    async fn request_headers(&self) -> (String, String) {
        (
            String::new(),
            "Origin: http://127.0.0.1:3000\r\nX-Lytronaut-Intent: mutation\r\n".into(),
        )
    }

    async fn source(&self, version: u16) -> Vec<u8> {
        let path = self.root.join(format!("synthetic-{version}.db"));
        let mut c = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&path)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        sqlx::raw_sql("CREATE TABLE library_metadata(version INTEGER);INSERT INTO library_metadata VALUES(23);
CREATE TABLE picture(uuid TEXT,imagebin_uuid TEXT,event_uuid TEXT,name TEXT,caption TEXT,rating INTEGER,flag_status INTEGER);INSERT INTO picture VALUES('synthetic-p1','bin','event','雪 🌌','Caption café',4,2),('synthetic-p2','bin','event','Second','other',1,3);
CREATE TABLE picture_metadata(uuid TEXT,lfe_major INTEGER,lfe_minor INTEGER);INSERT INTO picture_metadata VALUES('synthetic-p1',10,0),('synthetic-p2',11,0);
CREATE TABLE album(uuid TEXT,name TEXT,sort_order INTEGER,public INTEGER);INSERT INTO album VALUES('synthetic-a','Album α',8,1),('synthetic-b','Album β',3,0);
CREATE TABLE album_to_picture(album_uuid TEXT,picture_uuid TEXT,picture_index INTEGER);INSERT INTO album_to_picture VALUES('synthetic-a','synthetic-p2',0),('synthetic-a','synthetic-p1',1),('synthetic-b','synthetic-p1',0);
CREATE TABLE event(uuid TEXT,library_uuid TEXT);INSERT INTO event VALUES('event','synthetic-library-uuid');CREATE TABLE image_bin(uuid TEXT);INSERT INTO image_bin VALUES('bin');CREATE TABLE picture_list(uuid TEXT,filter INTEGER,sort_order INTEGER);INSERT INTO picture_list VALUES('list',0,0);CREATE TABLE imported_picture_history(hash TEXT); CREATE TABLE unknown_backup_meta(text_value TEXT, real_value REAL, blob_value BLOB); INSERT INTO unknown_backup_meta VALUES('future opaque',1.25,x'00ff'); ALTER TABLE picture ADD COLUMN unknown_scalar BLOB; UPDATE picture SET unknown_scalar=x'00ff80';").execute(&mut c).await.unwrap();
        sqlx::query("UPDATE library_metadata SET version=?")
            .bind(version)
            .execute(&mut c)
            .await
            .unwrap();
        c.close().await.unwrap();
        fs::read(path).unwrap()
    }
    async fn upload(&self, mutation: &str, version: u16) -> (String, Vec<u8>) {
        let req=json!({"version":1,"kind":"directory","declared_bytes":4*1024*1024,"declared_files":32}).to_string();
        let r = self
            .request("POST", "/imports", mutation, req.as_bytes())
            .await;
        assert_eq!(r.0, 201);
        let id = serde_json::from_slice::<Value>(&r.2).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new(&mut jpeg)
            .encode(
                &[255, 0, 0, 0, 255, 0],
                2,
                1,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        let db = self.source(version).await;
        for (path, data) in [
            ("lytrolibrary.db", db),
            ("bin/synthetic-p1/thumbnail.jpg", jpeg.clone()),
            (
                "bin/synthetic-p1/raw.lfp",
                b"unknown camera preserved original".to_vec(),
            ),
            ("bin/synthetic-p2/thumbnail.jpg", b"malformed JPEG".to_vec()),
            (
                "bin/synthetic-p2/raw.lfp",
                b"valid preservation bytes".to_vec(),
            ),
            ("empty.bin", vec![]),
            ("bin/synthetic-p1/stacklq.lfp", synthetic_focus(&jpeg)),
            (
                "bin/synthetic-p1/view_parameters.json",
                br#"{"orientation":6,"unknown":"preserve"}"#.to_vec(),
            ),
            (
                "bin/synthetic-p2/stacklq.lfp",
                b"preserved future JPEG planes".to_vec(),
            ),
        ] {
            let hex: String = path.bytes().map(|b| format!("{b:02x}")).collect();
            let h = format!(
                "{mutation}X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: {hex}\r\n"
            );
            assert_eq!(
                self.request("PUT", &format!("/imports/{id}/file"), &h, &data)
                    .await
                    .0,
                204
            );
        }
        (id, jpeg)
    }
}
#[tokio::test]
async fn direct_upload_catalog_preview_original_restart_and_recovery() {
    let mut s = Server::new().await;
    let (headers, mutation) = s.request_headers().await;
    let (job, jpeg) = s.upload(&mutation, 23).await;
    let progress = s.get(&format!("/imports/{job}"), &headers).await;
    assert_eq!(progress["phase"], "intake");
    assert!(progress["completed_bytes"].as_u64().unwrap() > 0);
    // Interrupt after intake completion, before adaptation. Startup must recover the durable job.
    s.store.as_ref().unwrap().finish(&job).await.unwrap();
    let progress = s.get(&format!("/imports/{job}"), &headers).await;
    assert_eq!(progress["phase"], "catalog_pending");
    s.restart().await;
    let progress = s.get(&format!("/imports/{job}"), &headers).await;
    assert_eq!(progress["phase"], "catalog_complete");
    assert_eq!(progress["catalog_state"], "ready");
    let lib = progress["library"].as_str().unwrap();
    let libraries = s.get("/libraries", &headers).await;
    assert_eq!(libraries["items"].as_array().unwrap().len(), 1);
    assert_eq!(libraries["items"][0]["captures"], 2);
    let albums = s.get(&format!("/libraries/{lib}/albums"), &headers).await;
    assert_eq!(albums["items"][0]["name"], "Album β");
    assert_eq!(albums["items"][1]["sort_order"], 8);
    let album = albums["items"][1]["id"].as_str().unwrap();
    let first = s
        .get(&format!("/albums/{album}/captures?limit=1"), &headers)
        .await;
    let second = s
        .get(
            &format!("/albums/{album}/captures?limit=1&offset=1"),
            &headers,
        )
        .await;
    assert_eq!(first["items"][0]["name"], "Second");
    assert_eq!(first["items"][0]["picture_index"], 0);
    assert_eq!(second["items"][0]["rating"], 4);
    assert_eq!(first["next_offset"], 1);
    assert!(second["next_offset"].is_null());
    let id = second["items"][0]["id"].as_str().unwrap();
    let detail = s.get(&format!("/captures/{id}"), &headers).await;
    assert_eq!(detail["caption"], "Caption café");
    assert_eq!(detail["capture"]["flag_status"], 2);
    assert!(!detail["item_errors"].as_array().unwrap().is_empty());
    assert_eq!(
        detail["capabilities"]["discrete_focus"]["state"],
        "available"
    );
    let stack_id = detail["originals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|v| v["role"] == "jpeg_focus_stack")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let stack = s.get(&format!("/assets/{stack_id}/focus"), &headers).await;
    assert_eq!(stack["lambdas"], json!([-1.0, 1.0]));
    assert!(
        stack["orientation_policy"]
            .as_str()
            .unwrap()
            .contains("default identity")
    );
    for orientation in 1..=8 {
        let r = s
            .request(
                "GET",
                &format!("/assets/{stack_id}/focus/0/{orientation}"),
                &headers,
                b"",
            )
            .await;
        assert_eq!(r.0, 200);
        let decoded =
            lytronaut_format::decode_jpeg(&r.2, lytronaut_format::Limits::default()).unwrap();
        assert_eq!(
            (decoded.geometry.width, decoded.geometry.height),
            if orientation >= 5 { (1, 2) } else { (2, 1) }
        );
    }
    for (suffix, status) in [
        ("0/0", 400),
        ("0/9", 400),
        ("0/no", 400),
        ("no/1", 400),
        ("99/1", 404),
    ] {
        assert_eq!(
            s.request(
                "GET",
                &format!("/assets/{stack_id}/focus/{suffix}"),
                &headers,
                b""
            )
            .await
            .0,
            status
        );
    }
    assert_eq!(
        s.request("GET", &format!("/assets/{stack_id}/focus"), "", b"")
            .await
            .0,
        200
    );
    assert_eq!(
        s.request("GET", &format!("/assets/{stack_id}/focus/0/1"), "", b"")
            .await
            .0,
        200
    );
    let original_stack = s
        .request(
            "GET",
            &format!("/assets/{stack_id}/download"),
            &headers,
            b"",
        )
        .await;
    assert_eq!(original_stack.2, synthetic_focus(&jpeg));
    let preview = detail["preview"].as_str().unwrap();
    let manifest = s
        .get(&format!("/assets/{preview}/manifest"), &headers)
        .await;
    assert_eq!(manifest["dimensions"], json!({"width":2,"height":1}));
    assert_eq!(manifest["decoded_bytes"], 8);
    assert_eq!(manifest["color_space"], "unknown");
    assert_eq!(manifest["representation"], "jpeg_preview");
    let encoded = serde_json::to_string(&manifest).unwrap();
    for marker in [
        "bin/",
        "synthetic-p1",
        "synthetic-library-uuid",
        "camera",
        "serial",
        "password",
        "source_path",
    ] {
        assert!(!encoded.contains(marker));
    }
    let r = s
        .request("GET", &format!("/assets/{preview}/chunks/0"), &headers, b"")
        .await;
    assert_eq!(r.0, 200);
    assert_eq!(r.2, jpeg);
    assert!(r.1.contains("content-length:"));
    for path in [
        format!("/assets/{preview}/manifest"),
        format!("/assets/{preview}/chunks/0"),
        format!("/assets/{preview}/download"),
    ] {
        assert_eq!(s.request("GET", &path, "", b"").await.0, 200);
    }
    assert_eq!(
        s.request(
            "GET",
            &format!("/assets/{preview}/chunks/99"),
            &headers,
            b""
        )
        .await
        .0,
        404
    );
    assert_eq!(
        s.request(
            "GET",
            &format!("/assets/{preview}/chunks/nope"),
            &headers,
            b""
        )
        .await
        .0,
        400
    );
    assert_eq!(
        s.request(
            "GET",
            &format!("/assets/{preview}/download"),
            &format!("{headers}Range: bytes=0-9\r\n"),
            b""
        )
        .await
        .0,
        400
    );
    let bad_id = first["items"][0]["id"].as_str().unwrap();
    let bad = s.get(&format!("/captures/{bad_id}"), &headers).await;
    assert!(bad["preview"].is_null());
    assert_eq!(
        bad["originals"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| f["role"] == "jpeg_focus_stack")
            .count(),
        1
    );
    assert_eq!(
        bad["capabilities"]["discrete_focus"]["state"],
        "unavailable"
    );
    assert_eq!(
        bad["capabilities"]["discrete_focus"]["reason"],
        "corrupt_asset"
    );
    assert_eq!(bad["preview_error"]["code"], "corrupt_asset");
    assert_eq!(bad["capabilities"]["originals"]["state"], "available");
    for file in bad["originals"].as_array().unwrap() {
        let fid = file["id"].as_str().unwrap();
        let r = s
            .request("GET", &format!("/assets/{fid}/download"), &headers, b"")
            .await;
        assert_eq!(r.0, 200);
        assert_eq!(r.2.len() as u64, file["encoded_bytes"].as_u64().unwrap());
    }
    let searched = s
        .get(
            &format!("/libraries/{lib}/captures?search=caf%C3%A9&sort=rating"),
            &headers,
        )
        .await;
    assert_eq!(searched["items"].as_array().unwrap().len(), 1);
    for query in [
        "limit=101",
        "offset=100001",
        "search=%ZZ",
        "sort=private-source-marker",
        "unknown=private-source-marker",
        "limit=1&limit=2",
    ] {
        let r = s
            .request(
                "GET",
                &format!("/libraries/{lib}/captures?{query}"),
                &headers,
                b"",
            )
            .await;
        assert_eq!(r.0, 400);
        assert!(!String::from_utf8_lossy(&r.2).contains("private-source-marker"));
    }
    let before = s.get(&format!("/albums/{album}/captures"), &headers).await;
    s.restart().await;
    assert_eq!(
        before,
        s.get(&format!("/albums/{album}/captures"), &headers).await
    );
    s.close().await;
}
#[tokio::test]
async fn finish_adapts_unknown_schema_preserved_download_and_cancel_consistency() {
    let s = Server::new().await;
    let (headers, mutation) = s.request_headers().await;
    let (job, _) = s.upload(&mutation, 24).await;
    assert_eq!(
        s.request("POST", &format!("/imports/{job}/finish"), &headers, b"")
            .await
            .0,
        403
    );
    assert_eq!(
        s.request(
            "POST",
            &format!("/imports/{job}/finish"),
            &format!("{mutation}Origin: https://evil.invalid\r\n"),
            b""
        )
        .await
        .0,
        403
    );
    assert_eq!(
        s.request("POST", &format!("/imports/{job}/finish"), &mutation, b"")
            .await
            .0,
        204
    );
    let p = s.get(&format!("/imports/{job}"), &headers).await;
    assert_eq!(p["catalog_state"], "preserved_only");
    let lib = p["library"].as_str().unwrap();
    assert_eq!(
        s.request("GET", &format!("/libraries/{lib}/captures"), &headers, b"")
            .await
            .0,
        409
    );
    let files = s.get(&format!("/libraries/{lib}/files"), &headers).await;
    assert_eq!(files["items"].as_array().unwrap().len(), 9);
    let mut empty = 0;
    for f in files["items"].as_array().unwrap() {
        let id = f["id"].as_str().unwrap();
        let r = s
            .request("GET", &format!("/assets/{id}/download"), &headers, b"")
            .await;
        assert_eq!(r.0, 200);
        assert_eq!(r.2.len() as u64, f["encoded_bytes"].as_u64().unwrap());
        if r.2.is_empty() {
            empty += 1;
            assert_eq!(
                s.request("GET", &format!("/assets/{id}/manifest"), &headers, b"")
                    .await
                    .0,
                422
            );
        }
    }
    assert_eq!(empty, 1);
    let req =
        json!({"version":1,"kind":"directory","declared_bytes":10,"declared_files":1}).to_string();
    let r = s
        .request("POST", "/imports", &mutation, req.as_bytes())
        .await;
    let id = serde_json::from_slice::<Value>(&r.2).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        s.request("POST", &format!("/imports/{id}/cancel"), &mutation, b"")
            .await
            .0,
        204
    );
    let p = s.get(&format!("/imports/{id}"), &headers).await;
    assert_eq!(p["phase"], "cancelled");
    assert!(p["library"].is_null());
    assert_eq!(
        s.get("/libraries", &headers).await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    s.close().await;
}

fn stored_zip(name: &str, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let crc = crc32fast::hash(data);
    for bytes in [
        0x04034b50u32.to_le_bytes().to_vec(),
        20u16.to_le_bytes().to_vec(),
        vec![0; 8],
        crc.to_le_bytes().to_vec(),
        (data.len() as u32).to_le_bytes().to_vec(),
        (data.len() as u32).to_le_bytes().to_vec(),
        (name.len() as u16).to_le_bytes().to_vec(),
        vec![0; 2],
        name.as_bytes().to_vec(),
        data.to_vec(),
    ] {
        out.extend(bytes)
    }
    let central = out.len() as u32;
    for bytes in [
        0x02014b50u32.to_le_bytes().to_vec(),
        20u16.to_le_bytes().to_vec(),
        20u16.to_le_bytes().to_vec(),
        vec![0; 8],
        crc.to_le_bytes().to_vec(),
        (data.len() as u32).to_le_bytes().to_vec(),
        (data.len() as u32).to_le_bytes().to_vec(),
        (name.len() as u16).to_le_bytes().to_vec(),
        vec![0; 16],
        name.as_bytes().to_vec(),
    ] {
        out.extend(bytes)
    }
    let length = out.len() as u32 - central;
    for bytes in [
        0x06054b50u32.to_le_bytes().to_vec(),
        vec![0; 4],
        1u16.to_le_bytes().to_vec(),
        1u16.to_le_bytes().to_vec(),
        length.to_le_bytes().to_vec(),
        central.to_le_bytes().to_vec(),
        vec![0; 2],
    ] {
        out.extend(bytes)
    }
    out
}
#[tokio::test]
async fn zip_completion_adapts_without_an_extra_finish_request() {
    let s = Server::new().await;
    let (headers, mutation) = s.request_headers().await;
    let request =
        json!({"version":1,"kind":"zip_library","declared_bytes":1024,"declared_files":1})
            .to_string();
    let r = s
        .request("POST", "/imports", &mutation, request.as_bytes())
        .await;
    assert_eq!(r.0, 201);
    let id = serde_json::from_slice::<Value>(&r.2).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let zip = stored_zip("unknown.bin", b"independent preserved ZIP bytes");
    let r = s
        .request("PUT", &format!("/imports/{id}/zip"), &mutation, &zip)
        .await;
    assert_eq!(r.0, 204, "{}", String::from_utf8_lossy(&r.2));
    let p = s.get(&format!("/imports/{id}"), &headers).await;
    assert_eq!(p["phase"], "catalog_complete");
    assert_eq!(p["catalog_state"], "preserved_only");
    assert_eq!(
        s.request("POST", &format!("/imports/{id}/finish"), &mutation, b"")
            .await
            .0,
        204
    );
    s.close().await;
}

#[tokio::test]
async fn polling_during_catalog_publication_keeps_completed_import_visible() {
    let s = Server::new().await;
    let (headers, mutation) = s.request_headers().await;
    for iteration in 0..6 {
        let (job, _) = s.upload(&mutation, 24 + iteration).await;
        let finish_path = format!("/imports/{job}/finish");
        let finish = s.request("POST", &finish_path, &mutation, b"");
        let path = format!("/imports/{job}");
        let polling = async {
            for _ in 0..40 {
                let (a, b, c) = tokio::join!(
                    s.request("GET", &path, &headers, b""),
                    s.request("GET", &path, &headers, b""),
                    s.request("GET", &path, &headers, b"")
                );
                for response in [a, b, c] {
                    assert_eq!(response.0, 200);
                }
            }
        };
        let (response, ()) = tokio::join!(finish, polling);
        assert_eq!(response.0, 204);
        let progress = s.get(&path, &headers).await;
        assert_eq!(progress["phase"], "catalog_complete");
        assert_eq!(progress["catalog_state"], "preserved_only");
    }
    s.close().await;
}

async fn wait_export(s: &Server, id: &str, headers: &str) -> Value {
    for _ in 0..400 {
        let r = s.get(&format!("/exports/{id}"), headers).await;
        if matches!(
            r["state"].as_str(),
            Some("succeeded" | "failed" | "cancelled")
        ) {
            return r;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("export did not reach durable terminal state")
}
#[tokio::test]
async fn direct_flatten_png_jpeg_exact_pixels_provenance_cancel_and_restart() {
    use sha2::{Digest, Sha256};
    let mut s = Server::new().await;
    let (headers, mutation) = s.request_headers().await;
    let (import, jpeg) = s.upload(&mutation, 23).await;
    assert_eq!(
        s.request("POST", &format!("/imports/{import}/finish"), &mutation, b"")
            .await
            .0,
        204
    );
    let library = s.get("/libraries", &headers).await["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let captures = s
        .get(
            &format!("/libraries/{library}/captures?sort=rating"),
            &headers,
        )
        .await;
    let capture = captures["items"][0]["id"].as_str().unwrap();
    let detail = s.get(&format!("/captures/{capture}"), &headers).await;
    let focus = detail["originals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["role"] == "jpeg_focus_stack")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let preview = detail["preview"].as_str().unwrap();
    let decoded = lytronaut_format::decode_jpeg(&jpeg, Default::default()).unwrap();
    for orientation in 1..=8 {
        let request=json!({"kind":"flatten","version":1,"asset":focus,"format":"png","plane_index":1,"orientation":orientation,"operations":[]}).to_string();
        assert_eq!(
            s.request(
                "POST",
                "/exports",
                "Origin: http://127.0.0.1:3000\r\n",
                request.as_bytes()
            )
            .await
            .0,
            403
        );
        assert_eq!(
            s.request("POST", "/exports", &headers, request.as_bytes())
                .await
                .0,
            403
        );
        let r = s
            .request("POST", "/exports", &mutation, request.as_bytes())
            .await;
        assert_eq!(r.0, 200);
        let id = serde_json::from_slice::<Value>(&r.2).unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(wait_export(&s, &id, &headers).await["state"], "succeeded");
        let recipe = s.get(&format!("/exports/{id}/recipe"), &headers).await;
        assert_eq!(recipe["plane_index"], 1);
        assert_eq!(recipe["lambda"], 1.0);
        assert_eq!(recipe["orientation"], orientation);
        assert_eq!(recipe["source_variant"], "stored_jpeg_focus_plane");
        assert_eq!(recipe["color_space"], "unknown");
        assert_eq!(recipe["refocus_loss"], true);
        assert!(
            recipe["imported_adjustments"]
                .as_str()
                .unwrap()
                .contains("preserved only")
        );
        let r = s
            .request("GET", &format!("/exports/{id}/download"), &headers, b"")
            .await;
        assert_eq!(r.0, 200);
        assert!(r.1.contains("image/png"));
        assert_eq!(
            recipe["content_sha256"],
            format!("{:x}", Sha256::digest(&r.2))
        );
        let output = image::load_from_memory_with_format(&r.2, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        assert_eq!(
            (output.width(), output.height()),
            if orientation >= 5 { (1, 2) } else { (2, 1) }
        );
        let reversed = matches!(orientation, 2 | 3 | 7 | 8);
        let expected = if reversed {
            [&decoded.rgba[4..8], &decoded.rgba[..4]].concat()
        } else {
            decoded.rgba.clone()
        };
        assert_eq!(
            output.into_raw(),
            expected,
            "PNG must preserve stored JPEG decoded pixels without another JPEG pass"
        );
        assert_eq!(
            recipe["visible_geometry"],
            if orientation >= 5 {
                json!([0, 0, 1, 2])
            } else {
                json!([0, 0, 2, 1])
            }
        );
        assert_eq!(
            s.request("GET", &format!("/assets/{focus}/download"), &headers, b"")
                .await
                .2,
            synthetic_focus(&jpeg)
        );
    }
    for (orientation, operations) in [
        (0, json!([])),
        (9, json!([])),
        (1, json!(["tone"])),
        (1, json!(["aperture"])),
        (1, json!(["animation"])),
    ] {
        let request=json!({"kind":"flatten","version":1,"asset":preview,"format":"jpeg","plane_index":null,"orientation":orientation,"operations":operations}).to_string();
        assert_eq!(
            s.request("POST", "/exports", &mutation, request.as_bytes())
                .await
                .0,
            400
        );
    }
    let request=json!({"kind":"flatten","version":1,"asset":preview,"format":"jpeg","plane_index":null,"orientation":1,"operations":[]}).to_string();
    let r = s
        .request("POST", "/exports", &mutation, request.as_bytes())
        .await;
    let id = serde_json::from_slice::<Value>(&r.2).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(wait_export(&s, &id, &headers).await["state"], "succeeded");
    let r = s
        .request("GET", &format!("/exports/{id}/download"), &headers, b"")
        .await;
    assert_eq!(r.0, 200);
    assert!(r.1.contains("image/jpeg"));
    let recipe = s.get(&format!("/exports/{id}/recipe"), &headers).await;
    assert_eq!(recipe["source_variant"], "verified_static_jpeg");
    assert!(
        recipe["processing_loss"]
            .as_str()
            .unwrap()
            .contains("Lossy")
    );
    s.restart().await;
    assert_eq!(
        s.get(&format!("/exports/{id}"), &headers).await["state"],
        "succeeded"
    );
    assert_eq!(
        s.request("DELETE", &format!("/exports/{id}"), &mutation, b"")
            .await
            .0,
        200
    );
    assert_eq!(
        s.get(&format!("/exports/{id}"), &headers).await["state"],
        "cancelled"
    );
    assert_eq!(
        s.request("GET", &format!("/exports/{id}/download"), &headers, b"")
            .await
            .0,
        404
    );
    assert_eq!(
        s.request("GET", &format!("/assets/{preview}/download"), &headers, b"")
            .await
            .2,
        jpeg
    );
    s.close().await;
}
#[tokio::test]
async fn public_lossless_bundle_unknown_bytes_catalog_restore_tamper_conflict_and_interruption() {
    use lytronaut_service::bundle;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    let s = Server::new().await;
    let (headers, mutation) = s.request_headers().await;
    let (import, _) = s.upload(&mutation, 23).await;
    assert_eq!(
        s.request("POST", &format!("/imports/{import}/finish"), &mutation, b"")
            .await
            .0,
        204
    );
    let catalog = s.catalog.as_ref().unwrap().clone();
    let pending = s
        .store
        .as_ref()
        .unwrap()
        .begin(&lytronaut_contracts::ImportRequest {
            version: lytronaut_contracts::WireVersion::V1,
            kind: lytronaut_contracts::ImportKind::Directory,
            declared_bytes: 1024,
            declared_files: 1,
        })
        .await
        .unwrap();
    assert!(!pending.is_empty());
    let store = s.store.as_ref().unwrap();
    let extra = store
        .begin(&lytronaut_contracts::ImportRequest {
            version: lytronaut_contracts::WireVersion::V1,
            kind: lytronaut_contracts::ImportKind::Directory,
            declared_bytes: 2 * 1024 * 1024,
            declared_files: 1,
        })
        .await
        .unwrap();
    store
        .append_directory_file(
            &extra,
            "unknown.large",
            std::io::Cursor::new(vec![19u8; 2 * 1024 * 1024]),
        )
        .await
        .unwrap();
    store.finish(&extra).await.unwrap();
    let expected = bundle::trusted_schema(&catalog).await.unwrap();
    let archive = s.root.join("bundle.lytronaut");
    let cancel = std::sync::Arc::new(AtomicBool::new(false));
    let progress = std::sync::Arc::new(AtomicU64::new(0));
    let result = bundle::backup(
        &catalog,
        s.store.as_ref().unwrap(),
        &archive,
        u64::MAX,
        &cancel,
        &progress,
    )
    .await
    .unwrap();
    assert_eq!(result.files, 10);
    let before = fs::read(&archive).unwrap();
    assert!(
        bundle::backup(
            &catalog,
            s.store.as_ref().unwrap(),
            &archive,
            u64::MAX,
            &cancel,
            &progress
        )
        .await
        .is_err()
    );
    assert_eq!(fs::read(&archive).unwrap(), before);
    let restored = s.root.join("restored");
    let result = bundle::restore(
        &archive,
        &restored,
        &expected,
        s.store.as_ref().unwrap().limits(),
        &cancel,
        &progress,
    )
    .await
    .unwrap();
    assert_eq!(result.files, 10);
    let c = Catalog::open_application_database(&restored.join("store/catalog.db"))
        .await
        .unwrap();
    for table in [
        "catalog_records",
        "catalog_pictures",
        "catalog_albums",
        "catalog_memberships",
        "catalog_sources",
        "api_ids",
    ] {
        let query = format!("SELECT * FROM {table} ORDER BY 1");
        let original = sqlx::query(sqlx::AssertSqlSafe(query.as_str()))
            .fetch_all(catalog.pool())
            .await
            .unwrap();
        let restored_rows = sqlx::query(sqlx::AssertSqlSafe(query.as_str()))
            .fetch_all(c.pool())
            .await
            .unwrap();
        assert_eq!(original.len(), restored_rows.len());
    }
    // Compare independently selected logical bytes; PostgreSQL's hex source
    // keys and bytea text are derived storage representations, not source values.
    for (sqlite, postgres) in [
        (
            "SELECT lower(hex(original)) FROM catalog_records ORDER BY version,kind,ordinal",
            "SELECT encode(original,'hex') FROM catalog_records ORDER BY version,kind,ordinal",
        ),
        (
            "SELECT lower(hex(CAST(uuid AS BLOB)))||':'||lower(hex(CAST(name AS BLOB)))||':'||lower(hex(CAST(caption AS BLOB)))||':'||rating||':'||flag_status FROM catalog_pictures ORDER BY version,uuid",
            "SELECT uuid||':'||encode(name,'hex')||':'||encode(caption,'hex')||':'||rating::text||':'||flag_status::text FROM catalog_pictures ORDER BY version,uuid",
        ),
        (
            "SELECT lower(hex(CAST(uuid AS BLOB)))||':'||lower(hex(CAST(name AS BLOB)))||':'||sort_order FROM catalog_albums ORDER BY version,uuid",
            "SELECT uuid||':'||encode(name,'hex')||':'||sort_order::text FROM catalog_albums ORDER BY version,uuid",
        ),
        (
            "SELECT lower(hex(CAST(album_uuid AS BLOB)))||':'||lower(hex(CAST(picture_uuid AS BLOB)))||':'||picture_index FROM catalog_memberships ORDER BY version,album_uuid,picture_index",
            "SELECT album_uuid||':'||picture_uuid||':'||picture_index::text FROM catalog_memberships ORDER BY version,album_uuid,picture_index",
        ),
        (
            "SELECT lower(hex(CAST(id AS BLOB)))||':'||lower(hex(CAST(kind AS BLOB)))||':'||lower(hex(CAST(source_key AS BLOB))) FROM api_ids ORDER BY id",
            "SELECT encode(convert_to(id,'UTF8'),'hex')||':'||encode(convert_to(kind,'UTF8'),'hex')||':'||source_key FROM api_ids ORDER BY id",
        ),
    ] {
        let original: Vec<String> = sqlx::query_scalar(catalog.dialect(sqlite, postgres))
            .fetch_all(catalog.pool())
            .await
            .unwrap();
        let restored_values: Vec<String> = sqlx::query_scalar(sqlite)
            .fetch_all(c.pool())
            .await
            .unwrap();
        assert_eq!(original, restored_values);
    }
    let rows = sqlx::query("SELECT digest,length,path FROM intake_sources ORDER BY path")
        .fetch_all(c.pool())
        .await
        .unwrap();
    use sqlx::Row;
    for r in rows {
        let digest: String = r.try_get(0).unwrap();
        let size: i64 = r.try_get(1).unwrap();
        let original = fs::read(s.root.join("originals").join(format!("{digest}-{size}"))).unwrap();
        let restored = fs::read(
            restored
                .join("store/originals")
                .join(format!("{digest}-{size}")),
        )
        .unwrap();
        assert_eq!(original, restored);
    }
    for table in ["export_jobs"] {
        assert_eq!(
            sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM {table}"
            )))
            .fetch_one(c.pool())
            .await
            .unwrap(),
            0
        );
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM intake_jobs WHERE state!='succeeded'")
            .fetch_one(c.pool())
            .await
            .unwrap(),
        0
    );
    assert!(
        bundle::restore(
            &archive,
            &restored,
            &expected,
            s.store.as_ref().unwrap().limits(),
            &cancel,
            &progress
        )
        .await
        .is_err()
    );
    let failed = s.root.join("failed");
    for bytes in [
        before[..59].to_vec(),
        {
            let mut b = before.clone();
            *b.last_mut().unwrap() ^= 1;
            b
        },
        {
            let mut b = before.clone();
            b.extend_from_slice(b"trailing");
            b
        },
    ] {
        let bad = s.root.join("bad.bundle");
        fs::write(&bad, bytes).unwrap();
        assert!(
            bundle::restore(
                &bad,
                &failed,
                &expected,
                s.store.as_ref().unwrap().limits(),
                &cancel,
                &progress
            )
            .await
            .is_err()
        );
        assert!(!failed.exists());
    }
    // Build a legacy SQLite archive independently of the portable backup encoder.
    // Its trusted schema is a fresh destination created above; VACUUM captures
    // committed WAL content rather than copying a possibly stale main file.
    use sha2::{Digest, Sha256};
    let legacy_seed = s.root.join("independent-legacy.db");
    sqlx::query("VACUUM INTO $1")
        .bind(legacy_seed.to_str().unwrap())
        .execute(c.pool())
        .await
        .unwrap();
    let legacy_database = fs::read(&legacy_seed).unwrap();
    let portable_size = u64::from_le_bytes(before[20..28].try_into().unwrap()) as usize;
    let mut legacy_before = before[..60].to_vec();
    legacy_before[..16].copy_from_slice(b"LYTRONAUT-BNDL01");
    legacy_before[20..28].copy_from_slice(&(legacy_database.len() as u64).to_le_bytes());
    legacy_before[28..60].copy_from_slice(&Sha256::digest(&legacy_database));
    legacy_before.extend_from_slice(&legacy_database);
    legacy_before.extend_from_slice(&before[60 + portable_size..]);
    let before = legacy_before;
    // Recomputed checksums do not authorize executable or inconsistent legacy
    // app databases. The earlier corruption/truncation checks exercise v2.
    let dbsize = u64::from_le_bytes(before[20..28].try_into().unwrap()) as usize;
    for sql in [
        "CREATE TRIGGER hostile AFTER INSERT ON catalog_albums BEGIN DELETE FROM catalog_pictures; END",
        "CREATE VIEW hostile AS SELECT * FROM export_jobs",
        "CREATE TABLE hostile(x INTEGER,y INTEGER GENERATED ALWAYS AS (x+1) STORED)",
        "CREATE TABLE auth_account (malformed TEXT)",
        "UPDATE intake_jobs SET state='running'",
        "UPDATE catalog_sources SET digest='ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff'",
    ] {
        let path = s.root.join("hostile.db");
        let _ = fs::remove_file(&path);
        fs::write(&path, &before[60..60 + dbsize]).unwrap();
        let mut db = SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&path))
            .await
            .unwrap();
        sqlx::raw_sql(sql).execute(&mut db).await.unwrap();
        db.close().await.unwrap();
        let db = fs::read(path).unwrap();
        let mut altered = before[..60].to_vec();
        altered[20..28].copy_from_slice(&(db.len() as u64).to_le_bytes());
        altered[28..60].copy_from_slice(&Sha256::digest(&db));
        altered.extend_from_slice(&db);
        altered.extend_from_slice(&before[60 + dbsize..]);
        let bad = s.root.join("bad.bundle");
        fs::write(&bad, altered).unwrap();
        assert!(
            bundle::restore(
                &bad,
                &failed,
                &expected,
                s.store.as_ref().unwrap().limits(),
                &cancel,
                &progress
            )
            .await
            .is_err(),
            "untrusted SQL schema/association must fail closed"
        );
        assert!(!failed.exists());
    }
    // Cancel after a real chunk has been written, not merely before starting.
    progress.store(0, std::sync::atomic::Ordering::Relaxed);
    let flag = cancel.clone();
    let observed = progress.clone();
    let done = std::sync::Arc::new(AtomicBool::new(false));
    let finished = done.clone();
    let canceller = std::thread::spawn(move || {
        while observed.load(std::sync::atomic::Ordering::Relaxed) < 65536
            && !finished.load(std::sync::atomic::Ordering::Acquire)
        {
            std::thread::yield_now();
        }
        flag.store(true, std::sync::atomic::Ordering::Release);
    });
    let cancelled_result = bundle::restore(
        &archive,
        &failed,
        &expected,
        s.store.as_ref().unwrap().limits(),
        &cancel,
        &progress,
    )
    .await;
    done.store(true, std::sync::atomic::Ordering::Release);
    canceller.join().unwrap();
    assert!(cancelled_result.is_err());
    assert!(!failed.exists());
    cancel.store(true, std::sync::atomic::Ordering::Release);
    assert!(
        bundle::restore(
            &archive,
            &failed,
            &expected,
            s.store.as_ref().unwrap().limits(),
            &cancel,
            &progress
        )
        .await
        .is_err()
    );
    assert!(!failed.exists());
    // A known legacy app schema accepts obsolete credential rows only to discard them.
    cancel.store(false, std::sync::atomic::Ordering::Relaxed);
    let legacy_db = s.root.join("legacy.db");
    fs::write(&legacy_db, &before[60..60 + dbsize]).unwrap();
    let mut db = SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&legacy_db))
        .await
        .unwrap();
    sqlx::raw_sql("CREATE TABLE auth_account (singleton INTEGER PRIMARY KEY CHECK(singleton=1), username_hash TEXT, password_hash TEXT, bootstrap_hash TEXT); CREATE TABLE auth_sessions (token_hash TEXT PRIMARY KEY, expires INTEGER NOT NULL, created INTEGER NOT NULL); CREATE TABLE auth_attempts (kind TEXT PRIMARY KEY, start INTEGER NOT NULL, count INTEGER NOT NULL); INSERT INTO auth_account VALUES(1,'synthetic-user','synthetic-hash',NULL); INSERT INTO auth_sessions VALUES('synthetic-session',100,0);").execute(&mut db).await.unwrap();
    db.close().await.unwrap();
    let db = fs::read(&legacy_db).unwrap();
    let mut altered = before[..60].to_vec();
    altered[20..28].copy_from_slice(&(db.len() as u64).to_le_bytes());
    altered[28..60].copy_from_slice(&Sha256::digest(&db));
    altered.extend_from_slice(&db);
    altered.extend_from_slice(&before[60 + dbsize..]);
    let legacy = s.root.join("legacy.bundle");
    fs::write(&legacy, altered).unwrap();
    let legacy_restore = s.root.join("legacy-restored");
    bundle::restore(
        &legacy,
        &legacy_restore,
        &expected,
        s.store.as_ref().unwrap().limits(),
        &cancel,
        &progress,
    )
    .await
    .unwrap();
    let mut restored_db = SqliteConnection::connect_with(
        &SqliteConnectOptions::new().filename(legacy_restore.join("store/catalog.db")),
    )
    .await
    .unwrap();
    let obsolete: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name LIKE 'auth_%'")
            .fetch_one(&mut restored_db)
            .await
            .unwrap();
    assert_eq!(obsolete, 0);
    restored_db.close().await.unwrap();
    // Exercise all four EXACT accepted v1 forms with an independently authored
    // SQLite payload, not the new logical encoder or an archive-import helper.
    for (has_order, has_auth) in [(true, false), (true, true), (false, false), (false, true)] {
        let fixture = s.root.join(format!("v1-{has_order}-{has_auth}.db"));
        fs::write(&fixture, &legacy_database).unwrap();
        let mut db =
            SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&fixture))
                .await
                .unwrap();
        if !has_order {
            sqlx::query("DROP TABLE application_job_order")
                .execute(&mut db)
                .await
                .unwrap();
        }
        if has_auth {
            sqlx::raw_sql("CREATE TABLE auth_account (singleton INTEGER PRIMARY KEY CHECK(singleton=1), username_hash TEXT, password_hash TEXT, bootstrap_hash TEXT); CREATE TABLE auth_sessions (token_hash TEXT PRIMARY KEY, expires INTEGER NOT NULL, created INTEGER NOT NULL); CREATE TABLE auth_attempts (kind TEXT PRIMARY KEY, start INTEGER NOT NULL, count INTEGER NOT NULL);").execute(&mut db).await.unwrap();
        }
        db.close().await.unwrap();
        let payload = fs::read(&fixture).unwrap();
        let mut wire = before[..60].to_vec();
        wire[20..28].copy_from_slice(&(payload.len() as u64).to_le_bytes());
        wire[28..60].copy_from_slice(&Sha256::digest(&payload));
        wire.extend_from_slice(&payload);
        wire.extend_from_slice(&before[60 + dbsize..]);
        let input = s.root.join(format!("v1-{has_order}-{has_auth}.bundle"));
        fs::write(&input, wire).unwrap();
        let target = s.root.join(format!("v1-{has_order}-{has_auth}-restored"));
        bundle::restore(
            &input,
            &target,
            &expected,
            store.limits(),
            &cancel,
            &progress,
        )
        .await
        .unwrap();
        let mut db = SqliteConnection::connect_with(
            &SqliteConnectOptions::new().filename(target.join("store/catalog.db")),
        )
        .await
        .unwrap();
        let auth_tables: i64 = sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name IN ('auth_account','auth_sessions','auth_attempts')").fetch_one(&mut db).await.unwrap();
        assert_eq!(auth_tables, 0);
        let copied: Vec<String> = sqlx::query_scalar(
            "SELECT hex(original) FROM catalog_records ORDER BY version,kind,ordinal",
        )
        .fetch_all(&mut db)
        .await
        .unwrap();
        let expected_records: Vec<String> = sqlx::query_scalar(
            "SELECT hex(original) FROM catalog_records ORDER BY version,kind,ordinal",
        )
        .fetch_all(c.pool())
        .await
        .unwrap();
        assert_eq!(copied, expected_records);
        db.close().await.unwrap();
    }
    // Actual HTTP archive and inbound streaming restore; obsolete authentication state never carried into restored DB.
    let r = s
        .request(
            "POST",
            "/exports",
            &mutation,
            br#"{"kind":"archive_originals","version":1}"#,
        )
        .await;
    let id = serde_json::from_slice::<Value>(&r.2).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(wait_export(&s, &id, &headers).await["state"], "succeeded");
    let r = s
        .request("GET", &format!("/exports/{id}/download"), &headers, b"")
        .await;
    assert_eq!(r.0, 200);
    assert_eq!(
        s.request(
            "POST",
            "/restores",
            "Origin: http://127.0.0.1:3000\r\n",
            b""
        )
        .await
        .0,
        403
    );
    assert_eq!(s.request("POST", "/restores", &headers, b"").await.0, 403);
    let r = s.request("POST", "/restores", &mutation, &r.2).await;
    assert_eq!(r.0, 200);
    let id = serde_json::from_slice::<Value>(&r.2).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(wait_export(&s, &id, &headers).await["state"], "succeeded");
    // The restore lands on the deployment-selected backend: a private SQLite
    // file, or an exact nonce-owned PostgreSQL schema named by restore.json.
    let restored_store = s
        .root
        .join("staging.exports")
        .join(format!("{id}.restored/store"));
    let descriptor: Value =
        serde_json::from_slice(&fs::read(restored_store.join("restore.json")).unwrap()).unwrap();
    let restored_schema = if s.postgres.is_some() {
        assert_eq!(descriptor["backend"], "postgres");
        assert!(!restored_store.join("catalog.db").exists());
        let schema = descriptor["schema"].as_str().unwrap().to_owned();
        assert!(schema.starts_with("lytronaut_restore_"));
        Some(schema)
    } else {
        assert_eq!(descriptor["backend"], "sqlite");
        assert!(restored_store.join("catalog.db").is_file());
        None
    };
    let restored_config = match &restored_schema {
        Some(schema) => s.database.clone().with_schema(schema).unwrap(),
        None => DatabaseConfig::Sqlite(restored_store.join("catalog.db")),
    };
    let restored = Catalog::open(restored_config).await.unwrap();
    for query in [
        "SELECT count(*) FROM catalog_records",
        "SELECT count(*) FROM catalog_pictures",
        "SELECT count(*) FROM intake_sources",
    ] {
        let original: i64 = sqlx::query_scalar(query)
            .fetch_one(catalog.pool())
            .await
            .unwrap();
        let copied: i64 = sqlx::query_scalar(query)
            .fetch_one(restored.pool())
            .await
            .unwrap();
        assert!(original > 0);
        assert_eq!(original, copied);
    }
    let originals = catalog.dialect(
        "SELECT lower(hex(original)) FROM catalog_records ORDER BY version,kind,ordinal",
        "SELECT encode(original,'hex') FROM catalog_records ORDER BY version,kind,ordinal",
    );
    let expected: Vec<String> = sqlx::query_scalar(originals)
        .fetch_all(catalog.pool())
        .await
        .unwrap();
    let copied: Vec<String> = sqlx::query_scalar(originals)
        .fetch_all(restored.pool())
        .await
        .unwrap();
    assert_eq!(expected, copied);
    restored.pool().close().await;
    // Removing a completed restore deletes only its owned copy and releases retained quota.
    let owned = s
        .root
        .join("staging.exports")
        .join(format!("{id}.restored"));
    let unrelated = s.root.join("staging.exports/unrelated-sentinel");
    fs::write(&unrelated, b"keep unrelated bytes").unwrap();
    let r = s
        .request("DELETE", &format!("/exports/{id}"), &mutation, b"")
        .await;
    assert_eq!(r.0, 200);
    assert_eq!(
        serde_json::from_slice::<Value>(&r.2).unwrap()["kind"],
        "restore"
    );
    assert!(!owned.exists());
    if let Some(schema) = &restored_schema {
        let remaining: i64 =
            sqlx::query_scalar("SELECT count(*) FROM pg_catalog.pg_namespace WHERE nspname=$1")
                .bind(schema)
                .fetch_one(catalog.pool())
                .await
                .unwrap();
        assert_eq!(remaining, 0, "DELETE must drop the owned restore schema");
    }
    assert_eq!(fs::read(&unrelated).unwrap(), b"keep unrelated bytes");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT retained FROM export_jobs WHERE id=$1")
            .bind(&id)
            .fetch_one(catalog.pool())
            .await
            .unwrap(),
        0
    );
    // Keep the upload body stalled; DELETE must terminate it without waiting for EOF.
    let port = s.port;
    let stalled_headers = mutation.clone();
    let prefix = before[..65536.min(before.len())].to_vec();
    let declared = before.len();
    let stalled=tokio::task::spawn_blocking(move||{let mut socket=TcpStream::connect(("127.0.0.1",port)).unwrap();let head=format!("POST /restores HTTP/1.1\r\nHost: 127.0.0.1:3000\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nContent-Length: {declared}\r\n{stalled_headers}\r\n");socket.write_all(head.as_bytes()).unwrap();socket.write_all(&prefix).unwrap();socket}).await.unwrap();
    let mut uploading = None;
    for _ in 0..100 {
        let queued = sqlx::query_scalar::<_, String>(
            "SELECT id FROM export_jobs WHERE kind='restore' AND state='queued'",
        )
        .fetch_all(catalog.pool())
        .await
        .unwrap();
        assert!(queued.len() <= 1, "only the stalled upload may be queued");
        uploading = queued.into_iter().next();
        if uploading.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let uploading = uploading.expect("streaming upload must create durable queued job");
    let start = std::time::Instant::now();
    assert_eq!(
        s.request("DELETE", &format!("/exports/{uploading}"), &mutation, b"")
            .await
            .0,
        200
    );
    assert_eq!(
        wait_export(&s, &uploading, &headers).await["state"],
        "cancelled"
    );
    assert!(start.elapsed() < std::time::Duration::from_secs(2));
    drop(stalled);
    assert!(
        !s.root
            .join("staging.exports")
            .join(format!("{uploading}.upload"))
            .exists()
    );
    assert_eq!(fs::read(&unrelated).unwrap(), b"keep unrelated bytes");
    // An actual disconnected upload cannot leave a runnable restore or partial store.
    let port = s.port;
    let headers = mutation.clone();
    let payload = before[..65536.min(before.len())].to_vec();
    let declared = before.len();
    let response=tokio::task::spawn_blocking(move||{let mut socket=TcpStream::connect(("127.0.0.1",port)).unwrap();socket.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();let head=format!("POST /restores HTTP/1.1\r\nHost: 127.0.0.1:3000\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nContent-Length: {declared}\r\n{headers}\r\n");socket.write_all(head.as_bytes()).unwrap();socket.write_all(&payload).unwrap();socket.shutdown(std::net::Shutdown::Write).unwrap();let mut reply=Vec::new();let _=socket.read_to_end(&mut reply);reply}).await.unwrap();
    assert!(!response.starts_with(b"HTTP/1.1 200"));
    let mut recorded = false;
    for _ in 0..100 {
        let failed:i64=sqlx::query_scalar("SELECT count(*) FROM export_jobs WHERE kind='restore' AND state='failed' AND error='upload'").fetch_one(catalog.pool()).await.unwrap();
        if failed > 0 {
            recorded = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        recorded,
        "interrupted restore upload must record a durable failed job"
    );
    // Close so a PostgreSQL run drops its disposable schema and re-checks the sentinel.
    s.close().await;
}

#[path = "support/live_focus.rs"]
mod live_focus;
use live_focus::{live_focus_fixture, live_focus_fixture_with};
fn live_zip_entries(bytes: &[u8]) -> std::collections::BTreeMap<String, Vec<u8>> {
    let mut out = std::collections::BTreeMap::new();
    let mut p = 0;
    while &bytes[p..p + 4] == b"PK\x03\x04" {
        let read32 = |i| u32::from_le_bytes(bytes[i..i + 4].try_into().unwrap());
        let read16 = |i| u16::from_le_bytes(bytes[i..i + 2].try_into().unwrap()) as usize;
        assert_eq!(read16(p + 8), 0);
        let size = read32(p + 18) as usize;
        assert_eq!(size, read32(p + 22) as usize);
        let name = read16(p + 26);
        let extra = read16(p + 28);
        let start = p + 30 + name + extra;
        let payload = &bytes[start..start + size];
        assert_eq!(crc32fast::hash(payload), read32(p + 14));
        let name = String::from_utf8(bytes[p + 30..p + 30 + name].to_vec()).unwrap();
        assert!(out.insert(name, payload.to_vec()).is_none());
        p = start + size;
    }
    assert_eq!(&bytes[p..p + 4], b"PK\x01\x02");
    assert_eq!(out.len(), 3);
    out
}
/// One imported synthetic JPEG focus stack. Runs on the configured backend
/// (`LYTRONAUT_TEST_BACKEND`), so every Live Photo case also runs on PostgreSQL.
async fn live_photo_server() -> (Server, String, String, String, Vec<u8>) {
    live_photo_server_with(live_focus_fixture()).await
}
async fn live_photo_server_with(source: Vec<u8>) -> (Server, String, String, String, Vec<u8>) {
    let s = Server::new().await;
    let (headers, mutation) = s.request_headers().await;
    let body =
        json!({"version":1,"kind":"directory","declared_bytes":4*1024*1024,"declared_files":32})
            .to_string();
    let response = s
        .request("POST", "/imports", &mutation, body.as_bytes())
        .await;
    assert_eq!(response.0, 201);
    let import = serde_json::from_slice::<Value>(&response.2).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let db = s.source(23).await;
    for (name, bytes) in [
        ("lytrolibrary.db", db),
        ("bin/synthetic-p1/stacklq.lfp", source.clone()),
    ] {
        let hex: String = name.bytes().map(|b| format!("{b:02x}")).collect();
        let headers =
            format!("{mutation}X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: {hex}\r\n");
        assert_eq!(
            s.request("PUT", &format!("/imports/{import}/file"), &headers, &bytes)
                .await
                .0,
            204
        );
    }
    assert_eq!(
        s.request("POST", &format!("/imports/{import}/finish"), &mutation, b"")
            .await
            .0,
        204
    );
    let library = s.get("/libraries", &headers).await["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let captures = s
        .get(
            &format!("/libraries/{library}/captures?sort=rating"),
            &headers,
        )
        .await;
    let capture = captures["items"][0]["id"].as_str().unwrap();
    let detail = s.get(&format!("/captures/{capture}"), &headers).await;
    let asset = detail["originals"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["role"] == "jpeg_focus_stack")
        .unwrap()["id"]
        .as_str()
        .unwrap();
    let asset = asset.to_owned();
    (s, headers, mutation, asset, source)
}
async fn live_photo_create(
    s: &Server,
    mutation: &str,
    asset: &str,
    plane: u32,
    orientation: u8,
) -> String {
    let request = json!({"kind":"live_photo","version":1,"asset":asset,"plane_index":plane,"orientation":orientation}).to_string();
    let response = s
        .request("POST", "/exports", mutation, request.as_bytes())
        .await;
    assert_eq!(response.0, 200, "{}", String::from_utf8_lossy(&response.2));
    serde_json::from_slice::<Value>(&response.2).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}
/// Quota held by a job row; DELETE must release it on both engines.
async fn live_photo_quota(s: &Server, id: &str) -> (i64, i64) {
    let row = sqlx::query(
        "SELECT CAST(retained AS BIGINT),CAST(reserved AS BIGINT) FROM export_jobs WHERE id=$1",
    )
    .bind(id)
    .fetch_one(s.catalog.as_ref().unwrap().pool())
    .await
    .unwrap();
    (row.try_get(0).unwrap(), row.try_get(1).unwrap())
}
/// No owned job directory or artifact for `id` remains anywhere under the root.
fn live_photo_files_removed(s: &Server, id: &str) -> bool {
    fn walk(path: &std::path::Path, id: &str) -> bool {
        fs::read_dir(path).unwrap().all(|entry| {
            let entry = entry.unwrap();
            !entry.file_name().to_string_lossy().contains(id)
                && (!entry.file_type().unwrap().is_dir() || walk(&entry.path(), id))
        })
    }
    walk(&s.root, id)
}
#[tokio::test]
async fn live_photo_create_download_complete_private_metadata_free_pair() {
    let (s, headers, mutation, asset, source) = live_photo_server().await;
    for orientation in 1..=8 {
        let id = live_photo_create(&s, &mutation, &asset, 1, orientation).await;
        let done = wait_export(&s, &id, &headers).await;
        assert_eq!(done["state"], "succeeded");
        assert_eq!(done["kind"], "live_photo");
        assert_eq!(
            live_photo_quota(&s, &id).await.1,
            0,
            "success releases the reservation"
        );
        let download = s
            .request("GET", &format!("/exports/{id}/download"), &headers, b"")
            .await;
        assert_eq!(download.0, 200);
        assert!(download.1.contains("application/zip"));
        let entries = live_zip_entries(&download.2);
        let jpg = &entries["focus.JPG"];
        let movie = &entries["focus.MOV"];
        let image = image::load_from_memory_with_format(jpg, image::ImageFormat::Jpeg)
            .unwrap()
            .into_rgb8();
        assert_eq!(
            (image.width(), image.height()),
            if orientation >= 5 { (16, 32) } else { (32, 16) }
        );
        assert!(image.pixels().all(|p| p[2] > 240 && p[0] < 10 && p[1] < 10));
        assert!(
            movie
                .windows(b"com.apple.quicktime.still-image-time".len())
                .any(|v| v == b"com.apple.quicktime.still-image-time")
        );
        let provenance: Value = serde_json::from_slice(&entries["provenance.json"]).unwrap();
        assert_eq!(provenance["key_seconds"], 1.5);
        assert_eq!(provenance["selected_stored_plane"], 1);
        assert_eq!(provenance["orientation"], orientation);
        assert_eq!(provenance["motion"], "generated_focus_animation");
        assert_eq!(provenance["camera_recorded_temporal_motion"], false);
        for forbidden in [
            b"invented-sensitive-note".as_slice(),
            b"invented-private-path",
            asset.as_bytes(),
        ] {
            assert!(!download.2.windows(forbidden.len()).any(|v| v == forbidden));
        }
    }
    assert_eq!(
        s.request("GET", &format!("/assets/{asset}/download"), &headers, b"")
            .await
            .2,
        source,
        "originals are never modified"
    );
    s.close().await;
}
#[tokio::test]
async fn live_photo_delete_succeeded_removes_pair_and_quota_across_restart() {
    let (mut s, headers, mutation, asset, _) = live_photo_server().await;
    let id = live_photo_create(&s, &mutation, &asset, 1, 1).await;
    assert_eq!(wait_export(&s, &id, &headers).await["state"], "succeeded");
    assert!(
        live_photo_quota(&s, &id).await.0 > 0,
        "a ready pair counts against staging"
    );
    assert_eq!(
        s.request("DELETE", &format!("/exports/{id}"), &mutation, b"")
            .await
            .0,
        200
    );
    assert_eq!(
        s.get(&format!("/exports/{id}"), &headers).await["state"],
        "cancelled"
    );
    assert_eq!(live_photo_quota(&s, &id).await, (0, 0));
    assert!(live_photo_files_removed(&s, &id));
    s.restart().await;
    assert_eq!(
        s.get(&format!("/exports/{id}"), &headers).await["state"],
        "cancelled"
    );
    assert_eq!(
        s.request("GET", &format!("/exports/{id}/download"), &headers, b"")
            .await
            .0,
        404
    );
    s.close().await;
}
#[tokio::test]
async fn live_photo_delete_after_create_never_offers_a_half_pair() {
    let (s, headers, mutation, asset, _) = live_photo_server().await;
    for _ in 0..4 {
        let id = live_photo_create(&s, &mutation, &asset, 1, 1).await;
        let early = s
            .request("GET", &format!("/exports/{id}/download"), &headers, b"")
            .await;
        if early.0 == 200 {
            assert_eq!(
                live_zip_entries(&early.2).len(),
                3,
                "only a complete pair is offered"
            );
        } else {
            assert_eq!(early.0, 404, "incomplete resources are unavailable");
        }
        assert_eq!(
            s.request("DELETE", &format!("/exports/{id}"), &mutation, b"")
                .await
                .0,
            200
        );
        // Whether the worker was running or had already finished, DELETE wins.
        assert_eq!(wait_export(&s, &id, &headers).await["state"], "cancelled");
        assert_eq!(
            s.request("GET", &format!("/exports/{id}/download"), &headers, b"")
                .await
                .0,
            404
        );
        for _ in 0..400 {
            if live_photo_quota(&s, &id).await == (0, 0) && live_photo_files_removed(&s, &id) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(live_photo_quota(&s, &id).await, (0, 0));
        assert!(live_photo_files_removed(&s, &id));
    }
    s.close().await;
}
#[tokio::test]
async fn live_photo_out_of_range_plane_fails_without_output_and_survives_restart() {
    let (mut s, headers, mutation, asset, source) = live_photo_server().await;
    let id = live_photo_create(&s, &mutation, &asset, 63, 1).await;
    assert_eq!(wait_export(&s, &id, &headers).await["state"], "failed");
    assert_eq!(
        s.request("GET", &format!("/exports/{id}/download"), &headers, b"")
            .await
            .0,
        404
    );
    assert_eq!(
        s.request("GET", &format!("/assets/{asset}/download"), &headers, b"")
            .await
            .2,
        source
    );
    s.restart().await;
    assert_eq!(
        s.request("GET", &format!("/exports/{id}/download"), &headers, b"")
            .await
            .0,
        404
    );
    assert_eq!(
        s.request("DELETE", &format!("/exports/{id}"), &mutation, b"")
            .await
            .0,
        200
    );
    assert_eq!(live_photo_quota(&s, &id).await, (0, 0));
    assert!(live_photo_files_removed(&s, &id));
    s.close().await;
}
/// lytronaut-0f6 item 2: a real missing stored plane (not an out-of-range index)
/// is admitted (the original exists) but rejected when the worker parses the
/// stack: no output is ever offered and the original stays byte-identical.
#[tokio::test]
async fn live_photo_missing_stored_plane_fails_in_the_worker_without_output() {
    let (s, headers, mutation, asset, source) =
        live_photo_server_with(live_focus_fixture_with(true)).await;
    let id = live_photo_create(&s, &mutation, &asset, 0, 1).await;
    let report = wait_export(&s, &id, &headers).await;
    assert_eq!(report["state"], "failed");
    assert_eq!(report["error"]["code"], "corrupt_asset");
    assert_eq!(
        s.request("GET", &format!("/exports/{id}/download"), &headers, b"")
            .await
            .0,
        404
    );
    assert_eq!(
        s.request("GET", &format!("/assets/{asset}/download"), &headers, b"")
            .await
            .2,
        source
    );
    // A non-stack original is refused at admission, before any job exists.
    let library = s.get("/libraries", &headers).await["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let files = s
        .get(&format!("/libraries/{library}/files"), &headers)
        .await;
    let catalog = files["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["id"].as_str() != Some(asset.as_str()))
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let request =
        json!({"kind":"live_photo","version":1,"asset":catalog,"plane_index":0,"orientation":1})
            .to_string();
    let refused = s
        .request("POST", "/exports", &mutation, request.as_bytes())
        .await;
    assert_eq!(refused.0, 422);
    assert_eq!(
        serde_json::from_slice::<Value>(&refused.2).unwrap()["code"],
        "unsupported_format"
    );
    s.close().await;
}
#[tokio::test]
async fn browse_queries_and_asset_requests_are_validated_before_storage() {
    let s = Server::new().await;
    let (headers, mutation) = s.request_headers().await;
    let (job, _) = s.upload(&mutation, 23).await;
    assert_eq!(
        s.request("POST", &format!("/imports/{job}/finish"), &mutation, b"")
            .await
            .0,
        204
    );
    let progress = s.get(&format!("/imports/{job}"), &headers).await;
    assert_eq!(progress["catalog_state"], "ready");
    let lib = progress["library"].as_str().unwrap().to_owned();
    // Form-encoded search: '+' is a space and %XX bytes are UTF-8.
    let found = s
        .get(
            &format!("/libraries/{lib}/captures?search=%E9%9B%AA+%F0%9F%8C%8C"),
            &headers,
        )
        .await;
    assert_eq!(found["items"].as_array().unwrap().len(), 1);
    assert_eq!(found["items"][0]["name"], "雪 🌌");
    let invalid = [
        "/libraries?search=x".to_owned(),
        "/libraries?sort=name".into(),
        format!("/libraries/{lib}/albums?sort=rating"),
        format!("/libraries/{lib}/files?search=x"),
        format!("/libraries/{lib}/files?sort=name"),
        format!("/libraries/{lib}/captures?search=%G1"),
        format!("/libraries/{lib}/captures?search=%E9"),
        format!("/libraries/{lib}/captures?search=%4"),
        "/libraries/%FF/albums".into(),
        "/assets/bad.identifier/manifest".into(),
    ];
    for path in &invalid {
        let r = s.request("GET", path, &headers, b"").await;
        assert_eq!(r.0, 400, "{path}");
        let body: Value = serde_json::from_slice(&r.2).unwrap();
        assert_eq!(body["code"], "invalid_input", "{path}");
    }
    // A well-formed identifier that names nothing is not found.
    assert_eq!(
        s.request("GET", "/assets/unknown-opaque-id/manifest", &headers, b"")
            .await
            .0,
        404
    );
    // Pick the first capture's preserved originals by role.
    let captures = s
        .get(&format!("/libraries/{lib}/captures?sort=name"), &headers)
        .await;
    let capture = captures["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["name"] == "雪 🌌")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let detail = s.get(&format!("/captures/{capture}"), &headers).await;
    let original = |role: &str| {
        detail["originals"]
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["role"] == role)
            .unwrap()["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let raw = original("raw");
    let thumbnail = original("thumbnail");
    // A raw original is not a focus stack and has no preview chunks to serve.
    assert_eq!(
        s.request("GET", &format!("/assets/{raw}/focus"), &headers, b"")
            .await
            .0,
        404
    );
    assert_eq!(
        s.request("GET", &format!("/assets/{raw}/chunks/0"), &headers, b"")
            .await
            .0,
        404
    );
    // Chunks are whole verified ranges: no client Range, numeric indices only.
    assert_eq!(
        s.request(
            "GET",
            &format!("/assets/{thumbnail}/chunks/0"),
            &format!("{headers}Range: bytes=0-1\r\n"),
            b""
        )
        .await
        .0,
        400
    );
    assert_eq!(
        s.request(
            "GET",
            &format!("/assets/{thumbnail}/chunks/first"),
            &headers,
            b""
        )
        .await
        .0,
        400
    );
    let chunk = s
        .request(
            "GET",
            &format!("/assets/{thumbnail}/chunks/0"),
            &headers,
            b"",
        )
        .await;
    assert_eq!(chunk.0, 200);
    assert!(lytronaut_format::decode_jpeg(&chunk.2, lytronaut_format::Limits::default()).is_ok());
    s.close().await;
}
