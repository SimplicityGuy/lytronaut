//! Scratch quota is durable and root-owned; no private source data is used.
use super::idle_tests::Fixture;
use super::*;

async fn sealed(store: &Store) -> String {
    let job = store
        .begin(&ImportRequest {
            version: WireVersion::V1,
            kind: lytronaut_contracts::ImportKind::Directory,
            declared_bytes: 1,
            declared_files: 1,
        })
        .await
        .unwrap();
    store
        .append_directory_file(&job, "synthetic.bin", &b"x"[..])
        .await
        .unwrap();
    store.finish(&job).await.unwrap();
    job
}
async fn reservation_contract(pg: bool) {
    let mut f = Fixture::new(pg).await;
    let other = Store::open(
        f.catalog.clone(),
        &f.root.join("other-originals"),
        &f.root.join("other-staging"),
        f.store.limits().clone(),
    )
    .await
    .unwrap();
    other.0.reaper_enabled.store(false, Ordering::Relaxed);
    // Both roots contain the same lawful one-byte blob; existing whole-database
    // verification sees valid originals whichever locked root is reopened.
    let a = sealed(&f.store).await;
    let b = sealed(&other).await;
    let one = f.store.reserve_catalog_scratch(&a, 12288).await.unwrap();
    let two = other.reserve_catalog_scratch(&b, 12288).await.unwrap();
    let request = ImportRequest {
        version: WireVersion::V1,
        kind: lytronaut_contracts::ImportKind::Directory,
        declared_bytes: 4096,
        declared_files: 1,
    };
    assert!(
        matches!(f.store.begin(&request).await, Err(StorageError::Busy)),
        "scratch from both roots must consume shared quota"
    );
    assert!(
        sqlx::query("DELETE FROM intake_jobs WHERE id=$1")
            .bind(&a)
            .execute(f.catalog.pool())
            .await
            .is_err(),
        "a live reservation must prevent deleting its parent job"
    );
    other.release_catalog_scratch(&one).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT count(*) FROM intake_scratch")
        .fetch_one(f.catalog.pool())
        .await
        .unwrap();
    assert_eq!(n, 2);
    let own_file = f.store.staging_root().join("synthetic-owned-index.db");
    let other_file = other.staging_root().join("synthetic-other-index.db");
    fs::write(&own_file, b"owned").unwrap();
    fs::write(&other_file, b"other").unwrap();
    let old = std::mem::replace(&mut f.store, other.clone());
    drop(old);
    let reopened = Store::open(
        f.catalog.clone(),
        &f.root.join("originals"),
        &f.root.join("staging"),
        other.limits().clone(),
    )
    .await
    .unwrap();
    reopened.0.reaper_enabled.store(false, Ordering::Relaxed);
    f.store = reopened;
    assert!(!own_file.exists());
    assert!(other_file.exists());
    let owners: Vec<String> = sqlx::query_scalar("SELECT owner FROM intake_scratch")
        .fetch_all(f.catalog.pool())
        .await
        .unwrap();
    assert_eq!(owners, vec![two.clone()]);
    // The old completed job remains completed with its original intake reservation
    // zero: scratch has independent ownership and never edits succeeded rows.
    let row = sqlx::query("SELECT state,reserved FROM intake_jobs WHERE id=$1")
        .bind(&a)
        .fetch_one(f.catalog.pool())
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>(0), "succeeded");
    assert_eq!(row.get::<i64, _>(1), 0);
    fs::remove_file(other_file).unwrap();
    other.release_catalog_scratch(&two).await.unwrap();
    let admitted = f.store.begin(&request).await.unwrap();
    f.store.cancel(&admitted).await.unwrap();
    assert!(
        f.store
            .reserve_catalog_scratch(&admitted, 4096)
            .await
            .is_err(),
        "unfinished/cancelled intake is never admitted"
    );
    let retained = f.store.reserve_catalog_scratch(&a, 4096).await.unwrap();
    let staging = f.store.staging_root().to_path_buf();
    let moved = f.root.join("moved-staging");
    fs::rename(&staging, &moved).unwrap();
    fs::create_dir(&staging).unwrap();
    assert!(matches!(
        f.store.reserve_catalog_scratch(&a, 4096).await,
        Err(StorageError::Conflict)
    ));
    assert!(matches!(
        f.store.release_catalog_scratch(&retained).await,
        Err(StorageError::Conflict)
    ));
    let still_reserved: i64 =
        sqlx::query_scalar("SELECT count(*) FROM intake_scratch WHERE owner=$1")
            .bind(&retained)
            .fetch_one(f.catalog.pool())
            .await
            .unwrap();
    assert_eq!(
        still_reserved, 1,
        "a replacement path never adopts the old lock/owner identity"
    );
    fs::remove_dir(&staging).unwrap();
    fs::rename(&moved, &staging).unwrap();
    f.store.release_catalog_scratch(&retained).await.unwrap();
    drop(other);
    f.close().await;
}
#[tokio::test]
async fn sqlite_reservation_accounting_owned_restart_and_foreign_root_isolation() {
    reservation_contract(false).await;
}
#[tokio::test]
#[ignore = "requires disposable actual PostgreSQL; deployment CI invokes explicitly"]
async fn postgres_reservation_accounting_owned_restart_and_foreign_root_isolation() {
    reservation_contract(true).await;
}
