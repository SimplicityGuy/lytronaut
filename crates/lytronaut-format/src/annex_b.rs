//! Independent, bounded AVC Annex-B inspection for archived AUD-delimited stacks.
//! This is framing/sequence validation, never a native video decoder.
use crate::{FormatError as E, Result};
use std::ops::Range;

pub const VIDEO_BYTES: usize = 8 * 1024 * 1024;
pub const VIDEO_FRAMES: usize = 64;

/// H.264 metadata stays in archived frame order; ordering by lambda would break
/// encoded-frame correspondence. Ties select the lower archived frame index.
pub fn nearest_lambda(lambdas: &[f64], requested: f64) -> Result<usize> {
    validate_lambdas(lambdas)?;
    if !requested.is_finite() {
        return Err(E::NonFinite);
    }
    lambdas
        .iter()
        .enumerate()
        .min_by(|(i, a), (j, b)| {
            let da = (*a - requested).abs();
            let db = (*b - requested).abs();
            // Finite hostile endpoints can overflow subtraction. When both
            // distances overflow they lie opposite the request's sign.
            let order = if da.is_infinite() && db.is_infinite() {
                if requested > 0.0 {
                    b.total_cmp(a)
                } else {
                    a.total_cmp(b)
                }
            } else {
                da.total_cmp(&db)
            };
            order.then(i.cmp(j))
        })
        .map(|(i, _)| i)
        .ok_or(E::Metadata)
}
fn validate_lambdas(lambdas: &[f64]) -> Result<()> {
    if lambdas.is_empty() || lambdas.len() > VIDEO_FRAMES {
        return Err(E::Budget);
    }
    if lambdas.iter().any(|v| !v.is_finite())
        || lambdas
            .iter()
            .enumerate()
            .any(|(i, value)| lambdas[..i].contains(value))
    {
        return Err(E::Metadata);
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sequence {
    pub codec: String,
    pub sps_id: u32,
    pub frame_num_bits: usize,
    pub frame_mbs_only: bool,
    pub crop_left: u32,
    pub crop_top: u32,
    pub coded_width: u32,
    pub coded_height: u32,
    pub visible_width: u32,
    pub visible_height: u32,
}
#[derive(Debug, Clone)]
pub struct AccessUnit {
    pub bytes: Range<usize>,
    pub key: bool,
}
#[derive(Debug)]
pub struct AnnexB {
    pub sequence: Sequence,
    pub units: Vec<AccessUnit>,
}
struct Bits {
    bytes: Vec<u8>,
    position: usize,
}
impl Bits {
    fn new(nal: &[u8]) -> Result<Self> {
        if nal.len() > 64 * 1024 {
            return Err(E::Budget);
        }
        let mut bytes = Vec::with_capacity(nal.len());
        let mut zeros = 0;
        for (i, &b) in nal.iter().enumerate() {
            if zeros == 2 && b == 3 {
                if nal.get(i + 1).is_none_or(|v| *v > 3) {
                    return Err(E::Metadata);
                }
                zeros = 0;
                continue;
            }
            bytes.push(b);
            zeros = if b == 0 { zeros + 1 } else { 0 };
        }
        Ok(Self { bytes, position: 0 })
    }
    fn n(&mut self, n: usize) -> Result<u32> {
        let mut value = 0;
        for _ in 0..n {
            let byte = self.bytes.get(self.position / 8).ok_or(E::Truncated)?;
            value = (value << 1) | u32::from((byte >> (7 - self.position % 8)) & 1);
            self.position += 1;
        }
        Ok(value)
    }
    fn ue(&mut self) -> Result<u32> {
        let mut zeros = 0;
        while self.n(1)? == 0 {
            zeros += 1;
            if zeros > 24 {
                return Err(E::Budget);
            }
        }
        Ok(((1u32 << zeros) - 1) + self.n(zeros)?)
    }
    fn se(&mut self) -> Result<i32> {
        let n = self.ue()?;
        Ok(if n % 2 == 0 {
            -(n as i32 / 2)
        } else {
            (n as i32 + 1) / 2
        })
    }
}
fn sps(nal: &[u8]) -> Result<Sequence> {
    let mut b = Bits::new(nal.get(1..).ok_or(E::Truncated)?)?;
    let profile = b.n(8)?;
    let constraints = b.n(8)?;
    let level = b.n(8)?;
    let sps_id = b.ue()?;
    if constraints & 3 != 0 || sps_id > 31 {
        return Err(E::Metadata);
    }
    let mut chroma = 1;
    if matches!(
        profile,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    ) {
        chroma = b.ue()?;
        // The accepted archived path is 8-bit 4:2:0; other formats stay preserved.
        if chroma != 1 || b.ue()? != 0 || b.ue()? != 0 {
            return Err(E::Unsupported);
        }
        b.n(1)?;
        if b.n(1)? != 0 {
            for i in 0..8 {
                if b.n(1)? != 0 {
                    let mut last = 8;
                    let mut next = 8;
                    for _ in 0..if i < 6 { 16 } else { 64 } {
                        if next != 0 {
                            next = (last + b.se()? + 256).rem_euclid(256);
                        }
                        if next != 0 {
                            last = next;
                        }
                    }
                }
            }
        }
    } else if !matches!(profile, 66 | 77 | 88) {
        return Err(E::Unsupported);
    }
    let log2_frame_num = b.ue()?;
    if log2_frame_num > 12 {
        return Err(E::Metadata);
    }
    match b.ue()? {
        0 => {
            if b.ue()? > 12 {
                return Err(E::Metadata);
            }
        }
        1 => {
            b.n(1)?;
            b.se()?;
            b.se()?;
            let cycle = b.ue()?;
            if cycle > 255 {
                return Err(E::Budget);
            }
            for _ in 0..cycle {
                b.se()?;
            }
        }
        2 => (),
        _ => return Err(E::Metadata),
    }
    if b.ue()? > 16 {
        return Err(E::Budget);
    }
    b.n(1)?;
    let width = b
        .ue()?
        .checked_add(1)
        .and_then(|v| v.checked_mul(16))
        .ok_or(E::Extent)?;
    let map_height = b.ue()?.checked_add(1).ok_or(E::Extent)?;
    let frame = b.n(1)?;
    if frame == 0 {
        b.n(1)?;
    } // mb_adaptive_frame_field_flag
    let height = map_height.checked_mul(16 * (2 - frame)).ok_or(E::Extent)?;
    b.n(1)?;
    let (mut crop_x, mut crop_y, mut crop_left, mut crop_top) = (0, 0, 0, 0);
    if b.n(1)? != 0 {
        crop_left = b.ue()?.checked_mul(2).ok_or(E::Extent)?;
        crop_x = (crop_left / 2)
            .checked_add(b.ue()?)
            .and_then(|v| v.checked_mul(2))
            .ok_or(E::Extent)?;
        crop_top = b.ue()?.checked_mul(2 * (2 - frame)).ok_or(E::Extent)?;
        crop_y = (crop_top / (2 * (2 - frame)))
            .checked_add(b.ue()?)
            .and_then(|v| v.checked_mul(2 * (2 - frame)))
            .ok_or(E::Extent)?;
    }
    let visible_width = width.checked_sub(crop_x).ok_or(E::Extent)?;
    let visible_height = height.checked_sub(crop_y).ok_or(E::Extent)?;
    if chroma != 1 || width > 2048 || height > 2048 || visible_width == 0 || visible_height == 0 {
        return Err(E::Extent);
    }
    Ok(Sequence {
        frame_num_bits: log2_frame_num as usize + 4,
        frame_mbs_only: frame == 1,
        sps_id,
        crop_left,
        crop_top,
        codec: format!("avc1.{profile:02x}{constraints:02x}{level:02x}"),
        coded_width: width,
        coded_height: height,
        visible_width,
        visible_height,
    })
}
fn start(bytes: &[u8], i: usize) -> Option<usize> {
    if bytes.get(i..i + 4) == Some(&[0, 0, 0, 1]) {
        Some(4)
    } else if bytes.get(i..i + 3) == Some(&[0, 0, 1]) {
        Some(3)
    } else {
        None
    }
}
impl AnnexB {
    pub fn parse(bytes: &[u8], lambdas: &[f64], width: u32, height: u32) -> Result<Self> {
        if bytes.is_empty() || bytes.len() > VIDEO_BYTES {
            return Err(E::Budget);
        }
        validate_lambdas(lambdas)?;
        if start(bytes, 0).is_none() {
            return Err(E::Magic);
        }
        let mut position = 0;
        let mut sequence = None;
        let mut sequence_bytes = None;
        let mut pps_bytes = None;
        let mut pps = None;
        let mut units: Vec<AccessUnit> = Vec::new();
        let mut pending_start = 0;
        let mut first_slice = false;
        let mut has_slice = false;
        let mut slice_kind = None;
        while position < bytes.len() {
            let prefix = start(bytes, position).ok_or(E::Magic)?;
            let payload = position + prefix;
            let mut end = payload;
            while end < bytes.len() && start(bytes, end).is_none() {
                end += 1;
            }
            let nal = bytes
                .get(payload..end)
                .filter(|n| !n.is_empty())
                .ok_or(E::Truncated)?;
            if nal[0] & 0x80 != 0 {
                return Err(E::Metadata);
            }
            match nal[0] & 31 {
                7 => {
                    let found = sps(nal)?;
                    if sequence_bytes.is_some_and(|previous| previous != nal) {
                        return Err(E::Metadata);
                    }
                    sequence_bytes = Some(nal);
                    if sequence.as_ref().is_some_and(|s| *s != found) {
                        return Err(E::Metadata);
                    }
                    sequence = Some(found);
                }
                8 => {
                    if pps_bytes.is_some_and(|previous| previous != nal) {
                        return Err(E::Metadata);
                    }
                    pps_bytes = Some(nal);
                    let mut b = Bits::new(&nal[1..])?;
                    let pps_id = b.ue()?;
                    let sps_id = b.ue()?;
                    if pps_id > 255
                        || sequence
                            .as_ref()
                            .is_none_or(|s: &Sequence| s.sps_id != sps_id)
                        || pps.is_some_and(|p| p != pps_id)
                    {
                        return Err(E::Metadata);
                    }
                    pps = Some(pps_id);
                }
                9 => {
                    if !units.is_empty() {
                        if !has_slice {
                            return Err(E::Metadata);
                        }
                        units.last_mut().unwrap().bytes.end = position;
                        pending_start = position;
                    }
                    if units.len() >= VIDEO_FRAMES {
                        return Err(E::Budget);
                    }
                    units.push(AccessUnit {
                        bytes: pending_start..bytes.len(),
                        key: false,
                    });
                    has_slice = false;
                    first_slice = false;
                    slice_kind = None;
                }
                kind @ (1 | 5) => {
                    if sequence.is_none() || pps.is_none() || units.is_empty() {
                        return Err(E::Metadata);
                    }
                    // Multiple slices are valid; a second first slice in an AUD unit is not.
                    let mut b = Bits::new(&nal[1..nal.len().min(64)])?;
                    let first_mb = b.ue()?;
                    if b.ue()? > 9 || Some(b.ue()?) != pps {
                        return Err(E::Metadata);
                    }
                    let sequence = sequence.as_ref().unwrap();
                    b.n(sequence.frame_num_bits)?;
                    if !sequence.frame_mbs_only && b.n(1)? != 0 {
                        return Err(E::Unsupported);
                    } // archived stack requires whole-frame pictures
                    if first_mb == 0 {
                        if first_slice {
                            return Err(E::Metadata);
                        }
                        first_slice = true;
                    } else if !first_slice {
                        return Err(E::Metadata);
                    }
                    if slice_kind.is_some_and(|k| k != kind) {
                        return Err(E::Metadata);
                    }
                    slice_kind = Some(kind);
                    has_slice = true;
                    let u = units.last_mut().unwrap();
                    if kind == 5 {
                        u.key = true;
                    } else if u.key {
                        return Err(E::Metadata);
                    }
                }
                6 | 10 | 11 | 12 => (),
                _ => return Err(E::Unsupported),
            }
            position = end;
        }
        let sequence = sequence.ok_or(E::Metadata)?;
        if !has_slice || units.len() != lambdas.len() || !units[0].key {
            return Err(E::Metadata);
        }
        if sequence.visible_width != width || sequence.visible_height != height {
            return Err(E::Extent);
        }
        Ok(Self { sequence, units })
    }
}
