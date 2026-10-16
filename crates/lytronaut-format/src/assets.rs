use crate::{Container, FormatError as E, Limits, ReferenceMode, Result, bounded_json};
use image::ImageDecoder;
use serde_json::Value;
use std::io::Cursor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Geometry {
    pub width: u32,
    pub height: u32,
}
fn geometry(v: &Value, limits: Limits, bpp: u64) -> Result<Geometry> {
    let width = v["width"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(E::Metadata)?;
    let height = v["height"]
        .as_u64()
        .and_then(|v| u32::try_from(v).ok())
        .ok_or(E::Metadata)?;
    limits.extent(width, height, bpp)?;
    Ok(Geometry { width, height })
}
fn supported_directory(v: &Value) -> bool {
    v.pointer("/version/major").and_then(Value::as_u64) == Some(1)
        && v.pointer("/version/minor").and_then(Value::as_u64) == Some(0)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cfa {
    BlueTopLeft,
}
#[derive(Clone, Copy)]
pub struct Sensor<'a> {
    pub geometry: Geometry,
    pub cfa: Cfa,
    pub packed: &'a [u8],
}
impl std::fmt::Debug for Sensor<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sensor")
            .field("geometry", &self.geometry)
            .field("cfa", &self.cfa)
            .field("bytes", &self.packed.len())
            .finish()
    }
}
impl Sensor<'_> {
    pub fn unpack(&self, limits: Limits) -> Result<Vec<u16>> {
        unpack12(self.packed, self.geometry, limits)
    }
}
#[derive(Debug)]
pub enum RawAsset<'a> {
    F01(Sensor<'a>),
    PreservedOnly,
}
/// Extract only the observed F01 representation; unknown cameras/packing/versions
/// remain preserved-only. Dimensions and CFA come from referenced public metadata.
pub fn raw_asset<'a>(container: &Container<'a>, limits: Limits) -> Result<RawAsset<'a>> {
    let dir = container.directory(limits)?;
    if !supported_directory(&dir) {
        return Ok(RawAsset::PreservedOnly);
    }
    container.references(&dir, ReferenceMode::Raw)?;
    let frames = dir
        .pointer("/picture/frameArray")
        .and_then(Value::as_array)
        .ok_or(E::Metadata)?;
    if frames.len() != 1 {
        return Ok(RawAsset::PreservedOnly);
    }
    let frame = &frames[0]["frame"];
    let metadata = bounded_json(
        container
            .section(frame["metadataRef"].as_str().ok_or(E::Metadata)?)?
            .payload,
        limits,
    )?;
    if metadata.pointer("/camera/model").and_then(Value::as_str) != Some("F01") {
        return Ok(RawAsset::PreservedOnly);
    }
    let image = &metadata["image"];
    let raw = &image["rawDetails"];
    if image["representation"] != "rawPacked"
        || raw["pixelPacking"]["bitsPerPixel"] != 12
        || raw["pixelPacking"]["endianness"] != "big"
        || raw["mosaic"]["upperLeftPixel"] != "b"
        || raw["mosaic"]["tile"] != "r,gr:gb,b"
    {
        return Ok(RawAsset::PreservedOnly);
    }
    let geometry = geometry(image, limits, 2)?;
    let packed = container
        .section(frame["imageRef"].as_str().ok_or(E::Metadata)?)?
        .payload;
    packed_extent(packed, geometry, limits)?;
    Ok(RawAsset::F01(Sensor {
        geometry,
        cfa: Cfa::BlueTopLeft,
        packed,
    }))
}
fn packed_extent(bytes: &[u8], g: Geometry, l: Limits) -> Result<usize> {
    let samples = l.extent(g.width, g.height, 2)? / 2;
    if samples % 2 != 0 || samples.checked_mul(3).map(|v| v / 2) != Some(bytes.len()) {
        return Err(E::Extent);
    }
    Ok(samples)
}
pub fn unpack12(bytes: &[u8], g: Geometry, limits: Limits) -> Result<Vec<u16>> {
    let samples = packed_extent(bytes, g, limits)?;
    let mut out = Vec::with_capacity(samples);
    for p in bytes.as_chunks::<3>().0 {
        let v = (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2]);
        out.push((v >> 12) as u16);
        out.push((v & 4095) as u16);
    }
    Ok(out)
}
pub fn pack12(samples: &[u16], g: Geometry, limits: Limits) -> Result<Vec<u8>> {
    let n = limits.extent(g.width, g.height, 2)? / 2;
    if samples.len() != n || n % 2 != 0 || samples.iter().any(|v| *v > 4095) {
        return Err(E::Extent);
    }
    let mut out = Vec::with_capacity(n * 3 / 2);
    for p in samples.as_chunks::<2>().0 {
        let v = (u32::from(p[0]) << 12) | u32::from(p[1]);
        out.extend_from_slice(&[(v >> 16) as u8, (v >> 8) as u8, v as u8]);
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Representation {
    Jpeg,
    H264,
    RelativeF32,
    ConfidenceF32,
    Unknown,
}
#[derive(Clone)]
pub struct Plane<'a> {
    /// Stable source acceleration/block and frame positions; never renumber filtered frames.
    pub block_index: usize,
    pub frame_index: usize,
    pub geometry: Geometry,
    pub lambda: Option<f64>,
    pub representation: Representation,
    pub payload: &'a [u8],
    pub section_identity: &'a str,
}
impl std::fmt::Debug for Plane<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Plane")
            .field("block_index", &self.block_index)
            .field("frame_index", &self.frame_index)
            .field("geometry", &self.geometry)
            .field("representation", &self.representation)
            .finish()
    }
}
/// Availability is per payload, never inferred from a directory version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeSupport {
    NativeJpeg,
    FiniteFloatInspection,
    RequiresPlatformCodec,
    PreservedOnly,
}
impl Plane<'_> {
    pub fn decode_support(&self) -> DecodeSupport {
        match self.representation {
            Representation::Jpeg => DecodeSupport::NativeJpeg,
            Representation::RelativeF32 | Representation::ConfidenceF32 => {
                DecodeSupport::FiniteFloatInspection
            }
            Representation::H264 => DecodeSupport::RequiresPlatformCodec,
            Representation::Unknown => DecodeSupport::PreservedOnly,
        }
    }
    pub fn decode_jpeg(&self, limits: Limits) -> Result<DecodedJpeg> {
        if self.representation != Representation::Jpeg {
            return Err(E::Unsupported);
        }
        let decoded = decode_jpeg(self.payload, limits)?;
        if decoded.geometry != self.geometry {
            return Err(E::Extent);
        }
        Ok(decoded)
    }
}
/// Original directory preserves unknown recipes, acceleration descriptions and
/// scalar types. H264 payloads are offered for a separately gated platform codec.
pub struct Processed<'a> {
    pub original: Value,
    pub planes: Vec<Plane<'a>>,
    pub maps: Vec<Plane<'a>>,
    /// True when no recognized payload descriptors exist; false is not a rendering capability.
    pub preserved_only: bool,
}
fn payload<'a>(c: &Container<'a>, v: &Value, key: &str) -> Result<(&'a str, &'a [u8])> {
    let s = c.section(v[key].as_str().ok_or(E::Metadata)?)?;
    Ok((s.identity, s.payload))
}
fn lambda(v: &Value) -> Result<Option<f64>> {
    if v.get("lambda").is_none() {
        return Ok(None);
    }
    let n = v["lambda"].as_f64().ok_or(E::Metadata)?;
    if !n.is_finite() {
        return Err(E::NonFinite);
    }
    Ok(Some(n))
}
pub fn processed_asset<'a>(c: &Container<'a>, limits: Limits) -> Result<Processed<'a>> {
    let original = c.directory(limits)?;
    #[cfg(feature = "test-harness")]
    crate::capacity_observations::metadata_json(&original);
    if !supported_directory(&original) {
        return Ok(Processed {
            original,
            planes: vec![],
            maps: vec![],
            preserved_only: true,
        });
    }
    c.references(&original, ReferenceMode::Processed)?;
    let mut planes = Vec::new();
    let mut maps = Vec::new();
    if let Some(acc) = original
        .pointer("/picture/accelerationArray")
        .and_then(Value::as_array)
    {
        for (block_index, a) in acc.iter().enumerate() {
            let v = &a["vendorContent"];
            if a["type"] == "com.lytro.acceleration.refocusStack" {
                if let Some(images) = v["imageArray"].as_array() {
                    let mut previous = None;
                    for (frame_index, i) in images.iter().enumerate() {
                        if i["representation"] != "jpeg" {
                            continue;
                        }
                        let g = geometry(i, limits, 4)?;
                        let lambda = lambda(i)?.ok_or(E::Metadata)?;
                        if previous.is_some_and(|p| lambda <= p) {
                            return Err(E::Metadata);
                        }
                        previous = Some(lambda);
                        let (section_identity, payload) = payload(c, i, "imageRef")?;
                        if jpeg_geometry(payload, limits)? != g {
                            return Err(E::Extent);
                        }
                        planes.push(Plane {
                            block_index,
                            frame_index,
                            geometry: g,
                            lambda: Some(lambda),
                            representation: Representation::Jpeg,
                            payload,
                            section_identity,
                        });
                    }
                }
                let b = &v["blockOfImages"];
                if b["representation"] == "h264" {
                    let (section_identity, payload) = payload(c, b, "blockOfImagesRef")?;
                    let frames = b["metadataArray"].as_array().ok_or(E::Metadata)?;
                    for (frame_index, i) in frames.iter().enumerate() {
                        planes.push(Plane {
                            block_index,
                            frame_index,
                            geometry: geometry(i, limits, 4)?,
                            lambda: lambda(i)?,
                            representation: Representation::H264,
                            payload,
                            section_identity,
                        });
                    }
                }
                if !v["depthLut"].is_null() {
                    let i = &v["depthLut"];
                    add_map(
                        c,
                        i,
                        geometry(i, limits, 4)?,
                        block_index,
                        Representation::RelativeF32,
                        limits,
                        &mut maps,
                    )?;
                }
            } else if a["type"] == "com.lytro.acceleration.depthMap" {
                let g = geometry(v, limits, 4)?;
                for (key, kind) in [
                    ("depthMap", Representation::RelativeF32),
                    ("confidenceMap", Representation::ConfidenceF32),
                ] {
                    add_map(c, &v[key], g, block_index, kind, limits, &mut maps)?;
                }
            }
        }
    }
    let preserved_only = planes.is_empty() && maps.is_empty();
    #[cfg(feature = "test-harness")]
    crate::capacity_observations::metadata_planes(&planes, &maps);
    Ok(Processed {
        original,
        planes,
        maps,
        preserved_only,
    })
}
fn add_map<'a>(
    c: &Container<'a>,
    v: &Value,
    g: Geometry,
    index: usize,
    kind: Representation,
    l: Limits,
    out: &mut Vec<Plane<'a>>,
) -> Result<()> {
    if v["representation"] != "raw" {
        return Ok(());
    }
    let (section_identity, payload) = payload(c, v, "imageRef")?;
    inspect_f32(payload, g, l)?;
    out.push(Plane {
        block_index: index,
        frame_index: 0,
        geometry: g,
        lambda: None,
        representation: kind,
        payload,
        section_identity,
    });
    Ok(())
}
#[derive(Debug, Clone, Copy)]
pub struct FloatInspection {
    pub geometry: Geometry,
    pub min: f32,
    pub max: f32,
    pub samples: usize,
}
/// Values are unmodified relative vendor scalars, never distances in metres.
pub fn inspect_f32(bytes: &[u8], g: Geometry, limits: Limits) -> Result<FloatInspection> {
    if bytes.len() != limits.extent(g.width, g.height, 4)? {
        return Err(E::Extent);
    }
    let mut min = f32::INFINITY;
    let mut max = f32::NEG_INFINITY;
    for b in bytes.as_chunks::<4>().0 {
        let v = f32::from_le_bytes(*b);
        if !v.is_finite() {
            return Err(E::NonFinite);
        }
        min = min.min(v);
        max = max.max(v);
    }
    Ok(FloatInspection {
        geometry: g,
        min,
        max,
        samples: bytes.len() / 4,
    })
}

