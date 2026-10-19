//! Synthetic processed-LFP fixtures (MIT, independently authored): typed relative
//! float maps, preserved-only fallbacks, framing rejections and redacted
//! diagnostics. Float maps are relative vendor scalars, never distances.
use lytronaut_format::*;
use serde_json::{Value, json};
use sha1::{Digest, Sha1};

fn id(p: &[u8]) -> String {
    format!(
        "sha1-{}",
        Sha1::digest(p)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}
fn section(out: &mut Vec<u8>, directory: bool, payload: &[u8]) {
    out.extend_from_slice(if directory {
        b"\x89LFM\r\n\x1a\n"
    } else {
        b"\x89LFC\r\n\x1a\n"
    });
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(id(payload).as_bytes());
    out.extend_from_slice(&[0; 35]);
    out.extend_from_slice(payload);
    while !out.len().is_multiple_of(16) {
        out.push(0);
    }
}
fn container(directory: &Value, content: &[&[u8]]) -> Vec<u8> {
    let mut b = b"\x89LFP\r\n\x1a\n\0\0\0\x01\0\0\0\0".to_vec();
    section(&mut b, true, &serde_json::to_vec(directory).unwrap());
    for payload in content {
        section(&mut b, false, payload);
    }
    b
}
fn floats(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
fn jpeg(width: u32) -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode(
            &vec![90; width as usize * 3],
            width,
            1,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    bytes
}
fn depth_directory(depth: &[u8], confidence: &[u8], representation: &str) -> Value {
    json!({"version":{"major":1,"minor":0},"picture":{"accelerationArray":[{"type":"com.lytro.acceleration.depthMap","vendorContent":{
        "width":2,"height":1,
        "depthMap":{"representation":representation,"imageRef":id(depth)},
        "confidenceMap":{"representation":representation,"imageRef":id(confidence)}}}]}})
}

#[test]
fn depth_and_confidence_maps_are_typed_relative_float_planes() {
    let l = Limits::default();
    let depth = floats(&[-3.5, 12.25]);
    let confidence = floats(&[0.0, 1.0]);
    let bytes = container(
        &depth_directory(&depth, &confidence, "raw"),
        &[&depth, &confidence],
    );
    let c = Container::parse(&bytes, l).unwrap();
    let p = processed_asset(&c, l).unwrap();
    assert!(!p.preserved_only);
    assert!(p.planes.is_empty());
    let kinds: Vec<_> = p.maps.iter().map(|m| m.representation).collect();
    assert_eq!(
        kinds,
        [Representation::RelativeF32, Representation::ConfidenceF32]
    );
    for (map, payload) in p.maps.iter().zip([&depth, &confidence]) {
        assert_eq!(map.decode_support(), DecodeSupport::FiniteFloatInspection);
        assert_eq!(map.payload, payload.as_slice());
        assert_eq!(map.section_identity, id(payload));
        assert_eq!(map.lambda, None);
        assert_eq!(
            map.geometry,
            Geometry {
                width: 2,
                height: 1
            }
        );
        // Float maps are never handed to the JPEG decoder.
        assert!(matches!(map.decode_jpeg(l), Err(FormatError::Unsupported)));
    }
    let inspected = inspect_f32(p.maps[0].payload, p.maps[0].geometry, l).unwrap();
    assert_eq!((inspected.min, inspected.max), (-3.5, 12.25));
}

#[test]
fn hostile_float_maps_are_rejected_and_unknown_encodings_preserved() {
    let l = Limits::default();
    let confidence = floats(&[0.0, 1.0]);
    for (depth, expected) in [
        (floats(&[f32::NAN, 1.0]), FormatError::NonFinite),
        (floats(&[1.0]), FormatError::Extent),
    ] {
        let bytes = container(
            &depth_directory(&depth, &confidence, "raw"),
            &[&depth, &confidence],
        );
        let c = Container::parse(&bytes, l).unwrap();
        assert_eq!(processed_asset(&c, l).err().unwrap(), expected);
    }
    // A non-raw map encoding is kept as original metadata only.
    let depth = floats(&[1.0, 2.0]);
    let directory = depth_directory(&depth, &confidence, "png");
    let bytes = container(&directory, &[&depth, &confidence]);
    let c = Container::parse(&bytes, l).unwrap();
    let p = processed_asset(&c, l).unwrap();
    assert!(p.preserved_only);
    assert!(p.maps.is_empty());
    assert_eq!(p.original, directory);
}

#[test]
fn refocus_stack_depth_lut_and_skipped_image_encodings() {
    let l = Limits::default();
    let jpg = jpeg(2);
    let lut = floats(&[0.5, -0.5, 0.25, 4.0]);
    let directory = json!({"version":{"major":1,"minor":0},"picture":{"accelerationArray":[{"type":"com.lytro.acceleration.refocusStack","vendorContent":{
        "imageArray":[
            {"representation":"png","width":2,"height":1,"lambda":-1,"imageRef":id(&jpg)},
            {"representation":"jpeg","width":2,"height":1,"lambda":1,"imageRef":id(&jpg)}],
        "depthLut":{"representation":"raw","width":4,"height":1,"imageRef":id(&lut)}}}]}});
    let bytes = container(&directory, &[&jpg, &lut]);
    let c = Container::parse(&bytes, l).unwrap();
    let p = processed_asset(&c, l).unwrap();
    assert_eq!(p.planes.len(), 1, "non-JPEG stack entries are not decoded");
    assert_eq!(p.planes[0].frame_index, 1);
    assert_eq!(p.planes[0].decode_support(), DecodeSupport::NativeJpeg);
    assert_eq!(
        p.planes[0].decode_jpeg(l).unwrap().geometry,
        Geometry {
            width: 2,
            height: 1
        }
    );
    assert_eq!(p.maps.len(), 1);
    assert_eq!(p.maps[0].representation, Representation::RelativeF32);
    assert_eq!(p.maps[0].geometry.width, 4);

    // Declared geometry must match the stored JPEG exactly.
    let mut lying = directory.clone();
    lying["picture"]["accelerationArray"][0]["vendorContent"]["imageArray"][1]["width"] = json!(3);
    let bytes = container(&lying, &[&jpg, &lut]);
    let c = Container::parse(&bytes, l).unwrap();
    assert_eq!(processed_asset(&c, l).err().unwrap(), FormatError::Extent);
}

#[test]
fn unsupported_directory_versions_and_layouts_are_preserved_only() {
    let l = Limits::default();
    let unknown = json!({"version":{"major":2,"minor":0},"picture":{"accelerationArray":[]}});
    let bytes = container(&unknown, &[]);
    let c = Container::parse(&bytes, l).unwrap();
    let p = processed_asset(&c, l).unwrap();
    assert!(p.preserved_only && p.planes.is_empty() && p.maps.is_empty());
    assert_eq!(p.original, unknown);
    assert!(matches!(raw_asset(&c, l), Ok(RawAsset::PreservedOnly)));

    let pixels = [1u8, 2, 3];
    let frame = |pixels: &[u8], meta: &[u8]| json!({"frame":{"imageRef":id(pixels),"metadataRef":id(meta)}});
    // Multi-frame raw pictures are preserved, not guessed at.
    let meta = serde_json::to_vec(&json!({"camera":{"model":"F01"}})).unwrap();
    let two = json!({"version":{"major":1,"minor":0},"picture":{"frameArray":[frame(&pixels, &meta), frame(&pixels, &meta)]}});
    let bytes = container(&two, &[&pixels, &meta]);
    let c = Container::parse(&bytes, l).unwrap();
    assert!(matches!(raw_asset(&c, l), Ok(RawAsset::PreservedOnly)));
    // An F01 with an unsupported packing is preserved instead of misread.
    let meta = serde_json::to_vec(&json!({"camera":{"model":"F01"},"image":{"width":2,"height":1,"representation":"rawPacked","rawDetails":{"pixelPacking":{"bitsPerPixel":10,"endianness":"big"},"mosaic":{"tile":"r,gr:gb,b","upperLeftPixel":"b"}}}})).unwrap();
    let one =
        json!({"version":{"major":1,"minor":0},"picture":{"frameArray":[frame(&pixels, &meta)]}});
    let bytes = container(&one, &[&pixels, &meta]);
    let c = Container::parse(&bytes, l).unwrap();
    assert!(matches!(raw_asset(&c, l), Ok(RawAsset::PreservedOnly)));
}

#[test]
fn framing_rejections_are_typed_and_mapped_to_stable_codes() {
    let l = Limits::default();
    let directory = json!({"version":{"major":1,"minor":0}});
    let good = container(&directory, &[b"payload"]);
    assert_eq!(
        Container::parse(
            &good,
            Limits {
                container_bytes: 16,
                ..l
            }
        )
        .unwrap_err(),
        FormatError::Budget
    );
    let mut reserved = good.clone();
    reserved[12] = 1;
    assert_eq!(
        Container::parse(&reserved, l).unwrap_err(),
        FormatError::Magic
    );
    let mut section_version = good.clone();
    section_version[16 + 8] = 1;
    assert_eq!(
        Container::parse(&section_version, l).unwrap_err(),
        FormatError::Version
    );
    let mut empty = good.clone();
    empty[16 + 12..16 + 16].copy_from_slice(&0u32.to_be_bytes());
    assert_eq!(
        Container::parse(&empty, l).unwrap_err(),
        FormatError::Extent
    );
    // Exactly one directory section is allowed.
    let mut two = good.clone();
    section(&mut two, true, b"{\"second\":true}");
    let c = Container::parse(&two, l).unwrap();
    assert_eq!(c.directory(l).unwrap_err(), FormatError::Metadata);

    for (error, code) in [
        (FormatError::Budget, ErrorCode::ResourceLimit),
        (FormatError::Version, ErrorCode::UnsupportedVersion),
        (FormatError::Unsupported, ErrorCode::UnsupportedFormat),
        (FormatError::Identity, ErrorCode::CorruptAsset),
    ] {
        assert_eq!(error.error_code(), code);
        assert_eq!(error.to_string(), format!("{error:?}"));
    }
    assert_eq!(
        bounded_json(b"[null,{\"a\":null}]", l).unwrap(),
        json!([null, {"a": null}])
    );
}

#[test]
fn diagnostics_never_print_payloads_or_identities() {
    let l = Limits::default();
    let depth = floats(&[7.0, 8.0]);
    let confidence = b"SECRETxx".to_vec();
    let directory = depth_directory(&depth, &confidence, "raw");
    let bytes = container(&directory, &[&depth, &confidence]);
    let c = Container::parse(&bytes, l).unwrap();
    let section_debug = format!("{:?}", c.sections[2]);
    assert!(section_debug.contains("bytes: 8"));
    assert!(!section_debug.contains(&id(&confidence)));
    let references = c.references(&directory, ReferenceMode::Processed).unwrap();
    let reference_debug = format!("{references:?}");
    assert!(reference_debug.contains("Local"));
    assert!(!reference_debug.contains("sha1-"));
    let p = processed_asset(&c, l).unwrap();
    let plane_debug = format!("{:?}", p.maps[1]);
    assert!(plane_debug.contains("ConfidenceF32"));
    assert!(!plane_debug.contains("SECRET") && !plane_debug.contains("83, 69"));
}
