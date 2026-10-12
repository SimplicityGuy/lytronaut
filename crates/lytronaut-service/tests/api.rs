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
                reserved_bytes: 16 * 1024 * 1024,
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
            socket.read_to_end(&mut reply).unwrap();
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
CREATE TABLE event(uuid TEXT,library_uuid TEXT);INSERT INTO event VALUES('event','synthetic-library-uuid');CREATE TABLE image_bin(uuid TEXT);INSERT INTO image_bin VALUES('bin');CREATE TABLE picture_list(uuid TEXT,filter INTEGER,sort_order INTEGER);INSERT INTO picture_list VALUES('list',0,0);CREATE TABLE imported_picture_history(hash TEXT);").execute(&mut c).await.unwrap();
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
    assert_eq!(bad["capabilities"]["discrete_focus"]["state"], "disabled");
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
    assert_eq!(files["items"].as_array().unwrap().len(), 7);
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
