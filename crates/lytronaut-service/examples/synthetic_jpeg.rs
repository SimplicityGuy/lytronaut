//! Public fixture pixels, no source photographs or imported format code.
fn main() {
    let path = std::env::args_os().nth(1).expect("output JPEG path");
    let mut pixels = vec![0_u8; 640 * 360 * 3];
    for y in 0..360 {
        for x in 0..640 {
            let p = (y * 640 + x) * 3;
            let circle = (x as i32 - 205).pow(2) + (y as i32 - 118).pow(2) < 38_i32.pow(2);
            pixels[p] = if circle { 75 } else { 20 + (x / 16) as u8 };
            pixels[p + 1] = if circle { 210 } else { 40 + (y / 12) as u8 };
            pixels[p + 2] = if circle { 235 } else { 65 + (x / 20) as u8 };
        }
    }
    image::codecs::jpeg::JpegEncoder::new_with_quality(std::fs::File::create(&path).unwrap(), 85)
        .encode(&pixels, 640, 360, image::ExtendedColorType::Rgb8)
        .unwrap();
    for plane in 0..3 {
        let mut corners = vec![0; 360 * 240 * 3];
        let colors = [[240, 20, 20], [20, 240, 20], [20, 20, 240], [240, 240, 20]];
        for y in 0..240 {
            for x in 0..360 {
                let color = colors[(usize::from(y >= 120) * 2 + usize::from(x >= 180) + plane) % 4];
                corners[(y * 360 + x) * 3..(y * 360 + x) * 3 + 3].copy_from_slice(&color);
            }
        }
        let output = std::path::Path::new(&path).with_extension(format!("focus{plane}.jpg"));
        image::codecs::jpeg::JpegEncoder::new_with_quality(
            std::fs::File::create(output).unwrap(),
            95,
        )
        .encode(&corners, 360, 240, image::ExtendedColorType::Rgb8)
        .unwrap();
    }
}
