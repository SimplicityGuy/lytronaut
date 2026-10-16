//! Independently authored MIT H.264 syntax fixtures for the Annex-B inspector's
//! rejection paths: every hostile parameter set must fail closed with a stable
//! error, and the accepted High-profile scaling-list syntax must parse exactly
//! like the equivalent baseline stream. No vendor bitstream or pixels.
use lytronaut_format::{FormatError, annex_b::AnnexB};

type Bits = Vec<(u32, usize)>;

fn pack(values: &[(u32, usize)]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut position = 0;
    for &(value, count) in values {
        for i in (0..count).rev() {
            if position % 8 == 0 {
                out.push(0);
            }
            *out.last_mut().unwrap() |= (((value >> i) & 1) as u8) << (7 - position % 8);
            position += 1;
        }
    }
    out
}
fn ue(value: u32) -> Bits {
    let n = value + 1;
    let len = (32 - n.leading_zeros()) as usize;
    vec![(0, len - 1), (n, len)]
}
fn se(value: i32) -> Bits {
    ue(if value > 0 {
        (2 * value - 1) as u32
    } else {
        (-2 * value) as u32
    })
}
/// Sequence parameters: profile, optional High-profile extension, frame_num +
/// picture-order syntax, reference frames and width in macroblocks minus one.
/// Height is two macroblocks; a 2-pixel right/bottom crop leaves 30x30 at width 1.
struct Sps {
    profile: u32,
    high: Bits,
    order: Bits,
    references: u32,
    width_mbs_minus1: u32,
}
impl Default for Sps {
    fn default() -> Self {
        Self {
            profile: 66,
            high: vec![],
            order: [ue(0), ue(0), ue(0)].concat(),
            references: 1,
            width_mbs_minus1: 1,
        }
    }
}
impl Sps {
    fn nal(&self) -> Vec<u8> {
        let mut bits = vec![(self.profile, 8), (0, 8), (30, 8)];
        bits.extend(ue(0));
        bits.extend(self.high.iter().copied());
        bits.extend(self.order.iter().copied());
        bits.extend(ue(self.references));
        bits.push((0, 1));
        bits.extend(ue(self.width_mbs_minus1));
        bits.extend(ue(1));
        bits.extend([(1, 1), (1, 1), (1, 1)]);
        bits.extend([ue(0), ue(1), ue(0), ue(1)].concat());
        bits.push((1, 1));
        let mut nal = vec![0x67];
        nal.extend(pack(&bits));
        nal
    }
}
fn slice(kind: u8, first_mb: u32, slice_type: u32) -> Vec<u8> {
    let mut nal = vec![0, 0, 1, kind];
    nal.extend(pack(
        &[ue(first_mb), ue(slice_type), ue(0), vec![(0, 4), (1, 1)]].concat(),
    ));
    nal
}
const AUD: [u8; 6] = [0, 0, 0, 1, 9, 0xf0];
const PPS: [u8; 5] = [0, 0, 1, 0x68, 0xe0];
fn stream(sps: &[u8]) -> Vec<u8> {
    let mut out = AUD.to_vec();
    out.extend([0, 0, 1]);
    out.extend(sps);
    out.extend(PPS);
    out.extend(slice(0x65, 0, 2));
    out
}
fn parse(bytes: &[u8]) -> Result<AnnexB, FormatError> {
    AnnexB::parse(bytes, &[0.0], 30, 30)
}

#[test]
fn high_profile_scaling_lists_parse_like_the_baseline_stream() {
    // chroma 4:2:0, 8-bit luma/chroma, no lossless bypass, scaling matrix present:
    // list 0 carries 16 explicit zero deltas, list 1 jumps to "use default"
    // (next == 0) after one delta, lists 2..7 are absent.
    let mut high = [ue(1), ue(0), ue(0), vec![(0, 1), (1, 1)]].concat();
    high.push((1, 1));
    for _ in 0..16 {
        high.extend(se(0));
    }
    high.push((1, 1));
    high.extend(se(-8));
    high.extend(std::iter::repeat_n((0, 1), 6));
    let parsed = parse(&stream(
        &Sps {
            profile: 100,
            high,
            ..Sps::default()
        }
        .nal(),
    ))
    .unwrap();
    let baseline = parse(&stream(&Sps::default().nal())).unwrap();
    assert_eq!(parsed.sequence.codec, "avc1.64001e");
    assert_eq!(baseline.sequence.codec, "avc1.42001e");
    assert_eq!(
        (
            parsed.sequence.visible_width,
            parsed.sequence.visible_height,
            parsed.sequence.reference_frames
        ),
        (30, 30, 1)
    );
    assert_eq!(parsed.units.len(), baseline.units.len());
    assert!(parsed.units[0].key);
}

#[test]
fn unsupported_profiles_and_chroma_formats_stay_preserved_only() {
    let chroma422 = Sps {
        profile: 100,
        high: [ue(2), ue(0), ue(0)].concat(),
        ..Sps::default()
    };
    assert_eq!(
        parse(&stream(&chroma422.nal())).unwrap_err(),
        FormatError::Unsupported
    );
    let unknown = Sps {
        profile: 99,
        ..Sps::default()
    };
    assert_eq!(
        parse(&stream(&unknown.nal())).unwrap_err(),
        FormatError::Unsupported
    );
}

