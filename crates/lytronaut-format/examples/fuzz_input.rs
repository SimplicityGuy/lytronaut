//! Bounded stdin fuzz target, suitable for an external mutational fuzzer.
//! Supply synthetic seeds only; failure means panic/assertion, format errors are normal.
use lytronaut_format::*;
use std::io::{self, Read};
fn main() {
    let mut bytes = Vec::new();
    io::stdin().take(131_073).read_to_end(&mut bytes).unwrap();
    if bytes.len() > 131_072 {
        return;
    }
    let limits = Limits {
        container_bytes: 131_072,
        json_bytes: 32_768,
        json_nodes: 1024,
        json_depth: 16,
        sections: 32,
        dimension: 256,
        pixels: 65_536,
        decoded_bytes: 262_144,
        decoder_working_bytes: 8_388_608,
        jpeg_bytes: 131_072,
    };
    if let Ok(c) = Container::parse(&bytes, limits) {
        let _ = raw_asset(&c, limits);
        let _ = processed_asset(&c, limits);
        for s in &c.sections {
            let _ = bounded_json(s.payload, limits);
            let _ = decode_jpeg(s.payload, limits);
        }
    }
    let _ = bounded_json(&bytes, limits);
    let _ = decode_jpeg(&bytes, limits);
    if bytes.len() >= 4 {
        let g = Geometry {
            width: u32::from(u16::from_be_bytes([bytes[0], bytes[1]])),
            height: u32::from(u16::from_be_bytes([bytes[2], bytes[3]])),
        };
        let p = &bytes[4..];
        let _ = inspect_f32(p, g, limits);
        if let Ok(samples) = unpack12(p, g, limits) {
            assert_eq!(pack12(&samples, g, limits).unwrap(), p);
        }
    }
}
