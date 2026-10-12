//! Backend-neutral restore failure, interruption and cleanup matrix through the
//! production upload/worker/DELETE/restart paths. Synthetic fixtures only.
//!
//! Every scenario runs on SQLite by default and on PostgreSQL through the
//! `postgres_` variants in the strict recipe. Unrelated SQL and filesystem
//! sentinels (including restore-named decoys) must survive every outcome, and
//! the deployment's existing library must remain byte-for-byte intact.
use super::*;
use crate::bundle::portable_bundle_tests::{BLOB, Fixture, IDS, verify};
use crate::restore_journal::{Journal, Phase};
use lytronaut_storage::DatabaseConfig;
use sqlx::{Connection, SqliteConnection, sqlite::SqliteConnectOptions};
use std::os::unix::fs::PermissionsExt;

const KILL_CHILD: &str = "LYTRONAUT_RESTORE_KILL_CHILD";
/// Rows large enough that row import takes seconds, so cancellation, shutdown
/// and SIGKILL land mid-restore rather than racing completion.
const BULK_ROWS: i64 = 200_000;

fn limits() -> lytronaut_storage::Limits {
    lytronaut_storage::Limits {
        free_floor: 0,
        ..Default::default()
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn random_hex(len: usize) -> String {
    let mut bytes = vec![0; len / 2];
    getrandom::fill(&mut bytes).unwrap();
    hex(&bytes)
}

/// Unrelated state that no restore, failure or cleanup may touch.
struct Sentinels {
    files: Vec<(PathBuf, Vec<u8>)>,
    sqlite: PathBuf,
    /// (unrelated schema, restore-named decoy schema with a foreign marker).
    schemas: Option<(String, String)>,
    restore_schemas: Vec<String>,
    library: Library,
}
/// Logical content of the live library. Auxiliary job-order ordinals may be
/// compacted by unrelated export admission, so intake ORDER is compared, not
/// raw ordinal values.
#[derive(PartialEq)]
struct Library {
    versions: Vec<i64>,
    records: Vec<(i64, String, i64, Vec<u8>)>,
    intake_order: Vec<String>,
    counts: Vec<i64>,
}
async fn library(catalog: &Catalog) -> Library {
    let pool = catalog.pool();
    let mut counts = Vec::new();
    for statement in [
        "SELECT count(*) FROM intake_jobs",
        "SELECT count(*) FROM intake_sources",
        "SELECT count(*) FROM catalog_sources",
        "SELECT count(*) FROM catalog_pictures",
        "SELECT count(*) FROM catalog_albums",
        "SELECT count(*) FROM catalog_memberships",
        "SELECT count(*) FROM api_ids",
        "SELECT count(*) FROM api_imports",
        "SELECT count(*) FROM api_adaptation_errors",
    ] {
        counts.push(sqlx::query_scalar(statement).fetch_one(pool).await.unwrap());
    }
    Library {
        versions: sqlx::query_scalar("SELECT id FROM catalog_versions ORDER BY id")
            .fetch_all(pool)
            .await
            .unwrap(),
        records: sqlx::query("SELECT version,kind,ordinal,original FROM catalog_records ORDER BY version,kind,ordinal")
            .fetch_all(pool)
            .await
            .unwrap()
            .iter()
            .map(|r| (r.try_get(0).unwrap(), r.try_get(1).unwrap(), r.try_get(2).unwrap(), r.try_get(3).unwrap()))
            .collect(),
        intake_order: sqlx::query_scalar("SELECT job FROM application_job_order WHERE kind='intake' ORDER BY ordinal")
            .fetch_all(pool)
            .await
            .unwrap(),
        counts,
    }
}
impl Sentinels {
    async fn plant(fx: &mut Fixture, store: &Store) -> Self {
        let directory = root(store);
        let decoy = directory.join(format!("{}.restored", random_id().unwrap()));
        fs::create_dir_all(decoy.join("store")).unwrap();
        let mut files = vec![
            (
                directory.join("unrelated-sentinel"),
                b"keep: unrelated".to_vec(),
            ),
            (decoy.join("restore-owner"), random_hex(64).into_bytes()),
            (
                decoy.join("store/catalog.db"),
                b"decoy library bytes".to_vec(),
            ),
        ];
        files.push((fx.root.join("unrelated-library.bin"), vec![0, 255, 7]));
        for (path, bytes) in &files {
            fs::write(path, bytes).unwrap();
        }
        let sqlite = fx.root.join("unrelated.db");
        let mut db = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&sqlite)
                .create_if_missing(true),
        )
        .await
        .unwrap();
        sqlx::raw_sql(
            "CREATE TABLE unrelated(v BLOB NOT NULL); INSERT INTO unrelated VALUES(x'00ff07')",
        )
        .execute(&mut db)
        .await
        .unwrap();
        db.close().await.unwrap();
        let schemas = if let Some((admin, _)) = fx.pg.as_mut() {
            let sentinel = format!("lytronaut_sentinel_{}", random_hex(32));
            let decoy = format!("lytronaut_restore_{}", random_hex(32));
            for statement in [
                format!("CREATE SCHEMA \"{sentinel}\""),
                format!("CREATE TABLE \"{sentinel}\".unrelated(v BYTEA NOT NULL)"),
                format!("INSERT INTO \"{sentinel}\".unrelated VALUES('\\x00ff07')"),
                format!("CREATE SCHEMA \"{decoy}\""),
                format!(
                    "CREATE TABLE \"{decoy}\".restore_ownership(singleton BIGINT PRIMARY KEY CHECK(singleton=1),job TEXT NOT NULL,nonce TEXT NOT NULL)"
                ),
                format!(
                    "INSERT INTO \"{decoy}\".restore_ownership VALUES(1,'{}','{}')",
                    random_id().unwrap(),
                    random_hex(64)
                ),
            ] {
                sqlx::query(sqlx::AssertSqlSafe(statement))
                    .execute(&mut *admin)
                    .await
                    .unwrap();
            }
            Some((sentinel, decoy))
        } else {
            None
        };
        let mut value = Self {
            files,
            sqlite,
            schemas,
            restore_schemas: Vec::new(),
            library: library(&fx.catalog).await,
        };
        value.restore_schemas = restore_schemas(&fx.catalog).await;
        value
    }
    async fn assert_intact(&self, fx: &Fixture) {
        for (path, bytes) in &self.files {
            assert_eq!(&fs::read(path).unwrap(), bytes, "sentinel {path:?} changed");
        }
        let mut db = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(&self.sqlite)
                .read_only(true),
        )
        .await
        .unwrap();
        let value: Vec<u8> = sqlx::query_scalar("SELECT v FROM unrelated")
            .fetch_one(&mut db)
            .await
            .unwrap();
        assert_eq!(value, [0, 255, 7]);
        db.close().await.unwrap();
        if let Some((sentinel, decoy)) = &self.schemas {
            let value: Vec<u8> = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT v FROM \"{sentinel}\".unrelated"
            )))
            .fetch_one(fx.catalog.pool())
            .await
            .unwrap();
            assert_eq!(value, [0, 255, 7]);
            let markers: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM \"{decoy}\".restore_ownership"
            )))
            .fetch_one(fx.catalog.pool())
            .await
            .unwrap();
            assert_eq!(markers, 1, "restore-named decoy schema must survive");
        }
        // The deployment's existing library is never overwritten by a restore.
        assert!(
            library(&fx.catalog).await == self.library,
            "live library changed"
        );
        let digest = format!("{:x}", Sha256::digest(BLOB));
        assert_eq!(
            fs::read(
                fx.root
                    .join("originals")
                    .join(format!("{digest}-{}", BLOB.len()))
            )
            .unwrap(),
            BLOB
        );
    }
    async fn remove(&self, fx: &mut Fixture) {
        if let (Some((sentinel, decoy)), Some((admin, _))) = (&self.schemas, fx.pg.as_mut()) {
            for schema in [sentinel, decoy] {
                sqlx::query(sqlx::AssertSqlSafe(format!(
                    "DROP SCHEMA \"{schema}\" CASCADE"
                )))
                .execute(&mut *admin)
                .await
                .unwrap();
            }
        }
    }
}
async fn restore_schemas(catalog: &Catalog) -> Vec<String> {
    if catalog.backend() != lytronaut_storage::Backend::Postgres {
        return Vec::new();
    }
    sqlx::query_scalar(
        "SELECT CAST(nspname AS TEXT) FROM pg_catalog.pg_namespace WHERE nspname LIKE 'lytronaut\\_restore\\_%' ORDER BY 1",
    )
    .fetch_all(catalog.pool())
    .await
    .unwrap()
}

