//! Lawfully authored schema-23 records and encoder-generated synthetic pixels only.
use lytronaut_service::{
    AppState, SecurityConfig, api::Api, auth::AuthStore, router_with_routes, uploads,
};
use lytronaut_storage::{Catalog, Limits, Store};
use serde_json::{Value, json};
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    os::unix::fs::{DirBuilderExt, PermissionsExt},
    path::PathBuf,
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
    port: u16,
    task: Option<tokio::task::JoinHandle<()>>,
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
        let mut s = Self {
            root,
            store: None,
            port: 0,
            task: None,
        };
        s.start().await;
        s
    }
    async fn start(&mut self) {
        let catalog = Catalog::open_application_database(&self.root.join("catalog.db"))
            .await
            .unwrap();
        let auth = AuthStore::open(catalog.pool().clone(), Some(&"a".repeat(64)))
            .await
            .unwrap();
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
                reserved_bytes: 256 * 1024 * 1024,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let api = Api::open(catalog.clone(), store.clone()).await.unwrap();
        let app = router_with_routes(
            AppState {
                catalog,
                auth,
                security: SecurityConfig::new("http://127.0.0.1:3000").unwrap(),
            },
            uploads::routes(store.clone()).merge(api.routes()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        self.port = listener.local_addr().unwrap().port();
        self.task = Some(tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap()
        }));
        self.store = Some(store);
    }
    async fn restart(&mut self) {
        let t = self.task.take().unwrap();
        t.abort();
        let _ = t.await;
        self.store.take();
        self.start().await;
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
    async fn get(&self, path: &str, cookie: &str) -> Value {
        let r = self.request("GET", path, cookie, b"").await;
        assert_eq!(r.0, 200, "{path}: {}", String::from_utf8_lossy(&r.2));
        serde_json::from_slice(&r.2).unwrap()
    }
    async fn session(&self) -> (String, String) {
        let origin = "Origin: http://127.0.0.1:3000\r\n";
        let setup=json!({"bootstrap":"a".repeat(64),"username":"api","password":"synthetic api password"}).to_string();
        assert_eq!(
            self.request(
                "POST",
                "/auth/setup",
                &format!("{origin}X-Lytronaut-Intent: setup\r\n"),
                setup.as_bytes()
            )
            .await
            .0,
            204
        );
        let login = json!({"username":"api","password":"synthetic api password"}).to_string();
        let r = self
            .request(
                "POST",
                "/auth/login",
                &format!("{origin}X-Lytronaut-Intent: login\r\n"),
                login.as_bytes(),
            )
            .await;
        assert_eq!(r.0, 204);
        let cookie =
            r.1.lines()
                .find_map(|line| line.strip_prefix("set-cookie: "))
                .unwrap()
                .split(';')
                .next()
                .unwrap();
        let cookie = format!("Cookie: {cookie}\r\n");
        let csrf = self.get("/auth/session", &cookie).await["csrf"]
            .as_str()
            .unwrap()
            .to_owned();
        let mutation = format!("{cookie}{origin}X-Lytronaut-Csrf: {csrf}\r\n");
        (cookie, mutation)
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
async fn authenticated_upload_catalog_preview_original_restart_and_recovery() {
    let mut s = Server::new().await;
    let (cookie, mutation) = s.session().await;
    let (job, jpeg) = s.upload(&mutation, 23).await;
    let progress = s.get(&format!("/imports/{job}"), &cookie).await;
    assert_eq!(progress["phase"], "intake");
    assert!(progress["completed_bytes"].as_u64().unwrap() > 0);
    // Interrupt after intake completion, before adaptation. Startup must recover the durable job.
    s.store.as_ref().unwrap().finish(&job).await.unwrap();
    let progress = s.get(&format!("/imports/{job}"), &cookie).await;
    assert_eq!(progress["phase"], "catalog_pending");
    s.restart().await;
    let progress = s.get(&format!("/imports/{job}"), &cookie).await;
    assert_eq!(progress["phase"], "catalog_complete");
    assert_eq!(progress["catalog_state"], "ready");
    let lib = progress["library"].as_str().unwrap();
    let libraries = s.get("/libraries", &cookie).await;
    assert_eq!(libraries["items"].as_array().unwrap().len(), 1);
    assert_eq!(libraries["items"][0]["captures"], 2);
    let albums = s.get(&format!("/libraries/{lib}/albums"), &cookie).await;
    assert_eq!(albums["items"][0]["name"], "Album β");
    assert_eq!(albums["items"][1]["sort_order"], 8);
    let album = albums["items"][1]["id"].as_str().unwrap();
    let first = s
        .get(&format!("/albums/{album}/captures?limit=1"), &cookie)
        .await;
    let second = s
        .get(
            &format!("/albums/{album}/captures?limit=1&offset=1"),
            &cookie,
        )
        .await;
    assert_eq!(first["items"][0]["name"], "Second");
    assert_eq!(first["items"][0]["picture_index"], 0);
    assert_eq!(second["items"][0]["rating"], 4);
    assert_eq!(first["next_offset"], 1);
    assert!(second["next_offset"].is_null());
    let id = second["items"][0]["id"].as_str().unwrap();
    let detail = s.get(&format!("/captures/{id}"), &cookie).await;
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
    let stack = s.get(&format!("/assets/{stack_id}/focus"), &cookie).await;
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
                &cookie,
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
                &cookie,
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
        401
    );
    assert_eq!(
        s.request("GET", &format!("/assets/{stack_id}/focus/0/1"), "", b"")
            .await
            .0,
        401
    );
    let original_stack = s
        .request("GET", &format!("/assets/{stack_id}/download"), &cookie, b"")
        .await;
    assert_eq!(original_stack.2, synthetic_focus(&jpeg));
    let preview = detail["preview"].as_str().unwrap();
    let manifest = s.get(&format!("/assets/{preview}/manifest"), &cookie).await;
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
        .request("GET", &format!("/assets/{preview}/chunks/0"), &cookie, b"")
        .await;
    assert_eq!(r.0, 200);
    assert_eq!(r.2, jpeg);
    assert!(r.1.contains("content-length:"));
    for path in [
        format!("/assets/{preview}/manifest"),
        format!("/assets/{preview}/chunks/0"),
        format!("/assets/{preview}/download"),
    ] {
        assert_eq!(s.request("GET", &path, "", b"").await.0, 401);
    }
    assert_eq!(
        s.request("GET", &format!("/assets/{preview}/chunks/99"), &cookie, b"")
            .await
            .0,
        404
    );
    assert_eq!(
        s.request(
            "GET",
            &format!("/assets/{preview}/chunks/nope"),
            &cookie,
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
            &format!("{cookie}Range: bytes=0-9\r\n"),
            b""
        )
        .await
        .0,
        400
    );
    let bad_id = first["items"][0]["id"].as_str().unwrap();
    let bad = s.get(&format!("/captures/{bad_id}"), &cookie).await;
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
            .request("GET", &format!("/assets/{fid}/download"), &cookie, b"")
            .await;
        assert_eq!(r.0, 200);
        assert_eq!(r.2.len() as u64, file["encoded_bytes"].as_u64().unwrap());
    }
    let searched = s
        .get(
            &format!("/libraries/{lib}/captures?search=caf%C3%A9&sort=rating"),
            &cookie,
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
                &cookie,
                b"",
            )
            .await;
        assert_eq!(r.0, 400);
        assert!(!String::from_utf8_lossy(&r.2).contains("private-source-marker"));
    }
    let before = s.get(&format!("/albums/{album}/captures"), &cookie).await;
    s.restart().await;
    assert_eq!(
        before,
        s.get(&format!("/albums/{album}/captures"), &cookie).await
    );
}
#[tokio::test]
async fn finish_adapts_unknown_schema_preserved_download_and_cancel_consistency() {
    let s = Server::new().await;
    let (cookie, mutation) = s.session().await;
    let (job, _) = s.upload(&mutation, 24).await;
    assert_eq!(
        s.request("POST", &format!("/imports/{job}/finish"), &cookie, b"")
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
    let p = s.get(&format!("/imports/{job}"), &cookie).await;
    assert_eq!(p["catalog_state"], "preserved_only");
    let lib = p["library"].as_str().unwrap();
    assert_eq!(
        s.request("GET", &format!("/libraries/{lib}/captures"), &cookie, b"")
            .await
            .0,
        409
    );
    let files = s.get(&format!("/libraries/{lib}/files"), &cookie).await;
    assert_eq!(files["items"].as_array().unwrap().len(), 9);
    let mut empty = 0;
    for f in files["items"].as_array().unwrap() {
        let id = f["id"].as_str().unwrap();
        let r = s
            .request("GET", &format!("/assets/{id}/download"), &cookie, b"")
            .await;
        assert_eq!(r.0, 200);
        assert_eq!(r.2.len() as u64, f["encoded_bytes"].as_u64().unwrap());
        if r.2.is_empty() {
            empty += 1;
            assert_eq!(
                s.request("GET", &format!("/assets/{id}/manifest"), &cookie, b"")
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
    let p = s.get(&format!("/imports/{id}"), &cookie).await;
    assert_eq!(p["phase"], "cancelled");
    assert!(p["library"].is_null());
    assert_eq!(
        s.get("/libraries", &cookie).await["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
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
    let (cookie, mutation) = s.session().await;
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
    let p = s.get(&format!("/imports/{id}"), &cookie).await;
    assert_eq!(p["phase"], "catalog_complete");
    assert_eq!(p["catalog_state"], "preserved_only");
    assert_eq!(
        s.request("POST", &format!("/imports/{id}/finish"), &mutation, b"")
            .await
            .0,
        204
    );
}

#[tokio::test]
async fn polling_during_catalog_publication_keeps_completed_import_visible() {
    let s = Server::new().await;
    let (cookie, mutation) = s.session().await;
    for iteration in 0..6 {
        let (job, _) = s.upload(&mutation, 24 + iteration).await;
        let finish_path = format!("/imports/{job}/finish");
        let finish = s.request("POST", &finish_path, &mutation, b"");
        let path = format!("/imports/{job}");
        let polling = async {
            for _ in 0..40 {
                let (a, b, c) = tokio::join!(
                    s.request("GET", &path, &cookie, b""),
                    s.request("GET", &path, &cookie, b""),
                    s.request("GET", &path, &cookie, b"")
                );
                for response in [a, b, c] {
                    assert_eq!(response.0, 200);
                }
            }
        };
        let (response, ()) = tokio::join!(finish, polling);
        assert_eq!(response.0, 204);
        let progress = s.get(&path, &cookie).await;
        assert_eq!(progress["phase"], "catalog_complete");
        assert_eq!(progress["catalog_state"], "preserved_only");
    }
}

async fn wait_export(s: &Server, id: &str, cookie: &str) -> Value {
    for _ in 0..400 {
        let r = s.get(&format!("/exports/{id}"), cookie).await;
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
async fn authenticated_flatten_png_jpeg_exact_pixels_provenance_cancel_and_restart() {
    use sha2::{Digest, Sha256};
    let mut s = Server::new().await;
    let (cookie, mutation) = s.session().await;
    let (import, jpeg) = s.upload(&mutation, 23).await;
    assert_eq!(
        s.request("POST", &format!("/imports/{import}/finish"), &mutation, b"")
            .await
            .0,
        204
    );
    let library = s.get("/libraries", &cookie).await["items"][0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let captures = s
        .get(
            &format!("/libraries/{library}/captures?sort=rating"),
            &cookie,
        )
        .await;
    let capture = captures["items"][0]["id"].as_str().unwrap();
    let detail = s.get(&format!("/captures/{capture}"), &cookie).await;
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
            401
        );
        assert_eq!(
            s.request("POST", "/exports", &cookie, request.as_bytes())
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
        assert_eq!(wait_export(&s, &id, &cookie).await["state"], "succeeded");
        let recipe = s.get(&format!("/exports/{id}/recipe"), &cookie).await;
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
            .request("GET", &format!("/exports/{id}/download"), &cookie, b"")
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
            s.request("GET", &format!("/assets/{focus}/download"), &cookie, b"")
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
    assert_eq!(wait_export(&s, &id, &cookie).await["state"], "succeeded");
    let r = s
        .request("GET", &format!("/exports/{id}/download"), &cookie, b"")
        .await;
    assert_eq!(r.0, 200);
    assert!(r.1.contains("image/jpeg"));
    let recipe = s.get(&format!("/exports/{id}/recipe"), &cookie).await;
    assert_eq!(recipe["source_variant"], "verified_static_jpeg");
    assert!(
        recipe["processing_loss"]
            .as_str()
            .unwrap()
            .contains("Lossy")
    );
    s.restart().await;
    assert_eq!(
        s.get(&format!("/exports/{id}"), &cookie).await["state"],
        "succeeded"
    );
    assert_eq!(
        s.request("DELETE", &format!("/exports/{id}"), &mutation, b"")
            .await
            .0,
        200
    );
    assert_eq!(
        s.get(&format!("/exports/{id}"), &cookie).await["state"],
        "cancelled"
    );
    assert_eq!(
        s.request("GET", &format!("/exports/{id}/download"), &cookie, b"")
            .await
            .0,
        404
    );
    assert_eq!(
        s.request("GET", &format!("/assets/{preview}/download"), &cookie, b"")
            .await
            .2,
        jpeg
    );
}
#[tokio::test]
async fn public_lossless_bundle_unknown_bytes_catalog_restore_tamper_conflict_and_interruption() {
    use lytronaut_service::bundle;
    use std::sync::atomic::{AtomicBool, AtomicU64};
    let s = Server::new().await;
    let (cookie, mutation) = s.session().await;
    let (import, _) = s.upload(&mutation, 23).await;
    assert_eq!(
        s.request("POST", &format!("/imports/{import}/finish"), &mutation, b"")
            .await
            .0,
        204
    );
    let catalog = Catalog::open_application_database(&s.root.join("catalog.db"))
        .await
        .unwrap();
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
        let query = format!("SELECT * FROM {table} ORDER BY rowid");
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
    for query in [
        "SELECT hex(original) FROM catalog_records ORDER BY version,kind,ordinal",
        "SELECT quote(uuid)||quote(name)||quote(caption)||quote(rating)||quote(flag_status) FROM catalog_pictures ORDER BY version,uuid",
        "SELECT quote(uuid)||quote(name)||quote(sort_order) FROM catalog_albums ORDER BY version,uuid",
        "SELECT quote(album_uuid)||quote(picture_uuid)||quote(picture_index) FROM catalog_memberships ORDER BY version,album_uuid,picture_index",
        "SELECT quote(id)||quote(kind)||quote(source_key) FROM api_ids ORDER BY id",
    ] {
        let original: Vec<String> = sqlx::query_scalar(query)
            .fetch_all(catalog.pool())
            .await
            .unwrap();
        let restored_values: Vec<String> =
            sqlx::query_scalar(query).fetch_all(c.pool()).await.unwrap();
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
    for table in [
        "auth_account",
        "auth_sessions",
        "auth_attempts",
        "export_jobs",
    ] {
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
    // Recomputed checksums do not authorize executable or inconsistent app databases.
    use sha2::{Digest, Sha256};
    let dbsize = u64::from_le_bytes(before[20..28].try_into().unwrap()) as usize;
    for sql in [
        "CREATE TRIGGER hostile AFTER INSERT ON catalog_albums BEGIN DELETE FROM catalog_pictures; END",
        "CREATE VIEW hostile AS SELECT * FROM auth_account",
        "CREATE TABLE hostile(x INTEGER,y INTEGER GENERATED ALWAYS AS (x+1) STORED)",
        "INSERT INTO auth_account VALUES(1,'restored-user','restored-password',NULL)",
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
    // Actual HTTP archive and inbound streaming restore; sessions never carried into restored DB.
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
    assert_eq!(wait_export(&s, &id, &cookie).await["state"], "succeeded");
    let r = s
        .request("GET", &format!("/exports/{id}/download"), &cookie, b"")
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
        401
    );
    assert_eq!(s.request("POST", "/restores", &cookie, b"").await.0, 403);
    let r = s.request("POST", "/restores", &mutation, &r.2).await;
    assert_eq!(r.0, 200);
    let id = serde_json::from_slice::<Value>(&r.2).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(wait_export(&s, &id, &cookie).await["state"], "succeeded");
    assert!(
        s.root
            .join("staging.exports")
            .join(format!("{id}.restored/store/catalog.db"))
            .is_file()
    );
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
    assert_eq!(fs::read(&unrelated).unwrap(), b"keep unrelated bytes");
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT retained FROM export_jobs WHERE id=?")
            .bind(&id)
            .fetch_one(catalog.pool())
            .await
            .unwrap(),
        0
    );
    // Keep the upload body stalled; DELETE must terminate it without waiting for EOF.
    let port = s.port;
    let headers = mutation.clone();
    let prefix = before[..65536.min(before.len())].to_vec();
    let declared = before.len();
    let stalled=tokio::task::spawn_blocking(move||{let mut socket=TcpStream::connect(("127.0.0.1",port)).unwrap();let head=format!("POST /restores HTTP/1.1\r\nHost: 127.0.0.1:3000\r\nConnection: close\r\nContent-Type: application/octet-stream\r\nContent-Length: {declared}\r\n{headers}\r\n");socket.write_all(head.as_bytes()).unwrap();socket.write_all(&prefix).unwrap();socket}).await.unwrap();
    let mut uploading = None;
    for _ in 0..100 {
        uploading=sqlx::query_scalar::<_,String>("SELECT id FROM export_jobs WHERE kind='restore' AND state='queued' ORDER BY rowid DESC LIMIT 1").fetch_optional(catalog.pool()).await.unwrap();
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
        wait_export(&s, &uploading, &cookie).await["state"],
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
    for _ in 0..100 {
        let failed:i64=sqlx::query_scalar("SELECT count(*) FROM export_jobs WHERE kind='restore' AND state='failed' AND error='upload'").fetch_one(catalog.pool()).await.unwrap();
        if failed > 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    panic!("interrupted restore upload must record a durable failed job");
}