#[test]
fn picture_order_and_reference_budgets_fail_closed() {
    // Type 1 picture order with a small cycle is accepted.
    let cycle = |n: u32| {
        let mut order = [ue(0), ue(1), vec![(0, 1)], se(1), se(-1), ue(n)].concat();
        for _ in 0..n.min(2) {
            order.extend(se(2));
        }
        order
    };
    assert!(
        parse(&stream(
            &Sps {
                order: cycle(2),
                ..Sps::default()
            }
            .nal()
        ))
        .is_ok()
    );
    // A cycle above 255 is rejected before reading its offsets.
    assert_eq!(
        parse(&stream(
            &Sps {
                order: cycle(256),
                ..Sps::default()
            }
            .nal()
        ))
        .unwrap_err(),
        FormatError::Budget
    );
    // Type 2 has no extra syntax; types above 2 do not exist.
    assert!(
        parse(&stream(
            &Sps {
                order: [ue(0), ue(2)].concat(),
                ..Sps::default()
            }
            .nal()
        ))
        .is_ok()
    );
    for order in [
        [ue(0), ue(3)].concat(),
        [ue(13), ue(0), ue(0)].concat(),
        [ue(0), ue(0), ue(13)].concat(),
    ] {
        assert_eq!(
            parse(&stream(
                &Sps {
                    order,
                    ..Sps::default()
                }
                .nal()
            ))
            .unwrap_err(),
            FormatError::Metadata
        );
    }
    assert_eq!(
        parse(&stream(
            &Sps {
                references: 17,
                ..Sps::default()
            }
            .nal()
        ))
        .unwrap_err(),
        FormatError::Budget
    );
    // 201 macroblocks wide exceeds the 2048-pixel archived geometry bound.
    assert_eq!(
        parse(&stream(
            &Sps {
                width_mbs_minus1: 200,
                ..Sps::default()
            }
            .nal()
        ))
        .unwrap_err(),
        FormatError::Extent
    );
}

#[test]
fn emulation_prevention_and_exp_golomb_lengths_are_validated() {
    // 00 00 03 must be followed by a byte <= 3.
    assert_eq!(
        parse(&stream(&[0x67, 66, 0, 0, 3, 4, 0x80])).unwrap_err(),
        FormatError::Metadata
    );
    // An Exp-Golomb code with more than 24 leading zeros is a budget failure.
    assert_eq!(
        parse(&stream(&[0x67, 66, 0, 30, 0, 0, 0, 0, 0x80])).unwrap_err(),
        FormatError::Budget
    );
}

#[test]
fn parameter_sets_and_slices_cannot_change_mid_stream() {
    let sps = Sps::default().nal();
    let mut changed = stream(&sps);
    let mut other = sps.clone();
    other[3] = 31; // different level, same syntax
    changed.extend([0, 0, 1]);
    changed.extend(other);
    assert_eq!(parse(&changed).unwrap_err(), FormatError::Metadata);

    // slice_type above 9 does not exist.
    let mut bad_type = AUD.to_vec();
    bad_type.extend([0, 0, 1]);
    bad_type.extend(&sps);
    bad_type.extend(PPS);
    bad_type.extend(slice(0x65, 0, 10));
    assert_eq!(parse(&bad_type).unwrap_err(), FormatError::Metadata);

    // A continuation slice without a first slice in its access unit.
    let mut orphan = AUD.to_vec();
    orphan.extend([0, 0, 1]);
    orphan.extend(&sps);
    orphan.extend(PPS);
    orphan.extend(slice(0x65, 1, 2));
    assert_eq!(parse(&orphan).unwrap_err(), FormatError::Metadata);

    // Mixed IDR and non-IDR slices in one access unit.
    let mut mixed = stream(&sps);
    mixed.extend(slice(0x61, 1, 0));
    assert_eq!(parse(&mixed).unwrap_err(), FormatError::Metadata);

    // B slices mark the unit bidirectional; a second access unit is accepted.
    let mut bidirectional = stream(&sps);
    bidirectional.extend(AUD);
    bidirectional.extend(slice(0x61, 0, 1));
    let parsed = AnnexB::parse(&bidirectional, &[0.0, 1.0], 30, 30).unwrap();
    assert!(!parsed.units[0].bidirectional);
    assert!(parsed.units[1].bidirectional);
}

#[test]
fn access_unit_and_lambda_counts_are_bounded() {
    let sps = Sps::default().nal();
    assert_eq!(
        AnnexB::parse(&stream(&sps), &[], 30, 30).unwrap_err(),
        FormatError::Budget
    );
    let lambdas: Vec<f64> = (0..65).map(f64::from).collect();
    assert_eq!(
        AnnexB::parse(&stream(&sps), &lambdas, 30, 30).unwrap_err(),
        FormatError::Budget
    );
    // 65 access units while only 64 lambdas are allowed.
    let mut long = stream(&sps);
    for _ in 0..64 {
        long.extend(AUD);
        long.extend(slice(0x61, 0, 0));
    }
    assert_eq!(
        AnnexB::parse(&long, &lambdas[..64], 30, 30).unwrap_err(),
        FormatError::Budget
    );
}
