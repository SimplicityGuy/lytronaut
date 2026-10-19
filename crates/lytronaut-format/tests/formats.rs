use lytronaut_format::*;
use serde_json::json;
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
fn container(parts: &[(bool, Vec<u8>)]) -> Vec<u8> {
    let mut b = b"\x89LFP\r\n\x1a\n\0\0\0\x01\0\0\0\0".to_vec();
    for (directory, p) in parts {
        b.extend_from_slice(if *directory {
            b"\x89LFM\r\n\x1a\n"
        } else {
            b"\x89LFC\r\n\x1a\n"
        });
        b.extend_from_slice(&[0; 4]);
        b.extend_from_slice(&(p.len() as u32).to_be_bytes());
        b.extend_from_slice(id(p).as_bytes());
        b.extend_from_slice(&[0; 35]);
        b.extend_from_slice(p);
        while !b.len().is_multiple_of(16) {
            b.push(0);
        }
    }
    b
}
fn raw(model: &str, version: u32) -> Vec<u8> {
    let pixels = vec![0xab, 0xcd, 0xef];
    let private = b"{}".to_vec();
    let meta=serde_json::to_vec(&json!({"camera":{"model":model},"image":{"width":2,"height":1,"representation":"rawPacked","rawDetails":{"pixelPacking":{"bitsPerPixel":12,"endianness":"big"},"mosaic":{"tile":"r,gr:gb,b","upperLeftPixel":"b"}}}})).unwrap();
    let dir=serde_json::to_vec(&json!({"version":{"major":version,"minor":0},"picture":{"frameArray":[{"frame":{"imageRef":id(&pixels),"metadataRef":id(&meta),"privateMetadataRef":id(&private)}}]}})).unwrap();
    container(&[
        (true, dir),
        (false, pixels),
        (false, meta),
        (false, private),
    ])
}
#[test]
fn exact_f01_and_preserved_variants() {
    let l = Limits::default();
    let bytes = raw("F01", 1);
    let c = Container::parse(&bytes, l).unwrap();
    let RawAsset::F01(s) = raw_asset(&c, l).unwrap() else {
        panic!()
    };
    let samples = s.unpack(l).unwrap();
    assert_eq!(samples, [0xabc, 0xdef]);
    assert_eq!(pack12(&samples, s.geometry, l).unwrap(), s.packed);
    for (m, v) in [("Illum", 1), ("F01", 2)] {
        let b = raw(m, v);
        let c = Container::parse(&b, l).unwrap();
        assert!(matches!(raw_asset(&c, l), Ok(RawAsset::PreservedOnly)));
    }
    assert!(pack12(&[4096, 0], s.geometry, l).is_err());
    assert!(unpack12(&[0, 0], s.geometry, l).is_err());
}
#[test]
fn bounded_framing_every_truncation_and_identity() {
    let l = Limits::default();
    let b = raw("F01", 1);
    for end in 0..b.len() {
        // A prefix ending exactly on a section boundary is a framed container,
        // but can never be a complete raw asset: required local refs are absent.
        if let Ok(c) = Container::parse(&b[..end], l) {
            assert!(
                raw_asset(&c, l).is_err(),
                "complete raw at truncated end {end}"
            );
        }
    }
    let mut bad = b.clone();
    bad[112] ^= 1;
    assert_eq!(
        Container::parse(&bad, l).unwrap_err(),
        FormatError::Identity
    );
    let mut bad = b.clone();
    bad[28..32].copy_from_slice(&u32::MAX.to_be_bytes());
    assert!(Container::parse(&bad, l).is_err());
    let duplicate = container(&[(true, b"{}".to_vec()), (false, b"{}".to_vec())]);
    assert_eq!(
        Container::parse(&duplicate, l).unwrap_err(),
        FormatError::Duplicate
    );
    let hidden = container(&[(false, b"not a header \x89LFM\r\n\x1a\n".to_vec())]);
    assert_eq!(Container::parse(&hidden, l).unwrap().sections.len(), 1);
    assert!(Container::parse(&b, Limits { sections: 1, ..l }).is_err());
    let mut bad = b.clone();
    bad[8] = 1;
    assert_eq!(Container::parse(&bad, l).unwrap_err(), FormatError::Version);
    let padded = container(&[(true, b"{}".to_vec())]);
    let mut bad = padded;
    *bad.last_mut().unwrap() = 1;
    assert_eq!(
        Container::parse(&bad, l).unwrap_err(),
        FormatError::Alignment
    );
}
#[test]
fn missing_local_vs_authorized_external() {
    let l = Limits::default();
    let absent = id(b"missing");
    let v = json!({"picture":{"frameArray":[{"frame":{"imageRef":absent}}]}});
    let b = container(&[(true, serde_json::to_vec(&v).unwrap())]);
    let c = Container::parse(&b, l).unwrap();
    assert_eq!(
        c.references(&v, ReferenceMode::Raw).unwrap_err(),
        FormatError::MissingReference
    );
    assert_eq!(
        c.references(&v, ReferenceMode::Processed).unwrap()[0].location,
        ReferenceLocation::ExternalOriginal
    );
    let v = json!({"picture":{"accelerationArray":[{"imageRef":absent}]}});
    assert_eq!(
        c.references(&v, ReferenceMode::Processed).unwrap_err(),
        FormatError::MissingReference
    );
    let v = json!({"imageRef":"sha1-invalid"});
    assert_eq!(
        c.references(&v, ReferenceMode::Processed).unwrap_err(),
        FormatError::Identity
    );
}
#[test]
fn hostile_json_and_original_scalar_types() {
    let l = Limits::default();
    assert_eq!(
        bounded_json(&vec![b' '; l.json_bytes + 1], l),
        Err(FormatError::Budget)
    );
    let deep = format!("{}0{}", "[".repeat(65), "]".repeat(65));
    assert_eq!(bounded_json(deep.as_bytes(), l), Err(FormatError::Budget));
    assert_eq!(
        bounded_json(b"{\"x\":1,\"x\":2}", l),
        Err(FormatError::Json)
    );
    assert!(bounded_json(b"[1e999]", l).is_err());
    assert!(bounded_json(b"{} garbage", l).is_err());
    assert_eq!(
        bounded_json(b"[0,0,0,0]", Limits { json_nodes: 3, ..l }),
        Err(FormatError::Budget)
    );
    let v = bounded_json(b"{\"focus\":\"1.25\",\"unknown\":42}", l).unwrap();
    let r = Recipe::from_value(v.clone()).unwrap();
    assert_eq!(r.original, v);
    assert_eq!(r.fields["focus"], "1.25");
}
fn jpeg() -> Vec<u8> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode(
            &[200, 0, 0, 0, 200, 0],
            2,
            1,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    bytes
}
#[test]
fn jpeg_decode_preflight_bomb_truncation() {
    let l = Limits::default();
    let b = jpeg();
    let j = decode_jpeg(&b, l).unwrap();
    assert_eq!(
        j.geometry,
        Geometry {
            width: 2,
            height: 1
        }
    );
    assert_eq!(j.rgba.len(), 8);
    assert!(decode_jpeg(&b, Limits { pixels: 1, ..l }).is_err());
    assert!(
        decode_jpeg(
            &b,
            Limits {
                decoder_working_bytes: 1,
                ..l
            }
        )
        .is_err()
    );
    assert!(decode_jpeg(&b, Limits { jpeg_bytes: 1, ..l }).is_err());
    let mut bomb = b.clone();
    let p = bomb.windows(2).position(|w| w == [255, 192]).unwrap();
    bomb[p + 5..p + 9].copy_from_slice(&[255, 255, 255, 255]);
    assert!(decode_jpeg(&bomb, l).is_err());
    for end in 0..p + 18 {
        assert!(jpeg_geometry(&b[..end], l).is_err());
    }
}
#[test]
fn finite_relative_maps_without_clamping() {
    let l = Limits::default();
    let g = Geometry {
        width: 2,
        height: 1,
    };
    let b = [(-20f32).to_le_bytes(), 1.000_000_2_f32.to_le_bytes()].concat();
    let i = inspect_f32(&b, g, l).unwrap();
    assert!(i.max > 1.);
    assert_eq!(i.min, -20.);
    for v in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        let b = [v.to_le_bytes(), 0f32.to_le_bytes()].concat();
        assert_eq!(inspect_f32(&b, g, l).unwrap_err(), FormatError::NonFinite);
    }
    assert!(inspect_f32(&b[..4], g, l).is_err());
    assert!(l.extent(u32::MAX, u32::MAX, 4).is_err());
    let arithmetic = Limits {
        dimension: u32::MAX,
        pixels: u64::MAX,
        decoded_bytes: u64::MAX,
        ..l
    };
    assert_eq!(
        arithmetic.extent(u32::MAX, u32::MAX, 4),
        Err(FormatError::Extent)
    );
}
#[test]
fn typed_jpeg_stack_and_stable_indices() {
    let l = Limits::default();
    let jpg = jpeg();
    let dir = json!({"version":{"major":1,"minor":0},"picture":{"accelerationArray":[{"type":"unknown"},{"type":"com.lytro.acceleration.refocusStack","vendorContent":{"imageArray":[{"representation":"jpeg","width":2,"height":1,"lambda":-1.25,"imageRef":id(&jpg)},{"representation":"jpeg","width":2,"height":1,"lambda":0.25,"imageRef":id(&jpg)}]}}]}});
    let b = container(&[(true, serde_json::to_vec(&dir).unwrap()), (false, jpg)]);
    let c = Container::parse(&b, l).unwrap();
    let p = processed_asset(&c, l).unwrap();
    assert_eq!(p.planes.len(), 2);
    assert_eq!(p.planes[1].block_index, 1);
    assert_eq!(p.planes[1].frame_index, 1);
    assert_eq!(p.planes[1].lambda, Some(0.25));
    assert_eq!(p.original, dir);
    let mut bad = dir;
    bad["picture"]["accelerationArray"][1]["vendorContent"]["imageArray"][1]["lambda"] = json!(-2);
    let b = container(&[(true, serde_json::to_vec(&bad).unwrap()), (false, jpeg())]);
    let c = Container::parse(&b, l).unwrap();
    assert!(processed_asset(&c, l).is_err());
}

