//! Local owner-operated diagnostics. Never emit source paths, row values, hashes or credentials.
use crate::{api, local};
use lytronaut_contracts::{ImportKind, ImportRequest, WireVersion};
use lytronaut_storage::{Catalog, Limits, Store, desktop};
use serde::Serialize;
use std::{
    fs,
    os::unix::fs::{MetadataExt, OpenOptionsExt},
    path::{Path, PathBuf},
};
#[derive(Debug, Clone, Serialize)]
pub struct SupportReport {
    pub version: u16,
    pub operation: &'static str,
    pub status: &'static str,
    pub files: u32,
    pub bytes: u64,
    pub captures: usize,
    pub albums: usize,
    pub memberships: usize,
    pub item_errors: usize,
}
struct Input {
    relative: String,
    path: PathBuf,
    metadata: fs::Metadata,
}
fn collect(source: &Path, limits: &Limits) -> Result<(Vec<Input>, u64), ()> {
    fn visit(
        root: &Path,
        dir: &Path,
        depth: usize,
        limits: &Limits,
        entries: &mut Vec<Input>,
        bytes: &mut u64,
        path_bytes: &mut u64,
    ) -> Result<(), ()> {
        let meta = fs::symlink_metadata(dir).map_err(|_| ())?;
        if !meta.is_dir() || meta.file_type().is_symlink() || depth > limits.depth {
            return Err(());
        }
        for entry in fs::read_dir(dir).map_err(|_| ())? {
            let path = entry.map_err(|_| ())?.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| ())?;
            if metadata.is_dir() {
                visit(root, &path, depth + 1, limits, entries, bytes, path_bytes)?
            } else {
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.nlink() != 1
                    || metadata.len() > limits.file_bytes
                {
                    return Err(());
                }
                let relative = path
                    .strip_prefix(root)
                    .map_err(|_| ())?
                    .to_str()
                    .ok_or(())?
                    .to_string();
                lytronaut_storage::intake::validate_path(&relative, limits).map_err(|_| ())?;
                *path_bytes = path_bytes.checked_add(relative.len() as u64).ok_or(())?;
                if *path_bytes > limits.metadata_bytes {
                    return Err(());
                }
                *bytes = bytes.checked_add(metadata.len()).ok_or(())?;
                if *bytes > limits.upload_bytes || entries.len() >= limits.files as usize {
                    return Err(());
                }
                entries.push(Input {
                    relative,
                    path,
                    metadata,
                });
            }
        }
        Ok(())
    }
    let mut entries = Vec::new();
    let mut bytes = 0;
    let mut path_bytes = 0;
    visit(
        source,
        source,
        0,
        limits,
        &mut entries,
        &mut bytes,
        &mut path_bytes,
    )?;
    entries.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok((entries, bytes))
}
fn open(input: &Input, root: &Path) -> Result<fs::File, ()> {
    // Recheck ancestors: this is a trusted owner CLI, never an HTTP filesystem import.
    let mut ancestor = input.path.parent();
    while let Some(dir) = ancestor {
        let m = fs::symlink_metadata(dir).map_err(|_| ())?;
        if !m.is_dir() || m.file_type().is_symlink() {
            return Err(());
        }
        if dir == root {
            break;
        }
        ancestor = dir.parent();
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&input.path)
        .map_err(|_| ())?;
    let m = file.metadata().map_err(|_| ())?;
    if !m.is_file()
        || m.nlink() != 1
        || m.ino() != input.metadata.ino()
        || m.dev() != input.metadata.dev()
        || m.len() != input.metadata.len()
        || m.mtime() != input.metadata.mtime()
        || m.mtime_nsec() != input.metadata.mtime_nsec()
    {
        return Err(());
    }
    Ok(file)
}
async fn run_inner(operation: &str, source: &Path, data: &Path) -> Result<SupportReport, ()> {
    if !matches!(operation, "inspect" | "import") {
        return Err(());
    }
    let source = if source.is_absolute() {
        source.to_owned()
    } else {
        std::env::current_dir().map_err(|_| ())?.join(source)
    };
    let limits = Limits::from_environment().map_err(|_| ())?;
    let (entries, bytes) = collect(&source, &limits)?;
    // Validate every entry as a read-only regular file, even in inspection mode.
    for entry in &entries {
        open(entry, &source)?;
    }
    let mut report = SupportReport {
        version: 1,
        operation: if operation == "inspect" {
            "inspect"
        } else {
            "import"
        },
        status: "inspected",
        files: entries.len() as u32,
        bytes,
        captures: 0,
        albums: 0,
        memberships: 0,
        item_errors: 0,
    };
    if operation == "inspect" {
        return Ok(report);
    }
    if entries.is_empty() || bytes == 0 {
        return Err(());
    }
    local::private_directory(data).map_err(|_| ())?;
    let database = local::application_database(data).map_err(|_| ())?;
    local::database_file(&database).map_err(|_| ())?;
    let catalog = Catalog::open_application_database(&database)
        .await
        .map_err(|_| ())?;
    let store = Store::open(
        catalog.clone(),
        &data.join("originals"),
        &data.join("staging"),
        limits,
    )
    .await
    .map_err(|_| ())?;
    let job = store
        .begin(&ImportRequest {
            version: WireVersion::V1,
            kind: ImportKind::Directory,
            declared_bytes: bytes,
            declared_files: entries.len() as u32,
        })
        .await
        .map_err(|_| ())?;
    let intake = async {
        for entry in &entries {
            store
                .append_directory_file(&job, &entry.relative, open(entry, &source)?)
                .await
                .map_err(|_| ())?;
        }
        store.finish(&job).await.map_err(|_| ())?;
        Ok::<(), ()>(())
    }
    .await;
    if intake.is_err() {
        let _ = store.cancel(&job).await;
        return Err(());
    }
    api::adapt(&catalog, &store, &job).await.map_err(|_| ())?;
    let version: i64 = sqlx::query_scalar("SELECT version FROM api_imports WHERE job=?")
        .bind(job)
        .fetch_one(catalog.pool())
        .await
        .map_err(|_| ())?;
    let summary = desktop::summary(&catalog, version).await.map_err(|_| ())?;
    report.status = match summary.state {
        desktop::CatalogState::Ready => "ready",
        desktop::CatalogState::PreservedOnly => "preserved_only",
        desktop::CatalogState::Quarantined => "quarantined",
    };
    report.captures = summary.pictures;
    report.albums = summary.albums;
    report.memberships = summary.memberships;
    report.item_errors = summary.item_errors;
    Ok(report)
}

