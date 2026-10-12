//! Optional browser platform AVC decoder. No decoded-frame cache or imported edits.
use js_sys::{Reflect, Uint8Array};
use lytronaut_format::{Container, Limits, Representation, annex_b::AnnexB, processed_asset};
use sha2::{Digest, Sha256};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};
use wasm_bindgen::{JsCast, JsValue, closure::Closure, prelude::wasm_bindgen};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortSignal, EncodedVideoChunk, EncodedVideoChunkInit, EncodedVideoChunkType,
    HardwareAcceleration, VideoDecoder, VideoDecoderConfig, VideoDecoderInit, VideoFrame,
};

type Result<T> = std::result::Result<T, &'static str>;
#[derive(Clone)]
struct Export {
    builder: Rc<RefCell<Option<lytronaut_format::live_photo::Builder>>>,
    progress: Rc<dyn Fn(usize, usize)>,
    total: usize,
}
thread_local! { static ACTIVE: Cell<bool> = const { Cell::new(false) }; }
struct Admission;
impl Drop for Admission {
    fn drop(&mut self) {
        ACTIVE.set(false);
    }
}
struct Frame {
    inner: VideoFrame,
    closed: Cell<bool>,
}
impl Frame {
    fn release(&self) {
        if !self.closed.replace(true) {
            self.inner.close();
        }
    }
}
impl Drop for Frame {
    fn drop(&mut self) {
        self.release();
    }
}
struct Canvas(web_sys::HtmlCanvasElement);
impl Drop for Canvas {
    fn drop(&mut self) {
        self.0.set_width(0);
        self.0.set_height(0);
    }
}
struct Decoder {
    codec: VideoDecoder,
    _output: Closure<dyn FnMut(VideoFrame)>,
    _error: Closure<dyn FnMut(JsValue)>,
}
impl Drop for Decoder {
    fn drop(&mut self) {
        let _ = self.codec.reset();
        let _ = self.codec.close();
    }
}
#[derive(serde::Serialize)]
pub struct Decoded {
    pub frames: usize,
    pub reference_frames: u32,
    pub coded_width: u32,
    pub coded_height: u32,
    pub bidirectional_frames: usize,
    pub software_retry: bool,
    pub width: u32,
    pub height: u32,
    pub index: usize,
    pub lambda: f64,
    pub lambdas: Vec<f64>,
    pub image: String,
    pub encoded_bytes: usize,
    pub content_sha256: String,
}
#[derive(Default)]
struct Output {
    seen: Vec<bool>,
    image: Option<String>,
    error: bool,
}
async fn pause() {
    let p = js_sys::Promise::new(&mut |resolve, _| {
        if let Some(w) = web_sys::window() {
            let _ = w.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 5);
        }
    });
    let _ = JsFuture::from(p).await;
}
fn cancelled(signal: &AbortSignal) -> Result<()> {
    if signal.aborted() {
        Err("cancelled")
    } else {
        Ok(())
    }
}
async fn wait_promise(
    promise: js_sys::Promise,
    signal: &AbortSignal,
    state: Option<&Rc<RefCell<Output>>>,
) -> Result<JsValue> {
    let started = js_sys::Date::now();
    let future = JsFuture::from(promise);
    let interrupted = async {
        loop {
            if signal.aborted() {
                return "cancelled";
            }
            if state.is_some_and(|s| s.borrow().error) {
                return "decode failed";
            }
            if js_sys::Date::now() - started > 10_000.0 {
                return "decode timeout";
            }
            pause().await;
        }
    };
    futures::pin_mut!(future, interrupted);
    // Losing futures are dropped, rather than spawning a permanently waiting
    // Rust task when a platform support/flush Promise never settles.
    match futures::future::select(future, interrupted).await {
        futures::future::Either::Left((result, _)) => {
            cancelled(signal)?;
            if state.is_some_and(|s| s.borrow().error) {
                return Err("decode failed");
            }
            result.map_err(|_| "platform promise failed")
        }
        futures::future::Either::Right((error, _)) => Err(error),
    }
}
async fn attempt(
    bytes: &[u8],
    parsed: &AnnexB,
    selected: usize,
    signal: &AbortSignal,
    software: bool,
    export: Option<Export>,
) -> Result<String> {
    cancelled(signal)?;
    let timeout = if export.is_some() { 30_000.0 } else { 10_000.0 };
    let queue_limit = if export.is_some() { 1 } else { 2 };
    let sequence = &parsed.sequence;
    let config = VideoDecoderConfig::new(&sequence.codec);
    config.set_coded_width(sequence.coded_width);
    config.set_coded_height(sequence.coded_height);
    config.set_hardware_acceleration(if software {
        HardwareAcceleration::PreferSoftware
    } else {
        HardwareAcceleration::NoPreference
    });
    // Deliberately omit description: the registration defines this as Annex-B.
    // Reflective invocation catches synchronous platform exceptions as well as
    // Promise rejection, and support probing itself remains cancellable/bounded.
    let constructor = Reflect::get(&js_sys::global(), &JsValue::from_str("VideoDecoder"))
        .map_err(|_| "decoder unavailable")?;
    let query = Reflect::get(&constructor, &JsValue::from_str("isConfigSupported"))
        .map_err(|_| "unsupported config")?
        .dyn_into::<js_sys::Function>()
        .map_err(|_| "unsupported config")?;
    let promise = query
        .call1(&constructor, &config)
        .map_err(|_| "unsupported config")?
        .dyn_into::<js_sys::Promise>()
        .map_err(|_| "unsupported config")?;
    let support = wait_promise(promise, signal, None).await?;
    if !Reflect::get(&support, &JsValue::from_str("supported"))
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
    {
        return Err("unsupported config");
    }
    let state = Rc::new(RefCell::new(Output {
        seen: vec![false; parsed.units.len()],
        ..Default::default()
    }));
    let out = state.clone();
    let seq = sequence.clone();
    let export_signal = signal.clone();
    let output = Closure::wrap(Box::new(move |frame: VideoFrame| {
        let frame = Frame {
            inner: frame,
            closed: Cell::new(false),
        };
        let f = &frame.inner;
        let result = (|| -> Result<()> {
            let rect = f.visible_rect().ok_or("missing visible geometry")?;
            let index = f.timestamp() / 1000.0;
            let mut s = out.borrow_mut();
            if !index.is_finite()
                || index.fract() != 0.0
                || index < 0.0
                || index >= s.seen.len() as f64
            {
                return Err("invalid frame timestamp");
            }
            let index = index as usize;
            if s.seen[index]
                || f.coded_width() < seq.visible_width
                || f.coded_width() > seq.coded_width.saturating_add(16)
                || f.coded_width() > 2048
                || f.coded_height() < seq.visible_height
                || f.coded_height() > seq.coded_height.saturating_add(16)
                || f.coded_height() > 2048
                || f.display_width() != seq.visible_width
                || f.display_height() != seq.visible_height
                || rect.width() != f64::from(seq.visible_width)
                || rect.height() != f64::from(seq.visible_height)
                || rect.x() != f64::from(seq.crop_left)
                || rect.y() != f64::from(seq.crop_top)
                || !rect.x().is_finite()
                || !rect.y().is_finite()
                || rect.x() < 0.0
                || rect.y() < 0.0
                || rect.right() > f64::from(f.coded_width())
                || rect.bottom() > f64::from(f.coded_height())
            {
                return Err("inconsistent visible geometry");
            }
            s.seen[index] = true;
            if index == selected
                || export
                    .as_ref()
                    .is_some_and(|e| e.builder.borrow().as_ref().is_some_and(|b| b.wants(index)))
            {
                let document = web_sys::window()
                    .and_then(|w| w.document())
                    .ok_or("canvas unavailable")?;
                let canvas = Canvas(
                    document
                        .create_element("canvas")
                        .map_err(|_| "canvas unavailable")?
                        .dyn_into::<web_sys::HtmlCanvasElement>()
                        .map_err(|_| "canvas unavailable")?,
                );
                canvas.0.set_width(f.display_width());
                canvas.0.set_height(f.display_height());
                let context = canvas
                    .0
                    .get_context("2d")
                    .map_err(|_| "context unavailable")?
                    .ok_or("context unavailable")?
                    .dyn_into::<web_sys::CanvasRenderingContext2d>()
                    .map_err(|_| "context unavailable")?;
                // CanvasImageSource VideoFrame draws its visible rectangle, excluding coded padding.
                context
                    .draw_image_with_video_frame(f, 0.0, 0.0)
                    .map_err(|_| "frame draw failed")?;
                let data = if index == selected {
                    let data = canvas
                        .0
                        .to_data_url_with_type("image/jpeg")
                        .map_err(|_| "frame encode failed")?;
                    if data.len() > 2 * 1024 * 1024 || !data.starts_with("data:image/jpeg;base64,")
                    {
                        return Err("frame output budget");
                    }
                    data
                } else {
                    String::new()
                };
                if let Some(export) = &export {
                    let mut owned = export.builder.borrow_mut();
                    let builder = owned.as_mut().ok_or("export builder unavailable")?;
                    if index == selected {
                        let raw = web_sys::window()
                            .ok_or("browser unavailable")?
                            .atob(
                                data.strip_prefix("data:image/jpeg;base64,")
                                    .ok_or("invalid key JPEG")?,
                            )
                            .map_err(|_| "invalid key JPEG")?
                            .chars()
                            .map(|c| c as u8)
                            .collect::<Vec<_>>();
                        builder.set_still(&raw).map_err(|_| "key JPEG mismatch")?;
                    }
                    if builder.wants(index) {
                        let width = f.display_width();
                        let height = f.display_height();
                        let mut rgba = Vec::new();
                        rgba.try_reserve_exact((width * height * 4) as usize)
                            .map_err(|_| "frame pixel budget")?;
                        // The platform ImageData copy stays <=128 KiB rather than
                        // retaining a second full 8 MiB image beside Rust pixels.
                        for row in (0..height).step_by(16) {
                            if export_signal.aborted() {
                                return Err("cancelled");
                            }
                            let pixels = context
                                .get_image_data(
                                    0.0,
                                    f64::from(row),
                                    f64::from(width),
                                    f64::from((height - row).min(16)),
                                )
                                .map_err(|_| "frame pixels unavailable")?;
                            rgba.extend(pixels.data().0);
                        }
                        let image = lytronaut_format::DecodedJpeg {
                            geometry: lytronaut_format::Geometry { width, height },
                            rgba,
                        };
                        // Release platform pixel owners before the Rust PCM encoder allocates
                        // its scratch/picture buffers. The selected data URL remains bounded.
                        canvas.0.set_width(0);
                        canvas.0.set_height(0);
                        frame.release();
                        builder
                            .push(index, &image, || export_signal.aborted())
                            .map_err(|_| "focus movie encoding failed")?;
                        (export.progress)(builder.completed_planes(), export.total);
                    }
                }
                if index == selected {
                    s.image = Some(data);
                }
            }
            Ok(())
        })();
        if result.is_err() {
            out.borrow_mut().error = true;
        }
    }) as Box<dyn FnMut(VideoFrame)>);
    let err = state.clone();
    let error = Closure::wrap(Box::new(move |_: JsValue| {
        err.borrow_mut().error = true;
    }) as Box<dyn FnMut(JsValue)>);
    let init = VideoDecoderInit::new(
        error.as_ref().unchecked_ref(),
        output.as_ref().unchecked_ref(),
    );
    let codec = VideoDecoder::new(&init).map_err(|_| "decoder unavailable")?;
    let decoder = Decoder {
        codec,
        _output: output,
        _error: error,
    };
    decoder
        .codec
        .configure(&config)
        .map_err(|_| "configure failed")?;
    let started = js_sys::Date::now();
    for (index, unit) in parsed.units.iter().enumerate() {
        while decoder.codec.decode_queue_size() >= queue_limit {
            cancelled(signal)?;
            if state.borrow().error {
                return Err("decode failed");
            }
            if js_sys::Date::now() - started > timeout {
                return Err("decode timeout");
            }
            pause().await;
        }
        cancelled(signal)?;
        if state.borrow().error {
            return Err("decode failed");
        }
        let data = Uint8Array::from(&bytes[unit.bytes.clone()]);
        let init = EncodedVideoChunkInit::new_with_u8_array(
            &data,
            (index * 1000) as i32,
            if unit.key {
                EncodedVideoChunkType::Key
            } else {
                EncodedVideoChunkType::Delta
            },
        );
        let chunk = EncodedVideoChunk::new(&init).map_err(|_| "chunk failed")?;
        decoder.codec.decode(&chunk).map_err(|_| "decode failed")?;
    }
    // Observe rejection immediately; polling remains cancellable while flush is pending.
    wait_promise(decoder.codec.flush().unchecked_into(), signal, Some(&state)).await?;
    cancelled(signal)?;
    let data = {
        let mut s = state.borrow_mut();
        if s.error || s.seen.iter().any(|v| !v) {
            return Err("incomplete decode");
        }
        s.image.take().ok_or("selected frame missing")?
    };
    // Verify the generated JPEG through a separate actual browser image decode;
    // a broken/lost canvas cannot publish a claimed derivative geometry.
    let image = web_sys::HtmlImageElement::new().map_err(|_| "output image unavailable")?;
    image.set_src(&data);
    let result = wait_promise(image.decode(), signal, None).await;
    let valid = result.is_ok()
        && image.natural_width() == sequence.visible_width
        && image.natural_height() == sequence.visible_height;
    image.set_src("");
    if !valid {
        return Err("output image extent mismatch");
    }
    Ok(data)
}
pub async fn decode_container(
    bytes: &[u8],
    selected: usize,
    signal: &AbortSignal,
) -> Result<Decoded> {
    if ACTIVE.replace(true) {
        return Err("decoder busy");
    }
    let _admission = Admission;
    if !Reflect::get(&js_sys::global(), &JsValue::from_str("VideoDecoder"))
        .ok()
        .is_some_and(|v| v.is_function())
    {
        return Err("WebCodecs unavailable");
    }
    cancelled(signal)?;
    let limits = Limits {
        container_bytes: 8 * 1024 * 1024,
        json_bytes: 256 * 1024,
        sections: 256,
        json_nodes: 16_384,
        dimension: 2048,
        pixels: 2048 * 2048,
        decoded_bytes: 16 * 1024 * 1024,
        ..Default::default()
    };
    let c = Container::parse(bytes, limits).map_err(|_| "invalid video container")?;
    let processed = processed_asset(&c, limits).map_err(|_| "invalid video metadata")?;
    let first = processed.planes.first().ok_or("video stack missing")?;
    // Keep a <=8 MiB decoded stage beside the previous <=16 MiB stage,
    // <=4 MiB data URLs and <=3 MiB grid reserve within persistent 32 MiB.
    if u64::from(first.geometry.width) * u64::from(first.geometry.height) > 2 * 1024 * 1024 {
        return Err("video persistent image budget");
    }
    if selected >= processed.planes.len()
        || processed.planes.len() > 64
        || processed.planes.iter().any(|p| {
            p.representation != Representation::H264
                || p.block_index != first.block_index
                || p.section_identity != first.section_identity
                || p.geometry != first.geometry
        })
    {
        return Err("unsupported video stack");
    }
    let lambdas: Vec<_> = processed
        .planes
        .iter()
        .map(|p| p.lambda.unwrap_or(f64::NAN))
        .collect();
    let parsed = AnnexB::parse(
        first.payload,
        &lambdas,
        first.geometry.width,
        first.geometry.height,
    )
    .map_err(|e| match e {
        lytronaut_format::FormatError::Extent => "video stream extent mismatch",
        lytronaut_format::FormatError::Metadata => "video stream metadata mismatch",
        lytronaut_format::FormatError::Unsupported => "video stream unsupported syntax",
        lytronaut_format::FormatError::Budget => "video stream budget",
        lytronaut_format::FormatError::Truncated => "video stream truncated",
        _ => "invalid Annex-B stream",
    })?;
    let (image, software_retry) =
        match attempt(first.payload, &parsed, selected, signal, false, None).await {
            Ok(image) => (image, false),
            Err(_) => {
                cancelled(signal)?;
                (
                    attempt(first.payload, &parsed, selected, signal, true, None).await?,
                    true,
                )
            }
        };
    let encoded = web_sys::window()
        .ok_or("browser unavailable")?
        .atob(
            image
                .strip_prefix("data:image/jpeg;base64,")
                .ok_or("invalid generated JPEG")?,
        )
        .map_err(|_| "invalid generated JPEG")?
        .chars()
        .map(|c| c as u8)
        .collect::<Vec<_>>();
    let geometry =
        lytronaut_format::jpeg_geometry(&encoded, limits).map_err(|_| "invalid generated JPEG")?;
    if geometry != first.geometry {
        return Err("output image extent mismatch");
    }
    let content_sha256 = format!("{:x}", Sha256::digest(&encoded));
    Ok(Decoded {
        frames: parsed.units.len(),
        reference_frames: parsed.sequence.reference_frames,
        coded_width: parsed.sequence.coded_width,
        coded_height: parsed.sequence.coded_height,
        bidirectional_frames: parsed.units.iter().filter(|u| u.bidirectional).count(),
        software_retry,
        width: first.geometry.width,
        height: first.geometry.height,
        index: selected,
        lambda: lambdas[selected],
        lambdas,
        image,
        encoded_bytes: encoded.len(),
        content_sha256,
    })
}
/// The functional acceptance harness invokes the same Rust path as the viewer.
/// Callers must admit <=8 MiB input; aggregate reports must never retain `image`.
#[wasm_bindgen]
pub async fn decode_archived_refocus(
    bytes: Vec<u8>,
    selected: usize,
    signal: AbortSignal,
) -> std::result::Result<String, JsValue> {
    let result = decode_container(&bytes, selected, &signal)
        .await
        .map_err(JsValue::from_str)?;
    serde_json::to_string(&result).map_err(|_| JsValue::from_str("video report failed"))
}
/// Actual configure/decode/encode path, never a support-query-only capability claim.
/// One decoder, two queued chunks, no decoded-plane cache; complete pair only.
pub async fn export_container(
    bytes: &[u8],
    selected: usize,
    id: &str,
    signal: &AbortSignal,
    progress: impl Fn(usize, usize) + 'static,
) -> Result<Vec<u8>> {
    if ACTIVE.replace(true) {
        return Err("decoder busy");
    }
    let _admission = Admission;
    cancelled(signal)?;
    let limits = Limits {
        container_bytes: 8 * 1024 * 1024,
        dimension: 2048,
        pixels: 2 * 1024 * 1024,
        ..Default::default()
    };
    let c = Container::parse(bytes, limits).map_err(|_| "invalid video container")?;
    let processed = processed_asset(&c, limits).map_err(|_| "invalid video metadata")?;
    let first = processed.planes.first().ok_or("video stack missing")?;
    if processed.planes.iter().any(|p| {
        p.representation != Representation::H264
            || p.geometry != first.geometry
            || p.block_index != first.block_index
            || p.section_identity != first.section_identity
    }) {
        return Err("unsupported video stack");
    }
    let lambdas: Vec<_> = processed
        .planes
        .iter()
        .map(|p| p.lambda.unwrap_or(f64::NAN))
        .collect();
    let parsed = AnnexB::parse(
        first.payload,
        &lambdas,
        first.geometry.width,
        first.geometry.height,
    )
    .map_err(|_| "invalid Annex-B stream")?;
    let plan = lytronaut_format::live_photo::Plan::new(first.geometry, &lambdas, selected)
        .map_err(|_| "Live Photo geometry or sweep exceeds limits; retain ordinary still export")?;
    let export = Export {
        builder: Rc::new(RefCell::new(None)),
        progress: Rc::new(progress),
        total: plan.planes().len(),
    };
    let mut verified = false;
    for software in [false, true] {
        *export.builder.borrow_mut() = Some(
            lytronaut_format::live_photo::Builder::new(plan.clone(), id)
                .map_err(|_| "invalid export identifier")?,
        );
        if attempt(
            first.payload,
            &parsed,
            selected,
            signal,
            software,
            Some(export.clone()),
        )
        .await
        .is_ok()
        {
            verified = true;
            break;
        }
        cancelled(signal)?;
    }
    if !verified {
        return Err("actual browser codec or movie encode failed; retain ordinary still export");
    }
    cancelled(signal)?;
    let pair = export
        .builder
        .borrow_mut()
        .take()
        .ok_or("export missing")?
        .finish()
        .map_err(|_| "incomplete resource pair")?;
    let provenance=serde_json::to_vec(&serde_json::json!({"version":1,"motion":"generated_focus_animation","duration_seconds":3,"key_seconds":plan.key_seconds(),"selected_stored_plane":selected,"sweep_stored_planes":plan.planes(),"source_variant":"verified_archived_avc_planes","width":plan.geometry().width,"height":plan.geometry().height,"orientation":1,"audio":"none","originals":"unchanged","camera_recorded_temporal_motion":false,"calibrated_synthesis":false,"imported_adjustments":"preserved_only_not_replayed","color":"source profile unknown; platform-decoded RGB; sRGB transfer, BT.709 primaries, BT.601 limited-range YCbCr; JPEG/chroma quantization are lossy"})).map_err(|_|"provenance encoding failed")?;
    lytronaut_format::live_photo::bundle(pair, &provenance)
        .map_err(|_| "complete bundle exceeds budget")
}
#[wasm_bindgen]
pub async fn export_archived_live_photo(
    bytes: Vec<u8>,
    selected: usize,
    id: String,
    signal: AbortSignal,
) -> std::result::Result<Vec<u8>, JsValue> {
    export_container(&bytes, selected, &id, &signal, |_, _| {})
        .await
        .map_err(JsValue::from_str)
}
