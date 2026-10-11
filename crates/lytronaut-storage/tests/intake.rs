//! Independently synthesized preservation inputs under the project MIT license.
use lytronaut_contracts::{ImportKind, ImportRequest, JobState, WireVersion};
use lytronaut_storage::{Catalog, FaultPoint, Limits, StorageError, Store};
use std::{fs, io::Cursor, path::PathBuf};
struct Fixture {
    root: PathBuf,
    catalog: Catalog,
}
impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "lytronaut-intake-{}-{}",
            std::process::id(),
            unique()
        ));
        fs::create_dir(&root).unwrap();
        let catalog = Catalog::open_application_database(&root.join("app.db"))
            .await
            .unwrap();
        Self { root, catalog }
    }
    async fn store(&self) -> Store {
        Store::open(
            self.catalog.clone(),
            &self.root.join("originals"),
            &self.root.join("staging"),
            limits(),
        )
        .await
        .unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn unique() -> u128 {
    let mut b = [0; 16];
    getrandom::fill(&mut b).unwrap();
    u128::from_le_bytes(b)
}
fn limits() -> Limits {
    Limits {
        upload_bytes: 1024,
        files: 8,
        file_bytes: 512,
        encoded_bytes: 4096,
        reserved_bytes: 32768,
        free_floor: 0,
        metadata_bytes: 4096,
        ..Limits::default()
    }
}
fn request(kind: ImportKind) -> ImportRequest {
    ImportRequest {
        version: WireVersion::V1,
        kind,
        declared_bytes: 1024,
        declared_files: 8,
    }
}
async fn append(s: &Store, id: &str, name: &str, bytes: &[u8]) -> Result<(), StorageError> {
    s.append_directory_file(id, name, Cursor::new(bytes.to_vec()))
        .await
}
fn blob_count(f: &Fixture) -> usize {
    fs::read_dir(f.root.join("originals"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|e| e.file_name() != "store.lock")
        .count()
}
#[tokio::test]
async fn logical_manifest_dedup_and_altered_bytes_never_overwrite() {
    let f = Fixture::new().await;
    let s = f.store().await;
    let a = s.begin(&request(ImportKind::Directory)).await.unwrap();
    append(&s, &a, "library/未知.bin", b"independent bytes")
        .await
        .unwrap();
    append(&s, &a, "library/second.bin", b"independent bytes")
        .await
        .unwrap();
    s.finish(&a).await.unwrap();
    let m = s.manifest(&a).await.unwrap();
    assert_eq!(m.len(), 2);
    assert_eq!(m[0].digest, m[1].digest);
    assert_eq!(blob_count(&f), 1);
    assert_eq!(s.job(&a).await.unwrap().total_bytes, 34);
    let b = s.begin(&request(ImportKind::Directory)).await.unwrap();
    append(&s, &b, "library/未知.bin", b"altered bytes")
        .await
        .unwrap();
    s.finish(&b).await.unwrap();
    assert_eq!(blob_count(&f), 2);
    assert_ne!(m[0].digest, s.manifest(&b).await.unwrap()[0].digest);
    let c = s.begin(&request(ImportKind::Directory)).await.unwrap();
    append(&s, &c, "library/未知.bin", b"independent bytes")
        .await
        .unwrap();
    s.finish(&c).await.unwrap();
    assert_eq!(blob_count(&f), 2);
    drop(s);
    let s = f.store().await;
    assert_eq!(s.manifest(&a).await.unwrap(), m);
}
#[tokio::test]
async fn actual_quota_paths_collisions_and_nested_archives_fail_closed() {
    let f = Fixture::new().await;
    let s = f.store().await;
    let a = s.begin(&request(ImportKind::Directory)).await.unwrap();
    for p in [
        "/root", "../x", "a/../x", "C:/x", "a\\x", "a\0x", "a//x", "a/./x", "a/",
    ] {
        assert_eq!(append(&s, &a, p, b"x").await, Err(StorageError::Invalid));
    }
    assert_eq!(
        append(&s, &a, "too-big", &vec![1; 513]).await,
        Err(StorageError::Quota)
    );
    assert_eq!(
        append(&s, &a, "nested", b"PK\x03\x04more").await,
        Err(StorageError::Invalid)
    );
    append(&s, &a, "A/x", b"x").await.unwrap();
    assert_eq!(
        append(&s, &a, "a/X", b"y").await,
        Err(StorageError::Conflict)
    );
    assert_eq!(append(&s, &a, "A", b"y").await, Err(StorageError::Conflict));
    assert_eq!(
        append(&s, &a, "A/x/y", b"y").await,
        Err(StorageError::Conflict)
    );
    assert_eq!(blob_count(&f), 1);
}
#[tokio::test]
async fn every_fault_boundary_recovers_only_verified_or_retryable_intake() {
    for point in [
        FaultPoint::BeforeWrite,
        FaultPoint::AfterWrite,
        FaultPoint::BeforeSync,
        FaultPoint::AfterSync,
        FaultPoint::BeforePublish,
        FaultPoint::AfterPublish,
        FaultPoint::BeforeTransaction,
        FaultPoint::AfterTransaction,
    ] {
        let f = Fixture::new().await;
        let s = f.store().await;
        let id = s.begin(&request(ImportKind::Directory)).await.unwrap();
        s.interrupt_once(point);
        assert_eq!(
            append(&s, &id, "unknown", b"complete original").await,
            Err(StorageError::Interrupted)
        );
        drop(s);
        let s = f.store().await;
        let j = s.job(&id).await.unwrap();
        assert_eq!(j.state, JobState::Failed);
        assert!(j.error.unwrap().retryable);
        assert_eq!(s.manifest(&id).await, Err(StorageError::Conflict));
        let retry = s.retry(&id).await.unwrap();
        append(&s, &retry, "unknown", b"complete original")
            .await
            .unwrap();
        s.finish(&retry).await.unwrap();
        assert_eq!(blob_count(&f), 1);
        assert_eq!(s.manifest(&retry).await.unwrap().len(), 1);
    }
}
#[tokio::test]
async fn durable_cancel_reservations_retry_and_active_spool_finish_guard() {
    let f = Fixture::new().await;
    let s = f.store().await;
    let id = s.begin(&request(ImportKind::Directory)).await.unwrap();
    let mut spool = s.spool(&id).await.unwrap();
    spool.write(b"partial").await.unwrap();
    assert!(matches!(s.spool(&id).await, Err(StorageError::Busy)));
    assert_eq!(s.finish(&id).await, Err(StorageError::Busy));
    s.cancel(&id).await.unwrap();
    assert_eq!(spool.write(b"more").await, Err(StorageError::Cancelled));
    drop(spool);
    drop(s);
    let s = f.store().await;
    assert_eq!(s.job(&id).await.unwrap().state, JobState::Cancelled);
    let retry = s.retry(&id).await.unwrap();
    append(&s, &retry, "file", b"x").await.unwrap();
    s.finish(&retry).await.unwrap();
    let reserved: i64 = sqlx::query_scalar("SELECT SUM(reserved) FROM intake_jobs")
        .fetch_one(f.catalog.pool())
        .await
        .unwrap();
    assert_eq!(reserved, 0);
}
#[tokio::test]
async fn corruption_and_writable_or_symlink_candidates_never_dedup() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new().await;
    let s = f.store().await;
    let id = s.begin(&request(ImportKind::Directory)).await.unwrap();
    append(&s, &id, "file", b"bytes").await.unwrap();
    s.finish(&id).await.unwrap();
    let e = s.manifest(&id).await.unwrap().remove(0);
    let p = f
        .root
        .join("originals")
        .join(format!("{}-{}", e.digest, e.length));
    fs::set_permissions(&p, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&p, b"wrong").unwrap();
    fs::set_permissions(&p, fs::Permissions::from_mode(0o400)).unwrap();
    let retry = s.begin(&request(ImportKind::Directory)).await.unwrap();
    assert_eq!(
        append(&s, &retry, "file", b"bytes").await,
        Err(StorageError::Conflict)
    );
    assert_eq!(fs::read(&p).unwrap(), b"wrong");
    drop(s);
    let s = f.store().await;
    assert_eq!(s.job(&id).await.unwrap().state, JobState::Failed);
}
/// Minimal classic stored ZIP, independently written from the documented framing.
fn zip(name: &str, bytes: &[u8], mode: u32) -> Vec<u8> {
    let crc = crc32fast::hash(bytes);
    let mut local = Vec::new();
    local.extend(b"PK\x03\x04");
    local.extend(20u16.to_le_bytes());
    local.extend([0; 8]);
    local.extend(crc.to_le_bytes());
    local.extend((bytes.len() as u32).to_le_bytes());
    local.extend((bytes.len() as u32).to_le_bytes());
    local.extend((name.len() as u16).to_le_bytes());
    local.extend(0u16.to_le_bytes());
    local.extend(name.as_bytes());
    local.extend(bytes);
    let offset = local.len() as u32;
    let mut c = Vec::new();
    c.extend(b"PK\x01\x02");
    c.extend(0x0314u16.to_le_bytes());
    c.extend(20u16.to_le_bytes());
    c.extend([0; 8]);
    c.extend(crc.to_le_bytes());
    c.extend((bytes.len() as u32).to_le_bytes());
    c.extend((bytes.len() as u32).to_le_bytes());
    c.extend((name.len() as u16).to_le_bytes());
    c.extend([0; 8]);
    c.extend((mode << 16).to_le_bytes());
    c.extend(0u32.to_le_bytes());
    c.extend(name.as_bytes());
    let size = c.len() as u32;
    local.extend(c);
    local.extend(b"PK\x05\x06");
    local.extend([0; 4]);
    local.extend(1u16.to_le_bytes());
    local.extend(1u16.to_le_bytes());
    local.extend(size.to_le_bytes());
    local.extend(offset.to_le_bytes());
    local.extend([0; 2]);
    local
}
async fn upload_zip(s: &Store, id: &str, bytes: &[u8]) -> Result<(), StorageError> {
    let mut spool = s.spool(id).await?;
    for c in bytes.chunks(3) {
        spool.write(c).await?;
    }
    spool.sync().await?;
    s.import_zip(id, spool).await
}
#[tokio::test]
async fn zip_unknown_files_preserved_unsafe_types_names_and_crc_rejected() {
    let f = Fixture::new().await;
    let s = f.store().await;
    let good = s.begin(&request(ImportKind::ZipLibrary)).await.unwrap();
    upload_zip(
        &s,
        &good,
        &zip("library/未知", b"unknown content", 0o100644),
    )
    .await
    .unwrap();
    assert_eq!(
        s.manifest(&good).await.unwrap()[0].logical_path,
        "library/未知"
    );
    for (name, mode) in [
        ("../bad", 0o100644),
        ("symlink", 0o120777),
        ("device", 0o020600),
    ] {
        let id = s.begin(&request(ImportKind::ZipLibrary)).await.unwrap();
        assert!(
            upload_zip(&s, &id, &zip(name, b"unknown", mode))
                .await
                .is_err()
        );
        s.cancel(&id).await.unwrap();
    }
    let id = s.begin(&request(ImportKind::ZipLibrary)).await.unwrap();
    let mut bad = zip("broken", b"good bytes", 0o100644);
    bad[36] ^= 1;
    assert!(upload_zip(&s, &id, &bad).await.is_err());
    assert_eq!(s.manifest(&id).await, Err(StorageError::Conflict));
}
#[tokio::test]
async fn empty_files_false_declarations_and_incomplete_zip_policy() {
    let f = Fixture::new().await;
    let s = f.store().await;
    let mut req = request(ImportKind::Directory);
    req.declared_bytes = 1;
    req.declared_files = 2;
    let id = s.begin(&req).await.unwrap();
    append(&s, &id, "empty", b"").await.unwrap();
    append(&s, &id, "one", b"x").await.unwrap();
    assert_eq!(
        append(&s, &id, "overflow", b"x").await,
        Err(StorageError::Quota)
    );
    s.finish(&id).await.unwrap();
    assert!(s.manifest(&id).await.unwrap().iter().any(|e| e.length == 0));
    let mut req = request(ImportKind::Directory);
    req.declared_bytes = 1;
    let id = s.begin(&req).await.unwrap();
    assert_eq!(
        append(&s, &id, "liar", b"xx").await,
        Err(StorageError::Quota)
    );
    s.cancel(&id).await.unwrap();
    let z = s.begin(&request(ImportKind::ZipLibrary)).await.unwrap();
    assert_eq!(s.finish(&z).await, Err(StorageError::Conflict));
}
#[tokio::test]
async fn terminal_transaction_faults_and_auth_namespace_survive_recovery() {
    for point in [FaultPoint::BeforeTransaction, FaultPoint::AfterTransaction] {
        let f = Fixture::new().await;
        sqlx::query("CREATE TABLE auth_synthetic (value TEXT)")
            .execute(f.catalog.pool())
            .await
            .unwrap();
        sqlx::query("INSERT INTO auth_synthetic VALUES ('retained')")
            .execute(f.catalog.pool())
            .await
            .unwrap();
        let s = f.store().await;
        let id = s.begin(&request(ImportKind::Directory)).await.unwrap();
        append(&s, &id, "file", b"x").await.unwrap();
        s.interrupt_once(point);
        assert_eq!(s.finish(&id).await, Err(StorageError::Interrupted));
        drop(s);
        let s = f.store().await;
        assert_eq!(
            s.job(&id).await.unwrap().state,
            if point == FaultPoint::BeforeTransaction {
                JobState::Failed
            } else {
                JobState::Succeeded
            }
        );
        let auth: String = sqlx::query_scalar("SELECT value FROM auth_synthetic")
            .fetch_one(f.catalog.pool())
            .await
            .unwrap();
        assert_eq!(auth, "retained");
    }
}
#[tokio::test]
async fn restart_removes_real_partial_pending_and_linked_publication() {
    let f = Fixture::new().await;
    let s = f.store().await;
    let id = s.begin(&request(ImportKind::Directory)).await.unwrap();
    append(&s, &id, "file", b"bytes").await.unwrap();
    s.finish(&id).await.unwrap();
    let e = s.manifest(&id).await.unwrap().remove(0);
    drop(s);
    fs::write(f.root.join("originals/pending-unfinished"), b"partial").unwrap();
    fs::hard_link(
        f.root
            .join("originals")
            .join(format!("{}-{}", e.digest, e.length)),
        f.root.join("originals/pending-published"),
    )
    .unwrap();
    fs::write(f.root.join("staging/abandoned"), b"partial archive").unwrap();
    let s = f.store().await;
    assert_eq!(blob_count(&f), 1);
    assert_eq!(s.manifest(&id).await.unwrap()[0], e);
    assert_eq!(fs::read_dir(f.root.join("staging")).unwrap().count(), 1);
}
#[tokio::test]
async fn small_default_directory_import_reserves_actual_work_and_cancellation_race() {
    let f = Fixture::new().await;
    let s = Store::open(
        f.catalog.clone(),
        &f.root.join("originals"),
        &f.root.join("staging"),
        Limits::default(),
    )
    .await
    .unwrap();
    let mut req = request(ImportKind::Directory);
    req.declared_bytes = 1;
    let id = s.begin(&req).await.unwrap();
    let reserved: i64 = sqlx::query_scalar("SELECT reserved FROM intake_jobs WHERE id=?")
        .bind(&id)
        .fetch_one(f.catalog.pool())
        .await
        .unwrap();
    assert_eq!(reserved, 3);
    append(&s, &id, "empty", b"").await.unwrap();
    let (finish, cancel) = tokio::join!(s.finish(&id), s.cancel(&id));
    assert!(cancel.is_ok());
    let state = s.job(&id).await.unwrap().state;
    assert!(matches!(state, JobState::Succeeded | JobState::Cancelled));
    if finish.is_ok() {
        assert_eq!(state, JobState::Succeeded);
    } else {
        assert_eq!(state, JobState::Cancelled);
    }
}
fn deflate_zip(name: &str, bytes: &[u8], declared: u32) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    let compressed = encoder.finish().unwrap();
    let original = zip(name, bytes, 0o100644);
    let central = 30 + name.len() + bytes.len();
    let mut out = original[..30 + name.len()].to_vec();
    out[8..10].copy_from_slice(&8u16.to_le_bytes());
    out[18..22].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
    out[22..26].copy_from_slice(&declared.to_le_bytes());
    out.extend(&compressed);
    let newcentral = out.len();
    out.extend(&original[central..]);
    out[newcentral + 10..newcentral + 12].copy_from_slice(&8u16.to_le_bytes());
    out[newcentral + 20..newcentral + 24].copy_from_slice(&(compressed.len() as u32).to_le_bytes());
    out[newcentral + 24..newcentral + 28].copy_from_slice(&declared.to_le_bytes());
    let end = out.len();
    out[end - 6..end - 2].copy_from_slice(&(newcentral as u32).to_le_bytes());
    out
}
#[tokio::test]
async fn deflate_actual_bomb_and_untrusted_central_counts_are_bounded() {
    let f = Fixture::new().await;
    let s = f.store().await;
    let id = s.begin(&request(ImportKind::ZipLibrary)).await.unwrap();
    upload_zip(
        &s,
        &id,
        &deflate_zip("deflated", b"independent compressed bytes", 28),
    )
    .await
    .unwrap();
    let id = s.begin(&request(ImportKind::ZipLibrary)).await.unwrap();
    assert_eq!(
        upload_zip(&s, &id, &deflate_zip("bomb", &vec![1; 4096], 1)).await,
        Err(StorageError::Quota)
    );
    s.cancel(&id).await.unwrap();
    let id = s.begin(&request(ImportKind::ZipLibrary)).await.unwrap();
    let mut hostile = zip("small", b"x", 0o100644);
    let n = hostile.len();
    hostile[n - 14..n - 10].copy_from_slice(&[255; 4]);
    assert!(upload_zip(&s, &id, &hostile).await.is_err());
    assert_eq!(blob_count(&f), 1);
}
#[tokio::test]
async fn metadata_admission_is_aggregate_and_directory_manifest_requires_explicit_finish() {
    let f = Fixture::new().await;
    let mut l = limits();
    l.metadata_bytes = 5;
    let s = Store::open(
        f.catalog.clone(),
        &f.root.join("originals"),
        &f.root.join("staging"),
        l,
    )
    .await
    .unwrap();
    let id = s.begin(&request(ImportKind::Directory)).await.unwrap();
    append(&s, &id, "abc", b"x").await.unwrap();
    assert_eq!(s.manifest(&id).await, Err(StorageError::Conflict));
    assert_eq!(append(&s, &id, "def", b"x").await, Err(StorageError::Quota));
    s.finish(&id).await.unwrap();
    assert_eq!(s.manifest(&id).await.unwrap().len(), 1);
}
fn zip64(name: &str, bytes: &[u8]) -> Vec<u8> {
    let base = zip(name, bytes, 0o100644);
    let at = 30 + name.len() + bytes.len();
    let mut out = base[..30 + name.len()].to_vec();
    out[18..26].fill(255);
    out[28..30].copy_from_slice(&20u16.to_le_bytes());
    out.extend(1u16.to_le_bytes());
    out.extend(16u16.to_le_bytes());
    out.extend((bytes.len() as u64).to_le_bytes());
    out.extend((bytes.len() as u64).to_le_bytes());
    out.extend(bytes);
    let offset = out.len() as u64;
    let mut central = base[at..at + 46 + name.len()].to_vec();
    central[20..28].fill(255);
    central[30..32].copy_from_slice(&28u16.to_le_bytes());
    central[42..46].fill(255);
    central.extend(1u16.to_le_bytes());
    central.extend(24u16.to_le_bytes());
    central.extend((bytes.len() as u64).to_le_bytes());
    central.extend((bytes.len() as u64).to_le_bytes());
    central.extend(0u64.to_le_bytes());
    let size = central.len() as u64;
    out.extend(central);
    let zoffset = out.len() as u64;
    out.extend(b"PK\x06\x06");
    out.extend(44u64.to_le_bytes());
    out.extend(45u16.to_le_bytes());
    out.extend(45u16.to_le_bytes());
    out.extend([0; 8]);
    out.extend(1u64.to_le_bytes());
    out.extend(1u64.to_le_bytes());
    out.extend(size.to_le_bytes());
    out.extend(offset.to_le_bytes());
    out.extend(b"PK\x06\x07");
    out.extend([0; 4]);
    out.extend(zoffset.to_le_bytes());
    out.extend(1u32.to_le_bytes());
    out.extend(b"PK\x05\x06");
    out.extend([0; 4]);
    out.extend([255; 12]);
    out.extend([0; 2]);
    out
}
#[tokio::test]
async fn bounded_zip64_framing_and_nested_archive_rejection() {
    let f = Fixture::new().await;
    let s = f.store().await;
    let id = s.begin(&request(ImportKind::ZipLibrary)).await.unwrap();
    upload_zip(&s, &id, &zip64("zip64-unknown", b"preserved zip64 bytes"))
        .await
        .unwrap();
    assert_eq!(s.manifest(&id).await.unwrap()[0].length, 21);
    let id = s.begin(&request(ImportKind::ZipLibrary)).await.unwrap();
    let nested = zip("inner", b"x", 0o100644);
    assert_eq!(
        upload_zip(&s, &id, &zip("nested.zip", &nested, 0o100644)).await,
        Err(StorageError::Invalid)
    );
    s.cancel(&id).await.unwrap();
    let id = s.begin(&request(ImportKind::ZipLibrary)).await.unwrap();
    let mut bad = zip64("hostile", b"x");
    let pos = bad.windows(4).position(|v| v == b"PK\x06\x06").unwrap();
    bad[pos + 24..pos + 40].fill(255);
    assert!(upload_zip(&s, &id, &bad).await.is_err());
}
#[tokio::test]
async fn zip64_local_size_mismatch_is_rejected_before_publication() {
    let f = Fixture::new().await;
    let s = f.store().await;
    let id = s.begin(&request(ImportKind::ZipLibrary)).await.unwrap();
    let mut input = zip64("mismatch", b"immutable");
    input[30 + "mismatch".len() + 4..30 + "mismatch".len() + 12]
        .copy_from_slice(&1u64.to_le_bytes());
    assert_eq!(
        upload_zip(&s, &id, &input).await,
        Err(StorageError::Invalid)
    );
    assert_eq!(blob_count(&f), 0);
}
