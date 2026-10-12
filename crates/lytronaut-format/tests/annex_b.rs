//! Independently authored MIT syntax fixtures; no vendor bitstream/pixels.
use lytronaut_format::annex_b::nearest_lambda;
use lytronaut_format::{FormatError, annex_b::AnnexB};
fn bits(values: &[(u32, usize)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut position = 0;
    for &(value, count) in values {
        for i in (0..count).rev() {
            if position % 8 == 0 {
                out.push(0);
            }
            let last = out.last_mut().unwrap();
            *last |= (((value >> i) & 1) as u8) << (7 - position % 8);
            position += 1;
        }
    }
    out
}
fn stream() -> Vec<u8> {
    // Baseline progressive 32x32, horizontal/vertical crop of 2 pixels.
    let mut out = vec![0, 0, 0, 1, 9, 0xf0, 0, 0, 1, 0x67];
    out.extend(bits(&[
        (66, 8),
        (0, 8),
        (30, 8),
        (1, 1),
        (1, 1),
        (1, 1),
        (1, 1),
        (2, 3),
        (0, 1),
        (2, 3),
        (2, 3),
        (1, 1),
        (1, 1),
        (1, 1),
        (1, 1),
        (2, 3),
        (1, 1),
        (2, 3),
        (1, 1),
    ]));
    out.extend([0, 0, 1, 0x68, 0xe0, 0, 0, 1, 0x65, 0xb8, 0]);
    out
}
#[test]
fn codec_dimensions_crop_and_idr_are_derived() {
    let b = stream();
    let parsed = AnnexB::parse(&b, &[0.0], 30, 30).unwrap();
    assert_eq!(parsed.sequence.codec, "avc1.42001e");
    assert_eq!(
        (parsed.sequence.coded_width, parsed.sequence.coded_height),
        (32, 32)
    );
    assert_eq!(
        (
            parsed.sequence.visible_width,
            parsed.sequence.visible_height
        ),
        (30, 30)
    );
    assert_eq!(&b[parsed.units[0].bytes.clone()], b.as_slice());
    assert!(parsed.units[0].key);
}
#[test]
fn missing_parameters_delta_start_cardinality_and_geometry_fail_closed() {
    let b = stream();
    assert!(AnnexB::parse(&b, &[0.0, 1.0], 30, 30).is_err());
    assert!(AnnexB::parse(&b, &[0.0], 32, 32).is_err());
    let mut delta = b.clone();
    let offset = delta.iter().rposition(|v| *v == 0x65).unwrap();
    delta[offset] = 0x61;
    assert!(AnnexB::parse(&delta, &[0.0], 30, 30).is_err());
    let mut duplicate = b.clone();
    duplicate.extend([0, 0, 1, 0x65, 0xb8, 0]);
    assert!(AnnexB::parse(&duplicate, &[0.0], 30, 30).is_err());
    assert!(AnnexB::parse(&b, &[f64::NAN], 30, 30).is_err());
    assert!(AnnexB::parse(&b, &[f64::INFINITY], 30, 30).is_err());
}
#[test]
fn aud_ranges_preserve_parameters_and_accept_mixed_start_codes() {
    let mut b = stream();
    let first = b.len();
    b.extend([0, 0, 0, 1, 9, 0xf0, 0, 0, 1, 0x61, 0xe0]);
    let parsed = AnnexB::parse(&b, &[-1.0, 1.0], 30, 30).unwrap();
    assert_eq!(parsed.units[0].bytes, 0..first);
    assert_eq!(parsed.units[1].bytes, first..b.len());
    assert!(!parsed.units[1].key);
    assert!(AnnexB::parse(&b, &[0.0, 0.0], 30, 30).is_err());
}
#[test]
fn malformed_prefixes_exp_golomb_and_parameter_changes_are_bounded() {
    let b = stream();
    for n in 0..b.len() - 1 {
        assert!(AnnexB::parse(&b[..n], &[0.0], 30, 30).is_err());
    }
    assert_eq!(
        AnnexB::parse(&vec![0; 8 * 1024 * 1024 + 1], &[0.0], 30, 30).unwrap_err(),
        FormatError::Budget
    );
    let mut forbidden = b.clone();
    forbidden[4] |= 0x80;
    assert!(AnnexB::parse(&forbidden, &[0.0], 30, 30).is_err());
    // Deterministic adversarial mutations must never panic, even when accepted syntax changes.
    for i in 0..b.len() {
        for mask in [1, 0x80, 0xff] {
            let mut m = b.clone();
            m[i] ^= mask;
            let _ = AnnexB::parse(&m, &[0.0], 30, 30);
        }
    }
}
#[test]
fn parameter_ids_and_mixed_slice_types_cannot_be_misclassified() {
    let b = stream();
    let mut mismatched = b.clone();
    let pps = mismatched.iter().position(|v| *v == 0x68).unwrap();
    mismatched[pps + 1] = 0xa0;
    assert!(AnnexB::parse(&mismatched, &[0.0], 30, 30).is_err());
    let mut mismatched = b.clone();
    let slice = mismatched.iter().position(|v| *v == 0x65).unwrap();
    mismatched[slice + 1] = 0xb4;
    assert!(AnnexB::parse(&mismatched, &[0.0], 30, 30).is_err());
    let mut idr_then_delta = b.clone();
    idr_then_delta.extend([0, 0, 1, 0x61, 0x58]);
    assert!(AnnexB::parse(&idr_then_delta, &[0.0], 30, 30).is_err());
    let mut delta_then_idr = b.clone();
    delta_then_idr[slice] = 0x61;
    delta_then_idr[slice + 1] = 0xe0;
    delta_then_idr.extend([0, 0, 1, 0x65, 0x4e]);
    assert!(AnnexB::parse(&delta_then_idr, &[0.0], 30, 30).is_err());
    let mut changed_pps = b;
    changed_pps.extend([0, 0, 1, 0x68, 0xf0]);
    assert!(AnnexB::parse(&changed_pps, &[0.0], 30, 30).is_err());
}
#[test]
fn nonmonotonic_lambdas_preserve_archived_index_and_nearest_ties() {
    assert_eq!(
        nearest_lambda(&[0.0, 3.0, -2.0, 1.0, 2.0], -2.0).unwrap(),
        2
    );
    assert_eq!(nearest_lambda(&[0.0, 3.0, -2.0, 1.0, 2.0], 0.5).unwrap(), 0);
    assert_eq!(
        nearest_lambda(&[0.0, 3.0, -2.0, 1.0, 2.0], 100.0).unwrap(),
        1
    );
    assert!(nearest_lambda(&[0.0, 0.0], 1.0).is_err());
    assert!(nearest_lambda(&[0.0, f64::NAN], 1.0).is_err());
    assert!(nearest_lambda(&[0.0], f64::INFINITY).is_err());
    assert_eq!(
        nearest_lambda(&[-f64::MAX, -f64::MAX / 2.0], f64::MAX).unwrap(),
        1
    );
    assert_eq!(
        nearest_lambda(&[f64::MAX, f64::MAX / 2.0], -f64::MAX).unwrap(),
        1
    );
    let mut b = stream();
    b.extend([0, 0, 0, 1, 9, 0xf0, 0, 0, 1, 0x61, 0xe0]);
    let parsed = AnnexB::parse(&b, &[1.0, -1.0], 30, 30).unwrap();
    assert_eq!(
        parsed.units[nearest_lambda(&[1.0, -1.0], -1.0).unwrap()]
            .bytes
            .end,
        b.len()
    );
}
#[test]
fn mbaff_whole_frames_use_map_height_and_vertical_crop_units() {
    let mut b = vec![0, 0, 0, 1, 9, 0xf0, 0, 0, 1, 0x67];
    b.extend(bits(&[
        (100, 8),
        (0, 8),
        (41, 8),
        (1, 1),
        (2, 3),
        (1, 1),
        (1, 1),
        (0, 1),
        (0, 1),
        (1, 1),
        (1, 1),
        (1, 1),
        (2, 3),
        (0, 1),
        (2, 3),
        (2, 3),
        (0, 1),
        (1, 1),
        (1, 1),
        (1, 1),
        (2, 3),
        (1, 1),
        (2, 3),
        (1, 1),
        (1, 1),
    ]));
    b.extend([0, 0, 1, 0x68, 0xe0, 0, 0, 1, 0x65, 0xb8, 0]);
    let parsed = AnnexB::parse(&b, &[0.0], 30, 60).unwrap();
    assert!(!parsed.sequence.frame_mbs_only);
    assert_eq!(
        (parsed.sequence.coded_width, parsed.sequence.coded_height),
        (32, 64)
    );
    assert_eq!(
        (parsed.sequence.crop_left, parsed.sequence.crop_top),
        (2, 4)
    );
    let end = b.len() - 1;
    b[end] = 0x40;
    assert_eq!(
        AnnexB::parse(&b, &[0.0], 30, 60).unwrap_err(),
        FormatError::Unsupported
    );
}
#[test]
fn nearest_matches_integer_oracle_for_permutations_and_outside_requests() {
    for a in -3i32..=3 {
        for b in -3..=3 {
            for c in -3..=3 {
                if a == b || a == c || b == c {
                    continue;
                }
                let values = [a, b, c];
                for requested in -8..=8 {
                    let expected = values
                        .iter()
                        .enumerate()
                        .min_by_key(|(i, v)| ((**v - requested).abs(), *i))
                        .unwrap()
                        .0;
                    assert_eq!(
                        nearest_lambda(&values.map(f64::from), f64::from(requested)).unwrap(),
                        expected
                    );
                }
            }
        }
    }
}
#[test]
fn bounded_parameter_fuzz_and_empty_access_units_never_panic() {
    let mut seed = 0x43a9_1057u32;
    for len in 0..128 {
        for _ in 0..8 {
            let mut bytes = vec![0, 0, 0, 1, 9, 0xf0, 0, 0, 1, 0x67];
            for _ in 0..len {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                bytes.push(seed as u8);
            }
            bytes.extend([0, 0, 1, 0x68, 0xe0, 0, 0, 1, 0x65, 0xb8, 0]);
            let _ = AnnexB::parse(&bytes, &[0.0], 30, 30);
        }
    }
    let mut b = stream();
    b.extend([0, 0, 1, 9, 0xf0]);
    assert!(AnnexB::parse(&b, &[0.0, 1.0], 30, 30).is_err());
    let mut oversized = vec![0, 0, 1, 9, 0xf0, 0, 0, 1, 0x67];
    oversized.extend(vec![0x55; 65 * 1024]);
    assert_eq!(
        AnnexB::parse(&oversized, &[0.0], 30, 30).unwrap_err(),
        FormatError::Budget
    );
}