#[test]
fn diagnostic_debug_redacts_samples() {
    let sensor = Sensor {
        geometry: Geometry {
            width: 2,
            height: 1,
        },
        cfa: Cfa::BlueTopLeft,
        packed: b"PRIVATE_PIXELS",
    };
    let debug = format!("{:?}", RawAsset::F01(sensor));
    assert!(debug.contains("bytes: 14"));
    assert!(!debug.contains("PRIVATE"));
    assert!(!debug.contains("80, 82"));
}

#[test]
fn deterministic_mutational_fuzz_smoke() {
    let l = Limits {
        container_bytes: 32_768,
        json_bytes: 4096,
        json_depth: 16,
        json_nodes: 1024,
        sections: 16,
        dimension: 64,
        pixels: 4096,
        decoded_bytes: 16_384,
        decoder_working_bytes: 1_048_576,
        jpeg_bytes: 4096,
    };
    let seeds = [
        raw("F01", 1),
        jpeg(),
        br#"{"version":{"major":1,"minor":0},"unknown":[1,"a",true,null]}"#.to_vec(),
    ];
    let mut state = 0x8abcf01u64;
    for seed in seeds {
        for _ in 0..512 {
            let mut b = seed.clone();
            for _ in 0..4 {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let index = state as usize % b.len();
                b[index] ^= (state >> 32) as u8;
            }
            let _ = bounded_json(&b, l);
            let _ = decode_jpeg(&b, l);
            if let Ok(c) = Container::parse(&b, l) {
                let _ = raw_asset(&c, l);
                let _ = processed_asset(&c, l);
            }
        }
    }
}

