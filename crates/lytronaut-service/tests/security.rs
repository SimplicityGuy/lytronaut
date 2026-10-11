//! Real loopback HTTP security tests with independent synthetic credentials only.
use axum::{
    Extension, Router,
    routing::{get, post},
};
use lytronaut_service::{
    AppState, Authenticated, SecurityConfig,
    auth::{AuthError, AuthStore},
    local, router_with_routes,
};
use lytronaut_storage::Catalog;
use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
};

const BOOTSTRAP: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const PASSWORD: &str = "synthetic test password 42";
struct Reply {
    status: u16,
    headers: String,
    body: String,
}
impl Reply {
    fn cookie(&self) -> String {
        self.headers
            .lines()
            .find_map(|s| s.strip_prefix("set-cookie: "))
            .unwrap()
            .split(';')
            .next()
            .unwrap()
            .to_owned()
    }
}
struct Server {
    directory: PathBuf,
    catalog: Catalog,
    port: u16,
    task: tokio::task::JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
        let _ = fs::remove_dir_all(&self.directory);
    }
}
impl Server {
    async fn new(secure: bool) -> Self {
        let directory = std::env::temp_dir().join(format!(
            "lytronaut-auth-test-{}-{}",
            std::process::id(),
            unique()
        ));
        local::private_directory(&directory).unwrap();
        local::database_file(&directory.join("catalog.db")).unwrap();
        let catalog = Catalog::open_application_database(&directory.join("catalog.db"))
            .await
            .unwrap();
        let auth = AuthStore::open(catalog.pool().clone(), Some(BOOTSTRAP))
            .await
            .unwrap();
        let (port, task) = Self::serve(catalog.clone(), auth, secure).await;
        Self {
            directory,
            catalog,
            port,
            task,
        }
    }
    async fn serve(
        catalog: Catalog,
        auth: AuthStore,
        secure: bool,
    ) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let origin = if secure {
            "https://library.example"
        } else {
            "http://127.0.0.1:3000"
        };
        let state = AppState {
            catalog,
            auth,
            security: SecurityConfig::new(origin).unwrap(),
        };
        let product: Router<AppState> = Router::new()
            .route(
                "/api/assets/{id}",
                get(|_: Extension<Authenticated>| async { "synthetic protected pixels" }),
            )
            .route(
                "/api/uploads",
                post(|_: Extension<Authenticated>| async { "synthetic accepted" }),
            );
        let app = router_with_routes(state, product);
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (port, task)
    }
    async fn restart(&mut self) {
        self.task.abort();
        self.catalog.pool().close().await;
        self.catalog = Catalog::open_application_database(&self.directory.join("catalog.db"))
            .await
            .unwrap();
        assert!(
            !AuthStore::needs_bootstrap(self.catalog.pool())
                .await
                .unwrap()
        );
        let auth = AuthStore::open(self.catalog.pool().clone(), None)
            .await
            .unwrap();
        (self.port, self.task) = Self::serve(self.catalog.clone(), auth, false).await;
    }
    async fn request(&self, method: &str, path: &str, headers: &str, body: Value) -> Reply {
        let body = if body.is_null() {
            String::new()
        } else {
            body.to_string()
        };
        raw(self.port,format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:3000\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}\r\n{body}",body.len())).await
    }
    async fn setup(&self) -> Reply {
        self.request(
            "POST",
            "/auth/setup",
            "Origin: http://127.0.0.1:3000\r\nX-Lytronaut-Intent: setup\r\n",
            json!({"bootstrap":BOOTSTRAP,"username":"admin","password":PASSWORD}),
        )
        .await
    }
    async fn login(&self, username: &str, password: &str) -> Reply {
        self.request(
            "POST",
            "/auth/login",
            "Origin: http://127.0.0.1:3000\r\nX-Lytronaut-Intent: login\r\n",
            json!({"username":username,"password":password}),
        )
        .await
    }
}
fn unique() -> u128 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos()
        + u128::from(NEXT.fetch_add(1, Ordering::Relaxed))
}
async fn raw(port: u16, request: String) -> Reply {
    tokio::task::spawn_blocking(move || {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(20)))
            .unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap();
        Reply {
            status: head.split_whitespace().nth(1).unwrap().parse().unwrap(),
            headers: head.to_ascii_lowercase(),
            body: body.to_owned(),
        }
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn every_private_route_family_and_fallback_is_closed() {
    let server = Server::new(false).await;
    let health = server.request("GET", "/healthz", "", Value::Null).await;
    assert_eq!(health.status, 200);
    assert!(health.body.contains("\"ready\":true"));
    let mut expected = None;
    for path in [
        "/api/assets/capture",
        "/api/assets/capture?public=1",
        "/api/downloads/capture",
        "/api/imports",
        "/api/jobs/capture",
        "/api/uploads",
        "/assets/capture",
        "/downloads/capture",
        "/jobs",
        "/auth/session",
        "/healthz/",
        "/auth/setup/",
        "/api/assets/%2e%2e%2fprivate",
        "/unknown",
    ] {
        let response = server.request("GET", path, "", Value::Null).await;
        assert_eq!(response.status, 401, "{path}");
        assert!(response.headers.contains("cache-control: no-store"));
        if let Some(expected) = &expected {
            assert_eq!(&response.body, expected);
        } else {
            expected = Some(response.body.clone());
        }
        for forbidden in [
            "capture",
            "pixels",
            "private",
            "catalog",
            "password",
            "bootstrap",
        ] {
            assert!(!response.body.contains(forbidden));
        }
    }
    assert_eq!(
        server
            .request(
                "POST",
                "/api/uploads",
                "Origin: http://127.0.0.1:3000\r\n",
                json!({})
            )
            .await
            .status,
        401
    );
    assert_eq!(
        server
            .request("HEAD", "/healthz", "", Value::Null)
            .await
            .status,
        401
    );
}
#[tokio::test]
async fn setup_is_local_one_time_atomic_and_credentials_are_hashed() {
    let server = Server::new(false).await;
    let wrong = server
        .request(
            "POST",
            "/auth/setup",
            "Origin: http://127.0.0.1:3000\r\nX-Lytronaut-Intent: setup\r\n",
            json!({"bootstrap":"b".repeat(64),"username":"admin","password":PASSWORD}),
        )
        .await;
    assert_eq!(wrong.status, 401);
    let (a, b) = tokio::join!(server.setup(), server.setup());
    let mut statuses = [a.status, b.status];
    statuses.sort();
    assert_eq!(statuses, [204, 401]);
    assert_eq!(server.setup().await.status, 401);
    let row: (String, String, Option<String>) =
        sqlx::query_as("SELECT username_hash,password_hash,bootstrap_hash FROM auth_account")
            .fetch_one(server.catalog.pool())
            .await
            .unwrap();
    assert_ne!(row.0, "admin");
    assert!(row.1.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
    assert!(!row.1.contains(PASSWORD));
    assert!(row.2.is_none());
}
#[tokio::test]
async fn durable_session_csrf_logout_expiry_and_fixation() {
    let mut server = Server::new(false).await;
    assert_eq!(server.setup().await.status, 204);
    let missing = server.login("missing", PASSWORD).await;
    let wrong = server.login("admin", "wrong synthetic password").await;
    assert_eq!(missing.status, 401);
    assert_eq!(missing.body, wrong.body);
    let first = server.login("admin", PASSWORD).await;
    assert_eq!(first.status, 204);
    assert!(first.headers.contains("httponly"));
    assert!(first.headers.contains("samesite=strict"));
    assert!(first.headers.contains("path=/"));
    assert!(!first.headers.contains("; secure"));
    assert!(!first.headers.contains("domain="));
    let cookie = first.cookie();
    let cookie_header = format!("Cookie: {cookie}\r\n");
    let report = server
        .request("GET", "/auth/session", &cookie_header, Value::Null)
        .await;
    assert_eq!(report.status, 200);
    let csrf = serde_json::from_str::<Value>(&report.body).unwrap()["csrf"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        server
            .request("GET", "/api/assets/synthetic", &cookie_header, Value::Null)
            .await
            .status,
        200
    );
    assert_eq!(
        server
            .request("GET", "/api/assets/%2e%2e", &cookie_header, Value::Null)
            .await
            .status,
        400
    );
    assert_eq!(
        server
            .request("GET", "/api/assets/../secret", &cookie_header, Value::Null)
            .await
            .status,
        400
    );
    assert_eq!(
        server
            .request(
                "GET",
                "/auth/session",
                &format!("{cookie_header}{cookie_header}"),
                Value::Null
            )
            .await
            .status,
        401
    );
    let origin = format!("{cookie_header}Origin: http://127.0.0.1:3000\r\n");
    assert_eq!(
        server
            .request("POST", "/auth/logout", &origin, json!({}))
            .await
            .status,
        403
    );
    assert_eq!(
        server
            .request(
                "POST",
                "/api/uploads",
                &format!("{origin}X-Lytronaut-Csrf: wrong\r\n"),
                json!({})
            )
            .await
            .status,
        403
    );
    assert_eq!(
        server
            .request(
                "POST",
                "/api/uploads",
                &format!("{origin}X-Lytronaut-Csrf: {csrf}\r\n"),
                json!({})
            )
            .await
            .status,
        200
    );
    let stored: String = sqlx::query_scalar("SELECT token_hash FROM auth_sessions")
        .fetch_one(server.catalog.pool())
        .await
        .unwrap();
    assert!(!cookie.contains(&stored));
    server.restart().await;
    assert_eq!(
        server
            .request("GET", "/auth/session", &cookie_header, Value::Null)
            .await
            .status,
        200
    );
    assert_eq!(server.setup().await.status, 401);
    let second = server.login("admin", PASSWORD).await;
    assert_ne!(cookie, second.cookie());
    let logout = server
        .request(
            "POST",
            "/auth/logout",
            &format!("{origin}X-Lytronaut-Csrf: {csrf}\r\n"),
            json!({}),
        )
        .await;
    assert_eq!(logout.status, 204);
    assert!(logout.headers.contains("max-age=0"));
    assert_eq!(
        server
            .request("GET", "/auth/session", &cookie_header, Value::Null)
            .await
            .status,
        401
    );
    sqlx::query("UPDATE auth_sessions SET expires=0")
        .execute(server.catalog.pool())
        .await
        .unwrap();
    assert_eq!(
        server
            .request(
                "GET",
                "/auth/session",
                &format!("Cookie: {}\r\n", second.cookie()),
                Value::Null
            )
            .await
            .status,
        401
    );
}
#[tokio::test]
async fn credential_rate_limit_survives_restart_and_has_a_bounded_window() {
    let mut server = Server::new(false).await;
    assert_eq!(server.setup().await.status, 204);
    let (a, b, c, d, e, f) = tokio::join!(
        server.login("missing_a", "wrong"),
        server.login("missing_b", "wrong"),
        server.login("missing_c", "wrong"),
        server.login("missing_d", "wrong"),
        server.login("missing_e", "wrong"),
        server.login("missing_f", "wrong")
    );
    let mut statuses = [a.status, b.status, c.status, d.status, e.status, f.status];
    statuses.sort();
    assert_eq!(statuses, [401, 401, 401, 401, 401, 429]);
    let limited = server.login("admin", PASSWORD).await;
    assert_eq!(limited.status, 429);
    assert!(limited.headers.contains("retry-after: 60"));
    server.restart().await;
    assert_eq!(server.login("admin", PASSWORD).await.status, 429);
    sqlx::query("UPDATE auth_attempts SET start=0 WHERE kind='login'")
        .execute(server.catalog.pool())
        .await
        .unwrap();
    assert_eq!(server.login("admin", PASSWORD).await.status, 204);
}
#[tokio::test]
async fn origin_host_and_intent_policy_applies_before_setup_or_login() {
    let server = Server::new(false).await;
    for path in ["/auth/setup", "/auth/login", "/api/uploads"] {
        for headers in [
            "",
            "Origin: https://evil.example\r\n",
            "Origin: null\r\n",
            "Origin: http://127.0.0.1:3000\r\nOrigin: http://127.0.0.1:3000\r\n",
            "Origin: http://127.0.0.1:3000\r\nSec-Fetch-Site: cross-site\r\n",
        ] {
            assert_eq!(
                server
                    .request("POST", path, headers, json!({}))
                    .await
                    .status,
                403
            );
        }
    }
    assert_eq!(
        server
            .request(
                "POST",
                "/auth/login",
                "Origin: http://127.0.0.1:3000\r\n",
                json!({})
            )
            .await
            .status,
        403
    );
    let reply=raw(server.port,"GET /healthz HTTP/1.1\r\nHost: evil.example\r\nX-Forwarded-Host: 127.0.0.1:3000\r\nConnection: close\r\n\r\n".into()).await;
    assert_eq!(reply.status, 403);
    assert!(SecurityConfig::new("http://library.example").is_err());
    assert!(SecurityConfig::new("https://library.example/path").is_err());
    assert!(SecurityConfig::new("https://user@library.example").is_err());
    assert!(SecurityConfig::new("https://library.example/").is_err());
}
#[tokio::test]
async fn tls_cookie_requires_external_origin_and_host_and_ignores_forwarded_claims() {
    let server = Server::new(true).await;
    let request = |path: &str, intent: &str, body: Value| {
        let body = body.to_string();
        format!(
            "POST {path} HTTP/1.1\r\nHost: library.example\r\nOrigin: https://library.example\r\nX-Lytronaut-Intent: {intent}\r\nContent-Type: application/json\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    };
    assert_eq!(
        raw(
            server.port,
            request(
                "/auth/setup",
                "setup",
                json!({"bootstrap":BOOTSTRAP,"username":"admin","password":PASSWORD})
            )
        )
        .await
        .status,
        204
    );
    let login = raw(
        server.port,
        request(
            "/auth/login",
            "login",
            json!({"username":"admin","password":PASSWORD}),
        ),
    )
    .await;
    assert_eq!(login.status, 204);
    assert!(login.headers.contains("__host-lytronaut_session="));
    assert!(login.headers.contains("; secure"));
    assert!(!login.headers.contains("domain="));
    assert_eq!(
        server
            .request(
                "GET",
                "/healthz",
                "X-Forwarded-Host: library.example\r\nX-Forwarded-Proto: https\r\n",
                Value::Null
            )
            .await
            .status,
        403
    );
}
#[test]
fn local_bootstrap_restart_permissions_symlinks_and_partial_writes_fail_closed() {
    let directory = std::env::temp_dir().join(format!("lytronaut-bootstrap-test-{}", unique()));
    local::private_directory(&directory).unwrap();
    let path = directory.join("bootstrap.secret");
    let first = local::bootstrap_file(&path).unwrap();
    assert_eq!(first, local::bootstrap_file(&path).unwrap());
    assert_eq!(first.len(), 64);
    assert_eq!(AuthError.to_string(), "authentication unavailable");
    assert_eq!(format!("{AuthError:?}"), "AuthError");
    #[cfg(unix)]
    {
        use std::os::unix::fs::{PermissionsExt, symlink};
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(local::bootstrap_file(&path).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, "").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(local::bootstrap_file(&path).is_err());
        fs::remove_file(&path).unwrap();
        let target = directory.join("target");
        fs::write(&target, BOOTSTRAP).unwrap();
        symlink(&target, &path).unwrap();
        assert!(local::bootstrap_file(&path).is_err());
        assert!(local::database_file(&path).is_err());
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(local::private_directory(&directory).is_err());
    }
    fs::remove_dir_all(directory).unwrap();
}
#[tokio::test]
async fn pending_bootstrap_must_match_on_restart() {
    let server = Server::new(false).await;
    assert!(
        AuthStore::open(server.catalog.pool().clone(), None)
            .await
            .is_err()
    );
    assert!(
        AuthStore::open(server.catalog.pool().clone(), Some(&"b".repeat(64)))
            .await
            .is_err()
    );
    assert!(
        AuthStore::open(server.catalog.pool().clone(), Some(BOOTSTRAP))
            .await
            .is_ok()
    );
}
#[tokio::test]
async fn fixed_generated_client_assets_are_public_without_exposing_product_routes() {
    use lytronaut_service::{ClientAssets, router_with_client_assets};
    let mut server = Server::new(false).await;
    server.task.abort();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    server.port = listener.local_addr().unwrap().port();
    let state = AppState {
        catalog: server.catalog.clone(),
        auth: AuthStore::open(server.catalog.pool().clone(), Some(BOOTSTRAP))
            .await
            .unwrap(),
        security: SecurityConfig::new("http://127.0.0.1:3000").unwrap(),
    };
    let product = Router::new()
        .route(
            "/client/user-upload",
            get(|| async { "private uploaded pixels" }),
        )
        .route("/api/assets/{id}", get(|| async { "private pixels" }));
    let assets = ClientAssets {
        html: "synthetic shell".into(),
        javascript: "synthetic glue".into(),
        wasm: "synthetic compiled bytes".into(),
        css: "synthetic css".into(),
    };
    let app = router_with_client_assets(state, product, assets);
    server.task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    for path in [
        "/",
        "/client/lytronaut_client.js",
        "/client/lytronaut_client_bg.wasm",
        "/client/style.css",
    ] {
        let reply = server.request("GET", path, "", Value::Null).await;
        assert_eq!(reply.status, 200);
        assert!(reply.body.contains("synthetic"));
        assert_eq!(
            server.request("HEAD", path, "", Value::Null).await.status,
            200
        );
        assert_eq!(
            server
                .request("POST", path, "Origin: http://127.0.0.1:3000\r\n", json!({}))
                .await
                .status,
            401
        );
    }
    for path in [
        "/client/user-upload",
        "/client/lytronaut_client.js/extra",
        "/client/%6cytronaut_client.js",
        "/client/../api/assets/test",
        "/api/assets/test?public=1",
    ] {
        assert_eq!(
            server.request("GET", path, "", Value::Null).await.status,
            401
        );
    }
}
