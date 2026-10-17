//! Independent H.264 Baseline I_PCM writer: ITU-T H.264 (08/2024),
//! sections 7.3.2.1, 7.3.2.2, 7.3.3, 7.3.5 and Annex E.
//! No prediction, transform, entropy-code implementation or external runtime.
use crate::{DecodedJpeg, FormatError as E, Result};
#[derive(Default)]
struct Bits {
    bytes: Vec<u8>,
    position: usize,
}
impl Bits {
    fn n(&mut self, value: u32, count: usize) {
        for bit in (0..count).rev() {
            if self.position.is_multiple_of(8) {
                self.bytes.push(0);
            }
            let last = self.bytes.len() - 1;
            self.bytes[last] |= (((value >> bit) & 1) as u8) << (7 - self.position % 8);
            self.position += 1;
        }
    }
    fn ue(&mut self, value: u32) {
        let value = value + 1;
        let count = 32 - value.leading_zeros();
        self.n(0, (count - 1) as usize);
        self.n(value, count as usize);
    }
    fn zero_se(&mut self) {
        self.ue(0);
    }
    fn align(&mut self) {
        while !self.position.is_multiple_of(8) {
            self.n(0, 1);
        }
    }
    fn finish(mut self, header: u8) -> Vec<u8> {
        self.n(1, 1);
        self.align();
        let mut output = Vec::with_capacity(self.bytes.len() * 3 / 2 + 1);
        #[cfg(feature = "test-harness")]
        if header == 0x65 {
            crate::capacity_observations::record(
                "pcm_escape",
                [self.bytes.capacity(), output.capacity(), 0, 0],
            );
        }
        output.push(header);
        let mut zeros = 0;
        for byte in self.bytes {
            if zeros == 2 && byte <= 3 {
                output.push(3);
                zeros = 0;
            }
            output.push(byte);
            zeros = if byte == 0 { zeros + 1 } else { 0 };
        }
        output
    }
}
pub(super) fn configuration(width: u32, height: u32) -> (Vec<u8>, Vec<u8>) {
    let mut s = Bits::default();
    s.n(66, 8);
    s.n(0xc0, 8);
    s.n(50, 8); // Baseline Level 5; movie byte cap alone is not an HRD proof.
    s.ue(0);
    s.ue(0);
    s.ue(2);
    s.ue(0);
    s.n(0, 1);
    s.ue(width.div_ceil(16) - 1);
    s.ue(height.div_ceil(16) - 1);
    s.n(1, 1);
    s.n(1, 1);
    let crop_x = (width.div_ceil(16) * 16 - width) / 2;
    let crop_y = (height.div_ceil(16) * 16 - height) / 2;
    s.n(u32::from(crop_x != 0 || crop_y != 0), 1);
    if crop_x != 0 || crop_y != 0 {
        s.ue(0);
        s.ue(crop_x);
        s.ue(0);
        s.ue(crop_y);
    }
    s.n(1, 1); // VUI: square pixels, explicitly declared numeric JPEG color policy.
    s.n(1, 1);
    s.n(1, 8);
    s.n(0, 1);
    s.n(1, 1);
    s.n(5, 3);
    s.n(0, 1);
    s.n(1, 1);
    s.n(1, 8);
    s.n(13, 8);
    s.n(6, 8); // BT.709 primaries, sRGB transfer, BT.601 matrix.
    s.n(0, 1);
    s.n(0, 1);
    s.n(0, 1);
    s.n(0, 1);
    s.n(0, 1);
    // E.2.1 infers max_bytes_per_pic_denom=2 when restrictions are absent.
    // I_PCM plus headers exceeds even uncompressed picture bytes; signal the
    // permitted unrestricted picture size rather than that false default.
    s.n(1, 1); // bitstream_restriction_flag
    s.n(0, 1); // No inter prediction, including across picture boundaries.
    s.ue(0); // max_bytes_per_pic_denom: no per-picture VUI byte limit.
    s.ue(1); // max_bits_per_mb_denom: <=3200 bits; PCM needs <=3088.
    s.ue(0); // No horizontal motion vectors.
    s.ue(0); // No vertical motion vectors.
    s.ue(0); // No reordered output pictures.
    s.ue(1); // One decoded frame; no inter-picture prediction.
    let mut p = Bits::default();
    p.ue(0);
    p.ue(0);
    p.n(0, 1);
    p.n(0, 1);
    p.ue(0);
    p.ue(0);
    p.ue(0);
    p.n(0, 1);
    p.n(0, 2);
    p.zero_se();
    p.zero_se();
    p.zero_se();
    p.n(1, 1);
    p.n(0, 1);
    p.n(0, 1);
    (s.finish(0x67), p.finish(0x68))
}
fn yuv(r: u8, g: u8, b: u8) -> [u8; 3] {
    let (r, g, b) = (i32::from(r), i32::from(g), i32::from(b));
    [
        ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16,
        ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128,
        ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128,
    ]
    .map(|v| v.clamp(0, 255) as u8)
}
/// `idr_pic_id` must differ between consecutive IDR pictures in decode order
/// (H.264 7.4.3); callers alternate it by the picture's playback position.
pub(super) fn frame(
    image: &DecodedJpeg,
    idr_pic_id: u32,
    cancel: &impl Fn() -> bool,
) -> Result<Vec<u8>> {
    let (w, h) = (image.geometry.width, image.geometry.height);
    if image.rgba.len() != (w * h * 4) as usize {
        return Err(E::Extent);
    }
    let mut bits = Bits::default();
    // PCM macroblocks contain 384 aligned pixel bytes plus <=2 header bytes.
    // Reserve once rather than retaining a power-of-two growth allocation.
    let raw_bound = w.div_ceil(16) as usize * h.div_ceil(16) as usize * 386 + 64;
    bits.bytes
        .try_reserve_exact(raw_bound)
        .map_err(|_| E::Budget)?;
    #[cfg(feature = "test-harness")]
    crate::capacity_observations::record(
        "pcm_input",
        [image.rgba.capacity(), bits.bytes.capacity(), 0, 0],
    );
    bits.ue(0);
    bits.ue(2);
    bits.ue(0);
    bits.n(0, 4);
    bits.ue(idr_pic_id);
    bits.n(0, 1);
    bits.n(0, 1);
    bits.zero_se();
    bits.ue(1); // Disable deblocking on exact PCM.
    let pixel = |x: u32, y: u32| {
        let p = ((y.min(h - 1) * w + x.min(w - 1)) * 4) as usize;
        yuv(image.rgba[p], image.rgba[p + 1], image.rgba[p + 2])
    };
    for my in 0..h.div_ceil(16) {
        if cancel() {
            return Err(E::Codec);
        }
        for mx in 0..w.div_ceil(16) {
            bits.ue(25);
            bits.align();
            let mut luma = [0u8; 256];
            let mut chroma = [[0u16; 64]; 2];
            for y in 0..16 {
                for x in 0..16 {
                    let value = pixel(mx * 16 + x, my * 16 + y);
                    luma[(y * 16 + x) as usize] = value[0];
                    let c = ((y / 2) * 8 + x / 2) as usize;
                    chroma[0][c] += u16::from(value[1]);
                    chroma[1][c] += u16::from(value[2]);
                }
            }
            bits.bytes.extend(luma);
            bits.position += 256 * 8;
            for channel in chroma {
                for sum in channel {
                    bits.bytes.push(((sum + 2) / 4) as u8);
                    bits.position += 8;
                }
            }
        }
    }
    Ok(bits.finish(0x65))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Independently read the emitted SPS rather than inspecting writer calls.
    struct Reader {
        bytes: Vec<u8>,
        bit: usize,
    }
    impl Reader {
        fn new(nal: &[u8]) -> Self {
            Self::with_header(nal, 0x67)
        }
        fn with_header(nal: &[u8], header: u8) -> Self {
            assert_eq!(nal[0], header);
            let mut bytes = Vec::new();
            let mut zeros = 0;
            for &byte in &nal[1..] {
                if zeros == 2 && byte == 3 {
                    zeros = 0;
                    continue;
                }
                bytes.push(byte);
                zeros = if byte == 0 { zeros + 1 } else { 0 };
            }
            Self { bytes, bit: 0 }
        }
        fn n(&mut self, count: usize) -> u32 {
            (0..count).fold(0, |value, _| {
                let next = (self.bytes[self.bit / 8] >> (7 - self.bit % 8)) & 1;
                self.bit += 1;
                (value << 1) | u32::from(next)
            })
        }
        fn ue(&mut self) -> u32 {
            let mut zeros = 0;
            while self.n(1) == 0 {
                zeros += 1;
                assert!(zeros < 32);
            }
            (1u32 << zeros) - 1 + self.n(zeros)
        }
    }

    fn read_restrictions(width: u32, height: u32) -> [u32; 7] {
        let (sps, _) = configuration(width, height);
        let mut r = Reader::new(&sps);
        assert_eq!([r.n(8), r.n(8), r.n(8)], [66, 192, 50]);
        assert_eq!([r.ue(), r.ue(), r.ue(), r.ue(), r.n(1)], [0, 0, 2, 0, 0]);
        assert_eq!(
            [r.ue() + 1, r.ue() + 1],
            [width.div_ceil(16), height.div_ceil(16)]
        );
        assert_eq!([r.n(1), r.n(1)], [1, 1]); // progressive, direct_8x8
        if r.n(1) == 1 {
            assert_eq!(
                [r.ue(), r.ue(), r.ue(), r.ue()],
                [
                    0,
                    (width.div_ceil(16) * 16 - width) / 2,
                    0,
                    (height.div_ceil(16) * 16 - height) / 2
                ]
            );
        } else {
            assert_eq!((width % 16, height % 16), (0, 0));
        }
        assert_eq!([r.n(1), r.n(1), r.n(8), r.n(1)], [1, 1, 1, 0]);
        assert_eq!([r.n(1), r.n(3), r.n(1), r.n(1)], [1, 5, 0, 1]);
        assert_eq!([r.n(8), r.n(8), r.n(8)], [1, 13, 6]);
        assert_eq!([r.n(1), r.n(1), r.n(1), r.n(1), r.n(1)], [0; 5]);
        assert_eq!(
            r.n(1),
            1,
            "explicit restrictions required for PCM picture size"
        );
        let restrictions = [r.n(1), r.ue(), r.ue(), r.ue(), r.ue(), r.ue(), r.ue()];
        assert_eq!(r.n(1), 1, "RBSP stop bit");
        while r.bit < r.bytes.len() * 8 {
            assert_eq!(r.n(1), 0, "RBSP alignment");
        }
        restrictions
    }

    #[test]
    fn pcm_sps_signals_only_restrictions_the_writer_satisfies() {
        for (width, height) in [(2, 2), (32, 16), (1138, 1842), (1842, 1138)] {
            assert_eq!(read_restrictions(width, height), [0, 0, 1, 0, 0, 0, 1]);
        }
    }

    #[test]
    fn padded_pcm_picture_requires_unrestricted_vui_picture_bytes() {
        let (width, height) = (1138u32, 1842u32);
        let image = DecodedJpeg {
            geometry: crate::Geometry { width, height },
            rgba: (0..width * height)
                .flat_map(|_| [64, 128, 192, 255])
                .collect(),
        };
        let nal = frame(&image, 0, &|| false).unwrap();
        let raw_picture_bytes = width.div_ceil(16) as usize * height.div_ceil(16) as usize * 384;
        assert!(
            nal.len() > raw_picture_bytes,
            "even denominator one is too strict"
        );
        let restrictions = read_restrictions(width, height);
        assert_eq!(restrictions[1], 0, "permitted no-picture-byte-limit signal");
        // Independently walk the actual slice RBSP, not just a declared
        // denominator or an aggregate NAL byte count. E-80 counts RBSP syntax
        // bits, excluding emulation-prevention bytes.
        let mut slice = Reader::with_header(&nal, 0x65);
        assert_eq!([slice.ue(), slice.ue(), slice.ue()], [0, 2, 0]);
        assert_eq!(slice.n(4), 0); // frame_num
        assert_eq!(slice.ue(), 0); // idr_pic_id
        assert_eq!([slice.n(1), slice.n(1)], [0, 0]);
        assert_eq!(slice.ue(), 0); // slice_qp_delta, signed zero
        assert_eq!(slice.ue(), 1); // disable_deblocking_filter_idc
        for _ in 0..width.div_ceil(16) * height.div_ceil(16) {
            let start = slice.bit;
            assert_eq!(slice.ue(), 25, "every macroblock must be I_PCM");
            while !slice.bit.is_multiple_of(8) {
                assert_eq!(slice.n(1), 0, "pcm_alignment_zero_bit");
            }
            slice.bit += 384 * 8; // 256 luma + 64 Cb + 64 Cr samples
            assert!(slice.bit < slice.bytes.len() * 8, "PCM extent");
            assert!(slice.bit - start <= 3088, "actual PCM syntax bound");
            assert!(slice.bit - start <= (3072 + 128) / restrictions[2] as usize);
        }
        assert_eq!(slice.n(1), 1, "slice RBSP stop bit");
        while slice.bit < slice.bytes.len() * 8 {
            assert_eq!(slice.n(1), 0, "slice RBSP alignment");
        }
    }
}