#[test]
fn unsupported_accelerations_and_h264_are_honest() {
    let l = Limits::default();
    let block = vec![0, 0, 0, 1, 9, 16];
    let dir = json!({"version":{"major":1,"minor":0},"picture":{"accelerationArray":[{"type":"unknown"},{"type":"com.lytro.acceleration.refocusStack","vendorContent":{"blockOfImages":{"representation":"h264","blockOfImagesRef":id(&block),"metadataArray":[{"width":2,"height":2,"lambda":-1},{"width":2,"height":2,"lambda":1}]}}}]}});
    let bytes = container(&[(true, serde_json::to_vec(&dir).unwrap()), (false, block)]);
    let c = Container::parse(&bytes, l).unwrap();
    let p = processed_asset(&c, l).unwrap();
    assert_eq!(p.planes.len(), 2);
    assert_eq!(p.planes[1].frame_index, 1);
    assert_eq!(p.planes[1].block_index, 1);
    assert_eq!(
        p.planes[1].decode_support(),
        DecodeSupport::RequiresPlatformCodec
    );
    assert!(matches!(
        p.planes[1].decode_jpeg(l),
        Err(FormatError::Unsupported)
    ));
    let unknown = json!({"version":{"major":1,"minor":0},"picture":{"accelerationArray":[{"type":"unknown","vendorContent":{"scalar":"1.25"}}]}});
    let bytes = container(&[(true, serde_json::to_vec(&unknown).unwrap())]);
    let c = Container::parse(&bytes, l).unwrap();
    let p = processed_asset(&c, l).unwrap();
    assert!(p.preserved_only);
    assert_eq!(p.original, unknown);
}

