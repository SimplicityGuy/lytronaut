//! Real HTTP streaming/request-boundary tests using only independent synthetic input.
//! `LYTRONAUT_TEST_BACKEND=postgres` (set only by the strict PostgreSQL recipe) runs
//! the same cases on an owned schema in the explicit disposable test database.
use lytronaut_service::{AppState, SecurityConfig, router_with_routes, uploads};
use lytronaut_storage::{Catalog, DatabaseConfig, Limits, Store};
use serde_json::{Value, json};
use sqlx::{
    ConnectOptions, Connection, PgConnection, any::AnyConnectOptions, postgres::PgConnectOptions,
};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
    str::FromStr,
};
struct Server {
    root: PathBuf,
    store: Store,
    catalog: Catalog,
    postgres: Option<(PgConnection, String)>,
    port: u16,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
        let _ = fs::remove_dir_all(&self.root);
    }
}
async fn postgres_admin() -> PgConnection {
    let url = std::env::var("LYTRONAUT_TEST_POSTGRES_URL")
        .expect("PostgreSQL upload tests require the explicit disposable test URL");
    let options = PgConnectOptions::from_str(&url)
        .expect("invalid test PostgreSQL configuration")
        .disable_statement_logging();
    PgConnection::connect_with(&options)
        .await
        .expect("test PostgreSQL connection failed")
}
/// The restricted role, untouched public sentinel and owned-schema-only objects.
async fn assert_postgres_boundary(admin: &mut PgConnection, schema: &str) {
    let restricted: bool = sqlx::query_scalar("SELECT NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication) AND NOT has_schema_privilege(current_user,'public','CREATE') FROM pg_roles WHERE rolname=current_user")
        .fetch_one(&mut *admin).await.unwrap();
    assert!(restricted, "upload parity requires a restricted role");
    let intact: bool = sqlx::query_scalar("SELECT count(*)=1 AND min(marker)='synthetic-persistence-parity' FROM public.lytronaut_parity_sentinel")
        .fetch_one(&mut *admin).await.unwrap();
    assert!(intact, "public sentinel must remain unchanged");
    let leaked: i64 = sqlx::query_scalar("SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='public' AND c.relname IN ('intake_jobs','application_job_order','catalog_versions','export_jobs')")
        .fetch_one(&mut *admin).await.unwrap();
    assert_eq!(
        leaked, 0,
        "application objects must remain in the owned schema"
    );
    assert!(schema.starts_with("lytronaut_upload_"));
}
impl Server {
    async fn new() -> Self {
        Self::with_idle_timeout(std::time::Duration::from_secs(1800)).await
    }
    async fn with_idle_timeout(idle: std::time::Duration) -> Self {
        let mut unique = [0; 16];
        getrandom::fill(&mut unique).unwrap();
        let root = std::env::temp_dir().join(format!(
            "lytronaut-upload-test-{:x}",
            u128::from_le_bytes(unique)
        ));
        fs::create_dir(&root).unwrap();
        let mut postgres = None;
        let catalog = match std::env::var("LYTRONAUT_TEST_BACKEND").as_deref() {
            Err(std::env::VarError::NotPresent) | Ok("sqlite") => {
                Catalog::open_application_database(&root.join("catalog.db"))
                    .await
                    .unwrap()
            }
            Ok("postgres") => {
                let mut admin = postgres_admin().await;
                let schema = format!("lytronaut_upload_{:x}", u128::from_le_bytes(unique));
                sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA \"{schema}\"")))
                    .execute(&mut admin)
                    .await
                    .unwrap();
                assert_postgres_boundary(&mut admin, &schema).await;
                let url = std::env::var("LYTRONAUT_TEST_POSTGRES_URL").unwrap();
                let config = DatabaseConfig::Postgres(
                    AnyConnectOptions::from_str(&url)
                        .expect("invalid test database URL")
                        .database_url
                        .to_string(),
                )
                .with_schema(&schema)
                .unwrap();
                let catalog = Catalog::open(config).await.unwrap();
                assert!(!root.join("catalog.db").exists());
                postgres = Some((admin, schema));
                catalog
            }
            _ => panic!("LYTRONAUT_TEST_BACKEND must be sqlite or postgres"),
        };
        let store = Store::open(
            catalog.clone(),
            &root.join("originals"),
            &root.join("staging"),
            Limits {
                import_idle_timeout: idle,
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
                catalog: catalog.clone(),
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
            catalog,
            postgres,
            port,
            task,
        }
    }
    /// Stop serving, close the pool, re-check the PostgreSQL boundary and remove
    /// only this test's own schema. A failed assertion may leave the schema in the
    /// disposable database; it never touches anything else.
    async fn close(mut self) {
        self.task.abort();
        self.catalog.pool().close().await;
        if let Some((mut admin, schema)) = self.postgres.take() {
            assert_postgres_boundary(&mut admin, &schema).await;
            let owned: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM pg_tables WHERE schemaname=$1 AND tablename='intake_jobs'",
            )
            .bind(&schema)
            .fetch_one(&mut admin)
            .await
            .unwrap();
            assert_eq!(owned, 1, "upload state must persist in the owned schema");
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
    async fn request_headers(&self) -> (String, String) {
        (
            String::new(),
            "Origin: http://127.0.0.1:3000\r\nX-Lytronaut-Intent: mutation\r\n".into(),
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
async fn real_streaming_intent_unicode_and_truthful_progress() {
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
        assert_eq!(r.0, 403);
        assert!(!r.2.contains("synthetic"));
    }
    let (headers, mutation) = s.request_headers().await;
    let id = s.begin(&mutation, 16384).await;
    let path = format!("/imports/{id}/file");
    let name = "library/未知.data";
    let header = format!(
        "{mutation}X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: {}\r\n",
        hex(name)
    );
    let intent_missing = format!(
        "{headers}Origin: http://127.0.0.1:3000\r\nX-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: {}\r\n",
        hex(name)
    );
    assert_eq!(
        s.request("PUT", &path, &intent_missing, b"x", true).await.0,
        403
    );
    assert_eq!(
        s.request("PUT", &path, &header, &vec![b'x'; 6000], true)
            .await
            .0,
        204
    );
    let report = s
        .request("GET", &format!("/imports/{id}"), &headers, b"", false)
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
    s.close().await;
}
#[tokio::test]
async fn actual_stream_overflow_content_encoding_and_unsafe_name_are_rejected() {
    let s = Server::new().await;
    let (_, mutation) = s.request_headers().await;
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
    s.close().await;
}
#[tokio::test]
async fn malformed_and_overlarge_declarations_never_echo_private_input() {
    let s = Server::new().await;
    let (_, mutation) = s.request_headers().await;
    let marker = "synthetic-sensitive-source-name";
    let unknown=json!({"version":1,"kind":"directory","declared_bytes":1,"declared_files":1,marker:"private"}).to_string();
    for body in [
        unknown,
        format!("{{\"{marker}\":\"{}\"}}", "x".repeat(5000)),
    ] {
        let response = s
            .request("POST", "/imports", &mutation, body.as_bytes(), false)
            .await;
        assert_eq!(response.0, 400);
        let value: Value = serde_json::from_str(&response.2).unwrap();
        assert_eq!(value["code"], "invalid_input");
        assert!(!response.2.contains(marker));
        assert!(!response.2.contains("private"));
    }
    s.close().await;
}
#[tokio::test]
async fn upload_framing_cancel_and_retry_routes_fail_closed_over_http() {
    let s = Server::new().await;
    let (_, mutation) = s.request_headers().await;
    let put = |id: &str, extra: &str| (format!("/imports/{id}/file"), format!("{mutation}{extra}"));
    // Header framing that can never name a safe logical file.
    for extra in [
        String::new(),
        "X-Lytronaut-Entry-Type: directory\r\n".into(),
        "X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: 61\r\n".into(),
        "X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: 6\r\n".into(),
        "X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: 6A\r\n".into(),
        "X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: zz\r\n".into(),
        "X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: ff\r\n".into(),
        format!("X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: {}\r\n", "61".repeat(5000)),
    ] {
        let id = s.begin(&mutation, 16).await;
        let (path, headers) = put(&id, &extra);
        let r = s.request("PUT", &path, &headers, b"x", false).await;
        assert_eq!(r.0, 400, "{extra}");
        assert_eq!(serde_json::from_str::<Value>(&r.2).unwrap()["code"], "invalid_input");
        let progress = s.request("GET", &format!("/imports/{id}"), &mutation, b"", false).await;
        assert_eq!(serde_json::from_str::<Value>(&progress.2).unwrap()["phase"], "failed");
    }
    // A declared Content-Length above the per-file limit is refused before streaming.
    let id = s.begin(&mutation, 16384).await;
    let (path, headers) = put(
        &id,
        &format!(
            "X-Lytronaut-Entry-Type: file\r\nX-Lytronaut-Path-Hex: {}\r\n",
            hex("big")
        ),
    );
    assert_eq!(
        s.request("PUT", &path, &headers, &vec![b'x'; 8193], false)
            .await
            .0,
        413
    );
    // Non-ZIP bytes on the ZIP route fail the job without publishing anything.
    let request = json!({"version":1,"kind":"zip_library","declared_bytes":64,"declared_files":1})
        .to_string();
    let r = s
        .request("POST", "/imports", &mutation, request.as_bytes(), false)
        .await;
    assert_eq!(r.0, 201, "{}", r.2);
    let zip = serde_json::from_str::<Value>(&r.2).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        s.request(
            "PUT",
            &format!("/imports/{zip}/zip"),
            &mutation,
            b"not a zip archive",
            false
        )
        .await
        .0,
        413
    );
    assert!(s.store.manifest(&zip).await.is_err());

    // Cancel is idempotent; a cancelled import can be retried as a new import.
    let id = s.begin(&mutation, 16).await;
    for _ in 0..2 {
        assert_eq!(
            s.request(
                "POST",
                &format!("/imports/{id}/cancel"),
                &mutation,
                b"",
                false
            )
            .await
            .0,
            204
        );
    }
    let progress = s
        .request("GET", &format!("/imports/{id}"), &mutation, b"", false)
        .await;
    assert_eq!(
        serde_json::from_str::<Value>(&progress.2).unwrap()["phase"],
        "cancelled"
    );
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
        409
    );
    let retried = s
        .request(
            "POST",
            &format!("/imports/{id}/retry"),
            &mutation,
            b"",
            false,
        )
        .await;
    assert_eq!(retried.0, 201, "{}", retried.2);
    let new_id = serde_json::from_str::<Value>(&retried.2).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(new_id, id);
    // A running import cannot be retried; finishing one without files is invalid.
    assert_eq!(
        s.request(
            "POST",
            &format!("/imports/{new_id}/retry"),
            &mutation,
            b"",
            false
        )
        .await
        .0,
        409
    );
    assert_eq!(
        s.request(
            "POST",
            &format!("/imports/{new_id}/finish"),
            &mutation,
            b"",
            false
        )
        .await
        .0,
        400
    );
    // An interrupted (retryable) failure is reported and retryable.
    s.store.fail(&new_id).await.unwrap();
    let progress = s
        .request("GET", &format!("/imports/{new_id}"), &mutation, b"", false)
        .await;
    let progress: Value = serde_json::from_str(&progress.2).unwrap();
    assert_eq!(progress["phase"], "failed");
    assert_eq!(
        s.request(
            "POST",
            &format!("/imports/{new_id}/retry"),
            &mutation,
            b"",
            false
        )
        .await
        .0,
        201
    );

    // Malformed or unknown identifiers on every route, without echoing input.
    for (method, route) in [
        ("GET", "/imports/%FF"),
        ("GET", "/imports/not-an-import"),
        ("PUT", "/imports/%FF/file"),
        ("PUT", "/imports/%FF/zip"),
        ("POST", "/imports/%FF/finish"),
        ("POST", "/imports/%FF/cancel"),
        ("POST", "/imports/%FF/retry"),
        ("POST", "/imports/not-an-import/retry"),
    ] {
        let r = s.request(method, route, &mutation, b"", false).await;
        assert_eq!(r.0, 400, "{method} {route}");
        assert!(!r.2.contains("not-an-import"));
    }
    s.close().await;
}

