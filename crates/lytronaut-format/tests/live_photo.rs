//! MIT independent solid RGB vectors. No photographic pixels or vendor code.
use lytronaut_format::{
    DecodedJpeg, FormatError, Geometry,
    live_photo::{Builder, Plan, bundle},
};
const ID: &str = "A8A8A8A8-1111-4444-8888-121212121212";
#[test]
fn admitted_geometry_fits_level_five_frame_size_and_processing_rate() {
    // Independent Annex A geometry oracle, including padding rather than only
    // visible pixels. No image allocation or codec run is needed for admission.
    let mut maximum = 0;
    for width in (2u32..=2048).step_by(2) {
        for height in (2u32..=2048).step_by(2) {
            let geometry = Geometry { width, height };
            let plan = Plan::new(geometry, &[-1.0, 1.0], 0);
            if width * height > 2 * 1024 * 1024 {
                assert!(matches!(plan, Err(FormatError::Unsupported)));
                continue;
            }
            assert!(plan.is_ok(), "admitted geometry {width}x{height}");
            let columns = width.div_ceil(16);
            let rows = height.div_ceil(16);
            let macroblocks = columns * rows;
            maximum = maximum.max(macroblocks);
            assert!(macroblocks <= 22_080, "Level 5 MaxFS");
            assert!(columns * columns <= 8 * 22_080);
            assert!(rows * rows <= 8 * 22_080);
            // Even the stack's maximum 64 samples in a three-second movie
            // stays below MaxMBPS; size admission can only lower this rate.
            assert!(macroblocks * 64 <= 589_824 * 3);
        }
    }
    assert_eq!(maximum, 8352);
    assert_eq!(1138u32.div_ceil(16) * 1842u32.div_ceil(16), maximum);
}
#[test]
fn platform_jpeg_metadata_at_two_million_pixels_keeps_real_decode_admission() {
    let geometry = Geometry {
        width: 2048,
        height: 1024,
    };
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 90)
        .encode(
            &vec![64; 2048 * 1024 * 3],
            geometry.width,
            geometry.height,
            image::ExtendedColorType::Rgb8,
        )
        .unwrap();
    let limits = lytronaut_format::Limits {
        dimension: 2048,
        pixels: 2 * 1024 * 1024,
        decoded_bytes: 8 * 1024 * 1024,
        decoder_working_bytes: 128 * 1024 * 1024,
        jpeg_bytes: 2 * 1024 * 1024,
        ..Default::default()
    };
    assert!(!jpeg.is_empty());
    assert_eq!(
        lytronaut_format::jpeg_metadata_geometry(&jpeg, limits),
        Ok(geometry)
    );
    assert_eq!(
        lytronaut_format::jpeg_geometry(&jpeg, limits),
        Err(FormatError::Budget)
    );
    assert!(matches!(
        lytronaut_format::decode_jpeg(&jpeg, limits),
        Err(FormatError::Budget)
    ));
    for (changed, expected) in [
        (
            lytronaut_format::Limits {
                pixels: limits.pixels - 1,
                ..limits
            },
            FormatError::Budget,
        ),
        (
            lytronaut_format::Limits {
                dimension: 1024,
                ..limits
            },
            FormatError::Extent,
        ),
        (
            lytronaut_format::Limits {
                jpeg_bytes: jpeg.len() - 1,
                ..limits
            },
            FormatError::Budget,
        ),
    ] {
        assert_eq!(
            lytronaut_format::jpeg_metadata_geometry(&jpeg, changed),
            Err(expected)
        );
    }
    assert_eq!(
        lytronaut_format::jpeg_metadata_geometry(
            &jpeg,
            lytronaut_format::Limits {
                decoder_working_bytes: 0,
                ..limits
            }
        ),
        Ok(geometry)
    );
    for malformed in [
        b"not a JPEG".as_slice(),
        &[255, 216, 255, 224, 0, 1],
        &[255, 216, 255, 218],
        &[255, 216, 255, 192, 0, 17, 8],
    ] {
        assert!(lytronaut_format::jpeg_metadata_geometry(malformed, limits).is_err());
    }
    let sof = jpeg.windows(2).position(|v| v == [255, 192]).unwrap();
    assert_eq!(
        lytronaut_format::jpeg_metadata_geometry(&jpeg[..sof + 4], limits),
        Err(FormatError::Truncated)
    );
    let mut builder = Builder::new(Plan::new(geometry, &[-1.0, 1.0], 0).unwrap(), ID).unwrap();
    builder.set_still(&jpeg).unwrap();
    assert!(builder.finish().is_err());
    let mut mismatched = Builder::new(
        Plan::new(
            Geometry {
                width: 1024,
                height: 2048,
            },
            &[-1.0, 1.0],
            0,
        )
        .unwrap(),
        ID,
    )
    .unwrap();
    assert_eq!(mismatched.set_still(&jpeg), Err(FormatError::Extent));
}
fn geometry() -> Geometry {
    Geometry {
        width: 32,
        height: 32,
    }
}
fn image(rgb: [u8; 3]) -> DecodedJpeg {
    DecodedJpeg {
        geometry: geometry(),
        rgba: (0..1024)
            .flat_map(|_| [rgb[0], rgb[1], rgb[2], 255])
            .collect(),
    }
}
#[test]
fn selected_key_order_dimensions_identifier_cancel_and_complete_pair() {
    let plan = Plan::new(geometry(), &[0.0, 3.0, -2.0], 0).unwrap();
    assert_eq!(plan.planes(), vec![2, 0, 1]);
    assert_eq!(plan.geometry(), geometry());
    assert_eq!(plan.selected(), 0);
    assert_eq!(plan.key_seconds(), 1.0);
    assert!(Builder::new(plan.clone(), "bad identifier").is_err());
    let mut b = Builder::new(plan.clone(), ID).unwrap();
    assert_eq!(
        b.push(0, &image([255, 0, 0]), || true),
        Err(FormatError::Codec)
    );
    assert!(b.push(63, &image([255, 0, 0]), || false).is_err());
    for (i, rgb) in [(0, [255, 0, 0]), (1, [0, 255, 0]), (2, [0, 0, 255])] {
        b.push(i, &image(rgb), || false).unwrap();
    }
    assert!(b.push(0, &image([255, 0, 0]), || false).is_err());
    let pair = b.finish().unwrap();
    assert!(pair.jpeg.windows(ID.len()).any(|v| v == ID.as_bytes()));
    assert!(pair.movie.windows(ID.len()).any(|v| v == ID.as_bytes()));
    assert!(pair.movie.windows(5).any(|v| v == [0, 0, 0, 9, 0]));
    assert!(
        pair.movie
            .windows(b"com.apple.quicktime.still-image-time".len())
            .any(|v| v == b"com.apple.quicktime.still-image-time")
    );
    // Storage remains in archived order; track chunk offsets select lambda order.
    let mut chunks = vec![];
    let mut offset = 28;
    for _ in 0..3 {
        chunks.push(offset as u32);
        let n = u32::from_be_bytes(pair.movie[offset..offset + 4].try_into().unwrap()) as usize;
        assert_eq!(pair.movie[offset + 4], 0x65);
        offset += 4 + n;
    }
    let stco = pair.movie.windows(4).position(|v| v == b"stco").unwrap();
    assert_eq!(
        u32::from_be_bytes(pair.movie[stco + 8..stco + 12].try_into().unwrap()),
        3
    );
    let offsets: Vec<_> = pair.movie[stco + 12..stco + 24]
        .as_chunks::<4>()
        .0
        .iter()
        .map(|v| u32::from_be_bytes(*v))
        .collect();
    assert_eq!(offsets, [chunks[2], chunks[0], chunks[1]]);
    let allocation = pair.movie.as_ptr();
    let mut provenance = br#"{"motion":"generated_focus_animation"}"#.to_vec();
    provenance.resize(16 * 1024, b' ');
    let zip = bundle(pair, &provenance).unwrap();
    assert_eq!(
        zip.as_ptr(),
        allocation,
        "final movie allocation includes ZIP slack"
    );
    assert!(zip.starts_with(b"PK\x03\x04"));
    for name in [b"focus.MOV".as_slice(), b"focus.JPG", b"provenance.json"] {
        assert!(zip.windows(name.len()).any(|v| v == name));
    }
    assert_eq!(&zip[zip.len() - 22..zip.len() - 18], b"PK\x05\x06");
}
#[test]
fn cropped_even_pixels_and_invalid_rgba_are_bounded() {
    let geometry = Geometry {
        width: 30,
        height: 18,
    };
    let plan = Plan::new(geometry, &[-1.0, 1.0], 1).unwrap();
    let mut builder = Builder::new(plan, ID).unwrap();
    let mut image = DecodedJpeg {
        geometry,
        rgba: vec![0; 30 * 18 * 4 - 1],
    };
    assert_eq!(builder.push(0, &image, || false), Err(FormatError::Extent));
    image.rgba = (0..30 * 18).flat_map(|_| [64, 128, 192, 255]).collect();
    builder.push(0, &image, || false).unwrap();
    builder.push(1, &image, || false).unwrap();
    let pair = builder.finish().unwrap();
    assert_eq!(
        lytronaut_format::jpeg_geometry(&pair.jpeg, Default::default()).unwrap(),
        geometry
    );
    assert!(pair.movie.len() < lytronaut_format::live_photo::MOVIE_CAP);
    assert_eq!(pair.plan.key_seconds(), 1.5);
}
#[test]
fn hostile_plan_and_incomplete_resources_fail_before_publication() {
    for lambdas in [
        vec![],
        vec![0.0],
        vec![0.0, 0.0],
        vec![0.0, f64::NAN],
        vec![0.0, f64::INFINITY],
        vec![0.0; 65],
    ] {
        assert!(Plan::new(geometry(), &lambdas, 0).is_err());
    }
    assert!(Plan::new(geometry(), &[0.0, 1.0], 2).is_err());
    for (w, h) in [(0, 32), (31, 32), (32, 31), (2050, 32), (2048, 2048)] {
        assert!(
            Plan::new(
                Geometry {
                    width: w,
                    height: h
                },
                &[0.0, 1.0],
                0
            )
            .is_err()
        );
    }
    let plan = Plan::new(geometry(), &[0.0, 1.0], 0).unwrap();
    assert!(Builder::new(plan.clone(), ID).unwrap().finish().is_err());
    let mut b = Builder::new(plan, ID).unwrap();
    b.push(0, &image([0, 0, 0]), || false).unwrap();
    assert!(b.finish().is_err());
}
#[test]
fn sweep_is_bounded_and_preserves_key_and_extrema() {
    let values: Vec<_> = (0..64).map(f64::from).collect();
    for selected in [0, 31, 63] {
        let p = Plan::new(
            Geometry {
                width: 2048,
                height: 1024,
            },
            &values,
            selected,
        )
        .unwrap();
        assert!(p.planes().len() < 64);
        assert!(p.planes().contains(&selected));
        assert_eq!(p.planes().first(), Some(&0));
        assert_eq!(p.planes().last(), Some(&63));
        assert!(p.planes().windows(2).all(|v| v[0] < v[1]));
        assert!(p.key_seconds() < 3.0);
    }
}
#[test]
fn interrupted_picture_does_not_advance_or_leave_partial_movie() {
    let plan = Plan::new(geometry(), &[-1.0, 1.0], 0).unwrap();
    let mut builder = Builder::new(plan, ID).unwrap();
    let probes = std::cell::Cell::new(0);
    assert_eq!(
        builder.push(0, &image([255, 0, 0]), || {
            probes.set(probes.get() + 1);
            probes.get() == 3
        }),
        Err(FormatError::Codec)
    );
    assert_eq!(builder.completed_planes(), 0);
    assert!(builder.wants(0));
    assert!(!builder.wants(63));
    builder.push(0, &image([255, 0, 0]), || false).unwrap();
    builder.push(1, &image([0, 0, 255]), || false).unwrap();
    let pair = builder.finish().unwrap();
    assert_eq!(pair.plan.planes(), &[0, 1]);
    assert_eq!(
        bundle(pair, &vec![0; 16 * 1024 + 1]),
        Err(FormatError::Budget)
    );
}
/// Independent reader: the slice header's idr_pic_id of each movie sample, in
/// decode (sample-table) order. Parses only this encoder's fixed IDR header:
/// first_mb, slice_type, pps_id, frame_num u(4), idr_pic_id.
fn decode_order_idr_pic_ids(movie: &[u8]) -> Vec<u32> {
    let stco = movie.windows(4).position(|v| v == b"stco").unwrap();
    let count = u32::from_be_bytes(movie[stco + 8..stco + 12].try_into().unwrap()) as usize;
    (0..count)
        .map(|i| {
            let at = stco + 12 + 4 * i;
            let offset = u32::from_be_bytes(movie[at..at + 4].try_into().unwrap()) as usize;
            let length = u32::from_be_bytes(movie[offset..offset + 4].try_into().unwrap()) as usize;
            assert_eq!(movie[offset + 4] & 0x1f, 5, "every sample is an IDR slice");
            // Remove emulation-prevention bytes from the RBSP prefix.
            let mut rbsp = Vec::new();
            for &byte in &movie[offset + 5..offset + 4 + length.min(64)] {
                if byte == 3 && rbsp.ends_with(&[0, 0]) {
                    continue;
                }
                rbsp.push(byte);
            }
            let mut bit = 0usize;
            let mut read = |n: usize| -> u32 {
                (0..n).fold(0, |v, _| {
                    let b = (rbsp[bit / 8] >> (7 - bit % 8)) & 1;
                    bit += 1;
                    (v << 1) | u32::from(b)
                })
            };
            let ue = |read: &mut dyn FnMut(usize) -> u32| {
                let mut zeros = 0;
                while read(1) == 0 {
                    zeros += 1;
                }
                (1u32 << zeros) - 1 + read(zeros)
            };
            assert_eq!(ue(&mut read), 0, "first_mb_in_slice");
            assert_eq!(ue(&mut read) % 5, 2, "I slice");
            assert_eq!(ue(&mut read), 0, "pps id");
            assert_eq!(read(4), 0, "IDR frame_num");
            ue(&mut read)
        })
        .collect()
}
#[test]
fn consecutive_idr_pictures_have_distinct_idr_pic_id() {
    // Ascending lambda order [-2, 0, 3] plays planes 2, 0, 1: two consecutive
    // even source planes, which plane parity would give the same identifier.
    for (lambdas, selected) in [
        (vec![0.0, 3.0, -2.0], 0),
        (vec![0.0, 2.0, 4.0, 6.0, -2.0], 2),
        (vec![1.0, 0.0], 1),
    ] {
        let plan = Plan::new(geometry(), &lambdas, selected).unwrap();
        let planes = plan.planes().to_vec();
        let mut b = Builder::new(plan, ID).unwrap();
        // Arrival order is archive order, not playback order.
        let mut arrival = planes.clone();
        arrival.sort_unstable();
        for i in arrival {
            b.push(i, &image([(i * 40) as u8, 0, 0]), || false).unwrap();
        }
        let ids = decode_order_idr_pic_ids(&b.finish().unwrap().movie);
        assert_eq!(ids.len(), planes.len());
        assert!(
            ids.windows(2).all(|pair| pair[0] != pair[1]),
            "planes {planes:?} idr_pic_id {ids:?}"
        );
    }
}