/// Bounded JPEG header inspection without decoding pixels or allocating codec scratch.
/// This does not validate entropy data; platform callers must independently decode
/// the generated image before publishing it as verified pixels.
pub fn jpeg_metadata_geometry(bytes: &[u8], limits: Limits) -> Result<Geometry> {
    if bytes.len() > limits.jpeg_bytes {
        return Err(E::Budget);
    }
    if bytes.get(..2) != Some(&[255, 216]) {
        return Err(E::Magic);
    }
    let mut p = 2;
    while p < bytes.len() {
        if bytes[p] != 255 {
            return Err(E::Codec);
        }
        while bytes.get(p) == Some(&255) {
            p += 1;
        }
        let m = *bytes.get(p).ok_or(E::Truncated)?;
        p += 1;
        if m == 0 || m == 216 || m == 217 || m == 218 {
            return Err(E::Codec);
        }
        if m == 1 || (208..=215).contains(&m) {
            continue;
        }
        let size = bytes
            .get(p..p.checked_add(2).ok_or(E::Truncated)?)
            .ok_or(E::Truncated)?;
        let n = u16::from_be_bytes(size.try_into().unwrap()) as usize;
        if n < 2 {
            return Err(E::Codec);
        }
        let end = p.checked_add(n).ok_or(E::Truncated)?;
        let segment = bytes.get(p + 2..end).ok_or(E::Truncated)?;
        if [192, 193, 194].contains(&m) {
            if segment.len() < 6 || segment[0] != 8 {
                return Err(E::Unsupported);
            }
            let height = u16::from_be_bytes([segment[1], segment[2]]) as u32;
            let width = u16::from_be_bytes([segment[3], segment[4]]) as u32;
            let components = segment[5] as usize;
            if ![1, 3, 4].contains(&components) || segment.len() != 6 + 3 * components {
                return Err(E::Codec);
            }
            limits.extent(width, height, 4)?;
            return Ok(Geometry { width, height });
        }
        if (195..=207).contains(&m) && ![196, 200, 204].contains(&m) {
            return Err(E::Unsupported);
        }
        p = end;
    }
    Err(E::Truncated)
}
/// JPEG geometry plus conservative admission for an actual Rust pixel decode.
pub fn jpeg_geometry(bytes: &[u8], limits: Limits) -> Result<Geometry> {
    let geometry = jpeg_metadata_geometry(bytes, limits)?;
    // Codec scratch/coefficient buffers, encoded copy and final RGBA remain
    // separately bounded from image's best-effort max_alloc.
    let working =
        u64::from(geometry.width) * u64::from(geometry.height) * 64 + (bytes.len() as u64) * 2;
    if working > limits.decoder_working_bytes {
        return Err(E::Budget);
    }
    Ok(geometry)
}
pub struct DecodedJpeg {
    pub geometry: Geometry,
    pub rgba: Vec<u8>,
}
pub fn decode_jpeg(bytes: &[u8], limits: Limits) -> Result<DecodedJpeg> {
    let geometry = jpeg_geometry(bytes, limits)?;
    let mut decoder =
        image::codecs::jpeg::JpegDecoder::new(Cursor::new(bytes)).map_err(|_| E::Codec)?;
    if decoder.dimensions() != (geometry.width, geometry.height) {
        return Err(E::Extent);
    }
    let mut codec_limits = image::Limits::default();
    codec_limits.max_image_width = Some(limits.dimension);
    codec_limits.max_image_height = Some(limits.dimension);
    codec_limits.max_alloc = Some(limits.decoded_bytes);
    decoder.set_limits(codec_limits).map_err(|_| E::Budget)?;
    if decoder.total_bytes() > limits.decoded_bytes {
        return Err(E::Budget);
    }
    let image = image::DynamicImage::from_decoder(decoder)
        .map_err(|_| E::Codec)?
        .into_rgba8();
    Ok(DecodedJpeg {
        geometry,
        rgba: image.into_raw(),
    })
}

