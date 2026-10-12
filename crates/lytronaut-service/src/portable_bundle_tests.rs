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
            sqlx::query("INSERT INTO intake_jobs(id,kind,state,total,budget,max_files,encoded_limit,reserved) VALUES($1,'directory','succeeded',0,0,1,1,0)").bind(&job).execute(&mut *tx).await.unwrap();
            sqlx::query("INSERT INTO intake_sources(job,path,canonical,digest,length) VALUES($1,'opaque.bin','opaque.bin',$2,$3)").bind(&job).bind(&digest).bind(BLOB.len() as i64).execute(&mut *tx).await.unwrap();
            sqlx::query(
                "INSERT INTO application_job_order(ordinal,kind,job) VALUES($1,'intake',$2)",
            )
            .bind(id)
            .bind(&job)
            .execute(&mut *tx)
            .await
            .unwrap();
            sqlx::query("INSERT INTO catalog_versions(id,fingerprint,source_job,library_uuid,state,reason,conflict) VALUES($1,$2,$3,$4,'supported',NULL,0)").bind(id).bind(format!("synthetic-fingerprint-{index}")).bind(&job).bind(if index == 0 { None } else { Some(self.catalog.encode_source_key("library\0雪")) }).execute(&mut *tx).await.unwrap();
            sqlx::query("INSERT INTO catalog_sources(version,job,path,digest,length) VALUES($1,$2,'opaque.bin',$3,$4)").bind(id).bind(&job).bind(&digest).bind(BLOB.len() as i64).execute(&mut *tx).await.unwrap();
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
fn logical_text(catalog: &Catalog, row: &sqlx::any::AnyRow, column: &str) -> Vec<u8> {
    if catalog.backend() == Backend::Postgres {
        row.try_get(column).unwrap()
    } else {
        row.try_get::<String, _>(column).unwrap().into_bytes()
    }
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
            reserved: 1024 * 1024 * 1024,
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
    backup(&source.catalog, &store, &archive, &cancel, &progress)
        .await
        .unwrap();
    let (reopened, result, journal) =
        restore_and_reopen(&destination, &archive, store.limits()).await;
    verify(&reopened, &result.join("originals")).await;
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
async fn legacy_archive(has_order: bool, has_auth: bool, bulk: Bulk) -> (Vec<u8>, i64) {
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
    source.finish().await;
    (archive, small_rows)
}
async fn legacy_restore(destination_pg: bool, has_order: bool, has_auth: bool, bulk: Bulk) {
    let (archive, small_rows) = legacy_archive(has_order, has_auth, bulk).await;
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