/// Runs unchanged on SQLite and the strict PostgreSQL HTTP-upload parity group.
#[tokio::test]
async fn abandoned_imports_expire_without_poll_heartbeats_and_release_owned_capacity() {
    use lytronaut_contracts::{ImportKind, ImportRequest, JobState, WireVersion};
    use sqlx::Row;
    use std::time::Duration;
    let server = Server::with_idle_timeout(Duration::from_secs(2)).await;
    let request = ImportRequest {
        version: WireVersion::V1,
        kind: ImportKind::Directory,
        declared_bytes: 1024,
        declared_files: 4,
    };
    let first = server.store.begin(&request).await.unwrap();
    let second = server.store.begin(&request).await.unwrap();
    assert_eq!(
        server.store.begin(&request).await.unwrap_err(),
        lytronaut_storage::StorageError::Busy
    );
    let preserved = b"synthetic sealed bytes survive an abandoned import";
    server
        .store
        .append_directory_file(&second, "public-sealed.txt", &preserved[..])
        .await
        .unwrap();
    let source = sqlx::query("SELECT digest,length FROM intake_sources WHERE job=$1")
        .bind(&second)
        .fetch_one(server.catalog.pool())
        .await
        .unwrap();
    let entry = lytronaut_storage::SourceEntry {
        logical_path: "public-sealed.txt".into(),
        digest: source.get("digest"),
        length: source.get::<i64, _>("length") as u64,
    };
    let mut spool = server.store.spool(&second).await.unwrap();
    spool.write(b"independent abandoned upload").await.unwrap();
    // A different job's scratch file must never be removed by prefix cleanup.
    let untouched = server.root.join("staging/unrelated-synthetic-scratch");
    fs::write(&untouched, b"leave untouched").unwrap();
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            // Merely polling status is deliberately not upload progress.
            if server.store.job(&first).await.unwrap().state == JobState::Cancelled
                && server.store.job(&second).await.unwrap().state == JobState::Cancelled
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("idle import slots must expire without a service restart");
    assert_eq!(
        spool.write(b"too late").await.unwrap_err(),
        lytronaut_storage::StorageError::Cancelled
    );
    drop(spool);
    let row = sqlx::query(
        "SELECT CAST(sum(reserved) AS BIGINT) AS reserved FROM intake_jobs WHERE id=$1 OR id=$2",
    )
    .bind(&first)
    .bind(&second)
    .fetch_one(server.catalog.pool())
    .await
    .unwrap();
    assert_eq!(row.get::<i64, _>("reserved"), 0);
    let files = fs::read_dir(server.root.join("staging"))
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        files.len(),
        2,
        "only store.lock and unrelated scratch survive"
    );
    assert!(files.contains(&"store.lock".into()));
    assert_eq!(fs::read(&untouched).unwrap(), b"leave untouched");
    let mut original = Vec::new();
    server
        .store
        .open_blob(&entry)
        .unwrap()
        .read_to_end(&mut original)
        .unwrap();
    assert_eq!(
        original, preserved,
        "expiry never deletes or changes sealed originals"
    );
    // Both the slot and the reservation can be reused immediately.
    let replacement = server.store.begin(&request).await.unwrap();
    server.store.cancel(&replacement).await.unwrap();
    server.close().await;
}
