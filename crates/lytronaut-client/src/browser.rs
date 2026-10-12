use crate::model::*;
use js_sys::{Array, Reflect, Uint8Array};
use leptos::prelude::*;
use lytronaut_contracts::*;
use serde::de::DeserializeOwned;
use std::{cell::RefCell, collections::BTreeMap, rc::Rc};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::{JsFuture, spawn_local};
use web_sys::{
    AbortController, Blob, BlobPropertyBag, File, Request, RequestCredentials, RequestInit,
    Response,
};

type Result<T> = std::result::Result<T, String>;
#[derive(Clone, Default)]
struct Net {
    admission: Rc<RefCell<Admission>>,
}
struct BusyGuard(RwSignal<bool>);
impl Drop for BusyGuard {
    fn drop(&mut self) {
        self.0.set(false);
    }
}
struct Lease {
    net: Net,
    bytes: u64,
}
impl Drop for Lease {
    fn drop(&mut self) {
        self.net.admission.borrow_mut().release(self.bytes);
    }
}
async fn pause(ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let _ = web_sys::window()
            .unwrap()
            .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(promise).await;
}
fn cancelled(abort: &AbortController) -> bool {
    abort.signal().aborted()
}
impl Net {
    async fn lease(&self, bytes: u64, abort: &AbortController) -> Result<Lease> {
        loop {
            if cancelled(abort) {
                return Err("Cancelled".into());
            }
            if self.admission.borrow_mut().acquire(bytes) {
                return Ok(Lease {
                    net: self.clone(),
                    bytes,
                });
            }
            pause(30).await;
        }
    }
    async fn fetch(
        &self,
        path: &str,
        method: &str,
        csrf: &str,
        body: Option<&JsValue>,
        extra: &[(&str, &str)],
        abort: &AbortController,
    ) -> Result<Response> {
        let init = RequestInit::new();
        init.set_method(method);
        init.set_credentials(RequestCredentials::SameOrigin);
        init.set_signal(Some(&abort.signal()));
        if let Some(body) = body {
            init.set_body(body);
        }
        let req = Request::new_with_str_and_init(path, &init).map_err(|_| "Request unavailable")?;
        if !csrf.is_empty() {
            req.headers()
                .set("X-Lytronaut-Csrf", csrf)
                .map_err(|_| "Request unavailable")?;
        }
        for (k, v) in extra {
            req.headers().set(k, v).map_err(|_| "Request unavailable")?;
        }
        for attempt in 0..3 {
            if cancelled(abort) {
                return Err("Cancelled".into());
            }
            let response = JsFuture::from(web_sys::window().unwrap().fetch_with_request(&req))
                .await
                .map_err(|_| {
                    if cancelled(abort) {
                        "Cancelled"
                    } else {
                        "Connection interrupted. Retry when the service is available"
                    }
                })?
                .dyn_into::<Response>()
                .map_err(|_| "Invalid response")?;
            if response.status() == 429 && method == "GET" && attempt < 2 {
                if let Some(stream) = response.body() {
                    let _ = JsFuture::from(stream.cancel()).await;
                }
                pause(250 * (attempt + 1)).await;
                continue;
            }
            if !response.ok() {
                let message = match response.status() {
                    401 => "Session expired or credentials denied. Sign in again",
                    429 => "Resource limit reached. Wait, then retry",
                    400 => "Selection or request was rejected. Check the input and retry",
                    404 => "This item is no longer available",
                    _ => {
                        "The service could not complete this request. Retry or inspect import status"
                    }
                };
                if let Some(stream) = response.body() {
                    let _ = JsFuture::from(stream.cancel()).await;
                }
                return Err(message.into());
            }
            return Ok(response);
        }
        Err("Resource limit reached".into())
    }
    async fn json<T: DeserializeOwned>(
        &self,
        path: &str,
        method: &str,
        csrf: &str,
        body: Option<serde_json::Value>,
    ) -> Result<T> {
        let abort = AbortController::new().map_err(|_| "Browser cancellation unavailable")?;
        let body = body.map(|b| JsValue::from_str(&b.to_string()));
        let response = self
            .fetch(
                path,
                method,
                csrf,
                body.as_ref(),
                &[("Content-Type", "application/json")],
                &abort,
            )
            .await?;
        let bytes = read_cancel(response, JSON_LIMIT, &abort).await?;
        serde_json::from_slice(&bytes).map_err(|_| "Invalid or oversized service response".into())
    }
    async fn mutation(&self, path: &str, csrf: &str) -> Result<()> {
        let abort = AbortController::new().map_err(|_| "Browser cancellation unavailable")?;
        self.fetch(path, "POST", csrf, None, &[], &abort).await?;
        Ok(())
    }
    async fn preview(&self, id: &OpaqueId, abort: &AbortController, grid: bool) -> Result<String> {
        let manifest_lease = self.lease(0, abort).await?;
        let response = self
            .fetch(
                &format!("/assets/{}/manifest", id.as_str()),
                "GET",
                "",
                None,
                &[],
                abort,
            )
            .await?;
        let manifest: AssetManifest =
            serde_json::from_slice(&read_cancel(response, JSON_LIMIT, abort).await?)
                .map_err(|_| "Invalid preview manifest")?;
        let bytes = viewer_bytes(&manifest).map_err(str::to_owned)?;
        // Reserve 32 MiB for persistent stage (<=28 MiB) and 24 downsampled grid images.
        let limit = if grid {
            64 * 1024 * 1024
        } else {
            28 * 1024 * 1024
        };
        if bytes > limit || manifest.id != *id {
            return Err("Preview exceeds display budget. Download the original instead".into());
        }
        drop(manifest_lease);
        let _lease = self.lease(bytes, abort).await?;
        let mut encoded = Vec::with_capacity(manifest.encoded_bytes as usize);
        for (index, chunk) in manifest.chunks.iter().enumerate() {
            if cancelled(abort) {
                return Err("Cancelled".into());
            }
            let response = self
                .fetch(
                    &format!("/assets/{}/chunks/{index}", id.as_str()),
                    "GET",
                    "",
                    None,
                    &[],
                    abort,
                )
                .await?;
            let part = read_cancel(response, chunk.length as usize, abort).await?;
            if part.len() as u64 != chunk.length {
                return Err("Preview length mismatch".into());
            }
            encoded.extend(part);
        }
        if cancelled(abort) {
            return Err("Cancelled".into());
        }
        if !verified(&encoded, &manifest) {
            return Err("Preview integrity check failed. Originals remain downloadable".into());
        }
        let parts = Array::new();
        parts.push(&Uint8Array::from(encoded.as_slice()));
        let options = BlobPropertyBag::new();
        options.set_type("image/jpeg");
        let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options)
            .map_err(|_| "Preview unavailable")?;
        let url =
            web_sys::Url::create_object_url_with_blob(&blob).map_err(|_| "Preview unavailable")?;
        if grid {
            downsample(&url, abort).await
        } else {
            Ok(url)
        }
    }
}
// Rust-controlled browser JPEG decode/downsample; original bytes are unchanged.
async fn downsample(url: &str, abort: &AbortController) -> Result<String> {
    let image = match web_sys::HtmlImageElement::new() {
        Ok(image) => image,
        Err(_) => {
            let _ = web_sys::Url::revoke_object_url(url);
            return Err("Image decode unavailable".into());
        }
    };
    image.set_src(url);
    let decoded = JsFuture::from(image.decode()).await;
    let result = (|| {
        if decoded.is_ok() && !cancelled(abort) {
            let canvas = web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .create_element("canvas")
                .map_err(|_| "Canvas unavailable")?
                .dyn_into::<web_sys::HtmlCanvasElement>()
                .map_err(|_| "Canvas unavailable")?;
            canvas.set_width(128);
            canvas.set_height(128);
            let context = canvas
                .get_context("2d")
                .map_err(|_| "Canvas unavailable")?
                .ok_or("Canvas unavailable")?
                .dyn_into::<web_sys::CanvasRenderingContext2d>()
                .map_err(|_| "Canvas unavailable")?;
            let scale =
                (128.0 / image.natural_width() as f64).min(128.0 / image.natural_height() as f64);
            let width = image.natural_width() as f64 * scale;
            let height = image.natural_height() as f64 * scale;
            context
                .draw_image_with_html_image_element_and_dw_and_dh(
                    &image,
                    (128.0 - width) / 2.0,
                    (128.0 - height) / 2.0,
                    width,
                    height,
                )
                .map_err(|_| "Thumbnail unavailable")?;
            let data = canvas
                .to_data_url_with_type("image/jpeg")
                .map_err(|_| "Thumbnail unavailable")?;
            if data.len() > 128 * 1024 {
                return Err("Thumbnail exceeds persistent budget".into());
            }
            Ok(data)
        } else {
            Err("Preview cancelled or decode failed".into())
        }
    })();
    image.set_src("");
    let _ = web_sys::Url::revoke_object_url(url);
    result
}
async fn read_cancel(response: Response, limit: usize, abort: &AbortController) -> Result<Vec<u8>> {
    let result = read(response, limit).await;
    if cancelled(abort) {
        Err("Cancelled".into())
    } else {
        result
    }
}
async fn read(response: Response, limit: usize) -> Result<Vec<u8>> {
    if response
        .headers()
        .get("Content-Length")
        .ok()
        .flatten()
        .and_then(|v| v.parse::<usize>().ok())
        .is_some_and(|v| v > limit)
    {
        if let Some(body) = response.body() {
            let _ = JsFuture::from(body.cancel()).await;
        }
        return Err("Response exceeds resource budget".into());
    }
    let stream = response.body().ok_or("Response body missing")?;
    let reader = stream
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()
        .map_err(|_| "Response stream unavailable")?;
    let mut bytes = Vec::new();
    loop {
        let part = JsFuture::from(reader.read())
            .await
            .map_err(|_| "Response interrupted")?;
        if Reflect::get(&part, &"done".into())
            .ok()
            .and_then(|v| v.as_bool())
            == Some(true)
        {
            break;
        }
        let part = Uint8Array::new(
            &Reflect::get(&part, &"value".into()).map_err(|_| "Invalid response stream")?,
        );
        if bytes.len().saturating_add(part.length() as usize) > limit {
            let _ = JsFuture::from(reader.cancel()).await;
            return Err("Response exceeds resource budget".into());
        }
        bytes.extend(part.to_vec());
    }
    Ok(bytes)
}
#[derive(Clone)]
struct Ui {
    net: Net,
    csrf: RwSignal<String>,
    signed_in: RwSignal<bool>,
    status: RwSignal<String>,
    libraries: RwSignal<Vec<LibrarySummary>>,
    library: RwSignal<Option<LibrarySummary>>,
    albums: RwSignal<Vec<AlbumSummary>>,
    album: RwSignal<Option<String>>,
    captures: RwSignal<Vec<CaptureSummary>>,
    detail: RwSignal<Option<CaptureDetail>>,
    thumbs: RwSignal<BTreeMap<String, String>>,
    stage: RwSignal<String>,
    gpu_enabled: RwSignal<bool>,
    gpu_busy: RwSignal<bool>,
    gpu_quality: RwSignal<u32>,
    gpu_visible: RwSignal<bool>,
    gpu_status: RwSignal<String>,
    gpu: Rc<RefCell<Option<crate::gpu::Renderer>>>,
    focus_stack: RwSignal<Option<FocusStack>>,
    video_lambdas: RwSignal<Vec<f64>>,
    video_label: RwSignal<String>,
    displayed_derivative: RwSignal<Option<FocusDerivative>>,
    focus_value: RwSignal<f64>,
    orientation: RwSignal<u8>,
    search: RwSignal<String>,
    sort: RwSignal<String>,
    offset: RwSignal<u32>,
    next: RwSignal<Option<u32>>,
    files: RwSignal<Vec<PreservedFile>>,
    list_next: RwSignal<Option<u32>>,
    album_next: RwSignal<Option<u32>>,
    generation: Rc<RefCell<u64>>,
    stage_generation: Rc<RefCell<u64>>,
    aborts: Rc<RefCell<Vec<AbortController>>>,
    stage_abort: Rc<RefCell<Option<AbortController>>>,
    busy: RwSignal<bool>,
    job: RwSignal<String>,
    progress: RwSignal<u64>,
    total: RwSignal<u64>,
    upload_abort: Rc<RefCell<Option<web_sys::XmlHttpRequest>>>,
    upload_cancelled: Rc<RefCell<bool>>,
    export_job: RwSignal<Option<ExportProgress>>,
    export_busy: RwSignal<bool>,
    export_abort: Rc<RefCell<Option<AbortController>>>,
}
impl Ui {
    fn new() -> Self {
        Self {
            net: Net::default(),
            csrf: RwSignal::new(String::new()),
            signed_in: RwSignal::new(false),
            status: RwSignal::new("Connecting to local service…".into()),
            libraries: RwSignal::new(Vec::new()),
            library: RwSignal::new(None),
            albums: RwSignal::new(Vec::new()),
            album: RwSignal::new(None),
            captures: RwSignal::new(Vec::new()),
            detail: RwSignal::new(None),
            thumbs: RwSignal::new(BTreeMap::new()),
            stage: RwSignal::new(String::new()),
            gpu_enabled: RwSignal::new(true),
            gpu_busy: RwSignal::new(false),
            gpu_quality: RwSignal::new(512),
            gpu_visible: RwSignal::new(false),
            gpu_status: RwSignal::new("CPU JPEG presentation".into()),
            gpu: Rc::default(),
            focus_stack: RwSignal::new(None),
            video_lambdas: RwSignal::new(Vec::new()),
            video_label: RwSignal::new(String::new()),
            displayed_derivative: RwSignal::new(None),
            focus_value: RwSignal::new(0.0),
            orientation: RwSignal::new(1),
            search: RwSignal::new(String::new()),
            sort: RwSignal::new("order".into()),
            offset: RwSignal::new(0),
            next: RwSignal::new(None),
            files: RwSignal::new(Vec::new()),
            list_next: RwSignal::new(None),
            album_next: RwSignal::new(None),
            generation: Rc::default(),
            stage_generation: Rc::default(),
            aborts: Rc::default(),
            stage_abort: Rc::default(),
            busy: RwSignal::new(false),
            job: RwSignal::new(String::new()),
            progress: RwSignal::new(0),
            total: RwSignal::new(1),
            upload_abort: Rc::default(),
            upload_cancelled: Rc::default(),
            export_job: RwSignal::new(None),
            export_busy: RwSignal::new(false),
            export_abort: Rc::default(),
        }
    }
    fn fail(&self, error: String) {
        if error.starts_with("Session expired") {
            self.clear_assets();
            self.csrf.set(String::new());
            self.signed_in.set(false);
        }
        self.status.set(error);
    }
    fn clear_stage(&self) {
        if self.gpu_enabled.get_untracked() {
            self.gpu_status.set("CPU JPEG presentation".into());
        }
        self.gpu_visible.set(false);
        self.gpu.borrow_mut().take();
        *self.stage_generation.borrow_mut() += 1;
        if let Some(a) = self.stage_abort.borrow_mut().take() {
            a.abort();
        }
        let url = self.stage.get_untracked();
        if url.starts_with("blob:") {
            let _ = web_sys::Url::revoke_object_url(&url);
        }
        self.stage.set(String::new());
        self.detail.set(None);
        self.focus_stack.set(None);
        self.video_lambdas.set(Vec::new());
        self.video_label.set(String::new());
        self.displayed_derivative.set(None);
        self.orientation.set(1);
    }
    fn clear_assets(&self) {
        *self.generation.borrow_mut() += 1;
        for a in self.aborts.borrow_mut().drain(..) {
            a.abort();
        }
        for url in self.thumbs.get_untracked().values() {
            if url.starts_with("blob:") {
                let _ = web_sys::Url::revoke_object_url(url);
            }
        }
        self.thumbs.set(BTreeMap::new());
        self.clear_stage();
    }
    async fn session(&self) -> Result<()> {
        let session: serde_json::Value = self.net.json("/auth/session", "GET", "", None).await?;
        let token = session["csrf"].as_str().ok_or("Invalid session response")?;
        self.csrf.set(token.into());
        self.signed_in.set(true);
        self.libraries(0).await
    }
    async fn watch_export(&self, mut report: ExportProgress) -> Result<()> {
        let session = self.csrf.get_untracked();
        if let Ok(Some(storage)) = web_sys::window().unwrap().session_storage() {
            let _ = storage.set_item("lytronaut-export", report.job.id.as_str());
        }
        loop {
            self.export_job.set(Some(report.clone()));
            if matches!(
                report.job.state,
                JobState::Succeeded | JobState::Failed | JobState::Cancelled
            ) {
                self.status.set(match (report.job.state,report.kind) {
                    (JobState::Succeeded,ExportKind::Flatten)=>"Flattened export complete. Download the image and separate provenance. Originals are unchanged.",
                    (JobState::Succeeded,ExportKind::Archive)=>"Private backup complete. Download the originals/catalog bundle.",
                    (JobState::Succeeded,ExportKind::Restore)=>"Isolated restore complete. Current library unchanged.",
                    (JobState::Cancelled,_)=>"Export / restore cancelled. Originals are unchanged.",
                    _=>"Export / restore failed. Remove unsupported operations or select a fresh valid bundle, then retry."
                }.into());
                return Ok(());
            }
            pause(150).await;
            if !self.signed_in.get_untracked() || self.csrf.get_untracked() != session {
                return Ok(());
            }
            report = self
                .net
                .json(
                    &format!("/exports/{}", report.job.id.as_str()),
                    "GET",
                    "",
                    None,
                )
                .await?;
        }
    }
    async fn recover_export(&self) -> Result<()> {
        if self.export_busy.get_untracked() {
            return Ok(());
        }
        let id = web_sys::window()
            .unwrap()
            .session_storage()
            .map_err(|_| "Job recovery unavailable")?
            .and_then(|s| s.get_item("lytronaut-export").ok().flatten())
            .ok_or("No saved export or restore job")?;
        OpaqueId::try_from(id.clone()).map_err(|_| "Invalid saved export job")?;
        self.export_busy.set(true);
        let _busy = BusyGuard(self.export_busy);
        let report = self
            .net
            .json(&format!("/exports/{id}"), "GET", "", None)
            .await?;
        self.watch_export(report).await
    }
    async fn export(&self, format: Option<ExportFormat>) -> Result<()> {
        if format.is_some() && !self.video_lambdas.get_untracked().is_empty() {
            return Err("Browser video export is unavailable. Select a verified static/JPEG preview before exporting pixels".into());
        }
        if self.export_busy.get_untracked() {
            return Ok(());
        }
        self.export_busy.set(true);
        let _busy = BusyGuard(self.export_busy);
        let request = if let Some(format) = format {
            let (asset, plane_index, orientation) =
                if let Some(d) = self.displayed_derivative.get_untracked() {
                    (d.id, Some(d.plane_index), d.orientation)
                } else {
                    (
                        self.detail
                            .get_untracked()
                            .and_then(|d| d.preview)
                            .ok_or("Select verified JPEG pixels first")?,
                        None,
                        1,
                    )
                };
            ExportRequest::Flatten {
                version: WireVersion::V1,
                asset,
                format,
                plane_index,
                orientation,
                operations: Vec::new(),
            }
        } else {
            ExportRequest::ArchiveOriginals {
                version: WireVersion::V1,
            }
        };
        self.status.set(
            "Creating export… Stored pixels only; imported adjustments remain preserved.".into(),
        );
        let report = self
            .net
            .json(
                "/exports",
                "POST",
                &self.csrf.get_untracked(),
                Some(serde_json::to_value(request).map_err(|_| "Invalid export")?),
            )
            .await?;
        self.watch_export(report).await
    }
    async fn restore_bundle(&self, file: File) -> Result<()> {
        if self.export_busy.get_untracked() {
            return Ok(());
        }
        self.export_busy.set(true);
        let _busy = BusyGuard(self.export_busy);
        if file.size() == 0.0
            || file.size() > 21.0 * 1024.0 * 1024.0 * 1024.0 + 32.0 * 1024.0 * 1024.0
        {
            return Err("Bundle exceeds the server restore limit".into());
        }
        self.export_job.set(None);
        self.status
            .set("Uploading private bundle for isolated restore…".into());
        let abort = AbortController::new().map_err(|_| "Browser cancellation unavailable")?;
        *self.export_abort.borrow_mut() = Some(abort.clone());
        let _lease = self.net.lease(0, &abort).await?;
        let response = self
            .net
            .fetch(
                "/restores",
                "POST",
                &self.csrf.get_untracked(),
                Some(file.as_ref()),
                &[("Content-Type", "application/octet-stream")],
                &abort,
            )
            .await?;
        let report = serde_json::from_slice(&read_cancel(response, JSON_LIMIT, &abort).await?)
            .map_err(|_| "Invalid restore report")?;
        drop(_lease);
        self.export_abort.borrow_mut().take();
        self.watch_export(report).await
    }
    async fn libraries(&self, offset: u32) -> Result<()> {
        let session = self.csrf.get_untracked();
        let page: Result<Page<LibrarySummary>> = self
            .net
            .json(
                &format!("/libraries?limit=24&offset={offset}"),
                "GET",
                "",
                None,
            )
            .await;
        if !self.signed_in.get_untracked() || self.csrf.get_untracked() != session {
            return Ok(());
        }
        let page = page?;
        self.list_next.set(page.next_offset);
        if offset == 0 {
            self.libraries.set(page.items);
        } else {
            self.libraries.update(|v| v.extend(page.items));
        }
        if self.library.get_untracked().is_none()
            && let Some(library) = self.libraries.get_untracked().first()
        {
            self.choose_library(library.clone()).await?;
        }
        self.status.set("Library ready".into());
        Ok(())
    }
    async fn choose_library(&self, library: LibrarySummary) -> Result<()> {
        let session = self.csrf.get_untracked();
        self.clear_assets();
        self.library.set(Some(library.clone()));
        self.album.set(None);
        self.albums.set(Vec::new());
        self.files.set(Vec::new());
        self.offset.set(0);
        if library.state == LibraryState::Ready {
            self.load_albums(0).await?;
            if self.csrf.get_untracked() != session
                || !self.signed_in.get_untracked()
                || self
                    .library
                    .get_untracked()
                    .is_none_or(|l| l.id != library.id)
            {
                return Ok(());
            }
            self.page(0).await?;
        } else {
            self.captures.set(Vec::new());
            self.load_files(0).await?;
            self.status
                .set("Library is preserved only; browse and download originals below".into());
        }
        Ok(())
    }
    async fn load_albums(&self, offset: u32) -> Result<()> {
        let session = self.csrf.get_untracked();
        let library = self.library.get_untracked().ok_or("Select a library")?;
        let page: Result<Page<AlbumSummary>> = self
            .net
            .json(
                &format!(
                    "/libraries/{}/albums?limit=24&offset={offset}",
                    library.id.as_str()
                ),
                "GET",
                "",
                None,
            )
            .await;
        if !self.signed_in.get_untracked()
            || self.csrf.get_untracked() != session
            || self
                .library
                .get_untracked()
                .is_none_or(|l| l.id != library.id)
        {
            return Ok(());
        }
        let page = page?;
        self.album_next.set(page.next_offset);
        if offset == 0 {
            self.albums.set(page.items);
        } else {
            self.albums.update(|a| a.extend(page.items));
        }
        Ok(())
    }
    async fn load_files(&self, offset: u32) -> Result<()> {
        let session = self.csrf.get_untracked();
        self.clear_assets();
        let generation = *self.generation.borrow();
        let library = self.library.get_untracked().ok_or("Select a library")?;
        let page: Result<Page<PreservedFile>> = self
            .net
            .json(
                &format!(
                    "/libraries/{}/files?limit=24&offset={offset}",
                    library.id.as_str()
                ),
                "GET",
                "",
                None,
            )
            .await;
        if !self.signed_in.get_untracked()
            || self.csrf.get_untracked() != session
            || generation != *self.generation.borrow()
            || self
                .library
                .get_untracked()
                .is_none_or(|l| l.id != library.id)
        {
            return Ok(());
        }
        let page = page?;
        self.next.set(page.next_offset);
        self.offset.set(offset);
        self.files.set(page.items);
        Ok(())
    }
    async fn page(&self, offset: u32) -> Result<()> {
        let session = self.csrf.get_untracked();
        let library = self.library.get_untracked().ok_or("Select a library")?;
        if library.state != LibraryState::Ready {
            return self.load_files(offset).await;
        }
        let query = query(
            &self.search.get_untracked(),
            &self.sort.get_untracked(),
            offset,
        )
        .map_err(str::to_owned)?;
        self.clear_assets();
        let generation = *self.generation.borrow();
        self.status.set("Loading captures…".into());
        self.captures.set(Vec::new());
        let base = self
            .album
            .get_untracked()
            .map(|a| format!("/albums/{a}/captures"))
            .unwrap_or_else(|| format!("/libraries/{}/captures", library.id.as_str()));
        let page: Result<Page<CaptureSummary>> = self
            .net
            .json(&format!("{base}?{query}"), "GET", "", None)
            .await;
        if generation != *self.generation.borrow()
            || self.csrf.get_untracked() != session
            || !self.signed_in.get_untracked()
        {
            return Ok(());
        }
        let page = page?;
        self.offset.set(offset);
        self.next.set(page.next_offset);
        self.captures.set(page.items.clone());
        self.status.set(if page.items.is_empty() {
            "No captures match this search".into()
        } else {
            format!("Showing {} captures on this page", page.items.len())
        });
        // Two workers share the same admission controller with stage requests.
        for worker in 0..2 {
            let ui = self.clone();
            let items = page.items.clone();
            spawn_local(async move {
                for capture in items.into_iter().skip(worker).step_by(2) {
                    if generation != *ui.generation.borrow() {
                        break;
                    }
                    let abort = AbortController::new().unwrap();
                    ui.aborts.borrow_mut().push(abort.clone());
                    let result = async {
                        let response = ui
                            .net
                            .fetch(
                                &format!("/captures/{}", capture.id.as_str()),
                                "GET",
                                "",
                                None,
                                &[],
                                &abort,
                            )
                            .await?;
                        let detail: CaptureDetail = serde_json::from_slice(
                            &read_cancel(response, JSON_LIMIT, &abort).await?,
                        )
                        .map_err(|_| "Invalid capture details")?;
                        if let Some(id) = detail.preview {
                            ui.net.preview(&id, &abort, true).await
                        } else {
                            Err("Preview unavailable; originals preserved".into())
                        }
                    }
                    .await;
                    if generation != *ui.generation.borrow() {
                        if let Ok(url) = result {
                            let _ = web_sys::Url::revoke_object_url(&url);
                        }
                        break;
                    }
                    ui.thumbs.update(|map| {
                        map.insert(
                            capture.id.as_str().into(),
                            result.unwrap_or_else(|_| "unavailable".into()),
                        );
                    });
                }
            });
        }
        if let Some(capture) = page.items.first() {
            self.select(capture.id.clone()).await?;
        }
        Ok(())
    }
    async fn select(&self, id: OpaqueId) -> Result<()> {
        self.clear_stage();
        let generation = *self.stage_generation.borrow();
        let abort = AbortController::new().map_err(|_| "Browser cancellation unavailable")?;
        *self.stage_abort.borrow_mut() = Some(abort.clone());
        self.status.set("Loading capture…".into());
        let response = self
            .net
            .fetch(
                &format!("/captures/{}", id.as_str()),
                "GET",
                "",
                None,
                &[],
                &abort,
            )
            .await?;
        let detail: CaptureDetail =
            serde_json::from_slice(&read_cancel(response, JSON_LIMIT, &abort).await?)
                .map_err(|_| "Invalid capture details")?;
        if generation != *self.stage_generation.borrow() {
            return Ok(());
        }
        self.detail.set(Some(detail.clone()));
        if let Some(id) = detail.preview {
            let result = self.net.preview(&id, &abort, false).await;
            if generation != *self.stage_generation.borrow() {
                if let Ok(url) = result {
                    let _ = web_sys::Url::revoke_object_url(&url);
                }
                return Ok(());
            }
            match result {
                Ok(url) => {
                    self.stage.set(url);
                    self.status
                        .set("Static JPEG preview ready. Color profile unverified".into());
                }
                Err(error) => self.fail(error),
            }
        } else {
            self.status
                .set("Preview unavailable. Preserved originals remain downloadable".into());
        }
        if let Some(file) = detail
            .originals
            .iter()
            .find(|f| f.role == OriginalRole::JpegFocusStack)
        {
            let response = self
                .net
                .fetch(
                    &format!("/assets/{}/focus", file.id.as_str()),
                    "GET",
                    "",
                    None,
                    &[],
                    &abort,
                )
                .await;
            let result = async {
                let response = response?;
                let stack: FocusStack =
                    serde_json::from_slice(&read_cancel(response, JSON_LIMIT, &abort).await?)
                        .map_err(|_| "Invalid focus manifest")?;
                stack.validate().map_err(|_| "Invalid focus stack")?;
                if stack.id != file.id {
                    return Err("Invalid focus identity".into());
                }
                Ok::<_, String>(stack)
            }
            .await;
            if generation != *self.stage_generation.borrow() {
                return Ok(());
            }
            match result {
                Ok(stack) => {
                    let first = stack.lambdas[0];
                    self.focus_stack.set(Some(stack));
                    self.change_focus(first).await?;
                }
                Err(_) => self.status.set(
                    "Discrete focus unavailable. Static preview and originals remain usable".into(),
                ),
            }
        }
        Ok(())
    }
    async fn change_focus(&self, requested: f64) -> Result<()> {
        let lambdas = self.video_lambdas.get_untracked();
        if !lambdas.is_empty() {
            if !requested.is_finite() {
                return Err("Invalid focus selection".into());
            }
            let index = lytronaut_format::annex_b::nearest_lambda(&lambdas, requested)
                .map_err(|_| "Invalid focus selection")?;
            self.focus_value.set(requested.clamp(
                lambdas.iter().copied().reduce(f64::min).unwrap(),
                lambdas.iter().copied().reduce(f64::max).unwrap(),
            ));
            return self.change_video(index).await;
        }
        let Some(stack) = self.focus_stack.get_untracked() else {
            return Ok(());
        };
        let index = stack
            .nearest(requested)
            .map_err(|_| "Invalid focus selection")?;
        self.focus_value
            .set(requested.clamp(stack.lambdas[0], *stack.lambdas.last().unwrap()));
        *self.stage_generation.borrow_mut() += 1;
        let generation = *self.stage_generation.borrow();
        if let Some(a) = self.stage_abort.borrow_mut().take() {
            a.abort();
        }
        let abort = AbortController::new().map_err(|_| "Browser cancellation unavailable")?;
        *self.stage_abort.borrow_mut() = Some(abort.clone());
        self.gpu_visible.set(false);
        let selected_orientation = self.orientation.get_untracked();
        let result = async {
            let _lease = self
                .net
                .lease(
                    if self.gpu_enabled.get_untracked() {
                        24 * 1024 * 1024
                    } else {
                        20 * 1024 * 1024
                    },
                    &abort,
                )
                .await?;
            let response = self
                .net
                .fetch(
                    &format!(
                        "/assets/{}/focus/{index}/{}",
                        stack.id.as_str(),
                        selected_orientation
                    ),
                    "GET",
                    "",
                    None,
                    &[],
                    &abort,
                )
                .await?;
            let metadata = response
                .headers()
                .get("X-Lytronaut-Derivative")
                .map_err(|_| "Derivative metadata unavailable")?
                .ok_or("Derivative metadata missing")?;
            if metadata.len() > JSON_LIMIT {
                return Err("Derivative metadata exceeds budget".into());
            }
            let derivative: FocusDerivative =
                serde_json::from_str(&metadata).map_err(|_| "Invalid derivative metadata")?;
            derivative
                .validate(&stack)
                .map_err(|_| "Invalid derivative extent or provenance")?;
            if derivative.plane_index != index || derivative.orientation != selected_orientation {
                return Err("Derivative selection mismatch".into());
            }
            let bytes = read_cancel(response, derivative.encoded_bytes as usize, &abort).await?;
            if !verified_derivative(&bytes, &derivative) {
                return Err(
                    "Derivative integrity check failed. Static preview and originals remain usable"
                        .into(),
                );
            }
            if cancelled(&abort) {
                return Err("Cancelled".into());
            }
            let parts = Array::new();
            parts.push(&Uint8Array::from(bytes.as_slice()));
            let options = BlobPropertyBag::new();
            options.set_type("image/jpeg");
            let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options)
                .map_err(|_| "Derivative unavailable")?;
            let url = web_sys::Url::create_object_url_with_blob(&blob)
                .map_err(|_| "Derivative unavailable")?;
            let result = async {
                let image = web_sys::HtmlImageElement::new()
                    .map_err(|_| "Derivative decode unavailable")?;
                image.set_src(&url);
                let decoded = JsFuture::from(image.decode()).await;
                let valid = decoded.is_ok()
                    && image.natural_width() == derivative.output_dimensions.width
                    && image.natural_height() == derivative.output_dimensions.height
                    && !cancelled(&abort);
                if !valid {
                    image.set_src("");
                    return Err("Derivative decode or geometry failed".into());
                }
                if generation == *self.stage_generation.borrow() && self.gpu_enabled.get_untracked()
                {
                    if selected_orientation == 1 {
                        self.present_gpu(&image, generation, 1).await;
                    } else {
                        match self.gpu_identity(&stack, index, &abort).await {
                            Ok((identity, url)) => {
                                self.present_gpu(&identity, generation, selected_orientation)
                                    .await;
                                identity.set_src("");
                                let _ = web_sys::Url::revoke_object_url(&url);
                            }
                            Err(_) => {
                                self.gpu.borrow_mut().take();
                                self.gpu_visible.set(false);
                                self.gpu_enabled.set(false);
                                self.gpu_status
                                    .set("CPU fallback · GPU source verification failed".into());
                            }
                        }
                    }
                }
                image.set_src("");
                Ok((url.clone(), derivative))
            }
            .await;
            if result.is_err() {
                let _ = web_sys::Url::revoke_object_url(&url);
            }
            result
        }
        .await;
        if generation != *self.stage_generation.borrow() {
            if let Ok((url, _)) = result {
                let _ = web_sys::Url::revoke_object_url(&url);
            }
            return Ok(());
        }
        match result {
            Ok((url, derivative)) => {
                self.displayed_derivative.set(Some(derivative));
                let old = self.stage.get_untracked();
                self.stage.set(url);
                if old.starts_with("blob:") {
                    let _ = web_sys::Url::revoke_object_url(&old);
                }
                self.status.set(format!("Discrete focus: stored plane {} of {}, lambda {} · {} × {} source · explicit EXIF {}. Imported edits preserved only", index+1, stack.lambdas.len(), stack.lambdas[index], stack.source_dimensions.width, stack.source_dimensions.height, self.orientation.get_untracked()));
                Ok(())
            }
            Err(error) => {
                if self.gpu_enabled.get_untracked() {
                    self.gpu_visible.set(false);
                    self.gpu_enabled.set(false);
                    self.gpu.borrow_mut().take();
                    self.gpu_status
                        .set("CPU fallback · derivative verification or geometry failed".into());
                }
                Err(error)
            }
        }
    }
    async fn gpu_identity(
        &self,
        stack: &FocusStack,
        index: usize,
        abort: &AbortController,
    ) -> Result<(web_sys::HtmlImageElement, String)> {
        // Remains inside the caller's 24 MiB lease. Focus DTO limits source to
        // 512² and each generated JPEG to 2 MiB; neither plane is cached.
        let response = self
            .net
            .fetch(
                &format!("/assets/{}/focus/{index}/1", stack.id.as_str()),
                "GET",
                "",
                None,
                &[],
                abort,
            )
            .await?;
        let metadata = response
            .headers()
            .get("X-Lytronaut-Derivative")
            .map_err(|_| "GPU metadata unavailable")?
            .ok_or("GPU metadata missing")?;
        if metadata.len() > JSON_LIMIT {
            return Err("GPU metadata exceeds budget".into());
        }
        let derivative: FocusDerivative =
            serde_json::from_str(&metadata).map_err(|_| "Invalid GPU metadata")?;
        derivative
            .validate(stack)
            .map_err(|_| "Invalid GPU derivative")?;
        if derivative.plane_index != index || derivative.orientation != 1 {
            return Err("GPU selection mismatch".into());
        }
        let bytes = read_cancel(response, derivative.encoded_bytes as usize, abort).await?;
        if !verified_derivative(&bytes, &derivative) {
            return Err("GPU integrity failed".into());
        }
        let parts = Array::new();
        parts.push(&Uint8Array::from(bytes.as_slice()));
        let options = BlobPropertyBag::new();
        options.set_type("image/jpeg");
        let blob = Blob::new_with_u8_array_sequence_and_options(&parts, &options)
            .map_err(|_| "GPU source unavailable")?;
        let url = web_sys::Url::create_object_url_with_blob(&blob)
            .map_err(|_| "GPU source unavailable")?;
        let result = async {
            let image = web_sys::HtmlImageElement::new().map_err(|_| "GPU decode unavailable")?;
            image.set_src(&url);
            JsFuture::from(image.decode())
                .await
                .map_err(|_| "GPU decode failed")?;
            if image.natural_width() != derivative.output_dimensions.width
                || image.natural_height() != derivative.output_dimensions.height
                || cancelled(abort)
            {
                image.set_src("");
                return Err("GPU geometry or cancellation failed".into());
            }
            Ok((image, url.clone()))
        }
        .await;
        if result.is_err() {
            let _ = web_sys::Url::revoke_object_url(&url);
        }
        result
    }
    async fn present_gpu(
        &self,
        image: &web_sys::HtmlImageElement,
        generation: u64,
        orientation: u8,
    ) {
        if self.gpu_busy.get_untracked() {
            self.gpu_status
                .set("CPU fallback · previous GPU operation pending".into());
            return;
        }
        self.gpu_busy.set(true);
        let _guard = BusyGuard(self.gpu_busy);
        let mut renderer = self.gpu.borrow_mut().take();
        let result = async {
            if renderer.is_none() {
                let canvas = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.get_element_by_id("gpu-stage"))
                    .ok_or("GPU canvas unavailable")?
                    .dyn_into::<web_sys::HtmlCanvasElement>()
                    .map_err(|_| "GPU canvas unavailable")?;
                renderer = Some(crate::gpu::Renderer::acquire(canvas).await?);
            }
            renderer.as_mut().unwrap().quality = self.gpu_quality.get_untracked();
            renderer.as_mut().unwrap().image(image, orientation).await
        }
        .await;
        if generation != *self.stage_generation.borrow() || !self.gpu_enabled.get_untracked() {
            return;
        }
        match result {
            Ok(budget) => {
                self.gpu_visible.set(true);
                self.gpu_status.set(format!(
                    "GPU verified derivative · {} × {} · color unverified",
                    budget.width, budget.height
                ));
                *self.gpu.borrow_mut() = renderer;
                self.watch_gpu(generation);
            }
            Err(error) => {
                self.gpu_visible.set(false);
                self.gpu_enabled.set(false);
                self.gpu_status.set(format!(
                    "CPU fallback · {error}. CPU presentation selected automatically"
                ));
            }
        }
    }
    fn watch_gpu(&self, generation: u64) {
        let task = self.clone();
        spawn_local(async move {
            // Each stage attempt rearms loss recovery for any retained GPU frame.
            // The generation and missing-renderer checks stop obsolete watchers.
            loop {
                pause(100).await;
                if generation != *task.stage_generation.borrow()
                    || !task.gpu_enabled.get_untracked()
                    || task.gpu.borrow().is_none()
                {
                    break;
                }
                let failed = task.gpu.borrow().as_ref().is_some_and(|r| r.is_failed());
                if failed {
                    task.gpu_visible.set(false);
                    task.gpu.borrow_mut().take();
                    task.gpu_enabled.set(false);
                    task.gpu_status
                        .set("CPU fallback · GPU device lost or presentation failed".into());
                    break;
                }
            }
        });
    }
    fn navigate(&self, delta: i32) {
        let items = self.captures.get_untracked();
        let index = self
            .detail
            .get_untracked()
            .and_then(|d| items.iter().position(|c| c.id == d.capture.id))
            .unwrap_or(0);
        if let Some(index) = adjacent(index, delta, items.len()) {
            let ui = self.clone();
            let id = items[index].id.clone();
            spawn_local(async move {
                if let Err(e) = ui.select(id).await
                    && e != "Cancelled"
                {
                    ui.fail(e);
                }
            });
        }
    }
    async fn status_job(&self) -> Result<()> {
        let id = self.job.get_untracked();
        if id.is_empty() {
            return Err("No retained import to recover".into());
        }
        let progress: ImportProgress = self
            .net
            .json(&format!("/imports/{id}"), "GET", "", None)
            .await?;
        self.progress.set(progress.intake.completed_bytes);
        self.total.set(progress.intake.total_bytes.max(1));
        self.status.set(format!(
            "Import: {:?}; catalog: {:?}. Committed {} of {} bytes",
            progress.phase,
            progress.catalog_state,
            progress.intake.completed_bytes,
            progress.intake.total_bytes
        ));
        if progress.phase == ImportPhase::CatalogComplete {
            self.libraries(0).await?;
            if let Some(id) = progress.library
                && let Some(library) = self.libraries.get_untracked().iter().find(|l| l.id == id)
            {
                self.choose_library(library.clone()).await?;
            }
        }
        Ok(())
    }
    async fn change_video(&self, index: usize) -> Result<()> {
        let Some(file) = self.detail.get_untracked().and_then(|d| {
            d.originals
                .into_iter()
                .find(|f| f.role == OriginalRole::RefocusStack)
        }) else {
            return Ok(());
        };
        if file.encoded_bytes > 8 * 1024 * 1024 {
            return Err(
                "Video original exceeds viewer budget. Static preview remains available".into(),
            );
        }
        *self.stage_generation.borrow_mut() += 1;
        let generation = *self.stage_generation.borrow();
        if self.gpu_visible.get_untracked() {
            self.watch_gpu(generation);
        }
        if let Some(a) = self.stage_abort.borrow_mut().take() {
            a.abort();
        }
        let abort = AbortController::new().map_err(|_| "Browser cancellation unavailable")?;
        *self.stage_abort.borrow_mut() = Some(abort.clone());
        let result = async {
            // One decoder plus four conservative encoded copies and two RGBA/canvas
            // allocations share the entire transient lease; grids/JPEG transfers wait.
            let _lease = self.net.lease(64 * 1024 * 1024, &abort).await?;
            let manifest_response = self
                .net
                .fetch(
                    &format!("/assets/{}/manifest", file.id.as_str()),
                    "GET",
                    "",
                    None,
                    &[],
                    &abort,
                )
                .await?;
            let manifest: AssetManifest =
                serde_json::from_slice(&read_cancel(manifest_response, JSON_LIMIT, &abort).await?)
                    .map_err(|_| "Invalid video original manifest")?;
            if manifest.id != file.id || manifest.encoded_bytes != file.encoded_bytes {
                return Err("Video original identity mismatch".into());
            }
            let response = self
                .net
                .fetch(
                    &format!("/assets/{}/download", file.id.as_str()),
                    "GET",
                    "",
                    None,
                    &[],
                    &abort,
                )
                .await?;
            let bytes = read_cancel(response, file.encoded_bytes as usize, &abort).await?;
            if !verified(&bytes, &manifest) {
                return Err("Video original integrity failed".into());
            }
            crate::video::decode_container(&bytes, index, &abort.signal())
                .await
                .map_err(|_| {
                    "Browser video decode unavailable. Verified JPEG/static preview retained"
                        .to_owned()
                })
        }
        .await;
        if generation != *self.stage_generation.borrow() {
            return Ok(());
        }
        match result {
            Ok(decoded) => {
                self.gpu_visible.set(false);
                self.gpu.borrow_mut().take();
                self.gpu_status
                    .set("Browser AVC verified frame · GPU device released".into());
                self.orientation.set(1);
                self.video_lambdas.set(decoded.lambdas);
                self.video_label.set(format!("Displayed: {} × {} visible · plane {} · relative lambda {} · browser AVC · EXIF identity · color unverified", decoded.width, decoded.height, decoded.index + 1, decoded.lambda));
                self.focus_value.set(decoded.lambda);
                self.displayed_derivative.set(None);
                let old = self.stage.get_untracked();
                self.stage.set(decoded.image);
                if old.starts_with("blob:") {
                    let _ = web_sys::Url::revoke_object_url(&old);
                }
                self.status.set("Archived discrete browser focus decoded. Software preference is a platform request; hardware acceleration unverified".into());
            }
            Err(error) => self.fail(error),
        }
        Ok(())
    }
    async fn upload(&self, kind: ImportKind, files: Vec<File>) -> Result<()> {
        if self.busy.get_untracked() {
            return Err("An import is already in progress".into());
        }
        self.busy.set(true);
        let _busy_guard = BusyGuard(self.busy);
        self.job.set(String::new());
        *self.upload_cancelled.borrow_mut() = false;
        self.status
            .set("Preparing bounded import selection…".into());
        let session = self.csrf.get_untracked();
        let (mut declared_bytes, mut declared_files) = declaration(
            kind,
            &files.iter().map(|f| f.size() as u64).collect::<Vec<_>>(),
        )
        .map_err(str::to_owned)?;
        self.clear_assets();
        let metadata_abort =
            AbortController::new().map_err(|_| "Browser cancellation unavailable")?;
        let metadata_lease = self.net.lease(VIEWER_BUDGET, &metadata_abort).await?;
        let mut path_bytes = 0;
        if kind == ImportKind::Directory {
            for file in &files {
                let path = Reflect::get(file, &"webkitRelativePath".into())
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_else(|| file.name());
                path_bytes = path_metadata(path_bytes, &path).map_err(str::to_owned)?;
            }
        } else {
            if files.len() != 1 {
                return Err("Select one ZIP archive".into());
            }
            let file = &files[0];
            let size = file.size() as u64;
            let tail = file
                .slice_with_f64_and_f64(size.saturating_sub(65577) as f64, size as f64)
                .map_err(|_| "ZIP read unavailable")?;
            let tail = Uint8Array::new(
                &JsFuture::from(tail.array_buffer())
                    .await
                    .map_err(|_| "ZIP read unavailable")?,
            )
            .to_vec();
            let zip64 =
                if let Some(offset) = zip64_record_offset(&tail, size).map_err(str::to_owned)? {
                    let record = file
                        .slice_with_f64_and_f64(offset as f64, (offset + 56) as f64)
                        .map_err(|_| "ZIP64 read unavailable")?;
                    Some(
                        Uint8Array::new(
                            &JsFuture::from(record.array_buffer())
                                .await
                                .map_err(|_| "ZIP64 read unavailable")?,
                        )
                        .to_vec(),
                    )
                } else {
                    None
                };
            let (offset, length) =
                zip_extent_with_record(&tail, size, zip64.as_deref()).map_err(str::to_owned)?;
            let central = file
                .slice_with_f64_and_f64(offset as f64, (offset + length as u64) as f64)
                .map_err(|_| "ZIP metadata unavailable")?;
            let central = Uint8Array::new(
                &JsFuture::from(central.array_buffer())
                    .await
                    .map_err(|_| "ZIP metadata unavailable")?,
            )
            .to_vec();
            (declared_bytes, declared_files) = zip_declaration(&central).map_err(str::to_owned)?;
        }
        drop(metadata_lease);
        if !self.signed_in.get_untracked() || self.csrf.get_untracked() != session {
            return Err("Cancelled".into());
        }
        self.progress.set(0);
        self.total
            .set(files.iter().map(|f| f.size() as u64).sum::<u64>().max(1));
        let result=async {
            let value:serde_json::Value=self.net.json("/imports","POST",&self.csrf.get_untracked(),Some(serde_json::json!({"version":1,"kind":kind,"declared_bytes":declared_bytes,"declared_files":declared_files}))).await?;
            let id:OpaqueId=value["id"].as_str().ok_or("Invalid import identifier")?.to_owned().try_into().map_err(|_|"Invalid import identifier")?;
            self.job.set(id.as_str().into());
            // Only the opaque job ID is retained; never account, CSRF or source paths.
            if let Ok(Some(storage))=web_sys::window().unwrap().session_storage(){let _=storage.set_item("lytronaut-import",id.as_str());}
            let mut base=0;
            for (index,file) in files.iter().enumerate() {
                if *self.upload_cancelled.borrow(){return Err("Import cancelled. Originals already committed remain preserved".into());}
                self.status.set(format!("Uploading file {} of {}",index+1,files.len()));
                let route=if kind==ImportKind::Directory {format!("/imports/{}/file",id.as_str())} else {format!("/imports/{}/zip",id.as_str())};
                let hex=if kind==ImportKind::Directory {let path=Reflect::get(file,&"webkitRelativePath".into()).ok().and_then(|v|v.as_string()).unwrap_or_else(||file.name());Some(path_hex(&path).map_err(str::to_owned)?)}else{None};
                self.send_file(&route,file,hex.as_deref(),base).await?;base+=file.size() as u64;
            }
            self.net.mutation(&format!("/imports/{}/finish",id.as_str()),&self.csrf.get_untracked()).await?;
            self.status_job().await
        }.await;
        self.upload_abort.borrow_mut().take();
        result
    }
    async fn send_file(&self, path: &str, file: &File, hex: Option<&str>, base: u64) -> Result<()> {
        let xhr = web_sys::XmlHttpRequest::new().map_err(|_| "Upload unavailable")?;
        xhr.open_with_async("PUT", path, true)
            .map_err(|_| "Upload unavailable")?;
        xhr.set_with_credentials(true);
        xhr.set_request_header("X-Lytronaut-Csrf", &self.csrf.get_untracked())
            .map_err(|_| "Upload unavailable")?;
        if let Some(hex) = hex {
            xhr.set_request_header("X-Lytronaut-Path-Hex", hex)
                .map_err(|_| "Upload unavailable")?;
            xhr.set_request_header("X-Lytronaut-Entry-Type", "file")
                .map_err(|_| "Upload unavailable")?;
        }
        let progress = self.progress;
        let onprogress = Closure::<dyn FnMut(web_sys::ProgressEvent)>::new(
            move |event: web_sys::ProgressEvent| {
                progress.set(base + event.loaded() as u64);
            },
        );
        let upload = xhr.upload().map_err(|_| "Upload unavailable")?;
        upload.set_onprogress(Some(onprogress.as_ref().unchecked_ref()));
        let (sender, receiver) = futures::channel::oneshot::channel();
        let sender = Rc::new(RefCell::new(Some(sender)));
        let tx = sender.clone();
        let done = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            if let Some(tx) = tx.borrow_mut().take() {
                let _ = tx.send(());
            }
        });
        xhr.set_onloadend(Some(done.as_ref().unchecked_ref()));
        *self.upload_abort.borrow_mut() = Some(xhr.clone());
        let sent = xhr.send_with_opt_blob(Some(file));
        if sent.is_ok() {
            let _ = receiver.await;
        }
        xhr.set_onloadend(None);
        upload.set_onprogress(None);
        if sent.is_err() {
            return Err("Upload could not start".into());
        }
        if *self.upload_cancelled.borrow() {
            return Err("Import cancelled. Choose files again to retry".into());
        }
        if !(200..300).contains(&xhr.status().unwrap_or(0)) {
            return Err(
                "Upload failed. Inspect status, then select the original files again to retry"
                    .into(),
            );
        }
        Ok(())
    }
}
fn launch(ui: Ui, task: impl std::future::Future<Output = Result<()>> + 'static) {
    spawn_local(async move {
        if let Err(e) = task.await
            && e != "Cancelled"
        {
            ui.fail(e);
        }
    });
}
fn file_links(files: Vec<PreservedFile>) -> impl IntoView {
    files.into_iter().map(|file|view!{<li><a href=format!("/assets/{}/download",file.id.as_str()) download="">{format!("Download {:?} · {} bytes",file.role,file.encoded_bytes)}</a></li>}).collect_view()
}
fn ui() -> Ui {
    use_context::<StoredValue<Ui, LocalStorage>>()
        .expect("application context")
        .get_value()
}
pub fn app() -> impl IntoView {
    let state = Ui::new();
    provide_context(StoredValue::new_local(state.clone()));
    if let Ok(Some(storage)) = web_sys::window().unwrap().session_storage()
        && let Ok(Some(id)) = storage.get_item("lytronaut-import")
        && OpaqueId::try_from(id.clone()).is_ok()
    {
        state.job.set(id);
    }
    let startup = state.clone();
    spawn_local(async move {
        if startup.session().await.is_err() {
            startup
                .status
                .set("Sign in or set up this local library".into());
        }
    });
    let cleanup = StoredValue::new_local(state.clone());
    on_cleanup(move || {
        let cleanup = cleanup.get_value();
        cleanup.clear_assets();
        if let Some(abort) = cleanup.export_abort.borrow_mut().take() {
            abort.abort();
        }
        if let Some(xhr) = cleanup.upload_abort.borrow().as_ref() {
            let _ = xhr.abort();
        }
    });
    let signed = state.signed_in;
    let status = state.status;
    view! {
        <a class="skip" href="#content">"Skip to library"</a>
        <header class="header"><a class="brand" href="/">"◉ LYTRONAUT"</a><span>"an explorer for your light-field library"</span>
            <Show when=move||signed.get()><button on:click=move |_| {let u=ui();let task=u.clone();launch(u,async move{task.net.mutation("/auth/logout",&task.csrf.get_untracked()).await?;task.clear_assets();task.library.set(None);task.libraries.set(Vec::new());task.albums.set(Vec::new());task.files.set(Vec::new());task.captures.set(Vec::new());task.csrf.set(String::new());task.signed_in.set(false);task.status.set("Signed out".into());Ok(())});}>"Sign out"</button></Show>
        </header>
        <div class="status" role="status" aria-live="polite" aria-atomic="true">{move||status.get()}</div>
        <Show when=move||signed.get() fallback=||view!{<Authentication/>}><Workspace/></Show>
    }
}
#[component]
fn Authentication() -> impl IntoView {
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let bootstrap = RwSignal::new(String::new());
    let setup = RwSignal::new(false);
    let pending = RwSignal::new(false);
    view! {
        <main id="content" class="authentication"><section class="panel"><h1>{move||if setup.get(){"Set up your library"}else{"Welcome back"}}</h1>
            <p>"Private, local access to your light-field originals."</p>
            <form on:submit=move |event|{event.prevent_default();if pending.get_untracked(){return;}pending.set(true);let u=ui();spawn_local(async move{
                let result=async {
                    let abort=AbortController::new().map_err(|_|"Browser cancellation unavailable")?;
                    let intent=if setup.get_untracked(){"setup"}else{"login"};
                    let body=if setup.get_untracked(){serde_json::json!({"username":username.get_untracked(),"password":password.get_untracked(),"bootstrap":bootstrap.get_untracked()})}else{serde_json::json!({"username":username.get_untracked(),"password":password.get_untracked()})};
                    u.net.fetch(&format!("/auth/{intent}"),"POST","",Some(&JsValue::from_str(&body.to_string())),&[("Content-Type","application/json"),("X-Lytronaut-Intent",intent)],&abort).await?;
                    password.set(String::new());bootstrap.set(String::new());
                    if setup.get_untracked(){setup.set(false);u.status.set("Administrator created. Sign in with your credentials".into());Ok(())}else{u.session().await}
                }.await;
                pending.set(false);if let Err(e)=result{u.fail(e);}
            });}>
                <label for="username">"Username"</label><input id="username" autocomplete="username" required maxlength="64" prop:value=move||username.get() on:input=move|e|username.set(event_target_value(&e))/>
                <label for="password">"Password"</label><input id="password" type="password" autocomplete=move||if setup.get(){"new-password"}else{"current-password"} required minlength="12" maxlength="1024" prop:value=move||password.get() on:input=move|e|password.set(event_target_value(&e))/>
                <Show when=move||setup.get()><label for="bootstrap">"Local bootstrap secret"</label><input id="bootstrap" type="password" autocomplete="off" required minlength="64" maxlength="64" prop:value=move||bootstrap.get() on:input=move|e|bootstrap.set(event_target_value(&e))/><p>"Read bootstrap.secret from your private application data directory. This secret is consumed once."</p></Show>
                <button class="primary" type="submit" disabled=move||pending.get()>{move||if pending.get(){"Working…"}else if setup.get(){"Create administrator"}else{"Sign in"}}</button>
            </form><button on:click=move |_|setup.update(|s|*s = !*s) disabled=move||pending.get()>{move||if setup.get(){"Back to sign in"}else{"First run? Set up administrator"}}</button>
        </section></main>
    }
}
#[component]
fn Workspace() -> impl IntoView {
    let directory_ref = NodeRef::<leptos::html::Input>::new();
    Effect::new(move |_| {
        if let Some(node) = directory_ref.get() {
            let _ = node.set_attribute("webkitdirectory", "");
        }
    });
    let s = ui();
    let library = s.library;
    let albums = s.albums;
    let libraries = s.libraries;
    let captures = s.captures;
    let detail = s.detail;
    let stage = s.stage;
    let gpu_visible = s.gpu_visible;
    let gpu_status = s.gpu_status;
    let gpu_quality = s.gpu_quality;
    let focus_stack = s.focus_stack;
    let video_lambdas = s.video_lambdas;
    let video_label = s.video_label;
    let displayed_derivative = s.displayed_derivative;
    let focus_value = s.focus_value;
    let orientation = s.orientation;
    let thumbs = s.thumbs;
    let next = s.next;
    let offset = s.offset;
    let search = s.search;
    let sort = s.sort;
    let files = s.files;
    let list_next = s.list_next;
    let album_next = s.album_next;
    let busy = s.busy;
    let job = s.job;
    let progress = s.progress;
    let total = s.total;
    let export_job = s.export_job;
    let export_busy = s.export_busy;
    view! {
        <div class="workstation">
            <nav class="rail" aria-label="Library and albums"><h2>"LIBRARIES"</h2>
                {move||libraries.get().into_iter().enumerate().map(|(index,l)|{let selected=library;let id=l.id.clone();view!{<button class="rail-item" aria-pressed=move||selected.get().is_some_and(|s|s.id==id) on:click=move |_|{let u=ui();let task=u.clone();let l=l.clone();launch(u,async move{task.choose_library(l).await});}>{format!("Library {}",index+1)}</button>}}).collect_view()}
                <Show when=move||list_next.get().is_some()><button on:click=move |_|{let u=ui();let task=u.clone();launch(u,async move{task.libraries(list_next.get_untracked().unwrap_or(0)).await});}>"More libraries"</button></Show>
                <p>{move||library.get().map(|l|format!("{} captures · {} albums · {:?}",l.captures,l.albums,l.state)).unwrap_or_else(||"No library yet. Import your first library.".into())}</p>
                <button class="rail-item" on:click=move |_|{let u=ui();u.album.set(None);let task=u.clone();launch(u,async move{task.page(0).await});}>"All captures"</button>
                <h2>"ALBUMS"</h2>
                {move||albums.get().into_iter().map(|album|{let selected=ui().album;let id=album.id.as_str().to_owned();let click_id=id.clone();view!{<button class="rail-item" aria-pressed=move||selected.get().as_deref()==Some(&id) on:click=move |_|{let u=ui();u.album.set(Some(click_id.clone()));u.sort.set("order".into());let task=u.clone();launch(u,async move{task.page(0).await});}>{format!("{}{} · {}",album.name,if album.name_truncated{" [truncated]"}else{""},album.captures)}</button>}}).collect_view()}
                <Show when=move||album_next.get().is_some()><button on:click=move |_|{let u=ui();let task=u.clone();launch(u,async move{task.load_albums(album_next.get_untracked().unwrap_or(0)).await});}>"More albums"</button></Show>
                <div class="import"><h2>"IMPORT LIBRARY"</h2><label for="directory">"Library directory"</label><input id="directory" type="file" multiple=true node_ref=directory_ref disabled=move||busy.get() on:change=move|event|{let input=event_target::<web_sys::HtmlInputElement>(&event);let files=input.files().map(|list|(0..list.length()).filter_map(|i|list.get(i)).collect::<Vec<_>>()).unwrap_or_default();input.set_value("");let u=ui();let task=u.clone();launch(u,async move{task.upload(ImportKind::Directory,files).await});}/>
                    <label for="zip">"ZIP library archive"</label><input id="zip" type="file" accept=".zip,application/zip" disabled=move||busy.get() on:change=move|event|{let input=event_target::<web_sys::HtmlInputElement>(&event);let files=input.files().map(|list|(0..list.length()).filter_map(|i|list.get(i)).collect::<Vec<_>>()).unwrap_or_default();input.set_value("");let u=ui();let task=u.clone();launch(u,async move{task.upload(ImportKind::ZipLibrary,files).await});}/>
                    <Show when=move||!job.get().is_empty()><label for="import-progress">"Upload / committed bytes"</label><progress id="import-progress" value=move||progress.get() max=move||total.get()></progress><span>{move||format!("{} / {} bytes",progress.get(),total.get())}</span>
                        <button disabled=move||!busy.get() || job.get().is_empty() on:click=move |_|{let u=ui();*u.upload_cancelled.borrow_mut()=true;if let Some(xhr)=u.upload_abort.borrow().as_ref(){let _=xhr.abort();}let task=u.clone();launch(u,async move{task.net.mutation(&format!("/imports/{}/cancel",task.job.get_untracked()),&task.csrf.get_untracked()).await?;task.status_job().await});}>"Cancel import"</button>
                        <button disabled=move||busy.get() on:click=move |_|{let u=ui();let task=u.clone();launch(u,async move{task.status_job().await});}>"Recover import status"</button>
                        <button disabled=move||busy.get() on:click=move |_|{let u=ui();let task=u.clone();launch(u,async move{task.net.mutation(&format!("/imports/{}/finish",task.job.get_untracked()),&task.csrf.get_untracked()).await?;task.status_job().await});}>"Retry catalog adaptation"</button>
                        <p>"Interrupted uploads: recover status, then reselect the original directory or ZIP to start a new bounded import. Completed original bytes deduplicate."</p>
                    </Show>
                </div>
            </nav>
            <main id="content" class="workspace"><div class="toolbar"><h1>"Library explorer"</h1><form class="search" on:submit=move|e|{e.prevent_default();let u=ui();let task=u.clone();launch(u,async move{task.page(0).await});}><label for="search">"Search captures"</label><input id="search" type="search" prop:value=move||search.get() on:input=move|e|search.set(event_target_value(&e))/><button type="submit">"Search"</button><label for="sort">"Sort"</label><select id="sort" prop:value=move||sort.get() on:change=move|e|{sort.set(event_target_value(&e));let u=ui();let task=u.clone();launch(u,async move{task.page(0).await});}><option value="order">"Library / album order"</option><option value="name">"Name"</option><option value="rating">"Imported rating"</option></select></form></div>
                <section class="export-controls" aria-label="Export and lossless backup"><button id="backup-library" disabled=move||export_busy.get() on:click=move |_|{let u=ui();let task=u.clone();launch(u,async move{task.export(None).await});}>"Back up originals and catalog"</button><button id="recover-export" disabled=move||export_busy.get() on:click=move |_|{let u=ui();let task=u.clone();launch(u,async move{task.recover_export().await});}>"Recover export / restore status"</button><label for="restore-bundle">"Restore private bundle to an isolated store"</label><input id="restore-bundle" type="file" accept=".lytronaut" disabled=move||export_busy.get() on:change=move|e|{let input=event_target::<web_sys::HtmlInputElement>(&e);let file=input.files().and_then(|f|f.get(0));input.set_value("");if let Some(file)=file{let u=ui();let task=u.clone();launch(u,async move{task.restore_bundle(file).await});}}/><button id="cancel-restore-upload" disabled=move||!export_busy.get()||export_job.get().is_some() on:click=move |_|{let u=ui();if let Some(abort)=u.export_abort.borrow_mut().take(){abort.abort();}}>"Cancel restore upload"</button><p>"Private backup includes all immutable originals and catalog associations. Credentials and pending jobs are excluded. Restore creates a separate store."</p>{move||export_job.get().map(|r|{let id=r.job.id.as_str().to_owned();let recipe_id=id.clone();let cancel_id=id.clone();view!{<p id="export-progress">{format!("{:?} · {} bytes processed",r.job.state,r.job.completed_bytes)}</p><Show when=move||r.job.state==JobState::Succeeded&&r.kind!=ExportKind::Restore><a id="export-download" href=format!("/exports/{id}/download") download="">"Download export / private bundle"</a></Show><Show when=move||r.job.state==JobState::Succeeded&&r.kind==ExportKind::Flatten><a id="export-recipe" href=format!("/exports/{recipe_id}/recipe") download="lytronaut-recipe.json">"Download Lytronaut provenance"</a></Show><button id="cancel-export" disabled=move||matches!(r.job.state,JobState::Cancelled|JobState::Failed) on:click=move |_|{let u=ui();let task=u.clone();let id=cancel_id.clone();launch(u,async move{let r=task.net.json(&format!("/exports/{id}"),"DELETE",&task.csrf.get_untracked(),None).await?;task.watch_export(r).await});}>"Cancel or remove result"</button>}})}</section>
                <section class="viewer" aria-label="Capture viewer"><div class="stage" tabindex="0" aria-label="Capture stage. Left and right arrows change capture" on:keydown=move|e|{if e.key()=="ArrowLeft"{e.prevent_default();ui().navigate(-1);}else if e.key()=="ArrowRight"{e.prevent_default();ui().navigate(1);}}>
                    <Show when=move||!stage.get().is_empty() fallback=move||view!{<div class="stage-empty"><span>"◉"</span><p>{move||if detail.get().is_some(){"Preview loading or unavailable. See capture details and originals."}else{"Select a capture, or import a library to begin."}}</p></div>}><img style:display=move||if gpu_visible.get(){"none"}else{"block"} src=move||stage.get() alt=move||detail.get().map(|d|format!("{}: {}",if !video_lambdas.get().is_empty(){"Archived browser focus derivative"}else if displayed_derivative.get().is_some(){"Stored JPEG focus derivative"}else{"Static JPEG preview"},d.capture.name)).unwrap_or_else(||"Capture preview".into())/></Show>
                    <canvas id="gpu-stage" role="img" aria-label="Verified stored derivative GPU presentation" style:display=move||if gpu_visible.get(){"block"}else{"none"} style="max-width:100%;max-height:100%;object-fit:contain"/>
                </div><div class="focus-controls"><label for="gpu-quality">"Preview resolution cap"</label><select id="gpu-quality" prop:value=move||gpu_quality.get().to_string() on:change=move|e|{if let Ok(value)=event_target_value(&e).parse(){gpu_quality.set(value);let u=ui();if u.gpu_enabled.get_untracked(){let task=u.clone();launch(u,async move{task.change_focus(task.focus_value.get_untracked()).await});}}}><option value="512">"512 · default"</option><option value="256">"256 · reduced"</option></select><output id="gpu-status" aria-live="polite">{move||gpu_status.get()}</output><button disabled=move||detail.get().is_none_or(|d|!d.originals.iter().any(|f|f.role==OriginalRole::RefocusStack)) on:click=move |_|{let u=ui();let task=u.clone();launch(u,async move{task.change_video(0).await});}>"Try browser H.264 refocus"</button><label for="focus">"Discrete focus · nearest stored plane"</label><input id="focus" type="range" min=move||video_lambdas.get().into_iter().reduce(f64::min).or_else(||focus_stack.get().map(|s|s.lambdas[0])).unwrap_or(0.0) max=move||video_lambdas.get().into_iter().reduce(f64::max).or_else(||focus_stack.get().map(|s|*s.lambdas.last().unwrap())).unwrap_or(1.0) step="any" prop:value=move||focus_value.get() disabled=move||focus_stack.get().is_none()&&video_lambdas.get().is_empty() aria-valuetext=move||if !video_lambdas.get().is_empty(){let values=video_lambdas.get();let i=lytronaut_format::annex_b::nearest_lambda(&values,focus_value.get()).unwrap_or(0);format!("Archived video plane {}, lambda {}",i+1,values[i])}else{focus_stack.get().map(|s|format!("Stored plane {}, lambda {}",s.nearest(focus_value.get()).unwrap_or(0)+1, s.lambdas[s.nearest(focus_value.get()).unwrap_or(0)])).unwrap_or_else(||"Preview only; browser H.264 focus is unverified".into())} on:input=move|e|{if let Ok(value)=event_target_value(&e).parse(){let u=ui();let task=u.clone();launch(u,async move{task.change_focus(value).await});}}/><output for="focus">{move||if !video_label.get().is_empty(){video_label.get()}else{displayed_derivative.get().map(|d|format!("Displayed: {} × {} source · plane {} · relative lambda {} · EXIF {} · lossy JPEG",d.source_dimensions.width,d.source_dimensions.height,d.plane_index+1,d.lambda,d.orientation)).unwrap_or_else(||"Preview only · H.264 browser verification pending".into())}}</output><label for="orientation">"Explicit derivative EXIF orientation"</label><select id="orientation" prop:value=move||orientation.get().to_string() disabled=move||focus_stack.get().is_none()||!video_lambdas.get().is_empty() on:change=move|e|{if let Ok(value)=event_target_value(&e).parse(){orientation.set(value);let u=ui();let task=u.clone();launch(u,async move{task.change_focus(focus_value.get_untracked()).await});}}>{(1..=8).map(|n|view!{<option value=n.to_string()>{n}</option>}).collect_view()}</select><p>"Processed recipe may be baked. Default identity; imported orientation, aperture, perspective, tone, denoise and animation are preserved only."</p></div><div class="viewer-strip"><button on:click=move |_|ui().navigate(-1) disabled=move||captures.get().is_empty()>"Previous capture"</button><span>{move||detail.get().map(|d|format!("{}{}",d.capture.name,if d.capture.name_truncated{" [truncated]"}else{""})).unwrap_or_else(||"No capture selected".into())}</span><button on:click=move |_|ui().navigate(1) disabled=move||captures.get().is_empty()>"Next capture"</button><button on:click=move |_|{if let Some(d)=detail.get_untracked(){let u=ui();let task=u.clone();launch(u,async move{task.select(d.capture.id).await});}}>"Retry preview"</button></div></section>
                <div class="lower"><section class="browser-panel" aria-label="Capture grid and filmstrip"><h2>"Captures"</h2><div class="capture-grid">
                    {move||captures.get().into_iter().map(|capture|{let id=capture.id.as_str().to_owned();let thumb_id=StoredValue::new(id.clone());let selected_id=capture.id.clone();let click_id=capture.id.clone();view!{<button class="capture" aria-pressed=move||detail.get().is_some_and(|d|d.capture.id==selected_id) on:click=move |_|{let u=ui();let task=u.clone();let id=click_id.clone();launch(u,async move{task.select(id).await});}><Show when=move||thumbs.get().get(&id).is_some_and(|u|u.starts_with("data:image/jpeg")) fallback=||view!{<div class="thumb-empty">"◉"</div>}><img src=move||thumbs.get().get(&thumb_id.get_value()).cloned().unwrap_or_default() alt=""/></Show><span>{format!("{}{}",capture.name,if capture.name_truncated{" [truncated]"}else{""})}</span></button>}}).collect_view()}
                </div><nav class="filmstrip" aria-label="Capture filmstrip">{move||captures.get().into_iter().enumerate().map(|(i,capture)|{let id=capture.id;view!{<button aria-label=format!("Select capture {}: {}",offset.get_untracked() as usize+i+1,capture.name) on:click=move |_|{let u=ui();let task=u.clone();let id=id.clone();launch(u,async move{task.select(id).await});}>{i+1}</button>}}).collect_view()}</nav>
                <div class="pagination"><button disabled=move||offset.get()==0 on:click=move |_|{let u=ui();let task=u.clone();launch(u,async move{task.page(offset.get_untracked().saturating_sub(PAGE_SIZE)).await});}>"Previous page"</button><span>{move||format!("Page {}",offset.get()/PAGE_SIZE+1)}</span><button disabled=move||next.get().is_none() on:click=move |_|{let u=ui();let task=u.clone();launch(u,async move{task.page(next.get_untracked().unwrap_or(0)).await});}>"Next page"</button></div>
                <Show when=move||!files.get().is_empty()><h2>"Preserved library files"</h2><ul>{move||file_links(files.get())}</ul></Show>
                </section><aside class="inspector" aria-label="Capture inspector"><h2>"Capture details"</h2>{move||detail.get().map(|d|view!{<p>{format!("{}{}",d.caption,if d.caption_truncated{" [truncated]"}else{""})}</p><dl><dt>"Imported rating"</dt><dd>{d.capture.rating}</dd><dt>"Originals"</dt><dd>{format!("{:?}",d.capabilities.originals)}</dd><dt>"Static preview"</dt><dd>{format!("{:?}",d.capabilities.static_preview)}</dd><dt>"Color profile"</dt><dd>"Unknown / unverified"</dd><dt>"Discrete focus"</dt><dd>{if !video_lambdas.get().is_empty(){"Available · discrete archived browser AVC planes".to_owned()}else if focus_stack.get().is_some(){"Available · discrete stored JPEG planes".to_owned()}else{"Disabled · preview only".to_owned()}}</dd><dt>"Calibrated synthesis"</dt><dd>"Unverified · unsupported"</dd><dt>"Legacy edits"</dt><dd>"Preserved only · pixel parity unverified"</dd><dt>"GPU / video"</dt><dd>{move||if !video_lambdas.get().is_empty(){"Browser AVC · hardware unverified".to_owned()}else{gpu_status.get()}}</dd></dl><h3>"Export verified pixels"</h3><Show when=move||!video_lambdas.get().is_empty()><p id="video-export-unavailable">"Browser video export is unavailable. Select a verified static/JPEG preview to export pixels. Original backup remains available."</p></Show><button id="export-png" disabled=move||export_busy.get()||stage.get().is_empty()||!video_lambdas.get().is_empty() on:click=move |_|{let u=ui();let task=u.clone();launch(u,async move{task.export(Some(ExportFormat::Png)).await});}>"Export PNG"</button><button id="export-jpeg" disabled=move||export_busy.get()||stage.get().is_empty()||!video_lambdas.get().is_empty() on:click=move |_|{let u=ui();let task=u.clone();launch(u,async move{task.export(Some(ExportFormat::Jpeg)).await});}>"Export JPEG"</button><p>"Flattened stored pixels lose refocus ability. PNG preserves decoded JPEG pixels; JPEG adds compression loss. Unknown color profile. Download the separate Lytronaut provenance record."</p><h3>"Preserved originals"</h3><ul>{file_links(d.originals)}</ul><ul>{d.item_errors.into_iter().map(|e|view!{<li>{format!("{:?}: {:?}",e.role,e.error.code)}</li>}).collect_view()}</ul>})}<p>"Static browsing works without GPU or video codecs. No new raw synthesis or Illum support is claimed."</p></aside></div>
            </main>
        </div>
    }
}