#[test]
fn reference_path_amplification_is_bounded() {
    let l = Limits::default();
    let b = container(&[(true, b"{}".to_vec())]);
    let c = Container::parse(&b, l).unwrap();
    let mut object = serde_json::Map::new();
    object.insert("a".repeat(4097), json!({"imageRef":id(b"external")}));
    assert_eq!(
        c.references(&serde_json::Value::Object(object), ReferenceMode::Processed)
            .unwrap_err(),
        FormatError::Budget
    );
    let mut object = serde_json::Map::new();
    object.insert(
        "a".repeat(1800),
        json!(vec![json!({"imageRef":id(b"external")}); 1400]),
    );
    let encoded = serde_json::to_vec(&object).unwrap();
    assert!(encoded.len() < l.json_bytes);
    let decoded = bounded_json(&encoded, l).unwrap();
    assert_eq!(
        c.references(&decoded, ReferenceMode::Processed)
            .unwrap_err(),
        FormatError::Budget
    );
}

#[test]
fn exif_all_eight_corner_patterns_geometry_and_inverse() {
    let limits = Limits::default();
    // Asymmetric six pixels: independent row-major coordinate oracle.
    let expected = [
        vec![1, 2, 3, 4, 5, 6],
        vec![3, 2, 1, 6, 5, 4],
        vec![6, 5, 4, 3, 2, 1],
        vec![4, 5, 6, 1, 2, 3],
        vec![1, 4, 2, 5, 3, 6],
        vec![4, 1, 5, 2, 6, 3],
        vec![6, 3, 5, 2, 4, 1],
        vec![3, 6, 2, 5, 1, 4],
    ];
    for code in 1..=8 {
        let rgba = (1..=6).flat_map(|n| [n, 10 * n, 20 * n, 255]).collect();
        let result = orient_derivative(
            DecodedJpeg {
                geometry: Geometry {
                    width: 3,
                    height: 2,
                },
                rgba,
            },
            code,
            limits,
        )
        .unwrap();
        assert_eq!(
            result.geometry,
            if code >= 5 {
                Geometry {
                    width: 2,
                    height: 3,
                }
            } else {
                Geometry {
                    width: 3,
                    height: 2,
                }
            }
        );
        assert_eq!(
            result
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[0])
                .collect::<Vec<_>>(),
            expected[code as usize - 1]
        );
        assert!(
            result
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|p| p[1] == p[0] * 10 && p[2] == p[0] * 20 && p[3] == 255)
        );
        let inverse = if code == 6 {
            8
        } else if code == 8 {
            6
        } else {
            code
        };
        let original = orient_derivative(result, inverse, limits).unwrap();
        assert_eq!(
            original
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .map(|p| p[0])
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5, 6]
        );
    }
    for code in [0, 9, 255] {
        assert!(
            orient_derivative(
                DecodedJpeg {
                    geometry: Geometry {
                        width: 1,
                        height: 1
                    },
                    rgba: vec![0; 4]
                },
                code,
                limits
            )
            .is_err()
        );
    }
    assert!(
        orient_derivative(
            DecodedJpeg {
                geometry: Geometry {
                    width: 2,
                    height: 1
                },
                rgba: vec![0; 4]
            },
            1,
            limits
        )
        .is_err()
    );
    let limited = Limits {
        decoder_working_bytes: 7,
        ..limits
    };
    assert!(
        orient_derivative(
            DecodedJpeg {
                geometry: Geometry {
                    width: 1,
                    height: 1
                },
                rgba: vec![0; 4]
            },
            2,
            limited
        )
        .is_err()
    );
    let decoded = decode_jpeg(&jpeg(), limits).unwrap();
    let encoded = encode_derivative(&decoded, limits).unwrap();
    assert_eq!(
        decode_jpeg(&encoded, limits).unwrap().geometry,
        decoded.geometry
    );
    assert!(
        encode_derivative(
            &decoded,
            Limits {
                jpeg_bytes: 1,
                ..limits
            }
        )
        .is_err()
    );
}

