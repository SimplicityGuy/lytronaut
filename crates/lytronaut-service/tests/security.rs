//! Actual credentialless HTTP boundary, synthetic state only.
use axum::{
    Router,
    routing::{get, post},
};
use lytronaut_service::{AppState, ClientAssets, SecurityConfig, local, router_with_client_assets};
use lytronaut_storage::Catalog;
use std::{
    fs,
    io::{Read, Write},
    net::TcpStream,
};

async fn request(port: u16, method: &str, path: &str, headers: &str) -> (u16, String) {
    let message = format!(
        "{method} {path} HTTP/1.1\r\nConnection: close\r\nContent-Length: 0\r\n{headers}\r\n"
    );
    tokio::task::spawn_blocking(move || {
        let mut socket = TcpStream::connect(("127.0.0.1", port)).unwrap();
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(20)))
            .unwrap();
        socket
            .set_write_timeout(Some(std::time::Duration::from_secs(20)))
            .unwrap();
        socket.write_all(message.as_bytes()).unwrap();
        let mut reply = String::new();
        socket.read_to_string(&mut reply).unwrap();
        (
            reply.split_whitespace().nth(1).unwrap().parse().unwrap(),
            reply,
        )
    })
    .await
    .unwrap()
}
#[tokio::test]
async fn direct_entry_and_mutations_require_explicit_same_origin_intent() {
    let catalog = Catalog::open_application_database(std::path::Path::new(":memory:"))
        .await
        .unwrap();
    let application = Router::new()
        .route("/libraries", get(|| async { "synthetic library" }))
        .route("/imports", post(|| async { "synthetic accepted" }));
    let app = router_with_client_assets(
        AppState {
            catalog: catalog.clone(),
            security: SecurityConfig::new("http://127.0.0.1:3000").unwrap(),
        },
        application,
        ClientAssets {
            html: "synthetic app".into(),
            javascript: "synthetic glue".into(),
            wasm: "synthetic wasm".into(),
            css: "synthetic css".into(),
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let host = "Host: 127.0.0.1:3000\r\n";
    for path in [
        "/",
        "/libraries",
        "/client/lytronaut_client.js",
        "/client/lytronaut_client_bg.wasm",
        "/client/style.css",
        "/healthz",
    ] {
        let (status, reply) = request(port, "GET", path, host).await;
        assert_eq!(status, 200, "{path}");
        assert!(reply.contains("cache-control: no-store"));
        assert!(reply.contains("x-content-type-options: nosniff"));
        assert!(reply.contains("referrer-policy: no-referrer"));
        assert!(!reply.contains("set-cookie"));
    }
    let mutation =
        format!("{host}Origin: http://127.0.0.1:3000\r\nX-Lytronaut-Intent: mutation\r\n");
    assert_eq!(request(port, "POST", "/imports", &mutation).await.0, 200);
    for headers in [host.to_owned(), format!("{host}Origin: http://127.0.0.1:3000\r\n"),
        "Host: attacker.invalid\r\nOrigin: http://127.0.0.1:3000\r\nX-Lytronaut-Intent: mutation\r\n".into(),
        format!("{host}Origin: https://foreign.invalid\r\nX-Lytronaut-Intent: mutation\r\n"),
        format!("{mutation}Sec-Fetch-Site: cross-site\r\n"),
        format!("{mutation}Origin: http://127.0.0.1:3000\r\n"),
        format!("{mutation}Host: attacker.invalid\r\n"),
        format!("{host}X-Forwarded-User: admin\r\nX-Forwarded-Host: 127.0.0.1:3000\r\n"),
    ] { assert_eq!(request(port, "POST", "/imports", &headers).await.0, 403); }
    for path in [
        "/auth/session",
        "/auth/login",
        "/auth/setup",
        "/auth/logout",
        "/missing",
    ] {
        assert_eq!(request(port, "GET", path, host).await.0, 404);
        assert_eq!(request(port, "POST", path, &mutation).await.0, 404);
    }
    for path in ["/bad%2fpath", "/bad\\path"] {
        assert_eq!(request(port, "GET", path, host).await.0, 400);
    }
    let long = format!("/libraries?q={}", "x".repeat(1025));
    assert_eq!(request(port, "GET", &long, host).await.0, 400);
    catalog.pool().close().await;
    assert_eq!(request(port, "GET", "/healthz", host).await.0, 503);
    task.abort();
}
#[test]
fn origin_and_private_storage_configuration_fail_closed() {
    for origin in [
        "",
        "http://example.test",
        "ftp://127.0.0.1",
        "https://user@example.test",
        "https://example.test/",
        "https://example.test/path",
        "https://example.test?q=a",
    ] {
        assert!(SecurityConfig::new(origin).is_err(), "{origin}");
    }
    for origin in [
        "http://127.0.0.1:3000",
        "http://localhost:3000",
        "http://[::1]:3000",
        "https://library.example.test",
    ] {
        assert!(SecurityConfig::new(origin).is_ok());
    }
    let root = std::env::temp_dir().join(format!("lytronaut-private-test-{}", std::process::id()));
    local::private_directory(&root).unwrap();
    let database = root.join("app.db");
    local::database_file(&database).unwrap();
    local::database_file(&database).unwrap();
    assert!(!root.join("bootstrap.secret").exists());
    local::remove_obsolete_bootstrap(&root).unwrap();
    fs::write(root.join("bootstrap.secret"), "obsolete synthetic secret").unwrap();
    local::remove_obsolete_bootstrap(&root).unwrap();
    assert!(!root.join("bootstrap.secret").exists());
    std::os::unix::fs::symlink(&database, root.join("bootstrap.secret")).unwrap();
    local::remove_obsolete_bootstrap(&root).unwrap();
    assert!(database.exists());
    fs::create_dir(root.join("bootstrap.secret")).unwrap();
    assert!(local::remove_obsolete_bootstrap(&root).is_err());
    fs::remove_dir(root.join("bootstrap.secret")).unwrap();
    use std::os::unix::fs::{PermissionsExt, symlink};
    fs::set_permissions(&database, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(local::database_file(&database).is_err());
    let link = root.join("link");
    symlink(&database, &link).unwrap();
    assert!(local::database_file(&link).is_err());
    fs::set_permissions(&root, fs::Permissions::from_mode(0o755)).unwrap();
    assert!(local::private_directory(&root).is_err());
    fs::remove_dir_all(root).unwrap();
}
#[tokio::test]
async fn existing_application_catalog_discards_obsolete_credentials_only() {
    let root =
        std::env::temp_dir().join(format!("lytronaut-auth-migration-{}", std::process::id()));
    local::private_directory(&root).unwrap();
    let database = root.join("app.db");
    let catalog = Catalog::open_application_database(&database).await.unwrap();
    sqlx::raw_sql("CREATE TABLE auth_account(secret TEXT); INSERT INTO auth_account VALUES('synthetic'); CREATE TABLE auth_sessions(secret TEXT); CREATE TABLE auth_attempts(count INTEGER); CREATE TABLE retained(payload BLOB); INSERT INTO retained VALUES(X'012345');").execute(catalog.pool()).await.unwrap();
    catalog.pool().close().await;
    let catalog = Catalog::open_application_database(&database).await.unwrap();
    let obsolete: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name LIKE 'auth_%'")
            .fetch_one(catalog.pool())
            .await
            .unwrap();
    assert_eq!(obsolete, 0);
    let payload: Vec<u8> = sqlx::query_scalar("SELECT payload FROM retained")
        .fetch_one(catalog.pool())
        .await
        .unwrap();
    assert_eq!(payload, [1, 35, 69]);
    catalog.pool().close().await;
    fs::remove_dir_all(root).unwrap();
}