/// Standard EXIF 1–8 transform of verified derivative pixels. Catalog code 0 is invalid.
/// Coordinates are defined independently of any imported/baked processing recipe.
pub fn orient_derivative(
    image: DecodedJpeg,
    orientation: u8,
    limits: Limits,
) -> Result<DecodedJpeg> {
    if !(1..=8).contains(&orientation) {
        return Err(E::Unsupported);
    }
    let Geometry {
        width: w,
        height: h,
    } = image.geometry;
    let n = limits.extent(w, h, 4)?;
    if image.rgba.len() != n
        || (n as u64)
            .checked_mul(if orientation == 1 { 1 } else { 2 })
            .is_none_or(|v| v > limits.decoder_working_bytes)
    {
        return Err(E::Budget);
    }
    if orientation == 1 {
        return Ok(image);
    }
    let (ow, oh) = if orientation >= 5 { (h, w) } else { (w, h) };
    let mut rgba = vec![0; n];
    for y in 0..h {
        for x in 0..w {
            let (dx, dy) = match orientation {
                1 => (x, y),
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (h - 1 - y, x),
                7 => (h - 1 - y, w - 1 - x),
                8 => (y, w - 1 - x),
                _ => unreachable!(),
            };
            let src = ((y * w + x) * 4) as usize;
            let dst = ((dy * ow + dx) * 4) as usize;
            rgba[dst..dst + 4].copy_from_slice(&image.rgba[src..src + 4]);
        }
    }
    Ok(DecodedJpeg {
        geometry: Geometry {
            width: ow,
            height: oh,
        },
        rgba,
    })
}
/// Capacity growth and every codec write are admitted before allocation.
struct CappedJpeg {
    bytes: Vec<u8>,
    cap: usize,
    exceeded: bool,
}
impl std::io::Write for CappedJpeg {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let Some(required) = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|n| *n <= self.cap)
        else {
            self.exceeded = true;
            return Err(std::io::ErrorKind::OutOfMemory.into());
        };
        if required > self.bytes.capacity() {
            let capacity = required
                .max(self.bytes.capacity().saturating_mul(2))
                .max(4096)
                .min(self.cap);
            self.bytes
                .try_reserve_exact(capacity - self.bytes.len())
                .map_err(|_| {
                    self.exceeded = true;
                    std::io::Error::from(std::io::ErrorKind::OutOfMemory)
                })?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
/// Conservative encoder admission includes borrowed RGBA, owned RGB, codec
/// scratch and two output capacities during allocator growth; never an unbounded Vec.
pub fn encode_derivative(image: &DecodedJpeg, limits: Limits) -> Result<Vec<u8>> {
    let n = limits.extent(image.geometry.width, image.geometry.height, 4)?;
    if image.rgba.len() != n {
        return Err(E::Extent);
    }
    let working = (n as u64)
        .checked_mul(16)
        .and_then(|v| v.checked_add((limits.jpeg_bytes as u64).checked_mul(2)?))
        .and_then(|v| v.checked_add(16 * 1024))
        .ok_or(E::Budget)?;
    if limits.jpeg_bytes == 0 || working > limits.decoder_working_bytes {
        return Err(E::Budget);
    }
    let rgb: Vec<u8> = image
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| p[..3].iter().copied())
        .collect();
    let mut output = CappedJpeg {
        bytes: Vec::new(),
        cap: limits.jpeg_bytes,
        exceeded: false,
    };
    let result = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut output, 90).encode(
        &rgb,
        image.geometry.width,
        image.geometry.height,
        image::ExtendedColorType::Rgb8,
    );
    result.map_err(|_| if output.exceeded { E::Budget } else { E::Codec })?;
    Ok(output.bytes)
}

/// Lossless PNG encoding of decoded stored JPEG pixels; no intervening JPEG re-encode.
pub fn encode_png_derivative(image: &DecodedJpeg, limits: Limits) -> Result<Vec<u8>> {
    use image::ImageEncoder;
    let n = limits.extent(image.geometry.width, image.geometry.height, 4)?;
    if image.rgba.len() != n {
        return Err(E::Extent);
    }
    if limits.jpeg_bytes == 0
        || (n as u64) * 8 + (limits.jpeg_bytes as u64) * 2 + 16384 > limits.decoder_working_bytes
    {
        return Err(E::Budget);
    }
    let mut output = CappedJpeg {
        bytes: Vec::new(),
        cap: limits.jpeg_bytes,
        exceeded: false,
    };
    let result = image::codecs::png::PngEncoder::new(&mut output).write_image(
        &image.rgba,
        image.geometry.width,
        image.geometry.height,
        image::ExtendedColorType::Rgba8,
    );
    result.map_err(|_| if output.exceeded { E::Budget } else { E::Codec })?;
    Ok(output.bytes)
}