struct Env {
    fx: Fixture,
    store: Store,
    api: Api,
    sentinels: Sentinels,
}
impl Env {
    async fn new(pg: bool) -> Self {
        Self::with_limits(pg, limits()).await
    }
    async fn with_limits(pg: bool, limits: lytronaut_storage::Limits) -> Self {
        let mut fx = Fixture::new(pg).await;
        let store = fx.seed_with(limits).await;
        let api = Api::open(fx.catalog.clone(), store.clone()).await.unwrap();
        let sentinels = Sentinels::plant(&mut fx, &store).await;
        Self {
            fx,
            store,
            api,
            sentinels,
        }
    }
    fn root(&self) -> PathBuf {
        root(&self.store)
    }
    fn catalog(&self) -> &Catalog {
        &self.fx.catalog
    }
    /// Restart reconciliation as the service performs it at startup.
    async fn restart(&self) -> bool {
        Api::open(self.fx.catalog.clone(), self.store.clone())
            .await
            .is_ok()
    }
    async fn job(&self, id: &str) -> (String, Option<String>, i64, i64) {
        let row = sqlx::query("SELECT state,error,reserved,retained FROM export_jobs WHERE id=$1")
            .bind(id)
            .fetch_one(self.catalog().pool())
            .await
            .unwrap();
        (
            row.try_get(0).unwrap(),
            row.try_get(1).unwrap(),
            row.try_get(2).unwrap(),
            row.try_get(3).unwrap(),
        )
    }
    async fn upload(&self, bytes: Vec<u8>) -> (StatusCode, Option<String>) {
        let response = restore_upload(
            Extension(self.api.clone()),
            Request::builder()
                .header(header::CONTENT_LENGTH, bytes.len().to_string())
                .body(Body::from(bytes))
                .unwrap(),
        )
        .await;
        let status = response.status();
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let id = serde_json::from_slice::<serde_json::Value>(&body)
            .ok()
            .and_then(|v| v["id"].as_str().map(str::to_owned));
        (status, id)
    }
    async fn delete(&self, id: &str) -> StatusCode {
        cancel(Extension(self.api.clone()), Ok(Path(id.to_owned())))
            .await
            .status()
    }
    async fn wait_terminal(&self, id: &str) -> JobState {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
        loop {
            let state = report(&self.api, id).await.unwrap().state;
            // Owned cleanup and quota release follow the terminal status write.
            if !matches!(
                state,
                JobState::Queued | JobState::Running | JobState::Cancelling
            ) && self.api.0.exports_inflight.load(Ordering::Acquire) == 0
            {
                return state;
            }
            assert!(std::time::Instant::now() < deadline, "job never settled");
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
    }
    /// Wait until the worker has durably prepared its journal and pending store.
    async fn wait_restoring(&self, id: &str) -> Journal {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
        loop {
            if let Some(journal) = Journal::load(&self.root(), id).unwrap()
                && journal
                    .pending(&self.root())
                    .join("store/originals")
                    .exists()
            {
                return journal;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "restore never started"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }
    fn owned_entries(&self, id: &str) -> Vec<String> {
        fs::read_dir(self.root())
            .unwrap()
            .filter_map(|entry| entry.unwrap().file_name().into_string().ok())
            .filter(|name| name.starts_with(&format!("{id}.")))
            .collect()
    }
    /// A failed, cancelled or deleted restore leaves nothing it owned and holds
    /// no quota; every unrelated sentinel and the live library are untouched.
    async fn assert_released(&self, id: &str) {
        let (state, _, reserved, retained) = self.job(id).await;
        assert_ne!(state, "succeeded");
        assert_eq!((reserved, retained), (0, 0), "quota must be released");
        assert!(Journal::load(&self.root(), id).unwrap().is_none());
        assert_eq!(self.owned_entries(id), Vec::<String>::new());
        assert_eq!(
            restore_schemas(self.catalog()).await,
            self.sentinels.restore_schemas
        );
        self.sentinels.assert_intact(&self.fx).await;
    }
    /// A cleanup-blocked job is visible as a failed, retryable storage error,
    /// keeps its journal-or-paths and quota, and is never ready.
    async fn assert_blocked(&self, id: &str) {
        let (state, error, reserved, retained) = self.job(id).await;
        assert_eq!(
            (state.as_str(), error.as_deref()),
            ("failed", Some("cleanup-blocked"))
        );
        assert!(reserved + retained > 0, "blocked job must keep its quota");
        let report = report(&self.api, id).await.unwrap();
        assert_eq!(report.state, JobState::Failed);
        assert_eq!(report.error.unwrap().code, ErrorCode::StorageUnavailable);
        assert!(
            !self.owned_entries(id).is_empty()
                || restore_schemas(self.catalog()).await != self.sentinels.restore_schemas
        );
        self.sentinels.assert_intact(&self.fx).await;
    }
    /// A ready restore is published, journaled and reopens with exact values.
    async fn assert_ready(&self, id: &str) -> Journal {
        assert_eq!(self.wait_terminal(id).await, JobState::Succeeded);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let journal = loop {
            let journal = Journal::load(&self.root(), id).unwrap().unwrap();
            if journal.phase() == Phase::JobSucceeded {
                break journal;
            }
            assert!(std::time::Instant::now() < deadline);
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        };
        let result = journal.result(&self.root()).join("store");
        let config = match journal.namespace() {
            Some(namespace) => self
                .fx
                .config
                .clone()
                .with_schema(namespace.schema())
                .unwrap(),
            None => DatabaseConfig::Sqlite(result.join("catalog.db")),
        };
        let restored = Catalog::open(config).await.unwrap();
        verify(&restored, &result.join("originals")).await;
        restored.pool().close().await;
        self.sentinels.assert_intact(&self.fx).await;
        journal
    }
    async fn finish(mut self) {
        self.sentinels.assert_intact(&self.fx).await;
        self.sentinels.remove(&mut self.fx).await;
        drop(self.api);
        self.fx.finish().await;
    }
}

/// An independently seeded SQLite source library, optionally with bulk rows.
async fn archive(bulk: i64) -> Vec<u8> {
    let source = Fixture::new(false).await;
    let store = source.seed().await;
    if bulk > 0 {
        sqlx::query("INSERT INTO catalog_records(version,kind,ordinal,original) WITH RECURSIVE n(i) AS (SELECT 1 UNION ALL SELECT i+1 FROM n WHERE i<$1) SELECT $2,'bulk',i,CAST(printf('%016x',i) AS BLOB) FROM n")
            .bind(bulk)
            .bind(IDS[3])
            .execute(source.catalog.pool())
            .await
            .unwrap();
    }
    let path = source.root.join("source.bundle");
    bundle::backup(
        &source.catalog,
        &store,
        &path,
        &AtomicBool::new(false),
        &AtomicU64::new(0),
    )
    .await
    .unwrap();
    let bytes = fs::read(&path).unwrap();
    source.finish().await;
    bytes
}

// ---- Group 3: corruption, quota, cancellation, shutdown, process kill ------

async fn corrupt_archives_fail_closed_and_release(pg: bool) {
    let env = Env::new(pg).await;
    let good = archive(0).await;
    let catalog_bytes = u64::from_le_bytes(good[20..28].try_into().unwrap()) as usize;
    // (a) Flipped catalog byte: the declared SHA-256 no longer matches.
    let mut flipped = good.clone();
    flipped[60 + catalog_bytes / 2] ^= 0x40;
    // (b) Garbage catalog with a recomputed hash: rejected by the decoder only
    // after the owned destination (and PostgreSQL namespace) exists.
    let mut forged = good.clone();
    for byte in &mut forged[60 + 8..60 + catalog_bytes] {
        *byte ^= 0x5a;
    }
    let digest = Sha256::digest(&forged[60..60 + catalog_bytes]);
    forged[28..60].copy_from_slice(&digest);
    // (c) Flipped original byte: blob hash mismatch after rows are staged.
    let mut original = good.clone();
    let last = original.len() - 1;
    original[last] ^= 0x01;
    for corrupt in [flipped, forged, original] {
        let (status, id) = env.upload(corrupt).await;
        assert_eq!(status, StatusCode::OK);
        let id = id.unwrap();
        assert_eq!(env.wait_terminal(&id).await, JobState::Failed);
        env.assert_released(&id).await;
    }
    // The same deployment still restores a good archive afterwards.
    let (_, id) = env.upload(good).await;
    let id = id.unwrap();
    env.assert_ready(&id).await;
    assert_eq!(env.delete(&id).await, StatusCode::OK);
    env.assert_released(&id).await;
    env.finish().await;
}

async fn quota_failures_fail_closed_and_release(pg: bool) {
    // Admission: the reservation cannot fit, so no job, file or namespace exists.
    let env = Env::with_limits(
        pg,
        lytronaut_storage::Limits {
            reserved_bytes: 1024 * 1024,
            ..limits()
        },
    )
    .await;
    let jobs_before: i64 = sqlx::query_scalar("SELECT count(*) FROM export_jobs")
        .fetch_one(env.catalog().pool())
        .await
        .unwrap();
    let (status, id) = env.upload(archive(0).await).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(id.is_none());
    let jobs_after: i64 = sqlx::query_scalar("SELECT count(*) FROM export_jobs")
        .fetch_one(env.catalog().pool())
        .await
        .unwrap();
    assert_eq!(jobs_before, jobs_after);
    assert_eq!(
        restore_schemas(env.catalog()).await,
        env.sentinels.restore_schemas
    );
    env.finish().await;
    // Mid-restore: an original larger than the per-file quota fails after the
    // owned destination exists; cleanup removes it and releases the reservation.
    let env = Env::with_limits(
        pg,
        lytronaut_storage::Limits {
            file_bytes: BLOB.len() as u64 - 1,
            ..limits()
        },
    )
    .await;
    let (status, id) = env.upload(archive(0).await).await;
    assert_eq!(status, StatusCode::OK);
    let id = id.unwrap();
    assert_eq!(env.wait_terminal(&id).await, JobState::Failed);
    env.assert_released(&id).await;
    env.finish().await;
}

async fn cancel_and_shutdown_mid_restore_never_publish(pg: bool) {
    let env = Env::new(pg).await;
    let bulk = archive(BULK_ROWS).await;
    let (_, id) = env.upload(bulk.clone()).await;
    let id = id.unwrap();
    env.wait_restoring(&id).await;
    assert_eq!(env.delete(&id).await, StatusCode::OK);
    assert_eq!(env.wait_terminal(&id).await, JobState::Cancelled);
    env.assert_released(&id).await;
    assert!(env.restart().await);
    assert_eq!(env.job(&id).await.0, "cancelled");
    // Graceful service shutdown cancels an in-flight restore before publication.
    let (_, id) = env.upload(bulk).await;
    let id = id.unwrap();
    env.wait_restoring(&id).await;
    env.api.shutdown().await.unwrap();
    assert_eq!(env.wait_terminal(&id).await, JobState::Cancelled);
    env.assert_released(&id).await;
    env.finish().await;
}

/// Re-entered by the SIGKILL test as a separate OS process. Not a test by itself.
#[tokio::test]
#[ignore = "child process entry for the SIGKILL restore test"]
async fn process_kill_child_entry() {
    let Ok(spec) = std::env::var(KILL_CHILD) else {
        return;
    };
    let spec: serde_json::Value = serde_json::from_str(&spec).unwrap();
    let fixture = PathBuf::from(spec["root"].as_str().unwrap());
    let config = match spec["schema"].as_str() {
        Some(schema) => {
            DatabaseConfig::Postgres(std::env::var("LYTRONAUT_TEST_POSTGRES_URL").unwrap())
                .with_schema(schema)
                .unwrap()
        }
        None => DatabaseConfig::Sqlite(fixture.join("catalog.db")),
    };
    let catalog = Catalog::open(config).await.unwrap();
    let store = Store::open(
        catalog.clone(),
        &fixture.join("originals"),
        &fixture.join("staging"),
        limits(),
    )
    .await
    .unwrap();
    let api = Api::open(catalog, store).await.unwrap();
    let bytes = fs::read(spec["archive"].as_str().unwrap()).unwrap();
    let response = restore_upload(
        Extension(api),
        Request::builder()
            .header(header::CONTENT_LENGTH, bytes.len().to_string())
            .body(Body::from(bytes))
            .unwrap(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    // Keep the worker running until the parent delivers SIGKILL.
    tokio::time::sleep(std::time::Duration::from_secs(600)).await;
    panic!("child was never killed");
}

async fn process_kill_mid_restore_then_restart_recovers(pg: bool) {
    let env = Env::new(pg).await;
    let archive_path = env.fx.root.join("kill.bundle");
    fs::write(&archive_path, archive(BULK_ROWS).await).unwrap();
    // One service process owns the store at a time (exclusive store lock), so
    // this process stops serving before the child starts, as in a real restart.
    let Env {
        fx,
        store,
        api,
        sentinels,
    } = env;
    let directory = root(&store);
    drop(api);
    drop(store);
    let spec = serde_json::json!({
        "root": fx.root,
        "schema": fx.pg.as_ref().map(|(_, schema)| schema.clone()),
        "archive": archive_path,
    });
    let log = fx.root.join("child.log");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "api::exports::restore_fault_tests::process_kill_child_entry",
            "--ignored",
            "--test-threads=1",
        ])
        .env(KILL_CHILD, spec.to_string())
        .stdout(fs::File::create(&log).unwrap())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let job = |id: String| {
        let catalog = fx.catalog.clone();
        async move {
            let row = sqlx::query("SELECT state,reserved FROM export_jobs WHERE id=$1")
                .bind(id)
                .fetch_one(catalog.pool())
                .await
                .unwrap();
            (
                row.try_get::<String, _>(0).unwrap(),
                row.try_get::<i64, _>(1).unwrap(),
            )
        }
    };
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    let id = loop {
        if let Some(status) = child.try_wait().unwrap() {
            panic!(
                "restore child exited before SIGKILL: {status}\n{}",
                fs::read_to_string(&log).unwrap_or_default()
            );
        }
        let running: Option<String> = sqlx::query_scalar(
            "SELECT id FROM export_jobs WHERE kind='restore' AND state='running'",
        )
        .fetch_optional(fx.catalog.pool())
        .await
        .unwrap();
        if let Some(id) = running
            && Journal::load(&directory, &id)
                .unwrap()
                .is_some_and(|j| j.pending(&directory).join("store/originals").exists())
        {
            break id;
        }
        assert!(std::time::Instant::now() < deadline, "child never started");
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    child.kill().unwrap();
    assert!(!child.wait().unwrap().success());
    let journal = Journal::load(&directory, &id).unwrap().unwrap();
    assert!(
        matches!(
            journal.phase(),
            Phase::Prepared | Phase::SchemaOwned | Phase::RowsCommitted
        ),
        "restore finished before SIGKILL; enlarge the fixture"
    );
    assert!(!journal.result(&directory).exists());
    let (state, reserved) = job(id.clone()).await;
    assert_eq!(state, "running");
    assert!(reserved > 0);
    let store = Store::open(
        fx.catalog.clone(),
        &fx.root.join("originals"),
        &fx.root.join("staging"),
        limits(),
    )
    .await
    .unwrap();
    // First restart with ownership proof denied: the service still starts, but
    // only this job is blocked, visibly, with everything it owns retained.
    let pending_marker = journal.pending(&directory).join("restore-owner");
    fs::set_permissions(&pending_marker, fs::Permissions::from_mode(0o000)).unwrap();
    let api = Api::open(fx.catalog.clone(), store.clone())
        .await
        .expect("one unprovable job must not refuse startup");
    let env = Env {
        fx,
        store,
        api,
        sentinels,
    };
    env.assert_blocked(&id).await;
    assert!(journal.pending(&directory).exists());
    // A blocked job retries at every startup and stays blocked while unprovable.
    assert!(env.restart().await);
    env.assert_blocked(&id).await;
    fs::set_permissions(&pending_marker, fs::Permissions::from_mode(0o600)).unwrap();
    // The next restart proves ownership and removes exactly the owned state.
    assert!(env.restart().await);
    let (state, error, _, _) = env.job(&id).await;
    assert_eq!(
        (state.as_str(), error.as_deref()),
        ("failed", Some("interrupted"))
    );
    env.assert_released(&id).await;
    // Dead-session leftovers never block a later restore.
    let (_, next) = env.upload(fs::read(&archive_path).unwrap()).await;
    let next = next.unwrap();
    env.assert_ready(&next).await;
    assert_eq!(env.delete(&next).await, StatusCode::OK);
    env.assert_released(&next).await;
    env.finish().await;
}

async fn successful_restore_publishes_and_delete_releases(pg: bool) {
    let env = Env::new(pg).await;
    let (status, id) = env.upload(archive(0).await).await;
    assert_eq!(status, StatusCode::OK);
    let id = id.unwrap();
    let journal = env.assert_ready(&id).await;
    let (_, _, reserved, retained) = env.job(&id).await;
    assert_eq!(reserved, 0);
    assert!(retained > 0);
    // Restart keeps a verified ready restore ready.
    assert!(env.restart().await);
    assert_eq!(env.job(&id).await.0, "succeeded");
    assert!(journal.result(&env.root()).exists());
    assert_eq!(env.delete(&id).await, StatusCode::OK);
    assert_eq!(env.job(&id).await.0, "cancelled");
    env.assert_released(&id).await;
    assert!(env.restart().await);
    assert_eq!(env.job(&id).await.0, "cancelled");
    // Recovery never rewrites a recorded cancellation as an interruption.
    assert_eq!(env.job(&id).await.1, None);
    env.finish().await;
}

// ---- Group 4: denied cleanup, restricted DROP, final UPDATE, late DELETE ---

/// Real permission denial: PostgreSQL revokes the owner's USAGE on its own
/// restore namespace; SQLite deployments lose read access to the owner marker.
async fn deny(env: &mut Env, journal: &Journal, denied: bool) {
    match (journal.namespace(), env.fx.pg.as_mut()) {
        (Some(namespace), Some((admin, _))) => {
            let statement = if denied {
                format!(
                    "REVOKE USAGE ON SCHEMA \"{}\" FROM CURRENT_USER",
                    namespace.schema()
                )
            } else {
                format!(
                    "GRANT USAGE ON SCHEMA \"{}\" TO CURRENT_USER",
                    namespace.schema()
                )
            };
            sqlx::query(sqlx::AssertSqlSafe(statement))
                .execute(&mut *admin)
                .await
                .unwrap();
        }
        _ => {
            let marker = journal.result(&env.root()).join("restore-owner");
            let mode = if denied { 0o000 } else { 0o600 };
            fs::set_permissions(marker, fs::Permissions::from_mode(mode)).unwrap();
        }
    }
}

async fn denied_cleanup_retains_ownership_and_quota_until_retry(pg: bool) {
    let mut env = Env::new(pg).await;
    let (_, id) = env.upload(archive(0).await).await;
    let id = id.unwrap();
    let journal = env.assert_ready(&id).await;
    let retained = env.job(&id).await.3;
    deny(&mut env, &journal, true).await;
    // Late DELETE: cleanup is denied, so ownership, files and quota remain and
    // the job is visibly blocked (failed) — never resurrected as ready.
    assert_ne!(env.delete(&id).await, StatusCode::OK);
    env.assert_blocked(&id).await;
    assert_eq!(env.job(&id).await.3, retained, "retained quota kept");
    let observed = Journal::load(&env.root(), &id).unwrap().unwrap();
    assert_eq!(observed.phase(), Phase::CleanupPending);
    assert!(journal.result(&env.root()).exists());
    // Admission keeps honouring the blocked job's retained bytes.
    sqlx::query("UPDATE export_jobs SET retained=$1 WHERE id=$2")
        .bind(env.store.limits().reserved_bytes as i64)
        .bind(&id)
        .execute(env.catalog().pool())
        .await
        .unwrap();
    assert_eq!(
        create(&env.api, "archive", 1).await.unwrap_err().0,
        StatusCode::TOO_MANY_REQUESTS
    );
    // The service still starts; the job stays blocked while denied.
    assert!(
        env.restart().await,
        "one blocked job must not refuse startup"
    );
    env.assert_blocked(&id).await;
    deny(&mut env, &journal, false).await;
    // DELETE retries the owned cleanup and releases everything.
    assert_eq!(env.delete(&id).await, StatusCode::OK);
    env.assert_released(&id).await;
    assert_eq!(env.job(&id).await.1.as_deref(), Some("interrupted"));
    assert!(env.restart().await);
    assert_ne!(env.job(&id).await.0, "succeeded");
    create(&env.api, "archive", 1).await.unwrap();
    env.finish().await;
}

/// PostgreSQL only: an unrelated object outside the namespace depends on a
/// restored table. Cleanup must refuse rather than CASCADE into unrelated SQL.
async fn restricted_drop_never_cascades_into_unrelated_objects(mut env: Env) {
    let (_, id) = env.upload(archive(0).await).await;
    let id = id.unwrap();
    let journal = env.assert_ready(&id).await;
    let namespace = journal.namespace().unwrap().schema().to_owned();
    let (sentinel, _) = env.sentinels.schemas.clone().unwrap();
    let (admin, _) = env.fx.pg.as_mut().unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE VIEW \"{sentinel}\".depends_on_restore AS SELECT count(*) AS n FROM \"{namespace}\".catalog_versions"
    )))
    .execute(&mut *admin)
    .await
    .unwrap();
    assert_ne!(env.delete(&id).await, StatusCode::OK);
    let view: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM pg_catalog.pg_views WHERE schemaname='{sentinel}' AND viewname='depends_on_restore'"
    )))
    .fetch_one(env.catalog().pool())
    .await
    .unwrap();
    assert_eq!(view, 1, "cleanup must not cascade into unrelated objects");
    assert!(restore_schemas(env.catalog()).await.contains(&namespace));
    env.assert_blocked(&id).await;
    assert!(
        env.restart().await,
        "one blocked job must not refuse startup"
    );
    env.assert_blocked(&id).await;
    let (admin, _) = env.fx.pg.as_mut().unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP VIEW \"{sentinel}\".depends_on_restore"
    )))
    .execute(&mut *admin)
    .await
    .unwrap();
    assert!(env.restart().await);
    env.assert_released(&id).await;
    env.finish().await;
}

