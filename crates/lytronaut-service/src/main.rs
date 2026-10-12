use lytronaut_service::{AppState, SecurityConfig, ServiceError, local};
use std::future::IntoFuture;
#[tokio::main]
async fn main() {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args == [std::ffi::OsString::from("--healthcheck")] {
        std::process::exit(if healthcheck() { 0 } else { 1 });
    }
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
async fn run() -> Result<(), ServiceError> {
    let directory = std::env::var_os("LYTRONAUT_DATA").unwrap_or_else(|| "data".into());
    let directory = std::path::Path::new(&directory);
    local::private_directory(directory)?;
    local::remove_obsolete_bootstrap(directory)?;
    let catalog = lytronaut_service::database::open(directory)
        .await
        .map_err(|error| {
            eprintln!("Lytronaut: {error}");
            ServiceError
        })?;
    let origin =
        std::env::var("LYTRONAUT_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:3000".to_owned());
    let security = SecurityConfig::new(&origin)?;
    let store = lytronaut_storage::Store::open(
        catalog.clone(),
        &std::env::var_os("LYTRONAUT_ORIGINALS")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| directory.join("originals")),
        &std::env::var_os("LYTRONAUT_STAGING")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| directory.join("staging")),
        lytronaut_storage::Limits::from_environment().map_err(|_| ServiceError)?,
    )
    .await
    .map_err(|_| ServiceError)?;
    let api = lytronaut_service::api::Api::open(catalog.clone(), store.clone())
        .await
        .map_err(|_| ServiceError)?;
    let listen = std::env::var("LYTRONAUT_LISTEN").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let address = listen
        .parse::<std::net::SocketAddr>()
        .map_err(|_| ServiceError)?;
    let listener = tokio::net::TcpListener::bind(address)
        .await
        .map_err(|_| ServiceError)?;
    let client_dir = std::env::var_os("LYTRONAUT_CLIENT").unwrap_or_else(|| "dist".into());
    let client_dir = std::path::Path::new(&client_dir);
    let javascript =
        std::fs::read(client_dir.join("lytronaut_client.js")).map_err(|_| ServiceError)?;
    let wasm =
        std::fs::read(client_dir.join("lytronaut_client_bg.wasm")).map_err(|_| ServiceError)?;
    let client = lytronaut_service::ClientAssets {
        html: include_bytes!("../../../assets/index.html")
            .as_slice()
            .into(),
        css: include_bytes!("../../../assets/style.css")
            .as_slice()
            .into(),
        javascript: javascript.into(),
        wasm: wasm.into(),
    };
    eprintln!("Lytronaut listening with Rust browser client");
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let store_shutdown = store.clone();
    let mut serving = tokio::spawn(
        axum::serve(
            listener,
            lytronaut_service::router_with_client_assets(
                AppState {
                    catalog: catalog.clone(),
                    security,
                },
                lytronaut_service::uploads::routes(store).merge(api.clone().routes()),
                client,
            ),
        )
        .with_graceful_shutdown(async {
            let _ = stopped.await;
        })
        .into_future(),
    );
    tokio::select! {
        result = &mut serving => {
            result.map_err(|_| ServiceError)?.map_err(|_| ServiceError)?;
            return Err(ServiceError);
        }
        _ = shutdown_signal() => {}
    }
    let _ = stop.send(());
    // Bound drain/cancellation as a whole, including malicious stalled HTTP bodies.
    // On timeout main exits immediately without closing SQLite under running workers;
    // exclusive-root startup performs durable interrupted-work recovery next time.
    tokio::time::timeout(std::time::Duration::from_secs(20), async {
        store_shutdown.shutdown().await.map_err(|_| ServiceError)?;
        api.shutdown().await.map_err(|_| ServiceError)?;
        serving
            .await
            .map_err(|_| ServiceError)?
            .map_err(|_| ServiceError)?;
        store_shutdown.shutdown().await.map_err(|_| ServiceError)?;
        api.shutdown().await.map_err(|_| ServiceError)?;
        Ok::<(), ServiceError>(())
    })
    .await
    .map_err(|_| ServiceError)??;
    catalog.pool().close().await;
    Ok(())
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
                .expect("install termination handler");
        tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
fn healthcheck() -> bool {
    use std::io::{Read, Write};
    let origin =
        std::env::var("LYTRONAUT_ORIGIN").unwrap_or_else(|_| "http://127.0.0.1:3000".into());
    let Ok(uri) = origin.parse::<axum::http::Uri>() else {
        return false;
    };
    let Some(authority) = uri.authority() else {
        return false;
    };
    let listen = std::env::var("LYTRONAUT_LISTEN").unwrap_or_else(|_| "127.0.0.1:3000".into());
    let Ok(mut address) = listen.parse::<std::net::SocketAddr>() else {
        return false;
    };
    if address.ip().is_unspecified() {
        address.set_ip(if address.is_ipv4() {
            std::net::Ipv4Addr::LOCALHOST.into()
        } else {
            std::net::Ipv6Addr::LOCALHOST.into()
        });
    }
    let timeout = std::time::Duration::from_secs(2);
    let Ok(mut stream) = std::net::TcpStream::connect_timeout(&address, timeout) else {
        return false;
    };
    if stream.set_read_timeout(Some(timeout)).is_err()
        || stream.set_write_timeout(Some(timeout)).is_err()
    {
        return false;
    }
    if write!(
        stream,
        "GET /healthz HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
    )
    .is_err()
    {
        return false;
    }
    let mut bytes = Vec::new();
    if stream.take(4096).read_to_end(&mut bytes).is_err() {
        return false;
    }
    let Ok(reply) = std::str::from_utf8(&bytes) else {
        return false;
    };
    reply.starts_with("HTTP/1.1 200 ") && reply.contains("\"ready\":true")
}
