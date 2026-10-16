//! Actual HTTP cancellation at deterministic native render/write boundaries.
//! Hooks and synthetic fixtures are absent from production builds.
use super::*;
use crate::bundle::portable_bundle_tests::Fixture;
use std::{
    io::{Read, Write},
    net::TcpStream,
    os::unix::fs::PermissionsExt,
};
#[path = "../tests/support/live_focus.rs"]
mod synthetic;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Stage {
    Render,
    Write,
}
pub(crate) struct Gate {
    stage: Stage,
    fired: AtomicBool,
    paused_bytes: AtomicU64,
    entered: tokio::sync::Notify,
    cancelled: tokio::sync::Notify,
    release: tokio::sync::Notify,
}
impl Gate {
    fn new(stage: Stage) -> Arc<Self> {
        Arc::new(Self {
            stage,
            fired: AtomicBool::new(false),
            paused_bytes: AtomicU64::new(0),
            entered: Default::default(),
            cancelled: Default::default(),
            release: Default::default(),
        })
    }
}
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release.notify_one();
    }
}
pub(super) async fn checkpoint(api: &Api, stage: Stage, cancel: &AtomicBool, progress: &AtomicU64) {
    let gate = api.0.export_gate.lock().unwrap().clone();
    if let Some(gate) = gate {
        if gate.stage != stage || gate.fired.swap(true, Ordering::AcqRel) {
            return;
        }
        gate.paused_bytes
            .store(progress.load(Ordering::Relaxed), Ordering::Relaxed);
        gate.entered.notify_one();
        loop {
            if cancel.load(Ordering::Acquire) {
                gate.cancelled.notify_one();
            }
            tokio::select! {
                _ = gate.release.notified() => break,
                _ = tokio::time::sleep(std::time::Duration::from_millis(1)) => (),
            }
        }
    }
}
async fn bounded<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(30), future)
        .await
        .expect("deterministic export stage must settle")
}
struct Http {
    port: u16,
    task: tokio::task::JoinHandle<()>,
}
impl Http {
    async fn new(api: &Api) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let router = crate::router_with_routes(
            crate::AppState {
                catalog: api.0.catalog.clone(),
                security: crate::SecurityConfig::new("http://localhost").unwrap(),
            },
            api.clone().routes(),
        );
        let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        Self { port, task }
    }
    async fn request(&self, method: &str, path: &str, body: &[u8]) -> (u16, Vec<u8>) {
        let port = self.port;
        let request = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nX-Lytronaut-Intent: mutation\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let body = body.to_vec();
        bounded(tokio::task::spawn_blocking(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(30)))
                .unwrap();
            stream.write_all(request.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
            let mut bytes = Vec::new();
            stream.read_to_end(&mut bytes).unwrap();
            let split = bytes.windows(4).position(|b| b == b"\r\n\r\n").unwrap();
            let status = std::str::from_utf8(&bytes[..split])
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .parse()
                .unwrap();
            (status, bytes[split + 4..].to_vec())
        }))
        .await
        .unwrap()
    }
    async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}
