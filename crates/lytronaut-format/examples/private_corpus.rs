//! Explicit local-only gate. Never discovers a library or uploads source assets.
use lytronaut_format::*;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Component, Path},
    time::UNIX_EPOCH,
};
fn check(
    root: &Path,
    files: &serde_json::Map<String, Value>,
) -> std::result::Result<u64, &'static str> {
    let mut total = 0;
    for (relative, v) in files {
        let relative = Path::new(relative);
        if relative
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
        {
            return Err("manifest path");
        }
        let p = root.join(relative);
        let meta = fs::symlink_metadata(&p).map_err(|_| "source metadata")?;
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err("source type");
        }
        let mtime = meta
            .modified()
            .map_err(|_| "mtime")?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "mtime")?
            .as_nanos();
        if meta.len() != v["bytes"].as_u64().ok_or("manifest bytes")?
            || mtime != u128::from(v["mtime_ns"].as_u64().ok_or("manifest mtime")?)
        {
            return Err("source size/mtime changed");
        }
        let bytes = fs::read(p).map_err(|_| "source read")?;
        if Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
            != v["sha256"].as_str().ok_or("manifest digest")?
        {
            return Err("source digest changed");
        }
        total += meta.len();
    }
    Ok(total)
}
fn run() -> std::result::Result<(), &'static str> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("supply explicit source root and private manifest");
    }
    let root = Path::new(&args[0]);
    let manifest = fs::read(&args[1]).map_err(|_| "manifest read")?;
    let manifest: Value = serde_json::from_slice(&manifest).map_err(|_| "manifest JSON")?;
    let files = manifest["files"].as_object().ok_or("manifest files")?;
    let total = check(root, files)?;
    let l = Limits::default();
    let (mut raw, mut thumbs, mut planes, mut maps, mut processed, mut samples, mut pixels) =
        (0, 0, 0, 0, 0, 0u64, 0u64);
    for relative in files.keys() {
        let path = root.join(relative);
        let name = path
            .file_name()
            .and_then(|x| x.to_str())
            .ok_or("filename")?;
        if name == "thumbnail.jpg" {
            let b = fs::read(&path).map_err(|_| "thumbnail read")?;
            let d = decode_jpeg(&b, l).map_err(|_| "thumbnail decode")?;
            pixels += u64::from(d.geometry.width) * u64::from(d.geometry.height);
            thumbs += 1;
        } else if name.ends_with(".lfp") {
            let b = fs::read(&path).map_err(|_| "container read")?;
            let c = Container::parse(&b, l).map_err(|_| "container framing")?;
            if name == "raw.lfp" {
                let RawAsset::F01(s) = raw_asset(&c, l).map_err(|_| "raw metadata")? else {
                    return Err("raw not F01");
                };
                let unpacked = s.unpack(l).map_err(|_| "unpack")?;
                samples += unpacked.len() as u64;
                if pack12(&unpacked, s.geometry, l).map_err(|_| "repack")? != s.packed {
                    return Err("roundtrip mismatch");
                }
                raw += 1;
            } else {
                let p = processed_asset(&c, l).map_err(|_| "processed metadata/references")?;
                processed += 1;
                maps += p.maps.len();
                for plane in p.planes {
                    if plane.representation == Representation::Jpeg {
                        let d = decode_jpeg(plane.payload, l).map_err(|_| "plane decode")?;
                        if d.geometry != plane.geometry {
                            return Err("plane dimensions");
                        }
                        pixels += u64::from(d.geometry.width) * u64::from(d.geometry.height);
                        planes += 1;
                    }
                }
            }
        }
    }
    if (raw, thumbs, planes) != (109, 109, 420) {
        return Err("expected corpus counts");
    }
    if check(root, files)? != total {
        return Err("source total changed");
    }
    println!(
        "Private corpus: {raw} raw byte-exact roundtrips ({samples} samples), {thumbs} thumbnails, {planes} JPEG planes ({pixels} pixels), {processed} processed containers, {maps} finite maps; {} source files/{total} bytes unchanged in size/SHA256/mtime before and after.",
        files.len()
    );
    Ok(())
}
fn main() {
    if let Err(reason) = run() {
        eprintln!("Private corpus failed: {reason}");
        std::process::exit(1);
    }
}