/// Real database failure of the worker's final status UPDATE: a trigger on the
/// application's own export_jobs table aborts the matching transition.
async fn install_update_fault(
    env: &mut Env,
    name: &str,
    condition_pg: &str,
    condition_sqlite: &str,
) {
    match env.fx.pg.as_mut() {
        Some((admin, schema)) => {
            for statement in [
                format!(
                    "CREATE FUNCTION \"{schema}\".{name}() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN RAISE EXCEPTION 'synthetic final update fault'; END$$"
                ),
                format!(
                    "CREATE TRIGGER {name} BEFORE UPDATE ON \"{schema}\".export_jobs FOR EACH ROW WHEN ({condition_pg}) EXECUTE FUNCTION \"{schema}\".{name}()"
                ),
            ] {
                sqlx::query(sqlx::AssertSqlSafe(statement))
                    .execute(&mut *admin)
                    .await
                    .unwrap();
            }
        }
        None => {
            sqlx::raw_sql(sqlx::AssertSqlSafe(format!("CREATE TRIGGER {name} BEFORE UPDATE ON export_jobs WHEN {condition_sqlite} BEGIN SELECT RAISE(ABORT,'synthetic final update fault'); END")))
                .execute(env.catalog().pool())
                .await
                .unwrap();
        }
    }
}
async fn remove_update_fault(env: &mut Env, name: &str) {
    match env.fx.pg.as_mut() {
        Some((admin, schema)) => {
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP FUNCTION \"{schema}\".{name}() CASCADE"
            )))
            .execute(&mut *admin)
            .await
            .unwrap();
        }
        None => {
            sqlx::query(sqlx::AssertSqlSafe(format!("DROP TRIGGER {name}")))
                .execute(env.catalog().pool())
                .await
                .unwrap();
        }
    }
}
async fn failing_final_update_never_reports_ready(pg: bool) {
    let mut env = Env::new(pg).await;
    let good = archive(0).await;
    // (a) Only the success transition fails: the best-effort fallback records
    // an interruption and owned cleanup releases everything.
    install_update_fault(
        &mut env,
        "deny_ready",
        "NEW.state='succeeded'",
        "NEW.state='succeeded'",
    )
    .await;
    let (_, id) = env.upload(good.clone()).await;
    let id = id.unwrap();
    assert_eq!(env.wait_terminal(&id).await, JobState::Failed);
    let (_, error, _, _) = env.job(&id).await;
    assert_eq!(error.as_deref(), Some("interrupted"));
    env.assert_released(&id).await;
    remove_update_fault(&mut env, "deny_ready").await;
    // (b) Every state transition fails, including the fallback. The job can
    // never become ready; with no durable terminal state the worker retains
    // ownership, files and quota, and restart records the interruption.
    install_update_fault(
        &mut env,
        "deny_transition",
        "NEW.state IS DISTINCT FROM OLD.state AND OLD.state='running'",
        "NEW.state IS NOT OLD.state AND OLD.state='running'",
    )
    .await;
    let (_, id) = env.upload(good).await;
    let id = id.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while env.api.0.exports_inflight.load(Ordering::Acquire) != 0
        || env.job(&id).await.0 == "queued"
    {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    assert_eq!(env.job(&id).await.0, "running");
    assert_ne!(
        report(&env.api, &id).await.unwrap().state,
        JobState::Succeeded
    );
    assert!(
        env.job(&id).await.2 > 0,
        "quota retained without a durable outcome"
    );
    assert!(Journal::load(&env.root(), &id).unwrap().is_some());
    remove_update_fault(&mut env, "deny_transition").await;
    assert!(env.restart().await);
    let (state, error, _, _) = env.job(&id).await;
    assert_eq!(
        (state.as_str(), error.as_deref()),
        ("failed", Some("interrupted"))
    );
    env.assert_released(&id).await;
    env.finish().await;
}

async fn late_delete_racing_publication_never_resurrects_ready(pg: bool) {
    let env = Env::new(pg).await;
    // Hold the reconciliation lock so the worker's success bookkeeping and the
    // DELETE cleanup contend exactly at publication.
    let guard = RECONCILE.lock().await;
    let (_, id) = env.upload(archive(0).await).await;
    let id = id.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while env.job(&id).await.0 != "succeeded" {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let api = env.api.clone();
    let job = id.clone();
    let delete = tokio::spawn(async move { cancel(Extension(api), Ok(Path(job))).await.status() });
    while env.job(&id).await.0 != "cancelled" {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    drop(guard);
    assert_eq!(delete.await.unwrap(), StatusCode::OK);
    while env.api.0.exports_inflight.load(Ordering::Acquire) != 0 {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        report(&env.api, &id).await.unwrap().state,
        JobState::Cancelled
    );
    env.assert_released(&id).await;
    assert!(env.restart().await);
    assert_eq!(
        report(&env.api, &id).await.unwrap().state,
        JobState::Cancelled
    );
    env.assert_released(&id).await;
    env.finish().await;
}

/// Crash cuts at every journal durability point of a real restore, followed
/// by restart reconciliation. Unprovable ownership (directory created, marker
/// never written) fails closed and keeps quota until an operator intervenes.
async fn journal_fault_matrix_recovers_or_retains(pg: bool) {
    let env = Env::new(pg).await;
    let input = env.fx.root.join("fault.bundle");
    fs::write(&input, archive(0).await).unwrap();
    let mut points = vec!["prepared", "mkdir", "marker", "rename"];
    if pg {
        // Between CREATE SCHEMA+marker commit and the SchemaOwned journal phase.
        points.push("namespace-created");
    }
    points.extend(["files-ready", "files-published"]);
    for point in points {
        let id = create(&env.api, "restore", 4096).await.unwrap();
        crate::restore_journal::inject_fault(point);
        let result = bundle::restore_owned(
            &input,
            env.catalog(),
            bundle::RestoreOwner {
                root: &env.root(),
                job: &id,
                reserved: 4096,
            },
            env.store.limits(),
            &AtomicBool::new(false),
            &AtomicU64::new(0),
        )
        .await;
        assert!(result.is_err(), "fault {point} must interrupt the restore");
        let journal = Journal::load(&env.root(), &id).unwrap().unwrap();
        if point == "mkdir" {
            // An unmarked pending directory is not proof: the job is blocked,
            // the service still starts.
            assert!(env.restart().await);
            env.assert_blocked(&id).await;
            assert!(journal.pending(&env.root()).exists());
            assert!(Journal::load(&env.root(), &id).unwrap().is_some());
            assert_eq!(env.job(&id).await.2, 4096, "quota retained");
            // Operator removes the verified-empty unmarked directory.
            assert_eq!(
                fs::read_dir(journal.pending(&env.root())).unwrap().count(),
                0
            );
            fs::remove_dir(journal.pending(&env.root())).unwrap();
        }
        if point == "files-published" {
            // Published, but the job never durably succeeded: not ready.
            assert!(journal.result(&env.root()).exists());
        }
        if let Some(namespace) = journal.namespace()
            && point == "namespace-created"
        {
            assert_eq!(journal.phase(), Phase::Prepared);
            assert!(
                restore_schemas(env.catalog())
                    .await
                    .contains(&namespace.schema().to_owned())
            );
        }
        assert!(env.restart().await, "fault {point} must reconcile");
        assert_eq!(env.job(&id).await.0, "failed");
        env.assert_released(&id).await;
    }
    // Crash right after publication when the success commit DID land: restart
    // verifies the published result and its namespace and keeps it ready.
    let id = create(&env.api, "restore", 4096).await.unwrap();
    crate::restore_journal::inject_fault("files-published");
    assert!(
        bundle::restore_owned(
            &input,
            env.catalog(),
            bundle::RestoreOwner {
                root: &env.root(),
                job: &id,
                reserved: 4096,
            },
            env.store.limits(),
            &AtomicBool::new(false),
            &AtomicU64::new(0),
        )
        .await
        .is_err()
    );
    sqlx::query("UPDATE export_jobs SET state='succeeded',reserved=0,retained=1 WHERE id=$1")
        .bind(&id)
        .execute(env.catalog().pool())
        .await
        .unwrap();
    assert!(env.restart().await);
    env.assert_ready(&id).await;
    assert_eq!(env.delete(&id).await, StatusCode::OK);
    env.assert_released(&id).await;
    env.finish().await;
}

/// Near-limit SOURCE: a cell at exactly the v2 cell limit round trips; one byte
/// more fails backup with a quota error and leaves no artifact or spool behind.
async fn near_limit_source_cell_round_trips_and_over_limit_fails_cleanly(pg: bool) {
    let limit = lytronaut_storage::portable::MAX_CELL;
    let cell = |len: usize| -> Vec<u8> { (0..len).map(|i| (i % 251) as u8).collect() };
    let source = Fixture::new(pg).await;
    let store = source.seed().await;
    sqlx::query("INSERT INTO catalog_records(version,kind,ordinal,original) VALUES($1,'near-limit-cell',0,$2)")
        .bind(IDS[1])
        .bind(cell(limit))
        .execute(source.catalog.pool())
        .await
        .unwrap();
    let path = source.root.join("near-limit.bundle");
    bundle::backup(
        &source.catalog,
        &store,
        &path,
        &AtomicBool::new(false),
        &AtomicU64::new(0),
    )
    .await
    .unwrap();
    let bytes = fs::read(&path).unwrap();
    sqlx::query("INSERT INTO catalog_records(version,kind,ordinal,original) VALUES($1,'over-limit-cell',0,$2)")
        .bind(IDS[2])
        .bind(cell(limit + 1))
        .execute(source.catalog.pool())
        .await
        .unwrap();
    let over = source.root.join("over-limit.bundle");
    assert!(matches!(
        bundle::backup(
            &source.catalog,
            &store,
            &over,
            &AtomicBool::new(false),
            &AtomicU64::new(0)
        )
        .await,
        Err(lytronaut_storage::StorageError::Quota)
    ));
    assert!(!over.exists());
    assert!(!over.with_extension("snapshot").exists());
    source.finish().await;
    let env = Env::new(pg).await;
    let (_, id) = env.upload(bytes).await;
    let id = id.unwrap();
    let journal = env.assert_ready(&id).await;
    let config = match journal.namespace() {
        Some(namespace) => env
            .fx
            .config
            .clone()
            .with_schema(namespace.schema())
            .unwrap(),
        None => DatabaseConfig::Sqlite(journal.result(&env.root()).join("store/catalog.db")),
    };
    let restored = Catalog::open(config).await.unwrap();
    let value: Vec<u8> =
        sqlx::query_scalar("SELECT original FROM catalog_records WHERE kind='near-limit-cell'")
            .fetch_one(restored.pool())
            .await
            .unwrap();
    assert_eq!(value.len(), limit);
    assert!(value == cell(limit));
    restored.pool().close().await;
    assert_eq!(env.delete(&id).await, StatusCode::OK);
    env.assert_released(&id).await;
    env.finish().await;
}

/// A large row import into the restore's private namespace must not hold the
/// live deployment's writer admission: live export admission and plain writes
/// complete promptly (well under the 5 s PostgreSQL lock_timeout) mid-import.
async fn live_writes_proceed_during_restore_import(pg: bool) {
    let env = Env::new(pg).await;
    let (_, id) = env.upload(archive(BULK_ROWS).await).await;
    let id = id.unwrap();
    let journal = env.wait_restoring(&id).await;
    // Originals are staged; the single row-import transaction is now running.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let started = std::time::Instant::now();
    let admitted = create(&env.api, "archive", 1).await.unwrap();
    let mut tx = env.catalog().begin_write().await.unwrap();
    sqlx::query("UPDATE export_jobs SET completed=completed WHERE id=$1")
        .bind(&admitted)
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.commit().await.unwrap();
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "live writers waited {elapsed:?} behind the restore import"
    );
    let observed = Journal::load(&env.root(), &id).unwrap().unwrap();
    assert!(
        matches!(observed.phase(), Phase::Prepared | Phase::SchemaOwned),
        "restore import finished before the live write; enlarge the fixture"
    );
    assert!(!journal.result(&env.root()).exists());
    assert_eq!(env.delete(&id).await, StatusCode::OK);
    assert_eq!(env.wait_terminal(&id).await, JobState::Cancelled);
    env.assert_released(&id).await;
    assert_eq!(env.delete(&admitted).await, StatusCode::OK);
    env.finish().await;
}

/// PostgreSQL only: the success commit lands but the worker's final UPDATE
/// reports an error (ambiguous outcome). A statement-level trigger makes every
/// export_jobs UPDATE fail once the job is durably succeeded, which is exactly
/// what the worker observes. The published result must never be deleted.
async fn ambiguous_final_update_keeps_published_success(mut env: Env) {
    let (_, id) = env.upload(archive(BULK_ROWS / 4).await).await;
    let id = id.unwrap();
    env.wait_restoring(&id).await;
    let (admin, schema) = env.fx.pg.as_mut().unwrap();
    for statement in [
        format!(
            "CREATE FUNCTION \"{schema}\".ambiguous() RETURNS trigger LANGUAGE plpgsql AS $$BEGIN IF EXISTS(SELECT 1 FROM \"{schema}\".export_jobs WHERE id='{id}' AND state='succeeded') THEN RAISE EXCEPTION 'synthetic ambiguous commit'; END IF; RETURN NULL; END$$"
        ),
        format!(
            "CREATE TRIGGER ambiguous BEFORE UPDATE ON \"{schema}\".export_jobs FOR EACH STATEMENT EXECUTE FUNCTION \"{schema}\".ambiguous()"
        ),
        // The durable outcome the worker's own commit would have produced.
        format!("UPDATE \"{schema}\".export_jobs SET state='succeeded' WHERE id='{id}'"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&mut *admin)
            .await
            .unwrap();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(300);
    while env.api.0.exports_inflight.load(Ordering::Acquire) != 0 {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let journal = Journal::load(&env.root(), &id).unwrap().unwrap();
    assert!(
        journal.result(&env.root()).exists(),
        "a committed success must keep its published result"
    );
    let (admin, schema) = env.fx.pg.as_mut().unwrap();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP FUNCTION \"{schema}\".ambiguous() CASCADE"
    )))
    .execute(&mut *admin)
    .await
    .unwrap();
    assert!(env.restart().await);
    env.assert_ready(&id).await;
    assert_eq!(env.delete(&id).await, StatusCode::OK);
    env.assert_released(&id).await;
    env.finish().await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL service"]
async fn postgres_ambiguous_final_update() {
    ambiguous_final_update_keeps_published_success(Env::new(true).await).await;
}

/// Without a journal, a restore-named result or pending store is not provably
/// owned: cleanup fails closed, leaves it untouched and keeps the reservation
/// (no quota released while bytes remain on disk), until it is gone.
async fn unjournaled_restore_paths_retain_quota(pg: bool) {
    let env = Env::new(pg).await;
    for name in ["restored", &format!("pending-{}", random_hex(64))] {
        let id = create(&env.api, "restore", 4096).await.unwrap();
        sqlx::query("UPDATE export_jobs SET state='failed',error='failed' WHERE id=$1")
            .bind(&id)
            .execute(env.catalog().pool())
            .await
            .unwrap();
        let path = env.root().join(format!("{id}.{name}"));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("unrelated"), b"not ours").unwrap();
        assert!(
            env.restart().await,
            "one unprovable job must not refuse startup"
        );
        env.assert_blocked(&id).await;
        assert_eq!(
            env.job(&id).await.2,
            4096,
            "quota retained while {name} remains"
        );
        assert_eq!(fs::read(path.join("unrelated")).unwrap(), b"not ours");
        // DELETE retries and still fails closed while the path remains.
        assert_ne!(env.delete(&id).await, StatusCode::OK);
        env.assert_blocked(&id).await;
        fs::remove_dir_all(&path).unwrap();
        assert!(env.restart().await);
        env.assert_released(&id).await;
    }
    env.finish().await;
}

/// Whole-store failures still refuse startup: an unreadable restore journal,
/// an unreadable export root, or an unavailable database.
async fn whole_store_failures_still_refuse_startup(pg: bool) {
    let env = Env::new(pg).await;
    let input = env.fx.root.join("journal.bundle");
    fs::write(&input, archive(0).await).unwrap();
    let id = create(&env.api, "restore", 4096).await.unwrap();
    crate::restore_journal::inject_fault("prepared");
    assert!(
        bundle::restore_owned(
            &input,
            env.catalog(),
            bundle::RestoreOwner {
                root: &env.root(),
                job: &id,
                reserved: 4096,
            },
            env.store.limits(),
            &AtomicBool::new(false),
            &AtomicU64::new(0),
        )
        .await
        .is_err()
    );
    let journal = env.root().join(format!("{id}.restore-journal"));
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o000)).unwrap();
    assert!(
        !env.restart().await,
        "unreadable journal must refuse startup"
    );
    fs::set_permissions(&journal, fs::Permissions::from_mode(0o600)).unwrap();
    fs::set_permissions(env.root(), fs::Permissions::from_mode(0o000)).unwrap();
    assert!(
        !env.restart().await,
        "unreadable export root must refuse startup"
    );
    fs::set_permissions(env.root(), fs::Permissions::from_mode(0o700)).unwrap();
    let unavailable = Catalog::open(env.fx.config.clone()).await.unwrap();
    unavailable.pool().close().await;
    assert!(
        Api::open(unavailable, env.store.clone()).await.is_err(),
        "an unavailable database must refuse startup"
    );
    assert!(env.restart().await);
    env.assert_released(&id).await;
    env.finish().await;
}

