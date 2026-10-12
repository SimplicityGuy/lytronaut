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
    s.n(50, 8); // Baseline Level 5, bounded <=90 Mbit/s.
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
    s.n(0, 1);
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
pub(super) fn frame(
    image: &DecodedJpeg,
    index: usize,
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
    bits.ue((index % 2) as u32);
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