async fn sentinel(fx: &Fixture) {
    if fx.pg.is_some() {
        let intact: bool=sqlx::query_scalar("SELECT count(*)=1 AND min(marker)='synthetic-persistence-parity' FROM public.lytronaut_parity_sentinel").fetch_one(fx.catalog.pool()).await.unwrap();
        assert!(intact);
    }
}
async fn fixture(pg: bool, stage: Stage) -> (Fixture, Store, Api, String, Vec<u8>, PathBuf) {
    let fx = Fixture::new(pg).await;
    sentinel(&fx).await;
    let store = fx.seed().await;
    let api = Api::open(fx.catalog.clone(), store.clone()).await.unwrap();
    // The fixture's immutable original and catalog mapping are independent
    // synthetic data. Admission, worker, monitor, HTTP and cleanup are real.
    let bytes = if stage == Stage::Write {
        // Three PCM-coded frames exceed the production 1 MiB write chunk.
        synthetic::sized(false, 512, 512, true)
    } else {
        synthetic::live_focus_fixture()
    };
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(&bytes));
    let path = fx
        .root
        .join("originals")
        .join(format!("{digest}-{}", bytes.len()));
    fs::write(&path, &bytes).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();
    sqlx::query("UPDATE catalog_versions SET state='ready' WHERE id=0")
        .execute(fx.catalog.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO intake_sources(job,path,canonical,digest,length) VALUES('synthetic-job-2','public/stacklq.lfp','public/stacklq.lfp',$1,$2)").bind(&digest).bind(bytes.len() as i64).execute(fx.catalog.pool()).await.unwrap();
    sqlx::query("INSERT INTO catalog_sources(version,job,path,digest,length) VALUES(0,'synthetic-job-2','public/stacklq.lfp',$1,$2)").bind(digest).bind(bytes.len() as i64).execute(fx.catalog.pool()).await.unwrap();
    let asset = random_id().unwrap();
    sqlx::query("INSERT INTO api_ids(id,kind,version,source_key) VALUES($1,'file',0,$2)")
        .bind(&asset)
        .bind(fx.catalog.encode_source_key("public/stacklq.lfp"))
        .execute(fx.catalog.pool())
        .await
        .unwrap();
    (fx, store, api, asset, bytes, path)
}
async fn begin(http: &Http, asset: &str) -> String {
    let request=serde_json::json!({"kind":"live_photo","version":1,"asset":asset,"plane_index":1,"orientation":1}).to_string();
    let (status, body) = http.request("POST", "/exports", request.as_bytes()).await;
    assert_eq!(status, 200, "{}", String::from_utf8_lossy(&body));
    serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned()
}
async fn quota(api: &Api, id: &str) -> (i64, i64) {
    let row = sqlx::query("SELECT reserved,retained FROM export_jobs WHERE id=$1")
        .bind(id)
        .fetch_one(api.0.catalog.pool())
        .await
        .unwrap();
    (row.get(0), row.get(1))
}
async fn cancel_case(pg: bool) {
    for stage in [Stage::Render, Stage::Write] {
        let (fx, store, api, asset, bytes, original) = fixture(pg, stage).await;
        let gate = Gate::new(stage);
        let release = Release(gate.clone());
        *api.0.export_gate.lock().unwrap() = Some(gate.clone());
        let http = Http::new(&api).await;
        let id = begin(&http, &asset).await;
        eprintln!("synthetic export stage {stage:?}: admitted");
        bounded(gate.entered.notified()).await;
        eprintln!("synthetic export stage {stage:?}: worker paused");
        assert_eq!(report(&api, &id).await.unwrap().state, JobState::Running);
        if stage == Stage::Write {
            assert_eq!(
                fs::metadata(artifact(&api, &id)).unwrap().len(),
                1024 * 1024,
                "one real artifact chunk written before cancellation"
            );
        } else {
            assert!(!artifact(&api, &id).exists());
        }
        assert_eq!(
            http.request("GET", &format!("/exports/{id}/download"), b"")
                .await
                .0,
            404
        );
        assert_eq!(
            http.request("DELETE", &format!("/exports/{id}"), b"")
                .await
                .0,
            200
        );
        eprintln!("synthetic export stage {stage:?}: DELETE returned");
        bounded(gate.cancelled.notified()).await;
        eprintln!("synthetic export stage {stage:?}: worker observed cancellation");
        assert_eq!(report(&api, &id).await.unwrap().state, JobState::Cancelling);
        assert!(
            quota(&api, &id).await.0 > 0,
            "reservation retained while blocked worker owns its IO"
        );
        drop(release);
        bounded(api.shutdown()).await.unwrap();
        eprintln!("synthetic export stage {stage:?}: drained");
        assert_eq!(report(&api, &id).await.unwrap().state, JobState::Cancelled);
        assert_eq!(
            report(&api, &id).await.unwrap().completed_bytes,
            gate.paused_bytes.load(Ordering::Relaxed),
            "cancellation stops progress at the paused render/write boundary"
        );
        assert_eq!(quota(&api, &id).await, (0, 0));
        assert!(!artifact(&api, &id).exists());
        assert_eq!(
            http.request("GET", &format!("/exports/{id}/download"), b"")
                .await
                .0,
            404
        );
        assert_eq!(fs::read(original).unwrap(), bytes);
        http.stop().await;
        drop(api);
        drop(store);
        sentinel(&fx).await;
        fx.finish().await;
    }
}
async fn restart_case(pg: bool) {
    let (fx, store, api, asset, bytes, original) = fixture(pg, Stage::Render).await;
    let gate = Gate::new(Stage::Render);
    let release = Release(gate.clone());
    *api.0.export_gate.lock().unwrap() = Some(gate.clone());
    let http = Http::new(&api).await;
    let id = begin(&http, &asset).await;
    bounded(gate.entered.notified()).await;
    // Match production/test-server ordering: stop HTTP, drain workers, then
    // close the pool and reopen. No Busy retry masks a retained Store owner.
    http.stop().await;
    let drain = api.clone();
    let stopping = tokio::spawn(async move { drain.shutdown().await });
    bounded(gate.cancelled.notified()).await;
    assert!(!stopping.is_finished());
    assert!(quota(&api, &id).await.0 > 0);
    drop(release);
    bounded(stopping).await.unwrap().unwrap();
    drop(api);
    drop(store);
    fx.catalog.pool().close().await;
    let catalog = Catalog::open(fx.config.clone()).await.unwrap();
    let reopened = Store::open(
        catalog.clone(),
        &fx.root.join("originals"),
        &fx.root.join("staging"),
        lytronaut_storage::Limits {
            free_floor: 0,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let api = Api::open(catalog.clone(), reopened.clone()).await.unwrap();
    assert_eq!(report(&api, &id).await.unwrap().state, JobState::Cancelled);
    assert_eq!(quota(&api, &id).await, (0, 0));
    assert_eq!(fs::read(original).unwrap(), bytes);
    api.shutdown().await.unwrap();
    drop(api);
    drop(reopened);
    if pg {
        let intact: bool = sqlx::query_scalar("SELECT count(*)=1 AND min(marker)='synthetic-persistence-parity' FROM public.lytronaut_parity_sentinel").fetch_one(catalog.pool()).await.unwrap();
        assert!(intact);
    }
    catalog.pool().close().await;
    fx.finish().await;
}
#[tokio::test]
async fn sqlite_http_cancel_during_render_and_partial_write() {
    cancel_case(false).await;
}
#[tokio::test]
async fn sqlite_http_shutdown_drains_workers_before_restart() {
    restart_case(false).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL with restricted role and sentinel"]
async fn postgres_http_cancel_during_render_and_partial_write() {
    cancel_case(true).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL with restricted role and sentinel"]
async fn postgres_http_shutdown_drains_workers_before_restart() {
    restart_case(true).await;
}
