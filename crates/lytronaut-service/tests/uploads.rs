//! Real HTTP streaming/authentication tests using only independent synthetic input.
use lytronaut_service::{AppState, SecurityConfig, auth::AuthStore, router_with_routes, uploads};
use lytronaut_storage::{Catalog, Limits, Store};
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
};
struct Server {
    root: PathBuf,
    store: Store,
    port: u16,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
        let _ = fs::remove_dir_all(&self.root);
    }
}
impl Server {
    async fn new() -> Self {
        let mut unique = [0; 16];
        getrandom::fill(&mut unique).unwrap();
        let root = std::env::temp_dir().join(format!(
            "lytronaut-upload-test-{:x}",
            u128::from_le_bytes(unique)
        ));
        fs::create_dir(&root).unwrap();
        let catalog = Catalog::open_application_database(&root.join("catalog.db"))
            .await
            .unwrap();
        let auth = AuthStore::open(catalog.pool().clone(), Some(&"a".repeat(64)))
            .await
            .unwrap();
        let store = Store::open(
            catalog.clone(),
            &root.join("originals"),
            &root.join("staging"),
            Limits {
                upload_bytes: 16384,
                file_bytes: 8192,
                encoded_bytes: 16384,
                reserved_bytes: 131072,
                free_floor: 0,
                files: 4,
                metadata_bytes: 4096,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        let app = router_with_routes(
            AppState {
                catalog,
                auth,
                security: SecurityConfig::new("http://127.0.0.1:3000").unwrap(),
            },
            uploads::routes(store.clone()),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            root,
            store,
            port,
            task,
        }
    }
    async fn request(
        &self,
        method: &str,
        path: &str,
        headers: &str,
        body: &[u8],
        chunked: bool,
    ) -> (u16, String, String) {
        let port = self.port;
        let framing = if chunked {
            "Transfer-Encoding: chunked\r\n".to_string()
        } else {
            format!("Content-Length: {}\r\n", body.len())
        };
        let mut request=format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:3000\r\nConnection: close\r\nContent-Type: application/json\r\n{framing}{headers}\r\n").into_bytes();
        if chunked {
            for chunk in body.chunks(731) {
                request.extend(format!("{:x}\r\n", chunk.len()).as_bytes());
                request.extend(chunk);
                request.extend(b"\r\n");
            }
            request.extend(b"0\r\n\r\n");
        } else {
            request.extend(body);
        }
        tokio::task::spawn_blocking(move || {
            let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
            socket
                .set_read_timeout(Some(std::time::Duration::from_secs(20)))
                .unwrap();
            socket.write_all(&request).unwrap();
            let mut reply = String::new();
            socket.read_to_string(&mut reply).unwrap();
            let (head, body) = reply.split_once("\r\n\r\n").unwrap();
            (
                head.split_whitespace().nth(1).unwrap().parse().unwrap(),
                head.into(),
                body.into(),
            )
        })
        .await
        .unwrap()
    }
    async fn session(&self) -> (String, String) {
        let setup=json!({"bootstrap":"a".repeat(64),"username":"synthetic","password":"synthetic password for uploads"}).to_string();
        assert_eq!(
            self.request(
                "POST",
                "/auth/setup",
                "Origin: http://127.0.0.1:3000\r\nX-Lytronaut-Intent: setup\r\n",
                setup.as_bytes(),
                false
            )
            .await
            .0,
            204
        );
        let login =
            json!({"username":"synthetic","password":"synthetic password for uploads"}).to_string();
        let reply = self
            .request(
                "POST",
                "/auth/login",
                "Origin: http://127.0.0.1:3000\r\nX-Lytronaut-Intent: login\r\n",
                login.as_bytes(),
                false,
            )
            .await;
        assert_eq!(reply.0, 204);
        let cookie = reply
            .1
            .lines()
            .find_map(|s| s.strip_prefix("set-cookie: "))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        let reply = self
            .request(
                "GET",
                "/auth/session",
                &format!("Cookie: {cookie}\r\n"),
                b"",
                false,
            )
            .await;
        let csrf = serde_json::from_str::<Value>(&reply.2).unwrap()["csrf"]
            .as_str()
            .unwrap()
            .to_owned();
        (
            format!("Cookie: {cookie}\r\n"),
            format!(
                "Cookie: {cookie}\r\nOrigin: http://127.0.0.1:3000\r\nX-Lytronaut-Csrf: {csrf}\r\n"
            ),
        )
    }
    async fn begin(&self, headers: &str, bytes: u64) -> String {
        let request =
            json!({"version":1,"kind":"directory","declared_bytes":bytes,"declared_files":4})
                .to_string();
        let r = self
            .request("POST", "/imports", headers, request.as_bytes(), false)
            .await;
        assert_eq!(r.0, 201, "{}", r.2);
        serde_json::from_str::<Value>(&r.2).unwrap()["id"]
            .as_str()
            .unwrap()
            .into()
    }
}
fn hex(path: &str) -> String {
    path.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
}
#[tokio::test]
async fn real_streaming_csrf_unicode_and_truthful_progress() {
    let s = Server::new().await;
    for path in [
        "/imports",
        "/imports/synthetic/file",
        "/imports/synthetic/zip",
        "/imports/synthetic/cancel",
    ] {
        let r = s
            .request(
                "PUT",
                path,
                "Origin: http://127.0.0.1:3000\r\n",
                b"private synthetic content",
                false,
            )
            .await;
        assert_eq!(r.0, 401);
        assert!(!r.2.contains("synthetic"));
    }
    let (cookie, mutation) = s.session().await;
    let id = s.begin(&mutation, 16384).await;
    let path = format!("/imports/{id}/file");
    let name = "library/未知.data";
    let header = format!(
        "{mutation}X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: {}\r\n",
        hex(name)
    );
    let csrf_missing = format!(
        "{cookie}Origin: http://127.0.0.1:3000\r\nX-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: {}\r\n",
        hex(name)
    );
    assert_eq!(
        s.request("PUT", &path, &csrf_missing, b"x", true).await.0,
        403
    );
    assert_eq!(
        s.request("PUT", &path, &header, &vec![b'x'; 6000], true)
            .await
            .0,
        204
    );
    let report = s
        .request("GET", &format!("/imports/{id}"), &cookie, b"", false)
        .await;
    let report: Value = serde_json::from_str(&report.2).unwrap();
    assert_eq!(report["completed_bytes"], 6000);
    assert_eq!(report["state"], "running");
    assert!(!report.to_string().contains(name));
    assert_eq!(
        s.request(
            "POST",
            &format!("/imports/{id}/finish"),
            &mutation,
            b"",
            false
        )
        .await
        .0,
        204
    );
    let m = s.store.manifest(&id).await.unwrap();
    assert_eq!(m[0].logical_path, name);
    let report = s.store.job(&id).await.unwrap();
    assert_eq!(report.completed_bytes, report.total_bytes);
    assert_eq!(report.total_bytes, 6000);
}
#[tokio::test]
async fn actual_stream_overflow_content_encoding_and_unsafe_name_are_rejected() {
    let s = Server::new().await;
    let (_, mutation) = s.session().await;
    for (name, extra, body, expected) in [
        ("safe", "", vec![b'x'; 8193], 413),
        ("../unsafe", "", vec![b'x'; 1], 400),
        ("safe", "Content-Encoding: gzip\r\n", vec![b'x'; 1], 400),
    ] {
        let id = s.begin(&mutation, 16384).await;
        let headers = format!(
            "{mutation}{extra}X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: {}\r\n",
            hex(name)
        );
        assert_eq!(
            s.request("PUT", &format!("/imports/{id}/file"), &headers, &body, true)
                .await
                .0,
            expected
        );
        assert!(s.store.manifest(&id).await.is_err());
        assert!(!s.store.job(&id).await.unwrap().error.unwrap().retryable);
    }
}
