//! Owner-only aggregate HTTP check against an existing private catalog. No source copy or output.
use lytronaut_service::{AppState, SecurityConfig, api::Api, local, router_with_routes, uploads};
use lytronaut_storage::{Catalog, Limits, Store};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    net::TcpStream,
    path::PathBuf,
};
async fn request(
    port: u16,
    method: &str,
    path: &str,
    headers: &str,
    body: Vec<u8>,
) -> Result<(u16, String, Vec<u8>), ()> {
    let mut bytes=format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:3000\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{headers}\r\n",body.len()).into_bytes();
    bytes.extend(body);
    tokio::task::spawn_blocking(move || {
        let mut socket = TcpStream::connect(("127.0.0.1", port)).map_err(|_| ())?;
        socket
            .set_read_timeout(Some(std::time::Duration::from_secs(30)))
            .map_err(|_| ())?;
        socket.write_all(&bytes).map_err(|_| ())?;
        let mut reply = Vec::new();
        socket.read_to_end(&mut reply).map_err(|_| ())?;
        let pos = reply.windows(4).position(|b| b == b"\r\n\r\n").ok_or(())?;
        let head = String::from_utf8(reply[..pos].to_vec()).map_err(|_| ())?;
        let status = head
            .split_whitespace()
            .nth(1)
            .ok_or(())?
            .parse()
            .map_err(|_| ())?;
        Ok((status, head, reply[pos + 4..].to_vec()))
    })
    .await
    .map_err(|_| ())?
}
async fn get(port: u16, path: &str, headers: &str) -> Result<Value, ()> {
    let r = request(port, "GET", path, headers, vec![]).await?;
    if r.0 != 200 {
        return Err(());
    }
    serde_json::from_slice(&r.2).map_err(|_| ())
}
async fn run() -> Result<(), ()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 2 {
        return Err(());
    }
    let root = PathBuf::from(&args[1]);
    local::private_directory(&root).map_err(|_| ())?;
    let database = root.join("app.db");
    local::database_file(&database).map_err(|_| ())?;
    let catalog = Catalog::open_application_database(&database)
        .await
        .map_err(|_| ())?;
    let store = Store::open(
        catalog.clone(),
        &root.join("originals"),
        &root.join("staging"),
        Limits {
            free_floor: 0,
            ..Default::default()
        },
    )
    .await
    .map_err(|_| ())?;
    let api = Api::open(catalog.clone(), store.clone())
        .await
        .map_err(|_| ())?;
    let app = router_with_routes(
        AppState {
            catalog: catalog.clone(),
            security: SecurityConfig::new("http://127.0.0.1:3000").map_err(|_| ())?,
        },
        uploads::routes(store).merge(api.routes()),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|_| ())?;
    let port = listener.local_addr().map_err(|_| ())?.port();
    let task = tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    let headers = String::new();
    let libs = get(port, "/libraries", &headers).await?;
    let libraries = libs["items"].as_array().ok_or(())?;
    if libraries.len() != 1 {
        return Err(());
    }
    let lib = libraries[0]["id"].as_str().ok_or(())?;
    if libraries[0]["state"] != "ready" {
        return Err(());
    }
    let albums = get(
        port,
        &format!("/libraries/{lib}/albums?limit=100"),
        &headers,
    )
    .await?;
    let albums = albums["items"].as_array().ok_or(())?;
    if albums.len() != 3 {
        return Err(());
    }
    let mut memberships = 0;
    let mut sizes = Vec::new();
    for album in albums {
        let id = album["id"].as_str().ok_or(())?;
        let page = get(port, &format!("/albums/{id}/captures?limit=100"), &headers).await?;
        let repeated = get(port, &format!("/albums/{id}/captures?limit=100"), &headers).await?;
        if page != repeated {
            return Err(());
        }
        let captures = page["items"].as_array().ok_or(())?;
        let mut prior = None;
        for capture in captures {
            let index = capture["picture_index"].as_i64().ok_or(())?;
            if prior.is_some_and(|p| index <= p) {
                return Err(());
            }
            prior = Some(index);
        }
        memberships += captures.len();
        sizes.push(captures.len());
    }
    let mut captures = Vec::new();
    let mut offset = 0;
    loop {
        let page = get(
            port,
            &format!("/libraries/{lib}/captures?limit=100&offset={offset}"),
            &headers,
        )
        .await?;
        captures.extend(page["items"].as_array().ok_or(())?.iter().cloned());
        if let Some(next) = page["next_offset"].as_u64() {
            offset = next;
        } else {
            break;
        }
    }
    let mut previews = 0;
    let mut jpeg_stack_originals = 0;
    for capture in &captures {
        let id = capture["id"].as_str().ok_or(())?;
        let detail = get(port, &format!("/captures/{id}"), &headers).await?;
        jpeg_stack_originals += detail["originals"]
            .as_array()
            .ok_or(())?
            .iter()
            .filter(|f| f["role"] == "jpeg_focus_stack")
            .count();
        let preview = detail["preview"].as_str().ok_or(())?;
        let path = format!("/assets/{preview}/manifest");
        if request(port, "GET", &path, "", vec![]).await?.0 != 200 {
            return Err(());
        }
        let manifest = get(port, &path, &headers).await?;
        let mut bytes = Vec::new();
        for (index, chunk) in manifest["chunks"].as_array().ok_or(())?.iter().enumerate() {
            let r = request(
                port,
                "GET",
                &format!("/assets/{preview}/chunks/{index}"),
                &headers,
                vec![],
            )
            .await?;
            if r.0 != 200 || r.2.len() as u64 != chunk["length"].as_u64().ok_or(())? {
                return Err(());
            }
            bytes.extend(r.2);
        }
        let hash = Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        if manifest["content_sha256"].as_str() != Some(hash.as_str()) {
            return Err(());
        }
        let original = request(
            port,
            "GET",
            &format!("/assets/{preview}/download"),
            &headers,
            vec![],
        )
        .await?;
        if original.0 != 200 || original.2 != bytes {
            return Err(());
        }
        previews += 1;
    }
    if (captures.len(), previews, memberships, jpeg_stack_originals) != (109, 109, 109, 58) {
        return Err(());
    }
    sizes.sort();
    if sizes != [12, 39, 58] {
        return Err(());
    }
    task.abort();
    let _ = task.await;
    println!(
        "{}",
        json!({"version":1,"captures":captures.len(),"validated_previews":previews,"jpeg_focus_stack_originals":jpeg_stack_originals,"albums":albums.len(),"ordered_memberships":memberships,"ordered_album_sizes":sizes,"direct_manifest_chunks_originals":true,"existing_catalog_reused":true})
    );
    Ok(())
}
#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!("private API aggregate validation failed (details redacted)");
        std::process::exit(1)
    }
}
