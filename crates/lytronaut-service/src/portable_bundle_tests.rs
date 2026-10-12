//! Independent synthetic database-engine differential tests; no private corpus.
use super::*;
use lytronaut_storage::{Backend, DatabaseConfig};
use sqlx::{ConnectOptions, postgres::PgConnectOptions};
use std::str::FromStr;

const IDS: [i64; 4] = [i64::MIN, -7, 0, i64::MAX];
const KEY: &str = "picture\0雪🛰";
const NAME: &str = "Na\0mé雪";
const CAPTION: &str = "Caption\0🛰";
const ORIGINAL: &[u8] = &[0, 255, 128, 0, 0, 0, 0, 0, 0, 0, 128, 127, 1, 2, 3];
const BLOB: &[u8] = b"synthetic original\0\xff\x80";
struct Fixture {
    root: PathBuf,
    catalog: Catalog,
    config: DatabaseConfig,
    pg: Option<(sqlx::PgConnection, String)>,
}
impl Fixture {
    async fn new(postgres: bool) -> Self {
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
    async fn seed(&self) -> Store {
        let store = Store::open(
            self.catalog.clone(),
            &self.root.join("originals"),
            &self.root.join("staging"),
            Limits {
                free_floor: 0,
                ..Default::default()
            },
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
    async fn finish(mut self) {
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
async fn verify(catalog: &Catalog, original_root: &Path) {
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
    let rows = sqlx::query("SELECT version,ordinal,original FROM catalog_records ORDER BY version")
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
    assert_eq!(ordinals, IDS);
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
    let export_root = destination.root.join("restore-results");
    directory(&export_root).unwrap();
    restore_owned(
        &archive,
        &destination.catalog,
        &export_root,
        "synthetic-restore",
        1024 * 1024 * 1024,
        store.limits(),
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
    // Reopen through the normal application connector, not the import connection.
    let reopened = Catalog::open(config).await.unwrap();
    verify(&reopened, &result.join("originals")).await;
    reopened.pool().close().await;
    let mut journal = journal;
    journal
        .cleanup(&destination.catalog, &export_root)
        .await
        .unwrap();
    journal.forget_after_quota_release(&export_root).unwrap();
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
