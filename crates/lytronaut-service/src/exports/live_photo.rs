//! Native portable stored JPEG-plane Live Photo export. Originals are read-only.
use super::*;
use lytronaut_format::live_photo::{Builder, Plan};
pub(super) async fn render(
    a: &Api,
    asset: OpaqueId,
    index: usize,
    orientation: u8,
    id: &str,
    cancel: &AtomicBool,
    progress: &AtomicU64,
) -> Result<Vec<u8>> {
    let (_, key) = a.lookup(asset.as_str(), "file", true).await?;
    if role(&key) != OriginalRole::JpegFocusStack {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::UnsupportedFormat,
        ));
    }
    let d = a.descriptor(asset.as_str(), false).await?;
    let _permit =
        a.0.validation
            .clone()
            .acquire_owned()
            .await
            .map_err(storage)?;
    if d.length > 8 * 1024 * 1024 {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::ResourceLimit,
        ));
    }
    let mut source = vec![0; d.length as usize];
    d.file.read_exact_at(&mut source, 0).map_err(storage)?;
    let err = |e: lytronaut_format::FormatError| {
        Failure(StatusCode::UNPROCESSABLE_ENTITY, e.error_code())
    };
    let container = lytronaut_format::Container::parse(&source, focus_limits()).map_err(err)?;
    let processed = lytronaut_format::processed_asset(&container, focus_limits()).map_err(err)?;
    let first = processed.planes.first().ok_or(Failure::missing())?;
    if processed.planes.iter().any(|p| {
        p.representation != lytronaut_format::Representation::Jpeg
            || p.geometry != first.geometry
            || p.block_index != first.block_index
    }) {
        return Err(Failure(
            StatusCode::UNPROCESSABLE_ENTITY,
            ErrorCode::UnsupportedFormat,
        ));
    }
    let geometry = if orientation >= 5 {
        lytronaut_format::Geometry {
            width: first.geometry.height,
            height: first.geometry.width,
        }
    } else {
        first.geometry
    };
    let lambdas: Vec<_> = processed
        .planes
        .iter()
        .map(|p| p.lambda.unwrap_or(f64::NAN))
        .collect();
    let plan = Plan::new(geometry, &lambdas, index).map_err(err)?;
    let hex = id.strip_prefix("app-").ok_or(Failure::invalid())?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Failure::invalid());
    }
    let pairing = format!(
        "{}-{}-4{}-8{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[13..16],
        &hex[17..20],
        &hex[20..32]
    );
    let mut builder = Builder::new(plan.clone(), &pairing).map_err(err)?;
    for &plane_index in plan.planes() {
        if cancel.load(Ordering::Acquire) {
            return Err(Failure::invalid());
        }
        let decoded = processed.planes[plane_index]
            .decode_jpeg(focus_limits())
            .map_err(err)?;
        let oriented = lytronaut_format::orient_derivative(decoded, orientation, focus_limits())
            .map_err(err)?;
        builder
            .push(plane_index, &oriented, || cancel.load(Ordering::Acquire))
            .map_err(err)?;
        progress.fetch_add(oriented.rgba.len() as u64, Ordering::Relaxed);
    }
    let pair = builder.finish().map_err(err)?;
    let provenance=serde_json::to_vec(&serde_json::json!({
        "version":1,"motion":"generated_focus_animation","duration_seconds":3,
        "key_seconds":plan.key_seconds(),"selected_stored_plane":index,"selected_relative_lambda":lambdas[index],
        "sweep_stored_planes":plan.planes(),"source_variant":"verified_stored_jpeg_planes",
        "orientation":orientation,"width":geometry.width,"height":geometry.height,
        "audio":"none","originals":"unchanged","imported_adjustments":"preserved_only_not_replayed",
        "color":"source profile unknown; decoded numeric RGB encoded with sRGB transfer, BT.709 primaries and BT.601 limited-range YCbCr; chroma subsampling/quantization and JPEG re-encoding are lossy",
        "calibrated_synthesis":false,"camera_recorded_temporal_motion":false
    })).map_err(storage)?;
    lytronaut_format::live_photo::bundle(pair, &provenance).map_err(err)
}
