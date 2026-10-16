//! Public synthetic tests of actual restore admission and HTTP/1.1 body frames.
//! The probe and its hooks are absent from production builds.
use super::*;
use crate::bundle::portable_bundle_tests::Fixture;
use std::{collections::HashSet, io::Read, net::TcpStream, sync::atomic::AtomicUsize};

pub(crate) struct Probe {
    inodes: Mutex<Option<u64>>,
    checks: AtomicUsize,
    hold_first: bool,
    entered: tokio::sync::Notify,
    waiting: tokio::sync::Notify,
    attempts: AtomicUsize,
    release: tokio::sync::Notify,
    extracted: Mutex<HashSet<String>>,
    frames: AtomicUsize,
    uploaded: AtomicU64,
    largest_frame: AtomicUsize,
}
impl Probe {
    pub(super) fn arm(api: &Api, hold_first: bool, inodes: Option<u64>) -> Arc<Self> {
        let probe = Arc::new(Self {
            inodes: Mutex::new(inodes),
            checks: AtomicUsize::new(0),
            hold_first,
            entered: Default::default(),
            waiting: Default::default(),
            attempts: AtomicUsize::new(0),
            release: Default::default(),
            extracted: Default::default(),
            frames: AtomicUsize::new(0),
            uploaded: AtomicU64::new(0),
            largest_frame: AtomicUsize::new(0),
        });
        *api.0.disk_probe.lock().unwrap() = Some(probe.clone());
        probe
    }
    pub(super) fn started(&self, job: &str) -> bool {
        self.extracted.lock().unwrap().contains(job)
    }
}
struct Release(Arc<Probe>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release.notify_one();
    }
}
pub(super) fn before_restore_permit(api: &Api) {
    if let Some(probe) = api.0.disk_probe.lock().unwrap().as_ref()
        && probe.attempts.fetch_add(1, Ordering::AcqRel) == 1
    {
        probe.waiting.notify_one();
    }
}
pub(super) fn observe_inode_check(api: &Api, actual: Option<u64>) -> Option<u64> {
    let probe = api.0.disk_probe.lock().unwrap().clone();
    let Some(probe) = probe else {
        return actual;
    };
    probe.checks.fetch_add(1, Ordering::AcqRel);
    probe.inodes.lock().unwrap().or(actual)
}
pub(super) async fn before_extraction(api: &Api, job: &str) {
    let probe = api.0.disk_probe.lock().unwrap().clone();
    if let Some(probe) = probe {
        let first = {
            let mut extracted = probe.extracted.lock().unwrap();
            let first = extracted.is_empty();
            extracted.insert(job.to_owned());
            first
        };
        if first && probe.hold_first {
            probe.entered.notify_one();
            probe.release.notified().await;
        }
    }
}
pub(super) fn observe_upload_frame(api: &Api, bytes: usize) {
    if let Some(probe) = api.0.disk_probe.lock().unwrap().as_ref() {
        probe.frames.fetch_add(1, Ordering::Relaxed);
        probe.uploaded.fetch_add(bytes as u64, Ordering::Relaxed);
        probe.largest_frame.fetch_max(bytes, Ordering::Relaxed);
    }
}
async fn bounded<T>(future: impl Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(30), future)
        .await
        .expect("restore proof must settle")
}
struct Http {
    port: u16,
    task: tokio::task::JoinHandle<()>,
}
impl Http {
    async fn new(api: &Api) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let app = crate::router_with_routes(
            crate::AppState {
                catalog: api.0.catalog.clone(),
                security: crate::SecurityConfig::new("http://localhost").unwrap(),
            },
            api.clone().routes(),
        );
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        Self { port, task }
    }
    async fn request(&self, method: &str, path: &str, body: &[u8]) -> (u16, serde_json::Value) {
        self.framed(
            method,
            path,
            &format!("Content-Length: {}\r\n", body.len()),
            body,
        )
        .await
    }
    async fn framed(
        &self,
        method: &str,
        path: &str,
        framing: &str,
        body: &[u8],
    ) -> (u16, serde_json::Value) {
        let port = self.port;
        let headers = format!(
            "{method} {path} HTTP/1.1\r\nHost: localhost\r\nOrigin: http://localhost\r\nX-Lytronaut-Intent: mutation\r\nConnection: close\r\nContent-Type: application/octet-stream\r\n{framing}\r\n"
        );
        let body = body.to_vec();
        bounded(tokio::task::spawn_blocking(move || {
            let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(30)))
                .unwrap();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(30)))
                .unwrap();
            stream.write_all(headers.as_bytes()).unwrap();
            // Large socket writes let hyper, rather than a synthetic Body,
            // split the upload into its actual transport frames.
            for chunk in body.chunks(1024 * 1024) {
                stream.write_all(chunk).unwrap();
            }
            let mut response = Vec::new();
            match stream.read_to_end(&mut response) {
                Ok(_) => (),
                Err(error)
                    if error.kind() == std::io::ErrorKind::ConnectionReset
                        && !response.is_empty() =>
                {
                    ()
                }
                Err(error) => panic!("HTTP restore read failed: {error}"),
            }
            let split = response
                .windows(4)
                .position(|bytes| bytes == b"\r\n\r\n")
                .unwrap();
            let status = std::str::from_utf8(&response[..split])
                .unwrap()
                .split_whitespace()
                .nth(1)
                .unwrap()
                .parse()
                .unwrap();
            let body = serde_json::from_slice(&response[split + 4..]).unwrap();
            (status, body)
        }))
        .await
        .unwrap()
    }
    async fn upload(&self, body: &[u8]) -> String {
        let (status, report) = self.request("POST", "/restores", body).await;
        assert_eq!(status, 200, "{report}");
        report["id"].as_str().unwrap().to_owned()
    }
    async fn terminal(&self, job: &str) -> serde_json::Value {
        bounded(async {
            loop {
                let (status, report) = self.request("GET", &format!("/exports/{job}"), b"").await;
                assert_eq!(status, 200, "{report}");
                if matches!(
                    report["state"].as_str(),
                    Some("succeeded" | "failed" | "cancelled")
                ) {
                    return report;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
    }
    async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}
async fn source(pg: bool, bytes: u64) -> (Fixture, Store, Api, Vec<u8>) {
    let fixture = Fixture::new(pg).await;
    let store = fixture.seed().await;
    if bytes > 0 {
        fixture.add_sparse_originals(1, bytes).await;
    }
    let output = fixture.root.join("public-http.bundle");
    bundle::backup(
        &fixture.catalog,
        &store,
        &output,
        u64::MAX,
        &AtomicBool::new(false),
        &AtomicU64::new(0),
    )
    .await
    .unwrap();
    let bytes = fs::read(output).unwrap();
    let api = Api::open(fixture.catalog.clone(), store.clone())
        .await
        .unwrap();
    (fixture, store, api, bytes)
}
async fn finish(fixture: Fixture, store: Store, api: Api, http: Http) {
    api.shutdown().await.unwrap();
    if fixture.pg.is_some() {
        let intact:bool=sqlx::query_scalar("SELECT count(*)=1 AND min(marker)='synthetic-persistence-parity' FROM public.lytronaut_parity_sentinel").fetch_one(fixture.catalog.pool()).await.unwrap();
        assert!(intact);
    }
    http.stop().await;
    drop(api);
    drop(store);
    fixture.finish().await;
}
async fn serialized_inode_checks(pg: bool) {
    let (fixture, store, api, bytes) = source(pg, 0).await;
    let probe = Probe::arm(&api, true, Some(u64::MAX));
    let release = Release(probe.clone());
    let http = Http::new(&api).await;
    let first = http.upload(&bytes).await;
    bounded(probe.entered.notified()).await;
    let second = http.upload(&bytes).await;
    // Wait until the second worker actually reaches permit admission. The
    // isolated API has one permit and only these two jobs; the paused first
    // restore must hold it, independent of second-worker scheduling latency.
    bounded(probe.waiting.notified()).await;
    let permit_held = api.0.validation.available_permits() == 0;
    let held_checks = probe.checks.load(Ordering::Acquire);
    *probe.inodes.lock().unwrap() = Some(0);
    drop(release);
    let first_report = http.terminal(&first).await;
    let failed = http.terminal(&second).await;
    let first_started = probe.started(&first);
    let second_started = probe.started(&second);
    let mut cleaned = Vec::new();
    for job in [first, second] {
        let status = http
            .request("DELETE", &format!("/exports/{job}"), b"")
            .await
            .0;
        let reserved: i64 =
            sqlx::query_scalar("SELECT reserved+retained FROM export_jobs WHERE id=$1")
                .bind(job)
                .fetch_one(fixture.catalog.pool())
                .await
                .unwrap();
        cleaned.push((status, reserved));
    }
    finish(fixture, store, api, http).await;
    assert!(
        permit_held,
        "first restore did not retain the validation permit through extraction"
    );
    assert_eq!(held_checks, 1);
    assert_eq!(first_report["state"], "succeeded");
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["error"]["code"], "resource_limit");
    assert_eq!(failed["error"]["retryable"], true);
    assert_eq!(probe.checks.load(Ordering::Acquire), 2);
    assert!(first_started);
    assert!(
        !second_started,
        "insufficient inodes must refuse before journal preparation/extraction"
    );
    assert_eq!(cleaned, vec![(200, 0), (200, 0)]);
}
async fn real_http_frames(pg: bool) {
    let (fixture, store, api, bytes) = source(pg, 2 * 1024 * 1024).await;
    let probe = Probe::arm(&api, false, None);
    let http = Http::new(&api).await;
    let job = http.upload(&bytes).await;
    assert_eq!(http.terminal(&job).await["state"], "succeeded");
    let frames = probe.frames.load(Ordering::Relaxed);
    let largest = probe.largest_frame.load(Ordering::Relaxed);
    assert!(frames > 1);
    assert_eq!(probe.uploaded.load(Ordering::Relaxed), bytes.len() as u64);
    // Pinned hyper1.12 HTTP/1.1 read buffer caps this transport path at417792.
    assert!(
        largest <= 417_792,
        "actual HTTP/1.1 body frame was{largest} bytes"
    );
    assert!(probe.started(&job));
    assert_eq!(
        http.request("DELETE", &format!("/exports/{job}"), b"")
            .await
            .0,
        200
    );
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM export_jobs")
        .fetch_one(fixture.catalog.pool())
        .await
        .unwrap();
    let (status, _) = http
        .framed(
            "POST",
            "/restores",
            "Transfer-Encoding: chunked\r\n",
            b"4\r\nnoop\r\n0\r\n\r\n",
        )
        .await;
    assert_eq!(status, 400, "restore requires a declared Content-Length");
    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM export_jobs")
        .fetch_one(fixture.catalog.pool())
        .await
        .unwrap();
    assert_eq!(before, after);
    finish(fixture, store, api, http).await;
}
#[tokio::test]
async fn sqlite_inode_checks_are_serialized_before_extraction() {
    serialized_inode_checks(false).await;
}
#[tokio::test]
async fn sqlite_real_http_upload_frames_and_missing_length_refusal() {
    real_http_frames(false).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL service"]
async fn postgres_inode_checks_are_serialized_before_extraction() {
    serialized_inode_checks(true).await;
}
#[tokio::test]
#[ignore = "requires explicit disposable PostgreSQL service"]
async fn postgres_real_http_upload_frames_and_missing_length_refusal() {
    real_http_frames(true).await;
}
