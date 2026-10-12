// Independent QuickTime writer from Apple QTFF public specifications.
fn u(v: u32) -> Vec<u8> {
    v.to_be_bytes().to_vec()
}
fn atom(t: &[u8; 4], b: Vec<u8>) -> Vec<u8> {
    [u((b.len() + 8) as u32), t.to_vec(), b].concat()
}
fn full(t: &[u8; 4], flags: u32, b: Vec<u8>) -> Vec<u8> {
    atom(t, [u(flags), b].concat())
}
fn matrix() -> Vec<u8> {
    [0x10000, 0, 0, 0, 0x10000, 0, 0, 0, 0x40000000]
        .into_iter()
        .flat_map(u)
        .collect()
}
fn handler(t: &[u8; 4]) -> Vec<u8> {
    full(
        b"hdlr",
        0,
        [b"mhlr".to_vec(), t.to_vec(), vec![0; 12], vec![0]].concat(),
    )
}
struct Track<'a> {
    id: u32,
    w: u32,
    h: u32,
    offsets: &'a [u32],
    sizes: &'a [u32],
    desc: Vec<u8>,
    meta: bool,
    key: u32,
    scale: u32,
}
fn track(track: Track<'_>) -> Vec<u8> {
    let Track {
        id,
        w,
        h,
        offsets,
        sizes,
        desc,
        meta,
        key,
        scale,
    } = track;
    let duration = if meta {
        3000
    } else {
        sizes.len() as u32 * 3000
    };
    let visible_duration = if meta { key * 3000 + 3000 } else { duration };
    let tkhd = full(
        b"tkhd",
        7,
        [
            u(0),
            u(0),
            u(id),
            u(0),
            u(visible_duration),
            vec![0; 16],
            matrix(),
            u(w << 16),
            u(h << 16),
        ]
        .concat(),
    );
    let mdhd = full(
        b"mdhd",
        0,
        [u(0), u(0), u(scale), u(duration), vec![0x55, 0xc4, 0, 0]].concat(),
    );
    let entry = full(b"stsd", 0, [u(1), desc].concat());
    let stts = full(b"stts", 0, [u(1), u(sizes.len() as u32), u(3000)].concat());
    let stsc = full(b"stsc", 0, [u(1), u(1), u(1), u(1)].concat());
    let stsz = full(
        b"stsz",
        0,
        [
            u(0),
            u(sizes.len() as u32),
            sizes.iter().flat_map(|v| u(*v)).collect(),
        ]
        .concat(),
    );
    let stco = full(
        b"stco",
        0,
        [
            u(offsets.len() as u32),
            offsets.iter().flat_map(|v| u(*v)).collect(),
        ]
        .concat(),
    );
    let stbl = atom(b"stbl", [entry, stts, stsc, stsz, stco].concat());
    let dref = full(b"dref", 0, [u(1), full(b"url ", 1, vec![])].concat());
    let dinf = atom(b"dinf", dref);
    let header = if meta {
        atom(
            b"gmhd",
            full(
                b"gmin",
                0,
                vec![0, 0x40, 0x80, 0, 0x80, 0, 0x80, 0, 0, 0, 0, 0],
            ),
        )
    } else {
        full(b"vmhd", 1, vec![0; 8])
    };
    let minf = atom(b"minf", [header, dinf, stbl].concat());
    let mdia = atom(
        b"mdia",
        [mdhd, handler(if meta { b"meta" } else { b"vide" }), minf].concat(),
    );
    let edts = if meta && key > 0 {
        atom(
            b"edts",
            full(
                b"elst",
                0,
                [
                    u(2),
                    u(key * 3000),
                    u(0xffffffff),
                    u(0x10000),
                    u(3000),
                    u(0),
                    u(0x10000),
                ]
                .concat(),
            ),
        )
    } else {
        vec![]
    };
    atom(b"trak", [tkhd, edts, mdia].concat())
}
use crate::{FormatError as E, Result};
/// Own bounded encoded pictures without growing a second movie allocation while
/// the platform decoder is alive. Final assembly consumes these segments.
pub(super) struct Movie {
    length: usize,
    samples: Vec<Sample>,
    width: u32,
    height: u32,
    sps: Vec<u8>,
    pps: Vec<u8>,
}
struct Sample {
    index: usize,
    offset: u32,
    bytes: Vec<u8>,
}
impl Movie {
    pub(super) fn new(width: u32, height: u32) -> Self {
        let (sps, pps) = super::avc::configuration(width, height);
        Self {
            length: 28, // ftyp plus mdat header, identical to final assembly.
            samples: Vec::new(),
            width,
            height,
            sps,
            pps,
        }
    }
    pub(super) fn push(&mut self, index: usize, sample: Vec<u8>) -> Result<()> {
        let length = self
            .length
            .checked_add(sample.len())
            .and_then(|n| n.checked_add(4))
            .ok_or(E::Budget)?;
        if length > super::MOVIE_CAP - 8192 {
            return Err(E::Budget);
        }
        if self.samples.iter().any(|v| v.index == index) {
            return Err(E::Duplicate);
        }
        self.samples.try_reserve(1).map_err(|_| E::Budget)?;
        self.samples.push(Sample {
            index,
            offset: self.length as u32,
            bytes: sample,
        });
        #[cfg(feature = "test-harness")]
        crate::capacity_observations::record(
            "segments",
            [
                self.samples.iter().map(|s| s.bytes.capacity()).sum(),
                self.samples.capacity() * std::mem::size_of::<Sample>(),
                self.sps.capacity(),
                self.pps.capacity(),
            ],
        );
        self.length = length;
        Ok(())
    }
    pub(super) fn finish(
        self,
        order: &[usize],
        selected: usize,
        id: &str,
        jpeg_bytes: usize,
    ) -> Result<Vec<u8>> {
        if self.samples.len() != order.len() {
            return Err(E::Metadata);
        }
        let mut offsets = Vec::new();
        let mut sizes = Vec::new();
        for index in order {
            let sample = self
                .samples
                .iter()
                .find(|v| v.index == *index)
                .ok_or(E::Metadata)?;
            offsets.push(sample.offset);
            sizes.push((sample.bytes.len() + 4) as u32);
        }
        let key = order
            .iter()
            .position(|i| *i == selected)
            .ok_or(E::Metadata)? as u32;
        let scale = order.len() as u32 * 1000;
        let metadata_offset = self.length as u32;
        let avcc = atom(
            b"avcC",
            [
                vec![1, self.sps[1], self.sps[2], self.sps[3], 255, 225],
                (self.sps.len() as u16).to_be_bytes().to_vec(),
                self.sps,
                vec![1],
                (self.pps.len() as u16).to_be_bytes().to_vec(),
                self.pps,
            ]
            .concat(),
        );
        let mut entry = vec![0; 6];
        entry.extend([0, 1]);
        entry.extend([0; 16]);
        entry.extend((self.width as u16).to_be_bytes());
        entry.extend((self.height as u16).to_be_bytes());
        entry.extend(u(0x480000));
        entry.extend(u(0x480000));
        entry.extend(u(0));
        entry.extend([0, 1]);
        entry.extend([0; 32]);
        entry.extend([0, 24, 255, 255]);
        entry.extend(avcc);
        entry.extend(atom(
            b"colr",
            [b"nclc".to_vec(), vec![0, 1, 0, 13, 0, 6]].concat(),
        ));
        let video = track(Track {
            id: 1,
            w: self.width,
            h: self.height,
            offsets: &offsets,
            sizes: &sizes,
            desc: atom(b"avc1", entry),
            meta: false,
            key: 0,
            scale,
        });
        let keyd = atom(
            b"keyd",
            [
                b"mdta".to_vec(),
                b"com.apple.quicktime.still-image-time".to_vec(),
            ]
            .concat(),
        );
        let dtyp = atom(b"dtyp", [u(0), u(65)].concat());
        let keys = atom(b"keys", atom(&1u32.to_be_bytes(), [keyd, dtyp].concat()));
        let desc = atom(b"mebx", [vec![0; 6], vec![0, 1], keys].concat());
        let metadata = track(Track {
            id: 2,
            w: 0,
            h: 0,
            offsets: &[metadata_offset],
            sizes: &[9],
            desc,
            meta: true,
            key,
            scale,
        });
        let mvhd = full(
            b"mvhd",
            0,
            [
                u(0),
                u(0),
                u(scale),
                u(3 * scale),
                u(0x10000),
                vec![1, 0],
                vec![0; 10],
                matrix(),
                vec![0; 24],
                u(3),
            ]
            .concat(),
        );
        let keys = full(
            b"keys",
            0,
            [
                u(1),
                atom(b"mdta", b"com.apple.quicktime.content.identifier".to_vec()),
            ]
            .concat(),
        );
        let data = atom(b"data", [u(1), u(0), id.as_bytes().to_vec()].concat());
        let ilst = atom(b"ilst", atom(&1u32.to_be_bytes(), data));
        let meta = atom(
            b"meta",
            [
                full(b"hdlr", 0, [u(0), b"mdta".to_vec(), vec![0; 13]].concat()),
                keys,
                ilst,
            ]
            .concat(),
        );
        let moov = atom(b"moov", [mvhd, video, metadata, meta].concat());
        let movie_size = self.length + 9 + moov.len();
        if movie_size > super::MOVIE_CAP {
            return Err(E::Budget);
        }
        // Reserve final ZIP slack now, after decoder cleanup. Packaging reuses
        // this allocation; no MOV/ZIP growth overlaps live codec buffers.
        let capacity = movie_size
            .checked_add(jpeg_bytes)
            .and_then(|n| n.checked_add(super::PROVENANCE_CAP + super::ZIP_OVERHEAD))
            .ok_or(E::Budget)?
            .min(super::BUNDLE_CAP);
        let mut bytes = Vec::new();
        bytes.try_reserve_exact(capacity).map_err(|_| E::Budget)?;
        #[cfg(feature = "test-harness")]
        crate::capacity_observations::record(
            "assembly",
            [
                self.samples.iter().map(|s| s.bytes.capacity()).sum(),
                bytes.capacity(),
                moov.capacity(),
                offsets.capacity() * std::mem::size_of::<u32>()
                    + sizes.capacity() * std::mem::size_of::<u32>(),
            ],
        );
        bytes.extend(atom(
            b"ftyp",
            [b"qt  ".to_vec(), u(0), b"qt  ".to_vec()].concat(),
        ));
        bytes.extend(u((self.length + 9 - 20) as u32));
        bytes.extend(b"mdat");
        for sample in self.samples {
            bytes.extend(u(sample.bytes.len() as u32));
            bytes.extend(sample.bytes);
        }
        bytes.extend(atom(&1u32.to_be_bytes(), vec![255]));
        bytes.extend(moov);
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn segments_move_without_copy_and_reject_without_advancing() {
        let mut movie = Movie::new(32, 32);
        let sample = vec![0x65; 1000];
        let allocation = sample.as_ptr();
        movie.push(0, sample).unwrap();
        assert_eq!(movie.samples[0].bytes.as_ptr(), allocation);
        assert_eq!(movie.length, 1032);
        assert_eq!(movie.push(0, vec![0x65]), Err(E::Duplicate));
        assert_eq!(movie.length, 1032);
        assert_eq!(movie.samples.len(), 1);
        assert_eq!(
            movie.push(1, vec![0; super::super::MOVIE_CAP]),
            Err(E::Budget)
        );
        assert_eq!(movie.length, 1032);
        assert_eq!(movie.samples.len(), 1);
        let remaining = super::super::MOVIE_CAP - 8192 - movie.length - 4;
        movie.push(1, vec![0x65; remaining]).unwrap();
        assert_eq!(movie.length, super::super::MOVIE_CAP - 8192);
        assert_eq!(movie.push(2, vec![0x65]), Err(E::Budget));
        assert_eq!(movie.samples.len(), 2);
        let bytes = movie
            .finish(&[1, 0], 0, "A8A8A8A8-1111-4444-8888-121212121212", 0)
            .unwrap();
        assert!(bytes.len() <= super::super::MOVIE_CAP);
    }
}
