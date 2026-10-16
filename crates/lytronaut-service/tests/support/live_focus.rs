use serde_json::json;
pub fn live_focus_fixture() -> Vec<u8> {
    live_focus_fixture_with(false)
}
/// `missing_second`: the directory still references the second stored plane but
/// its content section is absent from the container.
pub fn live_focus_fixture_with(missing_second: bool) -> Vec<u8> {
    sized(missing_second, 32, 16, false)
}
pub fn sized(missing_second: bool, width: u32, height: u32, third: bool) -> Vec<u8> {
    use sha1::{Digest, Sha1};
    let hash = |b: &[u8]| format!("sha1-{:x}", Sha1::digest(b));
    let planes: Vec<Vec<u8>> = [[255u8, 0, 0], [0, 0, 255], [0, 255, 0]]
        .into_iter()
        .take(if third { 3 } else { 2 })
        .map(|rgb| {
            let mut bytes = Vec::new();
            let pixels: Vec<_> = (0..width * height).flat_map(|_| rgb).collect();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
                .encode(&pixels, width, height, image::ExtendedColorType::Rgb8)
                .unwrap();
            bytes
        })
        .collect();
    let images: Vec<_> = planes.iter().enumerate().map(|(i, plane)| json!({"representation":"jpeg","width":width,"height":height,"lambda":if i==0 {-1} else {i as i32},"imageRef":hash(plane)})).collect();
    let directory=serde_json::to_vec(&json!({"version":{"major":1,"minor":0},"picture":{"recipe":{"cameraSerial":"invented-sensitive-note","sourcePath":"invented-private-path","orientation":8},"accelerationArray":[{"type":"com.lytro.acceleration.refocusStack","vendorContent":{"imageArray":images}}]}})).unwrap();
    let mut bytes = b"\x89LFP\r\n\x1a\n\0\0\0\x01\0\0\0\0".to_vec();
    let mut sections = vec![(true, directory.as_slice()), (false, planes[0].as_slice())];
    if !missing_second {
        sections.push((false, planes[1].as_slice()));
    }
    if third {
        sections.push((false, planes[2].as_slice()));
    }
    for (directory, payload) in sections {
        bytes.extend(if directory {
            b"\x89LFM\r\n\x1a\n"
        } else {
            b"\x89LFC\r\n\x1a\n"
        });
        bytes.extend([0; 4]);
        bytes.extend((payload.len() as u32).to_be_bytes());
        bytes.extend(hash(payload).as_bytes());
        bytes.extend([0; 35]);
        bytes.extend(payload);
        while !bytes.len().is_multiple_of(16) {
            bytes.push(0);
        }
    }
    bytes
}
