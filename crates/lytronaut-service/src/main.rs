use lytronaut_service::{
    AppState, SecurityConfig,
    auth::{AuthError, AuthStore},
    local,
};
#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!(
            "Lytronaut startup or service failed; check local configuration and application state"
        );
        std::process::exit(1);
    }
}
async fn run() -> Result<(), AuthError> {
    let directory = std::env::var_os("LYTRONAUT_DATA").unwrap_or_else(|| "data".into());
    let directory = std::path::Path::new(&directory);
    local::private_directory(directory)?;
    let database = directory.join("catalog.db");
    local::database_file(&database)?;
    let bootstrap_path = directory.join("bootstrap.secret");
    let catalog = lytronaut_storage::Catalog::open_application_database(&database)
        .await
        .map_err(|_| AuthError)?;
    let token = if AuthStore::needs_bootstrap(catalog.pool()).await? {
        Some(local::bootstrap_file(&bootstrap_path)?)
    } else {
        None
    };
    let auth = AuthStore::open(catalog.pool().clone(), token.as_deref()).await?;
    if !AuthStore::initialized(catalog.pool()).await? {
        eprintln!(
            "Local setup required; use the protected bootstrap.secret file in the application data directory"
        );
    }
    let origin =
        std::env::var("LYTRONAUT_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:3000".to_owned());
    let security = SecurityConfig::new(&origin)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .map_err(|_| AuthError)?;
    eprintln!("Lytronaut listening on loopback port 3000");
    axum::serve(
        listener,
        lytronaut_service::router(AppState {
            catalog,
            auth,
            security,
        }),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    .map_err(|_| AuthError)?;
    Ok(())
}
