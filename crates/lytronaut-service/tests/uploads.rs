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
