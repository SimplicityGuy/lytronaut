use lytronaut_service::{
    AppState, SecurityConfig,
    auth::{AuthError, AuthStore},
    local,
};
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if !args.is_empty() {
        let data = std::env::var_os("LYTRONAUT_DATA").unwrap_or_else(|| "data".into());
        let report = if args.len() == 2 {
            lytronaut_service::diagnostics::run(
                args[0].to_str().unwrap_or(""),
                std::path::Path::new(&args[1]),
                std::path::Path::new(&data),
            )
            .await
            .ok()
        } else {
            None
        };
        if let Some(report) = report {
            println!("{}", serde_json::to_string(&report).unwrap());
        } else {
            println!(
                "{{\"version\":1,\"status\":\"failed\",\"code\":\"invalid_or_unavailable_input\"}}"
            );
            std::process::exit(1);
        }
        return;
    }
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
    let store = lytronaut_storage::Store::open(
        catalog.clone(),
        &directory.join("originals"),
        &directory.join("staging"),
        lytronaut_storage::Limits::from_environment().map_err(|_| AuthError)?,
    )
    .await
    .map_err(|_| AuthError)?;
    let api = lytronaut_service::api::Api::open(catalog.clone(), store.clone())
        .await
        .map_err(|_| AuthError)?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000")
        .await
        .map_err(|_| AuthError)?;
    eprintln!("Lytronaut listening on loopback port 3000");
    axum::serve(
        listener,
        lytronaut_service::router_with_routes(
            AppState {
                catalog,
                auth,
                security,
            },
            lytronaut_service::uploads::routes(store).merge(api.routes()),
        ),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await
    .map_err(|_| AuthError)?;
    Ok(())
}
