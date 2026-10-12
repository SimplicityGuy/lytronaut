//! Owner-only existing sealed-catalog roundtrip; all output is aggregate counts.
//! Requires two arguments: protected existing app root and a NEW protected proof directory.
use lytronaut_service::{api::Api, bundle, local};
use lytronaut_storage::{Catalog, Limits, Store};
use sqlx::Row;
use std::{
    fs,
    os::unix::fs::{DirBuilderExt, MetadataExt},
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU64},
};
async fn run() -> Result<(), ()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err(());
    }
    let source = PathBuf::from(&args[0]);
    let proof = PathBuf::from(&args[1]);
    local::private_directory(&source).map_err(|_| ())?;
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&proof)
        .map_err(|_| ())?;
    let catalog = Catalog::open_application_database(&source.join("app.db"))
        .await
        .map_err(|_| ())?;
    let store = Store::open(
        catalog.clone(),
        &source.join("originals"),
        &source.join("staging"),
        Limits {
            free_floor: 2 * 1024 * 1024 * 1024,
            ..Default::default()
        },
    )
    .await
    .map_err(|_| ())?;
    let _api = Api::open(catalog.clone(), store.clone())
        .await
        .map_err(|_| ())?;
    let inventory =
        sqlx::query("SELECT path,digest,length FROM catalog_sources ORDER BY version,path")
            .fetch_all(catalog.pool())
            .await
            .map_err(|_| ())?;
    let mut original_metadata = Vec::new();
    for row in &inventory {
        let digest: String = row.try_get(1).map_err(|_| ())?;
        let length: i64 = row.try_get(2).map_err(|_| ())?;
        let p = source.join("originals").join(format!("{digest}-{length}"));
        let m = fs::metadata(&p).map_err(|_| ())?;
        original_metadata.push((p, m.len(), m.mtime(), m.mtime_nsec()));
    }
    let bytes: u64 = inventory.iter().map(|r| r.get::<i64, _>(2) as u64).sum();
    if store.available_staging_bytes().map_err(|_| ())? < 2 * bytes + 2 * 1024 * 1024 * 1024 {
        return Err(());
    }
    let cancel = AtomicBool::new(false);
    let progress = AtomicU64::new(0);
    let archive = proof.join("originals.lytronaut");
    let expected = bundle::trusted_schema(&catalog).await.map_err(|_| ())?;
    let report = bundle::backup(&catalog, &store, &archive, &cancel, &progress)
        .await
        .map_err(|_| ())?;
    let restored = proof.join("restored");
    bundle::restore(
        &archive,
        &restored,
        &expected,
        store.limits(),
        &cancel,
        &progress,
    )
    .await
    .map_err(|_| ())?;
    let target = restored.join("store");
    let copy = Catalog::open_application_database(&target.join("catalog.db"))
        .await
        .map_err(|_| ())?;
    let restored_store = Store::open(
        copy.clone(),
        &target.join("originals"),
        &target.join("staging"),
        Limits {
            free_floor: 0,
            ..Default::default()
        },
    )
    .await
    .map_err(|_| ())?;
    for row in &inventory {
        let e = lytronaut_storage::SourceEntry {
            logical_path: row.try_get(0).map_err(|_| ())?,
            digest: row.try_get(1).map_err(|_| ())?,
            length: row.get::<i64, _>(2) as u64,
        };
        store.open_blob(&e).map_err(|_| ())?;
        restored_store.open_blob(&e).map_err(|_| ())?;
    }
    for (p, length, seconds, nanos) in original_metadata {
        let m = fs::metadata(p).map_err(|_| ())?;
        if m.len() != length || m.mtime() != seconds || m.mtime_nsec() != nanos {
            return Err(());
        }
    }
    for query in [
        "SELECT hex(original) FROM catalog_records ORDER BY version,kind,ordinal",
        "SELECT quote(uuid)||quote(name)||quote(caption)||quote(rating)||quote(flag_status) FROM catalog_pictures ORDER BY version,uuid",
        "SELECT quote(uuid)||quote(name)||quote(sort_order) FROM catalog_albums ORDER BY version,uuid",
        "SELECT quote(album_uuid)||quote(picture_uuid)||quote(picture_index) FROM catalog_memberships ORDER BY version,album_uuid,picture_index",
        "SELECT quote(id)||quote(kind)||quote(source_key) FROM api_ids ORDER BY id",
    ] {
        let before: Vec<String> = sqlx::query_scalar(query)
            .fetch_all(catalog.pool())
            .await
            .map_err(|_| ())?;
        let after: Vec<String> = sqlx::query_scalar(query)
            .fetch_all(copy.pool())
            .await
            .map_err(|_| ())?;
        if before != after {
            return Err(());
        }
    }
    let captures: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_pictures")
        .fetch_one(copy.pool())
        .await
        .map_err(|_| ())?;
    let albums: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_albums")
        .fetch_one(copy.pool())
        .await
        .map_err(|_| ())?;
    let memberships: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_memberships")
        .fetch_one(copy.pool())
        .await
        .map_err(|_| ())?;
    let summary = serde_json::json!({"status":"passed","logical_files":inventory.len(),"original_bytes":bytes,"unique_blobs":report.files,"captures":captures,"albums":albums,"memberships":memberships,"source_hashes_and_mtimes_unchanged":true,"typed_records_and_associations_exact":true,"auth_and_pending_jobs_excluded":true});
    let text = serde_json::to_string(&summary).map_err(|_| ())?;
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(proof.join("aggregate.json"))
        .map_err(|_| ())?;
    f.write_all(text.as_bytes()).map_err(|_| ())?;
    f.sync_all().map_err(|_| ())?;
    println!("{text}");
    Ok(())
}
#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!("Private export roundtrip failed; aggregate checkpoint unavailable");
        std::process::exit(1)
    }
}
