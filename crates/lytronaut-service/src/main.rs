#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::var_os("LYTRONAUT_CATALOG").unwrap_or_else(|| "lytronaut.db".into());
    let catalog =
        lytronaut_storage::Catalog::open_application_database(std::path::Path::new(&path)).await?;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
    eprintln!("Lytronaut foundation listening on 127.0.0.1:3000");
    axum::serve(
        listener,
        lytronaut_service::router(lytronaut_service::AppState { catalog }),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