/// A fixed redacted error, deliberately carrying no filesystem or source details.
#[derive(Debug)]
pub struct DiagnosticError;
impl std::fmt::Display for DiagnosticError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid or unavailable diagnostic input")
    }
}
impl std::error::Error for DiagnosticError {}
pub async fn run(
    operation: &str,
    source: &Path,
    data: &Path,
) -> Result<SupportReport, DiagnosticError> {
    run_inner(operation, source, data)
        .await
        .map_err(|_| DiagnosticError)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::DirBuilderExt;
    #[test]
    fn inspection_bounds_total_retained_path_bytes() {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let root = std::env::temp_dir().join(format!(
            "lytronaut-path-budget-test-{:x}",
            u128::from_le_bytes(random)
        ));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        fs::write(root.join("aaaa"), b"x").unwrap();
        fs::write(root.join("bbbb"), b"y").unwrap();
        assert!(
            collect(
                &root,
                &Limits {
                    metadata_bytes: 7,
                    ..Default::default()
                }
            )
            .is_err()
        );
        assert_eq!(
            collect(
                &root,
                &Limits {
                    metadata_bytes: 8,
                    ..Default::default()
                }
            )
            .unwrap()
            .0
            .len(),
            2
        );
        fs::remove_dir_all(root).unwrap();
    }
}
