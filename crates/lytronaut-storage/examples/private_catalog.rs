//! Operator-only aggregate reconciliation. Inputs and scratch never belong in Git/CI.
use lytronaut_contracts::{ImportKind, ImportRequest, WireVersion};
use lytronaut_storage::{
    Catalog, Limits, Store,
    desktop::{self, CatalogState},
};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};
fn audit(source: &Path) -> Result<BTreeMap<String, serde_json::Value>, ()> {
    fn visit(
        base: &Path,
        dir: &Path,
        out: &mut BTreeMap<String, serde_json::Value>,
    ) -> Result<(), ()> {
        for e in fs::read_dir(dir).map_err(|_| ())? {
            let e = e.map_err(|_| ())?;
            let m = fs::symlink_metadata(e.path()).map_err(|_| ())?;
            if m.is_dir() {
                visit(base, &e.path(), out)?;
            } else if m.is_file() {
                let p = e.path();
                let name = p
                    .strip_prefix(base)
                    .map_err(|_| ())?
                    .to_str()
                    .ok_or(())?
                    .to_string();
                let mut file = fs::File::open(&p).map_err(|_| ())?;
                let mut hash = Sha256::new();
                let mut buf = [0; 1024 * 1024];
                loop {
                    let n = file.read(&mut buf).map_err(|_| ())?;
                    if n == 0 {
                        break;
                    }
                    hash.update(&buf[..n]);
                }
                out.insert(name,serde_json::json!({"sha256":hash.finalize().iter().map(|b| format!("{b:02x}")).collect::<String>(),"bytes":m.len(),"mtime_ns":m.modified().map_err(|_|())?.duration_since(UNIX_EPOCH).map_err(|_|())?.as_nanos() as u64}));
            } else {
                return Err(());
            }
        }
        Ok(())
    }
    let mut out = BTreeMap::new();
    visit(source, source, &mut out)?;
    Ok(out)
}
async fn run() -> Result<(), ()> {
    let args: Vec<_> = std::env::args_os().collect();
    if args.len() != 4 {
        return Err(());
    }
    let source = PathBuf::from(&args[1]);
    let baseline = PathBuf::from(&args[2]);
    let scratch = PathBuf::from(&args[3]);
    // Create-new root prevents accidental reuse/cleanup of an unrelated directory.
    fs::DirBuilder::new()
        .mode(0o700)
        .create(&scratch)
        .map_err(|_| ())?;
    let before = audit(&source)?;
    let baseline: serde_json::Value =
        serde_json::from_slice(&fs::read(baseline).map_err(|_| ())?).map_err(|_| ())?;
    if serde_json::to_value(&before).map_err(|_| ())? != baseline["files"] {
        return Err(());
    }
    let bytes: u64 = before.values().map(|v| v["bytes"].as_u64().unwrap()).sum();
    let max = before
        .values()
        .map(|v| v["bytes"].as_u64().unwrap())
        .max()
        .ok_or(())?;
    let catalog = Catalog::open_application_database(&scratch.join("app.db"))
        .await
        .map_err(|_| ())?;
    fs::set_permissions(scratch.join("app.db"), fs::Permissions::from_mode(0o600))
        .map_err(|_| ())?;
    let store = Store::open(
        catalog.clone(),
        &scratch.join("originals"),
        &scratch.join("staging"),
        Limits {
            upload_bytes: bytes,
            files: before.len() as u32,
            file_bytes: max,
            reserved_bytes: bytes + max * 2 + 1024 * 1024,
            free_floor: 0,
            ..Limits::default()
        },
    )
    .await
    .map_err(|_| ())?;
    let job = store
        .begin(&ImportRequest {
            version: WireVersion::V1,
            kind: ImportKind::Directory,
            declared_bytes: bytes,
            declared_files: before.len() as u32,
        })
        .await
        .map_err(|_| ())?;
    for name in before.keys() {
        store
            .append_directory_file(
                &job,
                name,
                fs::File::open(source.join(name)).map_err(|_| ())?,
            )
            .await
            .map_err(|_| ())?;
    }
    store.finish(&job).await.map_err(|_| ())?;
    let summary = desktop::import_desktop(&catalog, &store, &job)
        .await
        .map_err(|_| ())?;
    if summary.state != CatalogState::Ready
        || (
            summary.files,
            summary.pictures,
            summary.metadata,
            summary.albums,
            summary.memberships,
            summary.bins,
            summary.events,
            summary.picture_lists,
            summary.item_errors,
        ) != (728, 109, 109, 3, 109, 2, 3, 11, 0)
    {
        return Err(());
    }
    let lengths:Vec<i64>=sqlx::query_scalar("SELECT count(*) FROM catalog_memberships WHERE version=? GROUP BY album_uuid ORDER BY count(*)").bind(summary.version_id).fetch_all(catalog.pool()).await.map_err(|_|())?;
    if lengths != [12, 39, 58] {
        return Err(());
    }
    let manifest = store.manifest(&job).await.map_err(|_| ())?;
    for e in &manifest {
        let expected = before.get(&e.logical_path).ok_or(())?;
        if expected["sha256"].as_str() != Some(&e.digest)
            || expected["bytes"].as_u64() != Some(e.length)
        {
            return Err(());
        }
    }
    let reimport = desktop::import_desktop(&catalog, &store, &job)
        .await
        .map_err(|_| ())?;
    if reimport.version_id != summary.version_id {
        return Err(());
    }
    let reopened = Catalog::open_application_database(&scratch.join("app.db"))
        .await
        .map_err(|_| ())?;
    if desktop::summary(&reopened, summary.version_id)
        .await
        .map_err(|_| ())?
        .memberships
        != 109
    {
        return Err(());
    }
    let after = audit(&source)?;
    if before != after {
        return Err(());
    }
    let report = serde_json::json!({"summary":summary,"files":before.len(),"bytes":bytes,"source_hash_size_mtime_unchanged":true,"baseline_matches":true,"restart_and_reimport":true,"ordered_album_sizes":lengths});
    let mut output = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(scratch.join("aggregate-report.json"))
        .map_err(|_| ())?;
    output
        .write_all(
            serde_json::to_string_pretty(&report)
                .map_err(|_| ())?
                .as_bytes(),
        )
        .map_err(|_| ())?;
    output.sync_all().map_err(|_| ())?;
    println!("{}", serde_json::to_string(&report).map_err(|_| ())?);
    Ok(())
}
#[tokio::main]
async fn main() {
    if run().await.is_err() {
        eprintln!("private catalog reconciliation failed (details redacted)");
        std::process::exit(1);
    }
}
