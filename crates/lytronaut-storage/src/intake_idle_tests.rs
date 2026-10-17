//! Deterministic idle/cancellation tests, using independent synthetic bytes only.
use super::*;
use sqlx::{ConnectOptions, Connection, PgConnection, postgres::PgConnectOptions};
use std::str::FromStr;

pub(super) struct Fixture {
    pub(super) root: PathBuf,
    pub(super) store: Store,
    pub(super) catalog: Catalog,
    postgres: Option<(PgConnection, String)>,
}
impl Fixture {
    pub(super) async fn new(postgres: bool) -> Self {
        let name = format!("lytronaut_idle_{}", unique().unwrap());
        let root = std::env::temp_dir().join(&name);
        fs::create_dir(&root).unwrap();
        let mut admin = None;
        let config = if postgres {
            let url = std::env::var("LYTRONAUT_TEST_POSTGRES_URL")
                .expect("explicit disposable test database required");
            let options = PgConnectOptions::from_str(&url)
                .unwrap()
                .disable_statement_logging();
            let mut connection = PgConnection::connect_with(&options).await.unwrap();
            let restricted: bool = sqlx::query_scalar("SELECT NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication) AND NOT has_schema_privilege(current_user,'public','CREATE') FROM pg_roles WHERE rolname=current_user")
                .fetch_one(&mut connection).await.unwrap();
            assert!(restricted);
            let sentinel: bool = sqlx::query_scalar("SELECT count(*)=1 AND min(marker)='synthetic-persistence-parity' FROM public.lytronaut_parity_sentinel")
                .fetch_one(&mut connection).await.unwrap();
            assert!(sentinel);
            sqlx::query(sqlx::AssertSqlSafe(format!("CREATE SCHEMA \"{name}\"")))
                .execute(&mut connection)
                .await
                .unwrap();
            admin = Some((connection, name.clone()));
            crate::DatabaseConfig::Postgres(url)
                .with_schema(&name)
                .unwrap()
        } else {
            crate::DatabaseConfig::Sqlite(root.join("catalog.db"))
        };
        let catalog = Catalog::open(config).await.unwrap();
        let store = Store::open(
            catalog.clone(),
            &root.join("originals"),
            &root.join("staging"),
            Limits {
                import_idle_timeout: Duration::from_millis(50),
                active_jobs: 1,
                upload_bytes: 4096,
                file_bytes: 4096,
                encoded_bytes: 8192,
                free_floor: 0,
                reserved_bytes: 32768,
                ..Limits::default()
            },
        )
        .await
        .unwrap();
        // No timing assumption about filesystem/SQL setup, even on loaded PG.
        // Tests either call expiry directly or arm the real worker after setup.
        store.0.reaper_enabled.store(false, Ordering::Relaxed);
        Self {
            root,
            store,
            catalog,
            postgres: admin,
        }
    }
    fn overdue(&self, id: &str) {
        self.store
            .0
            .activity
            .lock()
            .unwrap()
            .get_mut(id)
            .unwrap()
            .last = tokio::time::Instant::now() - self.store.limits().import_idle_timeout;
    }
    async fn reservation(&self, id: &str) -> i64 {
        sqlx::query_scalar("SELECT reserved FROM intake_jobs WHERE id=$1")
            .bind(id)
            .fetch_one(self.catalog.pool())
            .await
            .unwrap()
    }
    pub(super) async fn close(self) {
        drop(self.store);
        self.catalog.pool().close().await;
        if let Some((mut connection, name)) = self.postgres {
            let sentinel: bool = sqlx::query_scalar("SELECT count(*)=1 AND min(marker)='synthetic-persistence-parity' FROM public.lytronaut_parity_sentinel")
                .fetch_one(&mut connection).await.unwrap();
            assert!(sentinel);
            sqlx::query(sqlx::AssertSqlSafe(format!(
                "DROP SCHEMA \"{name}\" CASCADE"
            )))
            .execute(&mut connection)
            .await
            .unwrap();
            connection.close().await.unwrap();
        }
        fs::remove_dir_all(self.root).unwrap();
    }
}
struct Release(Option<std::sync::mpsc::Sender<()>>);
impl Drop for Release {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
fn latch() -> (Arc<TestLatch>, Release) {
    let (sender, receiver) = std::sync::mpsc::channel();
    (
        Arc::new(TestLatch {
            entered: tokio::sync::Notify::new(),
            release: Mutex::new(receiver),
        }),
        Release(Some(sender)),
    )
}
fn request(zip: bool) -> ImportRequest {
    ImportRequest {
        version: WireVersion::V1,
        kind: if zip {
            lytronaut_contracts::ImportKind::ZipLibrary
        } else {
            lytronaut_contracts::ImportKind::Directory
        },
        declared_bytes: 4096,
        declared_files: 4,
    }
}
async fn cancelled(store: &Store, id: &str) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while store.job(id).await.unwrap().state != JobState::Cancelled {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("automatic idle expiry must end the job without restart");
}
async fn writer_case(postgres: bool) {
    for abort_caller in [false, true] {
        let f = Fixture::new(postgres).await;
        let id = f.store.begin(&request(false)).await.unwrap();
        f.store
            .append_directory_file(&id, "sealed.txt", &b"immutable synthetic original"[..])
            .await
            .unwrap();
        let row = sqlx::query("SELECT digest,length FROM intake_sources WHERE job=$1")
            .bind(&id)
            .fetch_one(f.catalog.pool())
            .await
            .unwrap();
        let source = SourceEntry {
            logical_path: "sealed.txt".into(),
            digest: row.get("digest"),
            length: row.get::<i64, _>("length") as u64,
        };
        let mut spool = f.store.spool(&id).await.unwrap();
        let path = spool.path.clone();
        let unrelated = f.root.join("staging/unrelated-synthetic");
        fs::write(&unrelated, b"untouched").unwrap();
        let (latch, release) = latch();
        *f.store.0.write_latch.lock().unwrap() = Some(latch.clone());
        let writer = tokio::spawn(async move {
            let result = spool.write(b"physically written, worker still live").await;
            (spool, result)
        });
        tokio::time::timeout(Duration::from_secs(20), latch.entered.notified())
            .await
            .expect("blocking worker must reach its deterministic latch");
        f.overdue(&id);
        let expiry_store = f.store.clone();
        let expiry = tokio::spawn(async move { expiry_store.expire_idle().await });
        tokio::time::timeout(Duration::from_secs(20), async {
            while !f.store.cancellation_requested(&id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(
            path.exists(),
            "expiry cannot unlink while an IO worker owns the FD"
        );
        assert!(f.reservation(&id).await > 0);
        assert_eq!(f.store.job(&id).await.unwrap().state, JobState::Running);
        assert_eq!(
            tokio::time::timeout(Duration::from_secs(20), f.store.begin(&request(false)))
                .await
                .unwrap()
                .unwrap_err(),
            StorageError::Busy,
            "capacity stays reserved while draining IO"
        );
        if abort_caller {
            writer.abort();
        }
        // Caller cancellation/drop cannot remove the worker-owned read lease.
        tokio::task::yield_now().await;
        assert!(!expiry.is_finished());
        assert!(path.exists());
        assert!(f.reservation(&id).await > 0);
        drop(release);
        if abort_caller {
            match writer.await {
                Err(error) => assert!(error.is_cancelled()),
                Ok(_) => panic!("writer caller must have been aborted"),
            }
        } else {
            let (spool, result) = writer.await.unwrap();
            assert_eq!(
                result,
                Err(StorageError::Cancelled),
                "late writes must not report fresh progress"
            );
            drop(spool);
        }
        expiry.await.unwrap();
        assert_eq!(f.store.job(&id).await.unwrap().state, JobState::Cancelled);
        assert_eq!(f.reservation(&id).await, 0);
        assert!(!path.exists());
        assert_eq!(fs::read(&unrelated).unwrap(), b"untouched");
        let mut bytes = Vec::new();
        f.store
            .open_blob(&source)
            .unwrap()
            .read_to_end(&mut bytes)
            .unwrap();
        assert_eq!(bytes, b"immutable synthetic original");
        // A reader owns a separate drain lease even before append starts.
        // Dropping its Spool must not unlink under that still-open descriptor.
        let reader_id = f.store.begin(&request(false)).await.unwrap();
        let mut input = f.store.spool(&reader_id).await.unwrap();
        input.write(b"reader-held input").await.unwrap();
        input.sync().await.unwrap();
        let reader_path = input.path.clone();
        let reader = input.reader().unwrap();
        drop(input);
        let stop_store = f.store.clone();
        let stop_id = reader_id.clone();
        let stop = tokio::spawn(async move { stop_store.cancel(&stop_id).await });
        tokio::time::timeout(Duration::from_secs(20), async {
            while !f.store.cancellation_requested(&reader_id) {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!stop.is_finished());
        assert!(reader_path.exists());
        assert!(f.reservation(&reader_id).await > 0);
        drop(reader);
        stop.await.unwrap().unwrap();
        assert!(!reader_path.exists());
        assert_eq!(f.reservation(&reader_id).await, 0);
        // A fresh idle spool has no persistent FD. Arm the real background
        // worker only after setup, then prove automatic cleanup and slot reuse.
        let replacement = f.store.begin(&request(false)).await.unwrap();
        let mut stalled = f.store.spool(&replacement).await.unwrap();
        stalled.write(b"abandoned").await.unwrap();
        let stalled_path = stalled.path.clone();
        f.overdue(&replacement);
        f.store.0.reaper_enabled.store(true, Ordering::Relaxed);
        cancelled(&f.store, &replacement).await;
        assert!(!stalled_path.exists());
        assert_eq!(f.reservation(&replacement).await, 0);
        assert_eq!(
            stalled.write(b"rejected").await,
            Err(StorageError::Cancelled)
        );
        drop(stalled);
        f.store.0.reaper_enabled.store(false, Ordering::Relaxed);
        let next = f.store.begin(&request(false)).await.unwrap();
        f.store.cancel(&next).await.unwrap();
        f.close().await;
    }
}
async fn zip_case(postgres: bool) {
    for mode in ["complete", "cancel", "orphan-caller"] {
        let explicit_cancel = mode != "complete";
        let f = Fixture::new(postgres).await;
        let id = f.store.begin(&request(true)).await.unwrap();
        let mut spool = f.store.spool(&id).await.unwrap();
        spool
            .write(&zip("synthetic.txt", b"public ZIP payload", 0o100644))
            .await
            .unwrap();
        spool.sync().await.unwrap();
        let path = spool.path.clone();
        let (latch, release) = latch();
        *f.store.0.preflight_latch.lock().unwrap() = Some(latch.clone());
        // Waiting for another job's decoder is active admitted processing too.
        let permit = f.store.0.decode.clone().acquire_owned().await.unwrap();
        let worker_store = f.store.clone();
        let worker_id = id.clone();
        let worker = tokio::spawn(async move { worker_store.import_zip(&worker_id, spool).await });
        let mut worker = Some(worker);
        tokio::time::timeout(Duration::from_secs(20), async {
            loop {
                if f.store.0.activity.lock().unwrap()[&id].processing > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        f.overdue(&id);
        f.store.expire_idle().await;
        assert!(
            !f.store.cancellation_requested(&id),
            "decoder admission wait is not idle network time"
        );
        assert!(path.exists());
        drop(permit);
        tokio::time::timeout(Duration::from_secs(20), latch.entered.notified())
            .await
            .expect("blocking worker must reach its deterministic latch");
        f.overdue(&id);
        f.store.expire_idle().await;
        assert!(
            !f.store.cancellation_requested(&id),
            "active preflight is processing, not idle network time"
        );
        assert!(path.exists());
        assert!(f.reservation(&id).await > 0);
        assert_eq!(f.store.job(&id).await.unwrap().state, JobState::Running);
        let stop = if explicit_cancel {
            let store = f.store.clone();
            let id = id.clone();
            Some(tokio::spawn(async move { store.cancel(&id).await }))
        } else {
            None
        };
        if let Some(stop) = &stop {
            tokio::time::timeout(Duration::from_secs(20), async {
                while !f.store.cancellation_requested(&id) {
                    tokio::task::yield_now().await;
                }
            })
            .await
            .unwrap();
            assert!(!stop.is_finished());
            assert!(path.exists());
            assert!(f.reservation(&id).await > 0);
        }
        if mode == "orphan-caller" {
            let caller = worker.take().unwrap();
            caller.abort();
            assert!(caller.await.unwrap_err().is_cancelled());
            // The preflight blocking task remains at the latch after the async
            // owner and its Spool are gone. Input and quota must remain intact.
            assert!(path.exists());
            assert!(f.reservation(&id).await > 0);
        }
        drop(release);
        let result = if let Some(worker) = worker {
            Some(worker.await.unwrap())
        } else {
            None
        };
        if let Some(stop) = stop {
            if let Some(result) = result {
                assert_eq!(result, Err(StorageError::Cancelled));
            }
            stop.await.unwrap().unwrap();
            assert_eq!(f.store.job(&id).await.unwrap().state, JobState::Cancelled);
        } else {
            result.unwrap().unwrap();
            assert_eq!(f.store.job(&id).await.unwrap().state, JobState::Succeeded);
            let entries = f.store.manifest(&id).await.unwrap();
            let mut bytes = Vec::new();
            f.store
                .open_blob(&entries[0])
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            assert_eq!(bytes, b"public ZIP payload");
        }
        assert_eq!(f.reservation(&id).await, 0);
        assert!(!path.exists());
        f.close().await;
    }
}
#[tokio::test]
async fn sqlite_expiry_drains_writers_even_after_caller_abort() {
    writer_case(false).await;
}
#[tokio::test]
async fn sqlite_preflight_past_idle_deadline_completes_without_losing_its_input() {
    zip_case(false).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL with restricted role and sentinel"]
async fn postgres_expiry_drains_writers_even_after_caller_abort() {
    writer_case(true).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL with restricted role and sentinel"]
async fn postgres_preflight_past_idle_deadline_completes_without_losing_its_input() {
    zip_case(true).await;
}

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