#[test]
fn encoder_admits_working_memory_before_allocating_and_caps_writes() {
    let limits = Limits::default();
    let image = DecodedJpeg {
        geometry: Geometry {
            width: 1,
            height: 1,
        },
        rgba: vec![240, 20, 20, 255],
    };
    assert_eq!(
        encode_derivative(
            &image,
            Limits {
                decoder_working_bytes: 1,
                ..limits
            }
        )
        .unwrap_err(),
        FormatError::Budget
    );
    assert_eq!(
        encode_derivative(
            &image,
            Limits {
                jpeg_bytes: 1,
                ..limits
            }
        )
        .unwrap_err(),
        FormatError::Budget
    );
    assert_eq!(
        encode_derivative(
            &image,
            Limits {
                jpeg_bytes: 0,
                ..limits
            }
        )
        .unwrap_err(),
        FormatError::Budget
    );
    let bounded = Limits {
        jpeg_bytes: 4096,
        decoder_working_bytes: 64 * 1024,
        ..limits
    };
    let encoded = encode_derivative(&image, bounded).unwrap();
    assert!(encoded.len() <= 4096);
    assert_eq!(
        decode_jpeg(&encoded, bounded).unwrap().geometry,
        image.geometry
    );
    let pointer = image.rgba.as_ptr();
    let unchanged = orient_derivative(
        image,
        1,
        Limits {
            decoder_working_bytes: 4,
            ..limits
        },
    )
    .unwrap();
    assert_eq!(
        unchanged.rgba.as_ptr(),
        pointer,
        "identity does not allocate another pixel frame"
    );
}
#[test]
fn png_preserves_decoded_pixels_and_rejects_extent_allocation_and_output_bombs() {
    let decoded = DecodedJpeg {
        geometry: Geometry {
            width: 3,
            height: 2,
        },
        rgba: vec![
            255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 12, 34, 56, 255, 78, 90, 123, 255, 200,
            201, 202, 255,
        ],
    };
    let bytes = encode_png_derivative(&decoded, Limits::default()).unwrap();
    let image = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png)
        .unwrap()
        .into_rgba8();
    assert_eq!(image.into_raw(), decoded.rgba);
    for limits in [
        Limits {
            jpeg_bytes: 0,
            ..Default::default()
        },
        Limits {
            jpeg_bytes: 1,
            ..Default::default()
        },
        Limits {
            decoder_working_bytes: 1,
            ..Default::default()
        },
        Limits {
            dimension: 1,
            ..Default::default()
        },
    ] {
        assert!(encode_png_derivative(&decoded, limits).is_err());
    }
    let invalid = DecodedJpeg {
        geometry: Geometry {
            width: 3,
            height: 2,
        },
        rgba: vec![],
    };
    assert_eq!(
        encode_png_derivative(&invalid, Default::default()),
        Err(FormatError::Extent)
    );
}
