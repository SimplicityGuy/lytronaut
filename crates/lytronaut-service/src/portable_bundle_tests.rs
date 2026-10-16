//! Independent synthetic database-engine differential tests; no private corpus.
use super::*;
use lytronaut_storage::{Backend, DatabaseConfig};
use sqlx::{ConnectOptions, postgres::PgConnectOptions};
use std::str::FromStr;

pub(crate) const IDS: [i64; 4] = [i64::MIN, -7, 0, i64::MAX];
const KEY: &str = "picture\0雪🛰";
const NAME: &str = "Na\0mé雪";
const CAPTION: &str = "Caption\0🛰";
const ORIGINAL: &[u8] = &[0, 255, 128, 0, 0, 0, 0, 0, 0, 0, 128, 127, 1, 2, 3];
pub(crate) const BLOB: &[u8] = b"synthetic original\0\xff\x80";
pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    pub(crate) catalog: Catalog,
    pub(crate) config: DatabaseConfig,
    pub(crate) pg: Option<(sqlx::PgConnection, String)>,
}
impl Fixture {
    pub(crate) async fn new(postgres: bool) -> Self {
        let mut random = [0; 16];
        getrandom::fill(&mut random).unwrap();
        let name = format!("lytronaut_bundle_{:x}", u128::from_le_bytes(random));
        let root = std::env::temp_dir().join(&name);
        directory(&root).unwrap();
        let mut pg = None;
        let config = if postgres {
            let url = std::env::var("LYTRONAUT_TEST_POSTGRES_URL")
                .expect("explicit disposable PostgreSQL URL required");
            let options = PgConnectOptions::from_str(&url)
                .unwrap()
                .disable_statement_logging();
            let mut connection = sqlx::PgConnection::connect_with(&options).await.unwrap();
            let restricted: bool = sqlx::query_scalar("SELECT NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication) FROM pg_roles WHERE rolname=current_user").fetch_one(&mut connection).await.unwrap();
            assert!(restricted);
            let public_create: bool =
                sqlx::query_scalar("SELECT has_schema_privilege(current_user,'public','CREATE')")
                    .fetch_one(&mut connection)
                    .await
                    .unwrap();
            assert!(!public_create);
            sqlx::query(AssertSqlSafe(format!("CREATE SCHEMA \"{name}\"")))
                .execute(&mut connection)
                .await
                .unwrap();
            pg = Some((connection, name.clone()));
            DatabaseConfig::Postgres(url).with_schema(&name).unwrap()
        } else {
            DatabaseConfig::Sqlite(root.join("catalog.db"))
        };
        let catalog = Catalog::open(config.clone()).await.unwrap();
        fresh_schema(&catalog).await.unwrap();
        Self {
            root,
            catalog,
            config,
            pg,
        }
    }
    pub(crate) async fn seed(&self) -> Store {
        self.seed_with(Limits {
            free_floor: 0,
            ..Default::default()
        })
        .await
    }
    pub(crate) async fn seed_with(&self, limits: Limits) -> Store {
        let store = Store::open(
            self.catalog.clone(),
            &self.root.join("originals"),
            &self.root.join("staging"),
            limits,
        )
        .await
        .unwrap();
        let digest = format!("{:x}", Sha256::digest(BLOB));
        let path = self
            .root
            .join("originals")
            .join(format!("{digest}-{}", BLOB.len()));
        fs::write(&path, BLOB).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o400)).unwrap();
        let mut tx = self.catalog.begin_write().await.unwrap();
        for (index, id) in IDS.iter().enumerate() {
            let job = format!("synthetic-job-{index}");
            // Distinct per-row values in every intake column so round trips
            // compare real values, not defaults.
            let n = index as i64 + 1;
            let path = format!("synthetic/{index}/opaque é.bin");
            sqlx::query("INSERT INTO intake_jobs(id,kind,state,total,budget,max_files,encoded_limit,zip_complete,completed,files,path_bytes,reserved,error) VALUES($1,'directory','succeeded',$2,$3,$4,$5,$6,$7,$8,$9,$10,$11)")
                .bind(&job).bind(n * 1000).bind(n * 2000).bind(n).bind(n * 3000).bind(index as i64 % 2).bind(n * 1000).bind(n).bind(path.len() as i64).bind(n * 4000)
                .bind(if index == 1 { Some("synthetic-recovered-warning") } else { None })
                .execute(&mut *tx).await.unwrap();
            sqlx::query("INSERT INTO intake_sources(job,path,canonical,digest,length) VALUES($1,$2,$3,$4,$5)").bind(&job).bind(&path).bind(format!("{path}#canonical")).bind(&digest).bind(BLOB.len() as i64).execute(&mut *tx).await.unwrap();
            sqlx::query(
                "INSERT INTO application_job_order(ordinal,kind,job) VALUES($1,'intake',$2)",
            )
            .bind(id)
            .bind(&job)
            .execute(&mut *tx)
            .await
            .unwrap();
            sqlx::query("INSERT INTO catalog_versions(id,fingerprint,source_job,library_uuid,state,reason,conflict) VALUES($1,$2,$3,$4,'supported',NULL,0)").bind(id).bind(format!("synthetic-fingerprint-{index}")).bind(&job).bind(if index == 0 { None } else { Some(self.catalog.encode_source_key("library\0雪")) }).execute(&mut *tx).await.unwrap();
            sqlx::query("INSERT INTO catalog_sources(version,job,path,digest,length) VALUES($1,$2,$3,$4,$5)").bind(id).bind(&job).bind(&path).bind(&digest).bind(BLOB.len() as i64).execute(&mut *tx).await.unwrap();
            sqlx::query("INSERT INTO catalog_records(version,kind,ordinal,original) VALUES($1,'unknown-typed-values',$1,$2)").bind(id).bind(ORIGINAL).execute(&mut *tx).await.unwrap();
            let key = self.catalog.encode_source_key(KEY);
            let album = self.catalog.encode_source_key("album\0雪");
            if self.catalog.backend() == Backend::Sqlite {
                sqlx::query("INSERT INTO catalog_pictures(version,uuid,bin_uuid,event_uuid,name,caption,rating,flag_status) VALUES($1,$2,$3,$4,$5,$6,$1,$1)").bind(id).bind(&key).bind(self.catalog.encode_source_key("bin\0雪")).bind(self.catalog.encode_source_key("event\0雪")).bind(NAME).bind(CAPTION).execute(&mut *tx).await.unwrap();
                sqlx::query(
                    "INSERT INTO catalog_albums(version,uuid,name,sort_order) VALUES($1,$2,$3,$1)",
                )
                .bind(id)
                .bind(&album)
                .bind(NAME)
                .execute(&mut *tx)
                .await
                .unwrap();
            } else {
                sqlx::query("INSERT INTO catalog_pictures(version,uuid,bin_uuid,event_uuid,name,caption,rating,flag_status,name_display,name_fold,name_bytes,caption_display,caption_fold,caption_bytes) VALUES($1,$2,$3,$4,$5,$6,$1,$1,'Na',$7,$8,'Caption',$9,$10)").bind(id).bind(&key).bind(self.catalog.encode_source_key("bin\0雪")).bind(self.catalog.encode_source_key("event\0雪")).bind(NAME.as_bytes()).bind(CAPTION.as_bytes()).bind(b"na".as_slice()).bind(NAME.len() as i64).bind(b"caption".as_slice()).bind(CAPTION.len() as i64).execute(&mut *tx).await.unwrap();
                sqlx::query("INSERT INTO catalog_albums(version,uuid,name,sort_order,name_display,name_fold,name_bytes) VALUES($1,$2,$3,$1,'Na',$4,$5)").bind(id).bind(&album).bind(NAME.as_bytes()).bind(b"na".as_slice()).bind(NAME.len() as i64).execute(&mut *tx).await.unwrap();
            }
            sqlx::query("INSERT INTO catalog_memberships(version,album_uuid,picture_uuid,picture_index) VALUES($1,$2,$3,$1)").bind(id).bind(&album).bind(&key).execute(&mut *tx).await.unwrap();
            sqlx::query(
                "INSERT INTO api_ids(id,kind,version,source_key) VALUES($1,'picture',$2,$3)",
            )
            .bind(format!("synthetic-asset-{index}"))
            .bind(id)
            .bind(&key)
            .execute(&mut *tx)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO catalog_item_errors(version,picture_uuid,code) VALUES($1,$2,$3)",
            )
            .bind(id)
            .bind(self.catalog.encode_source_key(KEY))
            .bind(format!("synthetic-item-error-{index}"))
            .execute(&mut *tx)
            .await
            .unwrap();
            sqlx::query("INSERT INTO api_imports(job,version) VALUES($1,$2)")
                .bind(&job)
                .bind(id)
                .execute(&mut *tx)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO api_adaptation_errors(job,code) VALUES('synthetic-job-0','synthetic-pending')").execute(&mut *tx).await.unwrap();
        tx.commit().await.unwrap();
        store
    }
    /// Add `count` deterministic, distinct originals of `size` bytes each to the
    /// seeded library: a 16-byte ordinal prefix and a sparse zero tail, so the
    /// source occupies almost no disk. They belong to the existing successful
    /// `synthetic-job-2`. Returns their total size.
    pub(crate) async fn add_sparse_originals(&self, count: u64, size: u64) -> u64 {
        self.add_sparse_sized(&vec![size; count as usize]).await
    }
    /// As `add_sparse_originals`, one original per entry of `sizes` (each >= 16).
    pub(crate) async fn add_sparse_sized(&self, sizes: &[u64]) -> u64 {
        // Hash the potentially multi-GiB fixture before opening a PostgreSQL
        // transaction: CPU-only preparation must not consume its idle timeout.
        let digests = sparse_sized_digests(sizes);
        let mut tx = self.catalog.begin_write().await.unwrap();
        for (index, (digest, &size)) in digests.into_iter().zip(sizes).enumerate() {
            let path = self.root.join("originals").join(format!("{digest}-{size}"));
            let mut file = new_file(&path).unwrap();
            file.write_all(&sparse_prefix(index as u64)).unwrap();
            file.set_len(size).unwrap();
            drop(file);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
            let logical = format!("beyond-upload-budget/{index}.lfp");
            sqlx::query("INSERT INTO intake_sources(job,path,canonical,digest,length) VALUES('synthetic-job-2',$1,$2,$3,$4)")
                .bind(&logical)
                .bind(format!("{logical}#canonical"))
                .bind(&digest)
                .bind(size as i64)
                .execute(&mut *tx)
                .await
                .unwrap();
        }
        tx.commit().await.unwrap();
        sizes.iter().sum()
    }
    pub(crate) async fn finish(mut self) {
        self.catalog.pool().close().await;
        if let Some((connection, schema)) = self.pg.as_mut() {
            sqlx::query(AssertSqlSafe(format!("DROP SCHEMA \"{schema}\" CASCADE")))
                .execute(&mut *connection)
                .await
                .unwrap();
        }
        fs::remove_dir_all(self.root).unwrap();
    }
}
/// Budgets far below the synthetic library: the former bundle ceiling was
/// `upload_bytes` + 288 MiB, the former restore transport ceiling
/// `encoded_bytes` + 288 MiB + framing, and admission required the whole
/// reservation to fit `reserved_bytes`.
pub(crate) const SPARSE_SIZE: u64 = 16 * 1024 * 1024;
pub(crate) const SPARSE_COUNT: u64 = 20;
pub(crate) fn small_budgets() -> Limits {
    Limits {
        upload_bytes: SPARSE_SIZE,
        file_bytes: SPARSE_SIZE,
        encoded_bytes: SPARSE_SIZE,
        reserved_bytes: SPARSE_SIZE,
        // Far fewer than the library's unique originals: the per-upload
        // file count no longer bounds a whole-library bundle either.
        files: 2,
        free_floor: 0,
        ..Default::default()
    }
}
fn sparse_prefix(index: u64) -> [u8; 16] {
    let mut prefix = *b"lbk-sparse-\0\0\0\0\0";
    prefix[11..].copy_from_slice(&index.to_le_bytes()[..5]);
    prefix
}
/// Independently computed digests of the sparse originals, in ordinal order.
fn sparse_digests(count: u64, size: u64) -> Vec<String> {
    sparse_sized_digests(&vec![size; count as usize])
}
fn sparse_sized_digests(sizes: &[u64]) -> Vec<String> {
    let zeros = [0u8; 65536];
    sizes
        .iter()
        .enumerate()
        .map(|(index, &size)| {
            let mut hash = Sha256::new();
            hash.update(sparse_prefix(index as u64));
            let mut remaining = size - 16;
            while remaining > 0 {
                let n = remaining.min(zeros.len() as u64) as usize;
                hash.update(&zeros[..n]);
                remaining -= n as u64;
            }
            format!("{:x}", hash.finalize())
        })
        .collect()
}
/// Every sparse original is present in `originals` with its exact length and
/// a freshly recomputed SHA-256 equal to the independent digest.
pub(crate) fn verify_sparse_originals(originals: &Path, count: u64, size: u64) {
    verify_sparse_sized(originals, &vec![size; count as usize]);
}
fn verify_sparse_sized(originals: &Path, sizes: &[u64]) {
    for (digest, &size) in sparse_sized_digests(sizes).into_iter().zip(sizes) {
        let path = originals.join(format!("{digest}-{size}"));
        let (actual, length) = hash_file(&path).unwrap();
        assert_eq!(length, size);
        assert_eq!(
            actual
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
            digest
        );
    }
}
fn logical_text(catalog: &Catalog, row: &sqlx::any::AnyRow, column: &str) -> Vec<u8> {
    if catalog.backend() == Backend::Postgres {
        row.try_get(column).unwrap()
    } else {
        row.try_get::<String, _>(column).unwrap().into_bytes()
    }
}
/// Every portable table's logical rows (source keys decoded, text compared as
/// bytes, nulls explicit), sorted. `skip` omits auxiliary tables by name.
pub(crate) async fn logical_tables(
    catalog: &Catalog,
    skip: &[&str],
) -> Vec<(&'static str, Vec<Vec<String>>)> {
    use lytronaut_storage::portable::{Kind, TABLES};
    let postgres = catalog.backend() == Backend::Postgres;
    let mut tables = Vec::new();
    for table in TABLES.iter().filter(|t| !skip.contains(&t.name)) {
        let names = table.columns.iter().map(|c| c.name).collect::<Vec<_>>();
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT {} FROM {}",
            names.join(","),
            table.name
        )))
        .fetch_all(catalog.pool())
        .await
        .unwrap();
        let mut values = rows
            .iter()
            .map(|row| {
                table
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(index, column)| {
                        let text = |bytes: Option<Vec<u8>>| {
                            bytes.map_or("NULL".to_owned(), |b| {
                                b.iter().map(|x| format!("{x:02x}")).collect()
                            })
                        };
                        match column.kind {
                            Kind::Integer => row
                                .try_get::<Option<i64>, _>(index)
                                .unwrap()
                                .map_or("NULL".to_owned(), |v| v.to_string()),
                            Kind::Blob => text(row.try_get(index).unwrap()),
                            Kind::SourceText if postgres => text(row.try_get(index).unwrap()),
                            Kind::SourceKey => text(
                                row.try_get::<Option<String>, _>(index)
                                    .unwrap()
                                    .map(|v| catalog.decode_source_key(v).unwrap().into_bytes()),
                            ),
                            _ => text(
                                row.try_get::<Option<String>, _>(index)
                                    .unwrap()
                                    .map(String::into_bytes),
                            ),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        values.sort();
        assert!(!values.is_empty(), "{} is not seeded", table.name);
        tables.push((table.name, values));
    }
    tables
}
pub(crate) async fn verify(catalog: &Catalog, original_root: &Path) {
    verify_ordered(catalog, original_root, &IDS).await;
}
/// `job_order` is the expected logical intake order: preserved ordinals for the
/// current forms, rowid-derived ordinals for a pre-job-order legacy database.
async fn verify_ordered(catalog: &Catalog, original_root: &Path, job_order: &[i64]) {
    let rows = sqlx::query("SELECT id,library_uuid FROM catalog_versions ORDER BY id")
        .fetch_all(catalog.pool())
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.try_get::<i64, _>("id").unwrap(), IDS[index]);
        let library: Option<String> = row.try_get("library_uuid").unwrap();
        assert_eq!(
            library.map(|value| catalog.decode_source_key(value).unwrap()),
            if index == 0 {
                None
            } else {
                Some("library\0雪".into())
            }
        );
    }
    let rows = sqlx::query("SELECT version,ordinal,original FROM catalog_records WHERE kind='unknown-typed-values' ORDER BY version")
        .fetch_all(catalog.pool())
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    for (index, row) in rows.iter().enumerate() {
        assert_eq!(row.try_get::<i64, _>("version").unwrap(), IDS[index]);
        assert_eq!(row.try_get::<i64, _>("ordinal").unwrap(), IDS[index]);
        assert_eq!(row.try_get::<Vec<u8>, _>("original").unwrap(), ORIGINAL);
    }
    for table in ["catalog_pictures", "catalog_albums"] {
        let rows = sqlx::query(AssertSqlSafe(format!(
            "SELECT * FROM {table} ORDER BY version"
        )))
        .fetch_all(catalog.pool())
        .await
        .unwrap();
        assert_eq!(rows.len(), 4);
        for (index, row) in rows.iter().enumerate() {
            assert_eq!(logical_text(catalog, row, "name"), NAME.as_bytes());
            let uuid: String = row.try_get("uuid").unwrap();
            assert_eq!(
                catalog.decode_source_key(uuid).unwrap(),
                if table == "catalog_pictures" {
                    KEY
                } else {
                    "album\0雪"
                }
            );
            if table == "catalog_pictures" {
                assert_eq!(logical_text(catalog, row, "caption"), CAPTION.as_bytes());
                assert_eq!(row.try_get::<i64, _>("rating").unwrap(), IDS[index]);
                assert_eq!(row.try_get::<i64, _>("flag_status").unwrap(), IDS[index]);
                for (column, expected) in [("bin_uuid", "bin\0雪"), ("event_uuid", "event\0雪")] {
                    assert_eq!(
                        catalog
                            .decode_source_key(row.try_get(column).unwrap())
                            .unwrap(),
                        expected
                    );
                }
            } else {
                assert_eq!(row.try_get::<i64, _>("sort_order").unwrap(), IDS[index]);
            }
        }
    }
    let indices: Vec<i64> =
        sqlx::query_scalar("SELECT picture_index FROM catalog_memberships ORDER BY version")
            .fetch_all(catalog.pool())
            .await
            .unwrap();
    assert_eq!(indices, IDS);
    let ordinals: Vec<i64> =
        sqlx::query_scalar("SELECT ordinal FROM application_job_order ORDER BY ordinal")
            .fetch_all(catalog.pool())
            .await
            .unwrap();
    assert_eq!(ordinals, job_order);
    let jobs: Vec<String> = sqlx::query_scalar(
        "SELECT job FROM application_job_order WHERE kind='intake' ORDER BY ordinal",
    )
    .fetch_all(catalog.pool())
    .await
    .unwrap();
    assert_eq!(
        jobs,
        (0..4)
            .map(|i| format!("synthetic-job-{i}"))
            .collect::<Vec<_>>()
    );
    let ids: Vec<String> = sqlx::query_scalar("SELECT id FROM api_ids ORDER BY version")
        .fetch_all(catalog.pool())
        .await
        .unwrap();
    assert_eq!(
        ids,
        (0..4)
            .map(|i| format!("synthetic-asset-{i}"))
            .collect::<Vec<_>>()
    );
    let pending: String =
        sqlx::query_scalar("SELECT code FROM api_adaptation_errors WHERE job='synthetic-job-0'")
            .fetch_one(catalog.pool())
            .await
            .unwrap();
    assert_eq!(pending, "synthetic-pending");
    let digest = format!("{:x}", Sha256::digest(BLOB));
    assert_eq!(
        fs::read(original_root.join(format!("{digest}-{}", BLOB.len()))).unwrap(),
        BLOB
    );
}
/// Restore through the production owned path and reopen the published result
/// through the normal application connector, never the import connection.
async fn restore_and_reopen(
    destination: &Fixture,
    archive: &Path,
    limits: &Limits,
) -> (Catalog, PathBuf, crate::restore_journal::Journal) {
    let cancel = AtomicBool::new(false);
    let progress = std::sync::atomic::AtomicU64::new(0);
    let export_root = destination.root.join("restore-results");
    directory(&export_root).unwrap();
    restore_owned(
        archive,
        &destination.catalog,
        RestoreOwner {
            root: &export_root,
            job: "synthetic-restore",
            reserved: 1 << 40,
        },
        limits,
        &cancel,
        &progress,
    )
    .await
    .unwrap();
    let journal = crate::restore_journal::Journal::load(&export_root, "synthetic-restore")
        .unwrap()
        .unwrap();
    let result = journal.result(&export_root).join("store");
    let config = if let Some(namespace) = journal.namespace() {
        destination
            .config
            .clone()
            .with_schema(namespace.schema())
            .unwrap()
    } else {
        DatabaseConfig::Sqlite(result.join("catalog.db"))
    };
    (Catalog::open(config).await.unwrap(), result, journal)
}
async fn release(
    destination: &Fixture,
    reopened: Catalog,
    mut journal: crate::restore_journal::Journal,
) {
    reopened.pool().close().await;
    let export_root = destination.root.join("restore-results");
    journal
        .cleanup(&destination.catalog, &export_root)
        .await
        .unwrap();
    journal.forget_after_quota_release(&export_root).unwrap();
}
async fn round_trip(source_pg: bool, destination_pg: bool) {
    let source = Fixture::new(source_pg).await;
    let store = source.seed().await;
    verify(&source.catalog, &source.root.join("originals")).await;
    let destination = Fixture::new(destination_pg).await;
    let archive = source.root.join("synthetic.bundle");
    let cancel = AtomicBool::new(false);
    let progress = std::sync::atomic::AtomicU64::new(0);
    backup(
        &source.catalog,
        &store,
        &archive,
        u64::MAX,
        &cancel,
        &progress,
    )
    .await
    .unwrap();
    let (reopened, result, journal) =
        restore_and_reopen(&destination, &archive, store.limits()).await;
    verify(&reopened, &result.join("originals")).await;
    assert!(logical_tables(&source.catalog, &[]).await == logical_tables(&reopened, &[]).await);
    release(&destination, reopened, journal).await;
    source.finish().await;
    destination.finish().await;
}
#[tokio::test]
async fn sqlite_to_sqlite_exact_logical_round_trip_and_reopen() {
    round_trip(false, false).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL service"]
async fn postgres_matrix_sqlite_to_postgres_exact_logical_round_trip_and_reopen() {
    round_trip(false, true).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL service"]
async fn postgres_matrix_postgres_to_sqlite_exact_logical_round_trip_and_reopen() {
    round_trip(true, false).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL service"]
async fn postgres_matrix_postgres_to_postgres_exact_logical_round_trip_and_reopen() {
    round_trip(true, true).await;
}

// ---- Library size is not bounded by the single-upload budgets (lytronaut-lbk) --

/// A library whose originals exceed every former budget-derived ceiling backs
/// up and restores losslessly; each transfer moves bytes through the fixed copy
/// buffer, never a size-proportional one.
async fn beyond_upload_budget_round_trip(source_pg: bool, destination_pg: bool) {
    let source = Fixture::new(source_pg).await;
    let store = source.seed_with(small_budgets()).await;
    let limits = store.limits().clone();
    let originals = source.add_sparse_originals(SPARSE_COUNT, SPARSE_SIZE).await;
    let catalog_maximum = lytronaut_storage::portable::MAX_BYTES as u64;
    assert!(originals > limits.upload_bytes + catalog_maximum);
    assert!(originals > limits.reserved_bytes);
    assert!(SPARSE_COUNT + 1 > u64::from(limits.files));
    let destination = Fixture::new(destination_pg).await;
    let archive = source.root.join("beyond-upload-budget.bundle");
    let cancel = AtomicBool::new(false);
    let progress = std::sync::atomic::AtomicU64::new(0);
    let report = backup(
        &source.catalog,
        &store,
        &archive,
        u64::MAX,
        &cancel,
        &progress,
    )
    .await
    .unwrap();
    assert_eq!(report.files, SPARSE_COUNT + 1);
    let size = fs::metadata(&archive).unwrap().len();
    assert!(
        size > limits.encoded_bytes
            + catalog_maximum
            + archive_framing(u64::from(limits.files)).unwrap()
    );
    let (reopened, result, journal) = restore_and_reopen(&destination, &archive, &limits).await;
    verify(&reopened, &result.join("originals")).await;
    verify_sparse_originals(&result.join("originals"), SPARSE_COUNT, SPARSE_SIZE);
    assert!(logical_tables(&source.catalog, &[]).await == logical_tables(&reopened, &[]).await);
    fs::remove_file(&archive).unwrap();
    release(&destination, reopened, journal).await;
    source.finish().await;
    destination.finish().await;
}
#[tokio::test]
async fn sqlite_library_beyond_upload_budget_round_trips_losslessly() {
    beyond_upload_budget_round_trip(false, false).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL service"]
async fn postgres_matrix_library_beyond_upload_budget_sqlite_to_postgres() {
    beyond_upload_budget_round_trip(false, true).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL service"]
async fn postgres_matrix_library_beyond_upload_budget_postgres_to_sqlite() {
    beyond_upload_budget_round_trip(true, false).await;
}

/// Version-1 bundles keep their 32 MiB catalog-file cap but their originals are
/// no longer bounded by `upload_bytes` + 32 MiB or the file by `encoded_bytes`.
#[tokio::test]
async fn legacy_library_beyond_upload_budget_restores_onto_sqlite() {
    const SIZE: u64 = 4 * 1024 * 1024;
    const COUNT: u64 = 10;
    let limits = Limits {
        upload_bytes: SIZE,
        file_bytes: SIZE,
        encoded_bytes: SIZE,
        files: 2,
        free_floor: 0,
        ..Default::default()
    };
    let source = Fixture::new(false).await;
    source.seed_with(limits.clone()).await;
    let originals = source.add_sparse_originals(COUNT, SIZE).await;
    assert!(originals > limits.upload_bytes + MAX_DB);
    let database = source.root.join("legacy.db");
    sqlx::query("VACUUM INTO $1")
        .bind(database.to_str().unwrap())
        .execute(source.catalog.pool())
        .await
        .unwrap();
    let (database_hash, database_bytes) = hash_file(&database).unwrap();
    let mut inventory = vec![(format!("{:x}", Sha256::digest(BLOB)), BLOB.len() as u64)];
    inventory.extend(
        sparse_digests(COUNT, SIZE)
            .into_iter()
            .map(|digest| (digest, SIZE)),
    );
    let input = source.root.join("legacy.bundle");
    let mut out = new_file(&input).unwrap();
    out.write_all(MAGIC).unwrap();
    out.write_all(&(inventory.len() as u32).to_le_bytes())
        .unwrap();
    out.write_all(&database_bytes.to_le_bytes()).unwrap();
    out.write_all(&database_hash).unwrap();
    std::io::copy(&mut File::open(&database).unwrap(), &mut out).unwrap();
    for (digest, length) in &inventory {
        out.write_all(&lytronaut_storage::portable_database::digest_bytes(digest).unwrap())
            .unwrap();
        out.write_all(&length.to_le_bytes()).unwrap();
        let blob = source
            .root
            .join("originals")
            .join(format!("{digest}-{length}"));
        std::io::copy(&mut File::open(blob).unwrap(), &mut out).unwrap();
    }
    out.sync_all().unwrap();
    drop(out);
    assert!(fs::metadata(&input).unwrap().len() > limits.encoded_bytes + MAX_DB);
    let destination = Fixture::new(false).await;
    let (reopened, result, journal) = restore_and_reopen(&destination, &input, &limits).await;
    verify(&reopened, &result.join("originals")).await;
    verify_sparse_originals(&result.join("originals"), COUNT, SIZE);
    assert!(
        logical_tables(&source.catalog, &["application_job_order"]).await
            == logical_tables(&reopened, &["application_job_order"]).await
    );
    release(&destination, reopened, journal).await;
    source.finish().await;
    destination.finish().await;
}

/// The inventory is checked as it streams: out-of-order records, a repeated
/// original and a catalog/inventory mismatch all fail closed.
#[tokio::test]
async fn inventory_disorder_duplicates_and_mismatch_fail_closed() {
    let source = Fixture::new(false).await;
    let store = source.seed().await;
    source.add_sparse_originals(2, 1024).await;
    let archive = source.root.join("ordered.bundle");
    backup(
        &source.catalog,
        &store,
        &archive,
        u64::MAX,
        &AtomicBool::new(false),
        &std::sync::atomic::AtomicU64::new(0),
    )
    .await
    .unwrap();
    let bytes = fs::read(&archive).unwrap();
    let head = 60 + u64::from_le_bytes(bytes[20..28].try_into().unwrap()) as usize;
    let mut segments = Vec::new();
    let mut at = head;
    while at < bytes.len() {
        let length = u64::from_le_bytes(bytes[at + 32..at + 40].try_into().unwrap()) as usize;
        segments.push(bytes[at..at + 40 + length].to_vec());
        at += 40 + length;
    }
    assert_eq!(segments.len(), 3);
    let forge = |count: u32, parts: &[usize]| {
        let mut forged = bytes[..head].to_vec();
        forged[16..20].copy_from_slice(&count.to_le_bytes());
        for &part in parts {
            forged.extend_from_slice(&segments[part]);
        }
        forged
    };
    // A well-formed segment for an original the catalog does not list, whose
    // digest sorts after `after` so only the catalog comparison can reject it.
    let foreign = |after: &[u8]| {
        (0u64..)
            .map(|n| format!("foreign original {n}").into_bytes())
            .find_map(|payload| {
                let hash: [u8; 32] = Sha256::digest(&payload).into();
                (hash.as_slice() > after).then(|| {
                    let mut segment = hash.to_vec();
                    segment.extend_from_slice(&(payload.len() as u64).to_le_bytes());
                    segment.extend_from_slice(&payload);
                    segment
                })
            })
            .unwrap()
    };
    let mut extra = forge(4, &[0, 1, 2]);
    extra.extend_from_slice(&foreign(&segments[2][..32]));
    let mut replaced = forge(3, &[0, 1]);
    replaced.extend_from_slice(&foreign(&segments[1][..32]));
    for (forged, expected) in [
        (forge(3, &[1, 0, 2]), E::Invalid),
        (forge(3, &[0, 0, 2]), E::Quota),
        (forge(2, &[0, 1]), E::Conflict),
        (extra, E::Conflict),
        (replaced, E::Conflict),
    ] {
        let destination = Fixture::new(false).await;
        let input = destination.root.join("forged.bundle");
        fs::write(&input, forged).unwrap();
        let export_root = destination.root.join("restore-results");
        directory(&export_root).unwrap();
        let result = restore_owned(
            &input,
            &destination.catalog,
            RestoreOwner {
                root: &export_root,
                job: "synthetic-forged",
                reserved: 1 << 40,
            },
            store.limits(),
            &AtomicBool::new(false),
            &std::sync::atomic::AtomicU64::new(0),
        )
        .await;
        assert!(
            matches!(
                (&result, &expected),
                (Err(E::Invalid), E::Invalid)
                    | (Err(E::Quota), E::Quota)
                    | (Err(E::Conflict), E::Conflict)
            ),
            "{result:?} != {expected:?}"
        );
        destination.finish().await;
    }
    // A catalog digest that is not lowercase hex cannot be framed: backup
    // fails before any bundle exists.
    sqlx::query("INSERT INTO intake_sources(job,path,canonical,digest,length) VALUES('synthetic-job-2','malformed','malformed',$1,1)")
        .bind("Z".repeat(64))
        .execute(source.catalog.pool())
        .await
        .unwrap();
    let rejected = source.root.join("rejected.bundle");
    assert!(matches!(
        backup(
            &source.catalog,
            &store,
            &rejected,
            u64::MAX,
            &AtomicBool::new(false),
            &std::sync::atomic::AtomicU64::new(0),
        )
        .await,
        Err(E::Invalid)
    ));
    assert!(!rejected.exists());
    assert!(!rejected.with_extension("snapshot").exists());
    assert!(!rejected.with_extension("inventory").exists());
    source.finish().await;
}

/// lytronaut-lbk B1, enforcement: a restore charges every file at its real
/// block cost against its reservation and stops with a retryable `Space`
/// before passing it; owned cleanup then leaves nothing. A backup does the
/// same against its own reservation.
#[tokio::test]
async fn disk_reservations_are_enforced_per_file_while_writing() {
    let source = Fixture::new(false).await;
    let store = source.seed().await;
    source.add_sparse_originals(4, 1024 * 1024).await;
    let archive = source.root.join("charged.bundle");
    let none = AtomicBool::new(false);
    let progress = std::sync::atomic::AtomicU64::new(0);
    // Backup: a reservation smaller than the artifact stops it; the
    // artifact and both spools are removed.
    let small = source.root.join("small.bundle");
    let result = backup(
        &source.catalog,
        &store,
        &small,
        2 * 1024 * 1024,
        &none,
        &progress,
    )
    .await;
    assert!(matches!(result, Err(E::Space { .. })), "{result:?}");
    for path in [
        small.clone(),
        small.with_extension("snapshot"),
        small.with_extension("inventory"),
    ] {
        assert!(!path.exists());
    }
    backup(
        &source.catalog,
        &store,
        &archive,
        u64::MAX,
        &none,
        &progress,
    )
    .await
    .unwrap();
    let (unit, _) = block_unit(&source.root).unwrap();
    let (count, need) = restore_disk_need(&archive, unit, u64::MAX, &none).unwrap();
    assert_eq!(count, 5);
    // Exactly the documented model: upload, SQLite headroom, job files,
    // both spools and every original at its block cost.
    let upload = fs::metadata(&archive).unwrap().len();
    let catalog = u64::from_le_bytes(fs::read(&archive).unwrap()[20..28].try_into().unwrap());
    let originals =
        4 * file_cost(1024 * 1024, unit).unwrap() + file_cost(BLOB.len() as u64, unit).unwrap();
    assert_eq!(
        need,
        file_cost(upload, unit).unwrap()
            + lytronaut_storage::portable::MAX_BYTES as u64
            + job_cost(unit).unwrap()
            + file_cost(catalog, unit).unwrap()
            + file_cost(5 * 40, unit).unwrap()
            + originals
    );
    // A reservation one block short of that need fails mid-extraction.
    let destination = Fixture::new(false).await;
    let export_root = destination.root.join("restore-results");
    directory(&export_root).unwrap();
    let result = restore_owned(
        &archive,
        &destination.catalog,
        RestoreOwner {
            root: &export_root,
            job: "synthetic-charged",
            reserved: need - unit,
        },
        store.limits(),
        &none,
        &progress,
    )
    .await;
    assert!(matches!(result, Err(E::Space { .. })), "{result:?}");
    let mut journal = crate::restore_journal::Journal::load(&export_root, "synthetic-charged")
        .unwrap()
        .unwrap();
    journal
        .cleanup(&destination.catalog, &export_root)
        .await
        .unwrap();
    journal.forget_after_quota_release(&export_root).unwrap();
    assert_eq!(
        fs::read_dir(&export_root).unwrap().count(),
        0,
        "nothing left behind"
    );
    // The exact need restores.
    let result = restore_owned(
        &archive,
        &destination.catalog,
        RestoreOwner {
            root: &export_root,
            job: "synthetic-exact",
            reserved: need,
        },
        store.limits(),
        &none,
        &progress,
    )
    .await;
    assert!(result.is_ok(), "{result:?}");
    let mut journal = crate::restore_journal::Journal::load(&export_root, "synthetic-exact")
        .unwrap()
        .unwrap();
    journal
        .cleanup(&destination.catalog, &export_root)
        .await
        .unwrap();
    journal.forget_after_quota_release(&export_root).unwrap();
    // Framing that cannot be in the upload is invalid before anything is summed.
    let mut bytes = fs::read(&archive).unwrap();
    bytes[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    let forged = destination.root.join("forged-count.bundle");
    fs::write(&forged, &bytes).unwrap();
    assert!(matches!(
        restore_disk_need(&forged, unit, u64::MAX, &none),
        Err(E::Invalid)
    ));
    bytes.truncate(bytes.len() - 1);
    bytes[16..20].copy_from_slice(&5u32.to_le_bytes());
    fs::write(&forged, &bytes).unwrap();
    assert!(matches!(
        restore_disk_need(&forged, unit, u64::MAX, &none),
        Err(E::Invalid)
    ));
    assert!(matches!(
        restore_disk_need(&archive, unit, 1, &none),
        Err(E::Quota)
    ));
    destination.finish().await;
    source.finish().await;
}

/// lytronaut-lbk B1, honest sizing for a real library shape: about 33,800
/// captures with the surveyed corpus's companions (6.5 files per capture on
/// average). Block rounding alone is under 0.1%; the separate metadata
/// allowance raises the combined modeled overhead just above 0.1%, within
/// 0.11%. Keep the payload shape fixed when updating accounting.
#[test]
fn library_shaped_restore_need_includes_metadata_within_point_one_one_percent() {
    let unit = 4096;
    let captures = 33_800u64;
    let mut files = 0u64;
    let mut payload = 0u64;
    let mut cost = 0u64;
    for capture in 0..captures {
        let mut sizes = vec![16_000_000, 2_000_000, 3_000_000, 3_000_000, 30_000, 1_500];
        if capture % 2 == 0 {
            sizes.push(1_000_000);
        }
        for size in sizes {
            files += 1;
            payload += size;
            cost += file_cost(size, unit).unwrap();
        }
    }
    let catalog = 8_934 * captures;
    assert!(
        catalog <= lytronaut_storage::portable::MAX_BYTES as u64,
        "library-shaped catalog must fit its format section"
    );
    let upload = 60 + catalog + 40 * files + payload;
    let need = file_cost(upload, unit).unwrap()
        + lytronaut_storage::portable::MAX_BYTES as u64
        + job_cost(unit).unwrap()
        + file_cost(catalog, unit).unwrap()
        + file_cost(40 * files, unit).unwrap()
        + cost;
    assert_eq!(files, 219_700);
    assert_eq!(
        payload, 829_164_700_000,
        "preserve the existing payload shape"
    );
    let metadata = files * FILE_METADATA;
    assert_eq!(metadata, 224_972_800);
    assert_eq!(cost - payload, 829_181_600);
    assert!(
        (cost - metadata - payload) * 1000 < payload,
        "data-block rounding remains under 0.1%"
    );
    assert!(
        (cost - payload) * 10_000 < 11 * payload,
        "rounding plus metadata under 0.11%"
    );
    assert!(need > restore_need(upload).unwrap());
    assert!((need - restore_need(upload).unwrap()) * 10_000 < 11 * upload);
}

/// L6: a library with no originals backs up and restores (zero records).
#[tokio::test]
async fn library_without_originals_round_trips() {
    let source = Fixture::new(false).await;
    let store = Store::open(
        source.catalog.clone(),
        &source.root.join("originals"),
        &source.root.join("staging"),
        Limits {
            free_floor: 0,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let archive = source.root.join("empty.bundle");
    let none = AtomicBool::new(false);
    let progress = std::sync::atomic::AtomicU64::new(0);
    let report = backup(
        &source.catalog,
        &store,
        &archive,
        u64::MAX,
        &none,
        &progress,
    )
    .await
    .unwrap();
    assert_eq!(report.files, 0);
    assert_eq!(
        u32::from_le_bytes(fs::read(&archive).unwrap()[16..20].try_into().unwrap()),
        0
    );
    let destination = Fixture::new(false).await;
    let (reopened, result, journal) =
        restore_and_reopen(&destination, &archive, store.limits()).await;
    let sources: i64 = sqlx::query_scalar("SELECT count(*) FROM intake_sources")
        .fetch_one(reopened.pool())
        .await
        .unwrap();
    assert_eq!(sources, 0);
    assert_eq!(fs::read_dir(result.join("originals")).unwrap().count(), 0);
    release(&destination, reopened, journal).await;
    source.finish().await;
    destination.finish().await;
}

/// CI-only proof (`just large-bundle-test`): one original larger than 4 GiB and
/// a bundle whose stream offsets pass 4 GiB round-trip with exact hashes. The
/// source is sparse; the bundle and the restored copy need about 9 GiB of
/// real scratch space in TMPDIR, so it is not part of the default suite.
#[tokio::test]
#[ignore = "explicit >4 GiB proof; run by `just large-bundle-test` on a large scratch disk"]
async fn large_bundle_beyond_4_gib_round_trips_losslessly() {
    const GIB: u64 = 1024 * 1024 * 1024;
    let sizes = [4 * GIB + 65_537, 64 * 1024 * 1024, 64 * 1024 * 1024 + 1];
    let limits = Limits {
        upload_bytes: 5 * GIB,
        file_bytes: 5 * GIB,
        files: 2,
        free_floor: 0,
        ..Default::default()
    };
    let source = Fixture::new(false).await;
    let store = source.seed_with(limits.clone()).await;
    let originals = source.add_sparse_sized(&sizes).await;
    assert!(originals > 4 * GIB && sizes[0] > u64::from(u32::MAX));
    let destination = Fixture::new(false).await;
    let archive = source.root.join("beyond-4-gib.bundle");
    let cancel = AtomicBool::new(false);
    let progress = std::sync::atomic::AtomicU64::new(0);
    let report = backup(
        &source.catalog,
        &store,
        &archive,
        u64::MAX,
        &cancel,
        &progress,
    )
    .await
    .unwrap();
    assert_eq!(report.files, sizes.len() as u64 + 1);
    assert!(fs::metadata(&archive).unwrap().len() > originals);
    let (reopened, result, journal) = restore_and_reopen(&destination, &archive, &limits).await;
    fs::remove_file(&archive).unwrap();
    verify(&reopened, &result.join("originals")).await;
    verify_sparse_sized(&result.join("originals"), &sizes);
    assert!(logical_tables(&source.catalog, &[]).await == logical_tables(&reopened, &[]).await);
    release(&destination, reopened, journal).await;
    source.finish().await;
    destination.finish().await;
}

// ---- Logical catalog capacity (288 MiB v2 section) ----------------------------

/// Counts written bytes and keeps none.
struct Counting(u64);
impl Write for Counting {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0 += bytes.len() as u64;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
/// Captures in the measured synthetic Desktop library.
const MEASURED_CAPTURES: u64 = 1000;
/// Companions per capture: the six required ones plus `stacklq.lfp`, which the
/// surveyed corpus has for about half its captures (counted for all here).
const COMPANIONS: [&str; 7] = [
    "raw.lfp",
    "dm.lfp",
    "stack.lfp",
    "stack.view.lfp",
    "stacklq.lfp",
    "thumbnail.jpg",
    "view_parameters.json",
];
/// A realistic, deliberately generous Lytro Desktop (schema 23) library:
/// 21 picture and 30 picture-metadata columns with real-length UUIDs, names,
/// ISO dates, serial/camera/lens/exposure fields, three albums covering every
/// capture, three events, two bins, eleven picture lists and an import-history
/// hash per capture, all within the adapter's 100,000-cell database bound.
fn realistic_desktop_sql(captures: u64) -> String {
    let uuid = |kind: u32, n: &str| format!("printf('%08x-0000-4000-8000-%012x',{kind},{n})");
    let picture = uuid(1, "i");
    let bin = uuid(2, "i%2");
    let event = uuid(3, "i%3");
    let album = uuid(4, "i%3");
    let range =
        format!("WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<{captures})");
    format!("CREATE TABLE library_metadata(id INTEGER,version INTEGER);INSERT INTO library_metadata VALUES(1,23);
CREATE TABLE picture(id INTEGER,uuid TEXT,hash TEXT,imagebin_uuid TEXT,event_uuid TEXT,name TEXT,caption TEXT,rating INTEGER,flag_status INTEGER,capture_date TEXT,import_date TEXT,modified_date TEXT,orientation INTEGER,broken_state INTEGER,protected INTEGER,lfe_state INTEGER,image_width INTEGER,image_height INTEGER,cropped_width INTEGER,cropped_height INTEGER,camera_model INTEGER);
INSERT INTO picture {range} SELECT i,{picture},'',{bin},{event},printf('Living Picture %05d',i),'',0,0,'2014-06-21T14:03:22.000Z','2014-07-01T09:12:45.000Z','2014-07-02T10:00:00.000Z',0,0,0,i%3,0,0,0,0,1 FROM n;
CREATE TABLE picture_metadata(id INTEGER,uuid TEXT,lfe_major INTEGER,lfe_minor INTEGER,orientation INTEGER,rotation INTEGER,serial_number TEXT,camera_make TEXT,camera_model TEXT,firmware TEXT,capture_date TEXT,timezone TEXT,exposure_duration REAL,iso INTEGER,f_number REAL,focal_length REAL,zoom REAL,white_balance INTEGER,focus REAL,depth_score REAL,lambda_min REAL,lambda_max REAL,latitude REAL,longitude REAL,altitude REAL,capabilities TEXT,refocus_range TEXT,title TEXT,description TEXT,flags INTEGER);
INSERT INTO picture_metadata {range} SELECT i,{picture},10+i%2,0,0,0,'SYN0000000001','Lytro','F01','1.2.2.1','2014-06-21T14:03:22.000Z','-07:00',0.004,80,2.0,6.45,1.0,0,0.53,0.71,-4.3,9.6,37.4419,-122.143,12.5,'refocus,perspective,depth','[-4.3,9.6]',printf('Living Picture %05d',i),'',0 FROM n;
CREATE TABLE album(id INTEGER,uuid TEXT,name TEXT,sort_order INTEGER,public INTEGER);
INSERT INTO album {range} SELECT i,{album_i},printf('Synthetic album %d',i),i,1 FROM n WHERE i<=3;
CREATE TABLE album_to_picture(id INTEGER,album_uuid TEXT,picture_uuid TEXT,picture_index INTEGER);
INSERT INTO album_to_picture {range} SELECT i,{album},{picture},i/3 FROM n;
CREATE TABLE event(id INTEGER,uuid TEXT,library_uuid TEXT,title TEXT);
INSERT INTO event {range} SELECT i,{event_i},'00000005-0000-4000-8000-000000000001',printf('Synthetic event %d',i) FROM n WHERE i<=3;
CREATE TABLE image_bin(uuid TEXT);
INSERT INTO image_bin {range} SELECT {bin_i} FROM n WHERE i<=2;
CREATE TABLE picture_list(id INTEGER,uuid TEXT,filter INTEGER,sort_order INTEGER);
INSERT INTO picture_list {range} SELECT i,{list},i-1,1 FROM n WHERE i<=11;
CREATE TABLE imported_picture_history(id INTEGER,hash TEXT);
INSERT INTO imported_picture_history {range} SELECT i,printf('%040x',i) FROM n;",
        album_i = uuid(4, "i-1"),
        event_i = uuid(3, "i-1"),
        bin_i = uuid(2, "i-1"),
        list = uuid(6, "i"),
    )
}
/// Measure the logical catalog bytes per capture through the real Desktop
/// intake, adaptation and API mapping, then capture. The 288 MiB v2 section
/// therefore holds about `MAX_BYTES / bytes_per_capture` captures; this test
/// fails if that capacity drops below the documented figure.
#[tokio::test]
async fn logical_catalog_capacity_measured_on_realistic_library() {
    let fixture = Fixture::new(false).await;
    let files = (MEASURED_CAPTURES as u32 + 1) * COMPANIONS.len() as u32 + 1;
    let store = Store::open(
        fixture.catalog.clone(),
        &fixture.root.join("originals"),
        &fixture.root.join("staging"),
        Limits {
            files,
            free_floor: 0,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let database = fixture.root.join("lytrolibrary.db");
    let mut connection = SqliteConnection::connect_with(
        &SqliteConnectOptions::new()
            .filename(&database)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::raw_sql(AssertSqlSafe(realistic_desktop_sql(MEASURED_CAPTURES)))
        .execute(&mut connection)
        .await
        .unwrap();
    let rows = sqlx::query("SELECT imagebin_uuid,uuid FROM picture ORDER BY id")
        .fetch_all(&mut connection)
        .await
        .unwrap()
        .into_iter()
        .map(|r| (r.get::<String, _>(0), r.get::<String, _>(1)))
        .collect::<Vec<_>>();
    connection.close().await.unwrap();
    let job = store
        .begin(&lytronaut_contracts::ImportRequest {
            version: lytronaut_contracts::WireVersion::V1,
            kind: lytronaut_contracts::ImportKind::Directory,
            declared_bytes: 1024 * 1024 * 1024,
            declared_files: files,
        })
        .await
        .unwrap();
    store
        .append_directory_file(
            &job,
            "Lytro Library/lytrolibrary.db",
            std::io::Cursor::new(fs::read(&database).unwrap()),
        )
        .await
        .unwrap();
    for (bin, picture) in &rows {
        for companion in COMPANIONS {
            let path = format!("Lytro Library/{bin}/{picture}/{companion}");
            store
                .append_directory_file(&job, &path, std::io::Cursor::new(path.clone().into_bytes()))
                .await
                .unwrap();
        }
    }
    store.finish(&job).await.unwrap();
    crate::api::adapt(&fixture.catalog, &store, &job)
        .await
        .unwrap();
    let pictures: i64 = sqlx::query_scalar("SELECT count(*) FROM catalog_pictures")
        .fetch_one(fixture.catalog.pool())
        .await
        .unwrap();
    assert_eq!(
        pictures as u64, MEASURED_CAPTURES,
        "the adapter must accept the library"
    );
    let (counted, _, inventory) = lytronaut_storage::portable_database::capture(
        &fixture.catalog,
        Counting(0),
        std::io::sink(),
        &AtomicBool::new(false),
    )
    .await
    .unwrap();
    assert_eq!(
        inventory.count,
        MEASURED_CAPTURES * COMPANIONS.len() as u64 + 1
    );
    let per_capture = counted.0.div_ceil(MEASURED_CAPTURES);
    let capacity = lytronaut_storage::portable::MAX_BYTES as u64 / per_capture;
    eprintln!(
        "logical catalog: {} bytes for {MEASURED_CAPTURES} captures = {per_capture} bytes per capture; 288 MiB holds about {capacity} captures",
        counted.0
    );
    assert!(
        capacity >= CATALOG_CAPACITY_CAPTURES,
        "{per_capture} bytes per capture; capacity {capacity}"
    );
    fixture.finish().await;
}
/// Regression floor under the measured ~33,800 captures (8,937 bytes per
/// capture); documented in docs/design/007 and docs/operations/linux.md.
const CATALOG_CAPACITY_CAPTURES: u64 = 30_000;

// ---- Version-1 (LYTRONAUT-BNDL01) compatibility -------------------------------
// The legacy payload is a SQLite file authored here with plain SQL against the
// trusted application schema, never by the portable encoder or a restore helper.

/// Version-1's acceptance bound is its 32 MiB SQLite FILE, not any v2 cap.
const LEGACY_FILE_LIMIT: u64 = 32 * 1024 * 1024;
const LEGACY_CREDENTIAL: &str = "synthetic-legacy-credential-7f3a";
#[derive(Clone, Copy, PartialEq)]
enum Bulk {
    None,
    /// One opaque record whose single cell nearly fills the legacy file.
    LargeCell,
    /// Many tiny records that together nearly fill the legacy file.
    SmallRows,
}
const LARGE_CELL: usize = 31 * 1024 * 1024 + 256 * 1024;
fn large_original() -> Vec<u8> {
    (0..LARGE_CELL as u64)
        .map(|i| (i.wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 56) as u8)
        .collect()
}
fn small_original(ordinal: i64) -> Vec<u8> {
    format!("{ordinal:016x}").into_bytes()
}
/// Build an exact accepted v1 form: `has_order` selects current vs pre-job-order,
/// `has_auth` adds the three known legacy authentication tables with synthetic
/// credential rows that restore must never carry forward. Returns the archive
/// bytes and the number of near-limit small rows.
type Logical = Vec<(&'static str, Vec<Vec<String>>)>;
async fn legacy_archive(
    has_order: bool,
    has_auth: bool,
    bulk: Bulk,
) -> (Vec<u8>, i64, Option<Logical>) {
    let source = Fixture::new(false).await;
    source.seed().await;
    let mut small_rows = 0;
    match bulk {
        Bulk::None => {}
        Bulk::LargeCell => {
            sqlx::query("INSERT INTO catalog_records(version,kind,ordinal,original) VALUES($1,'near-limit-large',0,$2)")
                .bind(IDS[1])
                .bind(large_original())
                .execute(source.catalog.pool())
                .await
                .unwrap();
        }
        Bulk::SmallRows => loop {
            // Measure the compacted file actually shipped, not live page usage,
            // and grow by the remaining gap so the fixture never overshoots.
            let probe = source.root.join("probe.db");
            let _ = fs::remove_file(&probe);
            sqlx::query("VACUUM INTO $1")
                .bind(probe.to_str().unwrap())
                .execute(source.catalog.pool())
                .await
                .unwrap();
            let size = fs::metadata(&probe).unwrap().len();
            fs::remove_file(&probe).unwrap();
            let target = LEGACY_FILE_LIMIT - 512 * 1024;
            if size >= target {
                break;
            }
            // Rows cost ~70 bytes with their index entry; 128 under-fills each step.
            let batch = ((target - size) / 128).max(1_000) as i64;
            sqlx::query("INSERT INTO catalog_records(version,kind,ordinal,original) WITH RECURSIVE n(i) AS (SELECT $1 UNION ALL SELECT i+1 FROM n WHERE i<$2) SELECT $3,'near-limit-small',i,CAST(printf('%016x',i) AS BLOB) FROM n")
                .bind(small_rows + 1)
                .bind(small_rows + batch)
                .bind(IDS[2])
                .execute(source.catalog.pool())
                .await
                .unwrap();
            small_rows += batch;
        },
    }
    let file = source.root.join("legacy.db");
    sqlx::query("VACUUM INTO $1")
        .bind(file.to_str().unwrap())
        .execute(source.catalog.pool())
        .await
        .unwrap();
    let mut db = SqliteConnection::connect_with(&SqliteConnectOptions::new().filename(&file))
        .await
        .unwrap();
    if !has_order {
        sqlx::query("DROP TABLE application_job_order")
            .execute(&mut db)
            .await
            .unwrap();
    }
    if has_auth {
        for (_, sql) in LEGACY_AUTH {
            sqlx::raw_sql(AssertSqlSafe(sql))
                .execute(&mut db)
                .await
                .unwrap();
        }
        sqlx::query("INSERT INTO auth_account VALUES(1,$1,$1,$1)")
            .bind(LEGACY_CREDENTIAL)
            .execute(&mut db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO auth_sessions VALUES($1,1,1)")
            .bind(LEGACY_CREDENTIAL)
            .execute(&mut db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO auth_attempts VALUES($1,1,1)")
            .bind(LEGACY_CREDENTIAL)
            .execute(&mut db)
            .await
            .unwrap();
    }
    sqlx::query("VACUUM").execute(&mut db).await.unwrap();
    db.close().await.unwrap();
    let database = fs::read(&file).unwrap();
    assert!(database.len() as u64 <= LEGACY_FILE_LIMIT);
    if bulk != Bulk::None {
        // Near-limit means within 1 MiB of the legacy file allowance.
        assert!(
            database.len() as u64 >= LEGACY_FILE_LIMIT - 1024 * 1024,
            "near-limit fixture is only {} bytes",
            database.len()
        );
    }
    // Header: magic, original count, database size, database SHA-256; then the
    // database, then (SHA-256, length, bytes) for each inventory original.
    let mut archive = MAGIC.to_vec();
    archive.extend_from_slice(&1u32.to_le_bytes());
    archive.extend_from_slice(&(database.len() as u64).to_le_bytes());
    archive.extend_from_slice(&Sha256::digest(&database));
    archive.extend_from_slice(&database);
    archive.extend_from_slice(&Sha256::digest(BLOB));
    archive.extend_from_slice(&(BLOB.len() as u64).to_le_bytes());
    archive.extend_from_slice(BLOB);
    // Pre-job-order forms derive ordinals from rowid, so the auxiliary order
    // table is compared by intake order in verify_ordered, not here.
    let logical = if bulk == Bulk::None {
        Some(logical_tables(&source.catalog, &["application_job_order"]).await)
    } else {
        None
    };
    source.finish().await;
    (archive, small_rows, logical)
}
async fn legacy_restore(destination_pg: bool, has_order: bool, has_auth: bool, bulk: Bulk) {
    let (archive, small_rows, logical) = legacy_archive(has_order, has_auth, bulk).await;
    let destination = Fixture::new(destination_pg).await;
    let input = destination.root.join("legacy.bundle");
    fs::write(&input, &archive).unwrap();
    drop(archive);
    let limits = Limits {
        free_floor: 0,
        ..Default::default()
    };
    let (reopened, result, journal) = restore_and_reopen(&destination, &input, &limits).await;
    // Pre-job-order databases derive intake order from intake_jobs rowid order.
    let order: Vec<i64> = if has_order {
        IDS.to_vec()
    } else {
        vec![1, 2, 3, 4]
    };
    verify_ordered(&reopened, &result.join("originals"), &order).await;
    if let Some(logical) = logical {
        assert!(logical == logical_tables(&reopened, &["application_job_order"]).await);
    }
    let auth: i64 = sqlx::query_scalar(reopened.dialect(
        "SELECT count(*) FROM sqlite_master WHERE name LIKE 'auth%'",
        "SELECT count(*) FROM information_schema.tables WHERE table_schema=current_schema() AND table_name LIKE 'auth%'",
    ))
    .fetch_one(reopened.pool())
    .await
    .unwrap();
    assert_eq!(auth, 0, "legacy credentials must never be restored");
    match bulk {
        Bulk::None => {}
        Bulk::LargeCell => {
            let original: Vec<u8> = sqlx::query_scalar(
                "SELECT original FROM catalog_records WHERE kind='near-limit-large'",
            )
            .fetch_one(reopened.pool())
            .await
            .unwrap();
            assert_eq!(original.len(), LARGE_CELL);
            assert!(
                original == large_original(),
                "large legacy cell must be byte-exact"
            );
        }
        Bulk::SmallRows => {
            // Keyset pages keep the verifier's own memory bounded.
            let mut seen = 0;
            loop {
                let rows = sqlx::query(
                    "SELECT version,ordinal,original FROM catalog_records WHERE kind='near-limit-small' AND ordinal>$1 ORDER BY ordinal LIMIT 50000",
                )
                .bind(seen)
                .fetch_all(reopened.pool())
                .await
                .unwrap();
                if rows.is_empty() {
                    break;
                }
                for row in rows {
                    seen += 1;
                    assert_eq!(row.try_get::<i64, _>(0).unwrap(), IDS[2]);
                    assert_eq!(row.try_get::<i64, _>(1).unwrap(), seen);
                    assert_eq!(row.try_get::<Vec<u8>, _>(2).unwrap(), small_original(seen));
                }
            }
            assert_eq!(seen, small_rows);
            assert!(small_rows > 100_000);
        }
    }
    if has_auth {
        // No credential byte survives anywhere in the published result.
        let mut pending = vec![result.clone()];
        while let Some(path) = pending.pop() {
            for entry in fs::read_dir(path).unwrap() {
                let entry = entry.unwrap();
                if entry.file_type().unwrap().is_dir() {
                    pending.push(entry.path());
                } else {
                    let bytes = fs::read(entry.path()).unwrap();
                    assert!(
                        !bytes
                            .windows(LEGACY_CREDENTIAL.len())
                            .any(|w| w == LEGACY_CREDENTIAL.as_bytes()),
                        "credential bytes leaked into the restored result"
                    );
                }
            }
        }
    }
    release(&destination, reopened, journal).await;
    destination.finish().await;
}
/// A version-1 count no 32 MiB catalog file could list is rejected before any
/// original is read; the in-memory check stays bounded by the legacy format.
#[tokio::test]
async fn legacy_count_beyond_its_catalog_file_is_rejected() {
    let (mut archive, _, _) = legacy_archive(true, false, Bulk::None).await;
    archive[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    let destination = Fixture::new(false).await;
    let input = destination.root.join("legacy-count.bundle");
    fs::write(&input, archive).unwrap();
    let target = destination.root.join("legacy-count-target");
    let result = restore(
        &input,
        &target,
        &trusted_schema(&destination.catalog).await.unwrap(),
        &Limits::default(),
        &AtomicBool::new(false),
        &std::sync::atomic::AtomicU64::new(0),
    )
    .await;
    assert!(matches!(result, Err(E::Invalid)), "{result:?}");
    assert!(!target.exists());
    destination.finish().await;
}
#[tokio::test]
async fn legacy_four_exact_forms_restore_onto_sqlite() {
    for (has_order, has_auth) in [(true, false), (true, true), (false, false), (false, true)] {
        legacy_restore(false, has_order, has_auth, Bulk::None).await;
    }
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL service"]
async fn postgres_matrix_legacy_four_exact_forms_restore_onto_postgres() {
    for (has_order, has_auth) in [(true, false), (true, true), (false, false), (false, true)] {
        legacy_restore(true, has_order, has_auth, Bulk::None).await;
    }
}
#[tokio::test]
async fn legacy_near_limit_large_cell_and_small_rows_restore_onto_sqlite() {
    legacy_restore(false, true, true, Bulk::LargeCell).await;
    legacy_restore(false, false, false, Bulk::SmallRows).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL service"]
async fn postgres_matrix_legacy_near_limit_large_cell_and_small_rows_restore_onto_postgres() {
    legacy_restore(true, true, true, Bulk::LargeCell).await;
    legacy_restore(true, false, false, Bulk::SmallRows).await;
}