macro_rules! matrix {
    ($($scenario:ident => $sqlite:ident, $postgres:ident;)*) => {$(
        #[tokio::test]
        async fn $sqlite() {
            $scenario(false).await;
        }
        #[tokio::test]
        #[ignore = "requires explicit disposable PostgreSQL service"]
        async fn $postgres() {
            $scenario(true).await;
        }
    )*};
}
matrix! {
    corrupt_archives_fail_closed_and_release => sqlite_corrupt_archives, postgres_corrupt_archives;
    quota_failures_fail_closed_and_release => sqlite_quota_failures, postgres_quota_failures;
    cancel_and_shutdown_mid_restore_never_publish => sqlite_cancel_and_shutdown, postgres_cancel_and_shutdown;
    process_kill_mid_restore_then_restart_recovers => sqlite_process_kill, postgres_process_kill;
    successful_restore_publishes_and_delete_releases => sqlite_publish_and_delete, postgres_publish_and_delete;
    denied_cleanup_retains_ownership_and_quota_until_retry => sqlite_denied_cleanup, postgres_denied_cleanup;
    failing_final_update_never_reports_ready => sqlite_failing_final_update, postgres_failing_final_update;
    late_delete_racing_publication_never_resurrects_ready => sqlite_late_delete, postgres_late_delete;
    journal_fault_matrix_recovers_or_retains => sqlite_journal_fault_matrix, postgres_journal_fault_matrix;
    near_limit_source_cell_round_trips_and_over_limit_fails_cleanly => sqlite_near_limit_source, postgres_near_limit_source;
    live_writes_proceed_during_restore_import => sqlite_live_writes_during_import, postgres_live_writes_during_import;
    unjournaled_restore_paths_retain_quota => sqlite_unjournaled_paths, postgres_unjournaled_paths;
    whole_store_failures_still_refuse_startup => sqlite_whole_store_failures, postgres_whole_store_failures;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL service"]
async fn postgres_restricted_drop() {
    restricted_drop_never_cascades_into_unrelated_objects(Env::new(true).await).await;
}
