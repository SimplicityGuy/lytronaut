//! Independent portable Live Photo resource assembly. Generated focus motion only.
//! The caller must supply verified stored-plane pixels and an ephemeral export UUID.
#[path = "live_photo/avc.rs"]
mod avc;
#[path = "live_photo/movie.rs"]
mod movie;
use crate::{DecodedJpeg, FormatError as E, Geometry, Limits, Result};
pub const MOVIE_CAP: usize = 16 * 1024 * 1024;
pub const BUNDLE_CAP: usize = MOVIE_CAP + 2 * 1024 * 1024;
/// Deterministic ascending-lambda sweep, preserving endpoints and selected key.
#[derive(Debug, Clone)]
pub struct Plan {
    geometry: Geometry,
    planes: Vec<usize>,
    selected: usize,
}
impl Plan {
    pub fn new(geometry: Geometry, lambdas: &[f64], selected: usize) -> Result<Self> {
        let Geometry { width, height } = geometry;
        if width == 0
            || height == 0
            || width > 2048
            || height > 2048
            || width * height > 2 * 1024 * 1024
            || !width.is_multiple_of(2)
            || !height.is_multiple_of(2)
        {
            return Err(E::Unsupported);
        }
        if !(2..=64).contains(&lambdas.len()) || selected >= lambdas.len() {
            return Err(E::Budget);
        }
        if lambdas
            .iter()
            .enumerate()
            .any(|(i, v)| !v.is_finite() || lambdas[..i].contains(v))
        {
            return Err(E::Metadata);
        }
        let mut order: Vec<usize> = (0..lambdas.len()).collect();
        order.sort_by(|a, b| lambdas[*a].total_cmp(&lambdas[*b]));
        // Maximum emulation-prevention expansion, headers, tables, and key JPEG
        // are admitted before encoding or allocating any movie pixels.
        let frame_bound = width.div_ceil(16) as usize * height.div_ceil(16) as usize * 580 + 64;
        let count = ((MOVIE_CAP - 8192) / frame_bound).min(order.len());
        if count < 3 && order.len() > 2 {
            return Err(E::Budget);
        }
        let mut planes = vec![order[0], *order.last().unwrap(), selected];
        planes.sort_unstable();
        planes.dedup();
        for step in 0..count {
            let candidate = order[step * (order.len() - 1) / (count - 1)];
            if !planes.contains(&candidate) && planes.len() < count {
                planes.push(candidate);
            }
        }
        planes.sort_by(|a, b| lambdas[*a].total_cmp(&lambdas[*b]));
        Ok(Self {
            geometry,
            planes,
            selected,
        })
    }
    pub fn geometry(&self) -> Geometry {
        self.geometry
    }
    pub fn planes(&self) -> &[usize] {
        &self.planes
    }
    pub fn selected(&self) -> usize {
        self.selected
    }
    pub fn key_seconds(&self) -> f64 {
        self.planes
            .iter()
            .position(|v| *v == self.selected)
            .unwrap() as f64
            * 3.0
            / self.planes.len() as f64
    }
}
/// Incremental encoder: only one decoded plane and one encoded picture at a time.
pub struct Builder {
    plan: Plan,
    movie: movie::Movie,
    jpeg: Option<Vec<u8>>,
    seen: Vec<usize>,
    id: String,
}
pub struct Pair {
    pub jpeg: Vec<u8>,
    pub movie: Vec<u8>,
    pub plan: Plan,
}
fn identifier(id: &str) -> Result<()> {
    if id.len() != 36
        || !id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) {
                b == b'-'
            } else {
                b.is_ascii_hexdigit()
            }
        })
    {
        return Err(E::Metadata);
    }
    Ok(())
}
impl Builder {
    pub fn new(plan: Plan, id: &str) -> Result<Self> {
        identifier(id)?;
        Ok(Self {
            movie: movie::Movie::new(plan.geometry.width, plan.geometry.height),
            plan,
            jpeg: None,
            seen: Vec::new(),
            id: id.into(),
        })
    }
    pub fn completed_planes(&self) -> usize {
        self.seen.len()
    }
    pub fn wants(&self, index: usize) -> bool {
        self.plan.planes.contains(&index)
    }
    pub fn push(
        &mut self,
        index: usize,
        image: &DecodedJpeg,
        cancel: impl Fn() -> bool,
    ) -> Result<()> {
        if image.geometry != self.plan.geometry
            || !self.plan.planes.contains(&index)
            || self.seen.contains(&index)
        {
            return Err(E::Metadata);
        }
        if cancel() {
            return Err(E::Codec);
        }
        let frame = avc::frame(image, index, &cancel)?;
        if index == self.plan.selected && self.jpeg.is_none() {
            let limits = Limits {
                dimension: 2048,
                pixels: 2 * 1024 * 1024,
                jpeg_bytes: 2 * 1024 * 1024,
                ..Default::default()
            };
            let jpeg = crate::encode_derivative(image, limits)?;
            self.jpeg = Some(paired_jpeg(&jpeg, &self.id)?);
        }
        self.movie.push(index, frame)?;
        self.seen.push(index);
        Ok(())
    }
    /// Browser caller supplies its selected verified decoded frame JPEG.
    pub fn set_still(&mut self, jpeg: &[u8]) -> Result<()> {
        let limits = Limits {
            dimension: 2048,
            pixels: 2 * 1024 * 1024,
            jpeg_bytes: 2 * 1024 * 1024,
            decoder_working_bytes: 160 * 1024 * 1024,
            ..Default::default()
        };
        if crate::jpeg_geometry(jpeg, limits)? != self.plan.geometry {
            return Err(E::Extent);
        }
        self.jpeg = Some(paired_jpeg(jpeg, &self.id)?);
        Ok(())
    }
    pub fn finish(self) -> Result<Pair> {
        let jpeg = self.jpeg.ok_or(E::Metadata)?;
        let movie = self
            .movie
            .finish(&self.plan.planes, self.plan.selected, &self.id)?;
        Ok(Pair {
            jpeg,
            movie,
            plan: self.plan,
        })
    }
}
/// Minimal EXIF orientation and Apple pairing note; no source metadata is copied.
fn paired_jpeg(jpeg: &[u8], id: &str) -> Result<Vec<u8>> {
    identifier(id)?;
    if !jpeg.starts_with(&[255, 216]) {
        return Err(E::Magic);
    }
    let u = |n: u32| n.to_be_bytes().to_vec();
    let note = [
        b"Apple iOS\0\0\x01MM".to_vec(),
        1u16.to_be_bytes().to_vec(),
        17u16.to_be_bytes().to_vec(),
        2u16.to_be_bytes().to_vec(),
        u(37),
        u(32),
        u(0),
        id.as_bytes().to_vec(),
        vec![0],
    ]
    .concat();
    let exif = [
        b"Exif\0\0MM\0\x2a".to_vec(),
        u(8),
        2u16.to_be_bytes().to_vec(),
        0x112u16.to_be_bytes().to_vec(),
        3u16.to_be_bytes().to_vec(),
        u(1),
        u(0x10000),
        0x8769u16.to_be_bytes().to_vec(),
        4u16.to_be_bytes().to_vec(),
        u(1),
        u(38),
        u(0),
        1u16.to_be_bytes().to_vec(),
        0x927cu16.to_be_bytes().to_vec(),
        7u16.to_be_bytes().to_vec(),
        u(note.len() as u32),
        u(56),
        u(0),
        note,
    ]
    .concat();
    let mut output = vec![255, 216, 255, 225];
    output
        .try_reserve_exact(jpeg.len() + exif.len())
        .map_err(|_| E::Budget)?;
    output.extend(((exif.len() + 2) as u16).to_be_bytes());
    output.extend(exif);
    let mut pos = 2;
    while jpeg.get(pos) == Some(&255) {
        let marker = *jpeg.get(pos + 1).ok_or(E::Truncated)?;
        if marker == 218 {
            break;
        }
        let size = u16::from_be_bytes(
            jpeg.get(pos + 2..pos + 4)
                .ok_or(E::Truncated)?
                .try_into()
                .unwrap(),
        ) as usize
            + 2;
        let section = jpeg.get(pos..pos + size).ok_or(E::Truncated)?;
        if ![225, 237].contains(&marker) {
            output.extend(section);
        }
        pos += size;
    }
    output.extend(jpeg.get(pos..).ok_or(E::Truncated)?);
    Ok(output)
}
fn crc(bytes: &[u8]) -> u32 {
    let table: [u32; 256] = std::array::from_fn(|i| {
        let mut c = i as u32;
        for _ in 0..8 {
            c = (c >> 1) ^ (0xedb88320u32.wrapping_mul(c & 1));
        }
        c
    });
    !bytes.iter().fold(u32::MAX, |c, b| {
        (c >> 8) ^ table[((c ^ u32::from(*b)) & 255) as usize]
    })
}
fn local(name: &str, size: usize, crc: u32) -> Vec<u8> {
    let mut b = Vec::new();
    b.extend(0x04034b50u32.to_le_bytes());
    for v in [20u16, 0, 0, 0, 0] {
        b.extend(v.to_le_bytes());
    }
    for v in [crc, size as u32, size as u32] {
        b.extend(v.to_le_bytes());
    }
    b.extend((name.len() as u16).to_le_bytes());
    b.extend(0u16.to_le_bytes());
    b.extend(name.as_bytes());
    b
}
/// One complete ZIP resource pair. Container metadata uses no private source IDs.
/// MOV allocation is reused in place; no second whole-movie buffer is created.
pub fn bundle(pair: Pair, provenance: &[u8]) -> Result<Vec<u8>> {
    if provenance.len() > 16 * 1024 {
        return Err(E::Budget);
    }
    let Pair {
        jpeg,
        movie: mut bytes,
        ..
    } = pair;
    let movie_size = bytes.len();
    let movie_crc = crc(&bytes);
    let header = local("focus.MOV", movie_size, movie_crc);
    let needed = header.len() + jpeg.len() + provenance.len() + 1024;
    if movie_size + needed > BUNDLE_CAP {
        return Err(E::Budget);
    }
    bytes.try_reserve_exact(needed).map_err(|_| E::Budget)?;
    bytes.resize(movie_size + header.len(), 0);
    bytes.copy_within(0..movie_size, header.len());
    bytes[..header.len()].copy_from_slice(&header);
    let mut entries = vec![("focus.MOV", movie_size, movie_crc, 0usize)];
    for (name, data) in [
        ("focus.JPG", jpeg.as_slice()),
        ("provenance.json", provenance),
    ] {
        let offset = bytes.len();
        let checksum = crc(data);
        bytes.extend(local(name, data.len(), checksum));
        bytes.extend(data);
        entries.push((name, data.len(), checksum, offset));
    }
    let central = bytes.len();
    for (name, size, checksum, offset) in entries {
        bytes.extend(0x02014b50u32.to_le_bytes());
        for v in [20u16, 20, 0, 0, 0, 0] {
            bytes.extend(v.to_le_bytes());
        }
        for v in [checksum, size as u32, size as u32] {
            bytes.extend(v.to_le_bytes());
        }
        for v in [name.len() as u16, 0, 0, 0, 0] {
            bytes.extend(v.to_le_bytes());
        }
        bytes.extend(0u32.to_le_bytes());
        bytes.extend((offset as u32).to_le_bytes());
        bytes.extend(name.as_bytes());
    }
    let central_size = bytes.len() - central;
    bytes.extend(0x06054b50u32.to_le_bytes());
    for v in [0u16, 0, 3, 3] {
        bytes.extend(v.to_le_bytes());
    }
    bytes.extend((central_size as u32).to_le_bytes());
    bytes.extend((central as u32).to_le_bytes());
    bytes.extend(0u16.to_le_bytes());
    Ok(bytes)
}
