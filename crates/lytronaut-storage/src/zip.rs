//! Independent bounded ZIP framing, following PKWARE APPNOTE 6.3.10.
//! No generic archive extractor and no archive-supplied filesystem paths.
use crate::intake::{Limits, StorageError, validate_path};
use std::{
    collections::BTreeSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
fn u16v(b: &[u8], i: usize) -> u16 {
    u16::from_le_bytes(b[i..i + 2].try_into().unwrap())
}
fn u32v(b: &[u8], i: usize) -> u32 {
    u32::from_le_bytes(b[i..i + 4].try_into().unwrap())
}
fn u64v(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes(b[i..i + 8].try_into().unwrap())
}
fn read<const N: usize>(f: &mut File) -> Result<[u8; N], StorageError> {
    let mut b = [0; N];
    f.read_exact(&mut b).map_err(|_| StorageError::Invalid)?;
    Ok(b)
}
fn range(start: u64, size: u64, end: u64) -> Result<u64, StorageError> {
    start
        .checked_add(size)
        .filter(|x| *x <= end)
        .ok_or(StorageError::Invalid)
}
#[derive(Clone)]
pub(crate) struct Entry {
    pub name: String,
    pub size: u64,
    pub crc: u32,
    compressed: u64,
    method: u16,
    data: u64,
}
impl Entry {
    pub fn reader(&self, path: &Path) -> Result<Box<dyn Read + Send>, StorageError> {
        let mut f = File::open(path)?;
        f.seek(SeekFrom::Start(self.data))?;
        let input = f.take(self.compressed);
        if self.method == 0 {
            Ok(Box::new(input))
        } else {
            Ok(Box::new(Deflated {
                inner: flate2::read::DeflateDecoder::new(input),
                compressed: self.compressed,
                done: false,
            }))
        }
    }
}
struct Deflated {
    inner: flate2::read::DeflateDecoder<std::io::Take<File>>,
    compressed: u64,
    done: bool,
}
impl Read for Deflated {
    fn read(&mut self, b: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(b)?;
        if n == 0 && !self.done {
            self.done = true;
            if self.inner.total_in() != self.compressed {
                return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
            }
        }
        Ok(n)
    }
}
/// EOCD tail is at most 65,557 bytes; ZIP64 end record fixed header is checked before use.
pub(crate) fn preflight(path: &Path, l: &Limits) -> Result<Vec<Entry>, StorageError> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    if len > l.encoded_bytes || len < 22 {
        return Err(StorageError::Quota);
    }
    let tail_size = len.min(65557) as usize;
    f.seek(SeekFrom::Start(len - tail_size as u64))?;
    let mut tail = vec![0; tail_size];
    f.read_exact(&mut tail)?;
    let pos = (0..=tail_size - 22)
        .rev()
        .find(|&i| {
            tail[i..i + 4] == *b"PK\x05\x06" && i + 22 + u16v(&tail, i + 20) as usize == tail_size
        })
        .ok_or(StorageError::Invalid)?;
    let end = len - tail_size as u64 + pos as u64;
    let e = &tail[pos..];
    if u16v(e, 4) != 0 || u16v(e, 6) != 0 || u16v(e, 8) != u16v(e, 10) {
        return Err(StorageError::Invalid);
    }
    let mut count = u16v(e, 10) as u64;
    let mut size = u32v(e, 12) as u64;
    let mut offset = u32v(e, 16) as u64;
    let mut central_end = end;
    if count == 65535 || size == u32::MAX as u64 || offset == u32::MAX as u64 {
        if end < 20 {
            return Err(StorageError::Invalid);
        }
        f.seek(SeekFrom::Start(end - 20))?;
        let locator = read::<20>(&mut f)?;
        if &locator[..4] != b"PK\x06\x07" || u32v(&locator, 4) != 0 || u32v(&locator, 16) != 1 {
            return Err(StorageError::Invalid);
        }
        let zoffset = u64v(&locator, 8);
        range(zoffset, 56, end - 20)?;
        f.seek(SeekFrom::Start(zoffset))?;
        let z = read::<56>(&mut f)?;
        if &z[..4] != b"PK\x06\x06"
            || u64v(&z, 4) != 44
            || zoffset + 56 != end - 20
            || u32v(&z, 16) != 0
            || u32v(&z, 20) != 0
            || u64v(&z, 24) != u64v(&z, 32)
        {
            return Err(StorageError::Invalid);
        }
        count = u64v(&z, 32);
        size = u64v(&z, 40);
        offset = u64v(&z, 48);
        central_end = zoffset;
    }
    if count == 0
        || count > l.files as u64
        || size > l.metadata_bytes
        || range(offset, size, central_end)? != central_end
        || count > size / 46
    {
        return Err(StorageError::Quota);
    }
    f.seek(SeekFrom::Start(offset))?;
    let mut entries = Vec::new();
    let mut names = BTreeSet::new();
    let mut intervals = Vec::new();
    let mut total = 0u64;
    for _ in 0..count {
        let at = f.stream_position()?;
        range(at, 46, central_end)?;
        let c = read::<46>(&mut f)?;
        if &c[..4] != b"PK\x01\x02" {
            return Err(StorageError::Invalid);
        }
        let flags = u16v(&c, 8);
        let method = u16v(&c, 10);
        if flags & !(0x0800 | 0x0008 | 0x0006) != 0 || !matches!(method, 0 | 8) || u16v(&c, 34) != 0
        {
            return Err(StorageError::Invalid);
        }
        let name_len = u16v(&c, 28) as usize;
        let extra_len = u16v(&c, 30) as usize;
        let comment_len = u16v(&c, 32) as usize;
        if name_len == 0 || name_len > l.path_bytes + 1 {
            return Err(StorageError::Quota);
        }
        range(
            at + 46,
            (name_len + extra_len + comment_len) as u64,
            central_end,
        )?;
        let mut name = vec![0; name_len];
        f.read_exact(&mut name)?;
        let mut extra = vec![0; extra_len];
        f.read_exact(&mut extra)?;
        f.seek(SeekFrom::Current(comment_len as i64))?;
        let name = String::from_utf8(name).map_err(|_| StorageError::Invalid)?;
        let dir = name.ends_with('/');
        let logical = if dir { &name[..name.len() - 1] } else { &name };
        validate_path(logical, l)?;
        let canonical = logical.to_lowercase();
        if !names.insert(canonical.clone()) {
            return Err(StorageError::Conflict);
        }
        let mode = u32v(&c, 38) >> 16;
        let typ = mode & 0o170000;
        if typ != 0 && typ != 0o100000 && typ != 0o040000
            || typ == 0o040000 && !dir
            || typ == 0o100000 && dir
            || u32v(&c, 38) & 0x18 != 0 && !dir
        {
            return Err(StorageError::Invalid);
        }
        let mut uncompressed = u32v(&c, 24) as u64;
        let mut compressed = u32v(&c, 20) as u64;
        let mut local = u32v(&c, 42) as u64;
        let mut i = 0;
        let mut zip64 = false;
        while i < extra.len() {
            if i + 4 > extra.len() {
                return Err(StorageError::Invalid);
            }
            let tag = u16v(&extra, i);
            let n = u16v(&extra, i + 2) as usize;
            i += 4;
            if i + n > extra.len() {
                return Err(StorageError::Invalid);
            }
            let value = &extra[i..i + n];
            match tag {
                1 => {
                    if zip64 {
                        return Err(StorageError::Invalid);
                    }
                    zip64 = true;
                    let mut p = 0;
                    for v in [&mut uncompressed, &mut compressed, &mut local] {
                        if *v == u32::MAX as u64 {
                            if p + 8 > n {
                                return Err(StorageError::Invalid);
                            }
                            *v = u64v(value, p);
                            p += 8;
                        }
                    }
                    if p != n {
                        return Err(StorageError::Invalid);
                    }
                }
                0x5455 | 0x7875 | 0x000a => {}
                _ => return Err(StorageError::Invalid),
            }
            i += n;
        }
        if uncompressed == u32::MAX as u64
            || compressed == u32::MAX as u64
            || local == u32::MAX as u64
        {
            return Err(StorageError::Invalid);
        }
        total = total.checked_add(uncompressed).ok_or(StorageError::Quota)?;
        if total > l.upload_bytes
            || uncompressed > l.file_bytes
            || compressed > l.encoded_bytes
            || method == 0 && compressed != uncompressed
            || dir && (uncompressed != 0 || method == 0 && compressed != 0)
        {
            return Err(StorageError::Quota);
        }
        // Local framing/name must agree with central framing. No preamble or overlapping payloads.
        let next = f.stream_position()?;
        range(local, 30, offset)?;
        f.seek(SeekFrom::Start(local))?;
        let h = read::<30>(&mut f)?;
        if &h[..4] != b"PK\x03\x04"
            || u16v(&h, 6) != flags
            || u16v(&h, 8) != method
            || u16v(&h, 26) as usize != name.len()
        {
            return Err(StorageError::Invalid);
        }
        range(local, 30 + name.len() as u64 + u16v(&h, 28) as u64, offset)?;
        let mut local_name = vec![0; name.len()];
        f.read_exact(&mut local_name)?;
        if local_name != name.as_bytes() {
            return Err(StorageError::Invalid);
        }
        let data = range(local, 30 + name.len() as u64 + u16v(&h, 28) as u64, offset)?;
        let mut local_extra = vec![0; u16v(&h, 28) as usize];
        f.read_exact(&mut local_extra)?;
        let mut extra_at = 0;
        let mut local_zip64 = false;
        let mut local_size = u32v(&h, 22) as u64;
        let mut local_compressed = u32v(&h, 18) as u64;
        while extra_at < local_extra.len() {
            if extra_at + 4 > local_extra.len() {
                return Err(StorageError::Invalid);
            }
            let tag = u16v(&local_extra, extra_at);
            let n = u16v(&local_extra, extra_at + 2) as usize;
            extra_at += 4;
            if extra_at + n > local_extra.len() || !matches!(tag, 1 | 0x5455 | 0x7875 | 0x000a) {
                return Err(StorageError::Invalid);
            }
            if tag == 1 {
                if local_zip64 {
                    return Err(StorageError::Invalid);
                }
                local_zip64 = true;
                let value = &local_extra[extra_at..extra_at + n];
                if local_size != u32::MAX as u64 && local_compressed != u32::MAX as u64 {
                    // Chromium's macOS package archiver emits this unused ZIP64
                    // placeholder while keeping both authoritative 32-bit sizes.
                    // Accept only that exact inert form; no other surplus ZIP64
                    // payload is ignored.
                    if flags & 8 != 0 || n != 16 || value.iter().any(|byte| *byte != 0) {
                        return Err(StorageError::Invalid);
                    }
                } else {
                    let mut at = 0;
                    for size in [&mut local_size, &mut local_compressed] {
                        if *size == u32::MAX as u64 {
                            if at + 8 > value.len() {
                                return Err(StorageError::Invalid);
                            }
                            *size = u64v(value, at);
                            at += 8;
                        }
                    }
                    if at != n {
                        return Err(StorageError::Invalid);
                    }
                }
            }
            extra_at += n;
        }
        if flags & 8 == 0 && (local_size != uncompressed || local_compressed != compressed) {
            return Err(StorageError::Invalid);
        }
        let payload_end = range(data, compressed, offset)?;
        let mut record_end = payload_end;
        if flags & 8 != 0 {
            f.seek(SeekFrom::Start(payload_end))?;
            range(payload_end, 4, offset)?;
            let first = read::<4>(&mut f)?;
            let crc = if &first == b"PK\x07\x08" {
                record_end = range(record_end, 4, offset)?;
                range(payload_end, 8, offset)?;
                u32v(&read::<4>(&mut f)?, 0)
            } else {
                u32v(&first, 0)
            };
            let wide = u32v(&c, 20) == u32::MAX || u32v(&c, 24) == u32::MAX;
            record_end = range(record_end, if wide { 20 } else { 12 }, offset)?;
            let (cs, us) = if wide {
                let b = read::<16>(&mut f)?;
                (u64v(&b, 0), u64v(&b, 8))
            } else {
                let b = read::<8>(&mut f)?;
                (u32v(&b, 0) as u64, u32v(&b, 4) as u64)
            };
            if crc != u32v(&c, 16) || cs != compressed || us != uncompressed {
                return Err(StorageError::Invalid);
            }
        }
        if flags & 8 == 0
            && (u32v(&h, 14) != u32v(&c, 16)
                || (u32v(&h, 18) != u32::MAX && u32v(&h, 18) as u64 != compressed)
                || (u32v(&h, 22) != u32::MAX && u32v(&h, 22) as u64 != uncompressed))
        {
            return Err(StorageError::Invalid);
        }
        intervals.push((local, record_end));
        f.seek(SeekFrom::Start(next))?;
        if dir {
            // Chrome's package archiver writes the canonical two-byte raw
            // DEFLATE empty stream for directory records. Keep this exception
            // exact so truncated streams, trailing bytes and compressed work
            // disguised as structure never enter an inflater.
            if method == 8 {
                if compressed != 2 || u32v(&c, 16) != 0 {
                    return Err(StorageError::Invalid);
                }
                f.seek(SeekFrom::Start(data))?;
                if read::<2>(&mut f)? != [0x03, 0x00] {
                    return Err(StorageError::Invalid);
                }
            }
        } else {
            entries.push(Entry {
                name,
                size: uncompressed,
                crc: u32v(&c, 16),
                compressed,
                method,
                data,
            });
        }
        f.seek(SeekFrom::Start(next))?;
    }
    if f.stream_position()? != central_end {
        return Err(StorageError::Invalid);
    }
    intervals.sort_unstable();
    if intervals.first().is_none_or(|x| x.0 != 0)
        || intervals.windows(2).any(|x| x[0].1 != x[1].0)
        || intervals.last().is_none_or(|x| x.1 != offset)
    {
        return Err(StorageError::Invalid);
    }
    // Reject file/directory ambiguity before any source publication.
    for e in &entries {
        let p = e.name.to_lowercase();
        if names
            .range(format!("{p}/")..)
            .next()
            .is_some_and(|n| n.starts_with(&format!("{p}/")))
        {
            return Err(StorageError::Conflict);
        }
    }
    Ok(entries)
}
