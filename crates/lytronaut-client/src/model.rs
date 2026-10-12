//! Platform independent admission, navigation and import policy.
use lytronaut_contracts::{AssetManifest, ImportKind, Representation};
use sha2::{Digest, Sha256};

pub const PAGE_SIZE: u32 = 24;
pub const JSON_LIMIT: usize = 256 * 1024;
// 32 MiB reserved for persistent stage and 24 downsampled grid images.
pub const VIEWER_BUDGET: u64 = 64 * 1024 * 1024;

#[derive(Default, Debug)]
pub struct Admission {
    active: usize,
    bytes: u64,
}
impl Admission {
    pub fn acquire(&mut self, bytes: u64) -> bool {
        if self.active >= 2 || self.bytes.saturating_add(bytes) > VIEWER_BUDGET {
            return false;
        }
        self.active += 1;
        self.bytes += bytes;
        true
    }
    pub fn release(&mut self, bytes: u64) {
        assert!(self.active > 0 && bytes <= self.bytes);
        self.active -= 1;
        self.bytes -= bytes;
    }
}
pub fn viewer_bytes(manifest: &AssetManifest) -> Result<u64, &'static str> {
    manifest
        .validate()
        .map_err(|_| "Invalid preview manifest")?;
    if manifest.representation != Representation::JpegPreview {
        return Err("This representation is preserved only");
    }
    let bytes = manifest.encoded_bytes * 4
        + manifest.chunks.iter().map(|c| c.length).max().unwrap_or(0)
        + manifest.decoded_bytes.unwrap_or(0) * 2
        + 128 * 128 * 4;
    if bytes > VIEWER_BUDGET {
        return Err("Preview exceeds the browser memory budget");
    }
    Ok(bytes)
}
pub fn verified(bytes: &[u8], manifest: &AssetManifest) -> bool {
    bytes.len() as u64 == manifest.encoded_bytes
        && format!("{:x}", Sha256::digest(bytes)) == String::from(manifest.content_sha256.clone())
}
pub fn query(search: &str, sort: &str, offset: u32) -> Result<String, &'static str> {
    if search.len() > 128 || search.chars().any(char::is_control) {
        return Err("Search must contain at most 128 UTF-8 bytes without control characters");
    }
    if !["order", "name", "rating"].contains(&sort) || offset > 100_000 {
        return Err("Invalid navigation request");
    }
    let escaped = search
        .bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect::<String>();
    Ok(format!(
        "limit={PAGE_SIZE}&offset={offset}&sort={sort}&search={escaped}"
    ))
}
pub fn path_hex(path: &str) -> Result<String, &'static str> {
    if path.is_empty()
        || path.len() > 4096
        || path.contains(['\\', '\0'])
        || path.split('/').count() > 32
        || path
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'))
    {
        return Err("Selection contains an unsafe or oversized relative path");
    }
    Ok(path.bytes().map(|b| format!("{b:02x}")).collect())
}
pub fn declaration(kind: ImportKind, sizes: &[u64]) -> Result<(u64, u32), &'static str> {
    if sizes.is_empty() || sizes.len() > 100_000 {
        return Err("Select between 1 and 100,000 regular files");
    }
    let mut total = 0_u64;
    for size in sizes {
        if kind == ImportKind::Directory && *size > 512 * 1024 * 1024 {
            return Err("An individual file exceeds 512 MiB");
        }
        total = total.checked_add(*size).ok_or("Selection is too large")?;
    }
    if total == 0 || total > 20 * 1024 * 1024 * 1024 {
        return Err("Selection must contain bytes and fit the 20 GiB import budget");
    }
    // ZIP expansion is admitted separately by the server, bounded at 20 GiB.
    Ok((
        if kind == ImportKind::ZipLibrary {
            20 * 1024 * 1024 * 1024
        } else {
            total
        },
        sizes.len() as u32,
    ))
}
pub fn adjacent(index: usize, delta: i32, count: usize) -> Option<usize> {
    if count == 0 {
        return None;
    }
    Some(index.saturating_add_signed(delta as isize).min(count - 1))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_shared_admission_retains_memory_and_transfer_leases() {
        let mut a = Admission::default();
        assert!(a.acquire(50 * 1024 * 1024));
        assert!(!a.acquire(50 * 1024 * 1024));
        assert!(a.acquire(1));
        assert!(!a.acquire(0));
        a.release(1);
        assert!(a.acquire(0));
        a.release(0);
        a.release(50 * 1024 * 1024);
        assert!(a.acquire(VIEWER_BUDGET));
    }
    #[test]
    fn navigation_encodes_literal_unicode_and_punctuation() {
        assert_eq!(
            query("雪%_&", "order", 24).unwrap(),
            "limit=24&offset=24&sort=order&search=%E9%9B%AA%25_%26"
        );
        assert!(query(&"雪".repeat(43), "order", 0).is_err());
        assert!(query("a\nb", "name", 0).is_err());
        assert!(query("", "unknown", 0).is_err());
        assert!(query("", "rating", 100001).is_err());
        assert_eq!(adjacent(0, -1, 3), Some(0));
        assert_eq!(adjacent(2, 1, 3), Some(2));
        assert_eq!(adjacent(0, 1, 0), None);
    }
    #[test]
    fn bounded_selection_preserves_utf8_paths() {
        assert_eq!(path_hex("a/雪").unwrap(), "612fe99baa");
        for p in ["", "../a", "/a", "a//b", "a\\b", "a/./b", "C:/x", "a\0b"] {
            assert!(path_hex(p).is_err());
        }
        assert!(path_hex(&"a".repeat(4097)).is_err());
        assert!(path_hex(&vec!["a"; 33].join("/")).is_err());
        assert_eq!(declaration(ImportKind::Directory, &[0, 3]).unwrap(), (3, 2));
        assert!(declaration(ImportKind::Directory, &[]).is_err());
        assert!(declaration(ImportKind::Directory, &[0]).is_err());
        assert!(declaration(ImportKind::Directory, &[513 * 1024 * 1024]).is_err());
        assert!(declaration(ImportKind::ZipLibrary, &[u64::MAX, 1]).is_err());
        assert!(declaration(ImportKind::ZipLibrary, &[21 * 1024 * 1024 * 1024]).is_err());
        assert!(declaration(ImportKind::Directory, &vec![1; 100001]).is_err());
        assert_eq!(
            declaration(ImportKind::ZipLibrary, &[1]).unwrap().0,
            20 * 1024 * 1024 * 1024
        );
    }
    #[test]
    fn viewer_rejects_nonpreview_and_changed_content() {
        let mut m: AssetManifest =
            serde_json::from_str(include_str!("../../../fixtures/synthetic/asset-v1.json"))
                .unwrap();
        assert!(viewer_bytes(&m).is_err());
        m.representation = Representation::JpegPreview;
        m.dimensions = Some(lytronaut_contracts::Dimensions {
            width: 2,
            height: 2,
        });
        m.decoded_bytes = Some(16);
        assert!(viewer_bytes(&m).is_ok());
        assert!(!verified(&[0; 16], &m));
        assert!(!verified(&[0; 15], &m));
        m.content_sha256 = format!("{:x}", Sha256::digest([0; 16])).try_into().unwrap();
        assert!(verified(&[0; 16], &m));
        m.encoded_bytes = 48 * 1024 * 1024;
        m.chunks = (0..6)
            .map(|i| lytronaut_contracts::ChunkRange {
                offset: i * 8 * 1024 * 1024,
                length: 8 * 1024 * 1024,
            })
            .collect();
        assert!(viewer_bytes(&m).is_err());
        m.encoded_bytes = 0;
        assert!(viewer_bytes(&m).is_err());
    }
}

/// Bounded ZIP admission metadata only; the server independently validates all framing.
fn zip_end_pos(tail: &[u8]) -> Result<usize, &'static str> {
    (0..tail.len().saturating_sub(21))
        .rev()
        .find(|&p| {
            tail[p..].starts_with(b"PK\x05\x06")
                && p + 22 + u16::from_le_bytes([tail[p + 20], tail[p + 21]]) as usize == tail.len()
        })
        .ok_or("ZIP end record missing")
}
pub fn zip64_record_offset(tail: &[u8], archive_size: u64) -> Result<Option<u64>, &'static str> {
    let pos = zip_end_pos(tail)?;
    if pos >= 20 && &tail[pos - 20..pos - 16] == b"PK\x06\x07" {
        let locator = &tail[pos - 20..pos];
        if locator[4..8] != [0; 4] || u32::from_le_bytes(locator[16..20].try_into().unwrap()) != 1 {
            return Err("Multi-volume ZIP is unsupported");
        }
        let offset = u64::from_le_bytes(locator[8..16].try_into().unwrap());
        if offset.checked_add(56).is_none_or(|end| end > archive_size) {
            return Err("ZIP64 end record exceeds archive");
        }
        Ok(Some(offset))
    } else {
        Ok(None)
    }
}
pub fn zip_extent(tail: &[u8], archive_size: u64) -> Result<(u64, usize), &'static str> {
    zip_extent_with_record(tail, archive_size, None)
}
pub fn zip_extent_with_record(
    tail: &[u8],
    archive_size: u64,
    record: Option<&[u8]>,
) -> Result<(u64, usize), &'static str> {
    let pos = zip_end_pos(tail)?;
    let at16 = |p| u16::from_le_bytes([tail[p], tail[p + 1]]);
    let at32 = |p| u32::from_le_bytes(tail[p..p + 4].try_into().unwrap());
    if at16(pos + 4) != 0 || at16(pos + 6) != 0 || at16(pos + 8) != at16(pos + 10) {
        return Err("Multi-volume ZIP is unsupported");
    }
    let (count, size, offset) = if zip64_record_offset(tail, archive_size)?.is_some() {
        let record = record.ok_or("ZIP64 admission record required")?;
        if record.len() != 56
            || !record.starts_with(b"PK\x06\x06")
            || u64::from_le_bytes(record[4..12].try_into().unwrap()) < 44
            || record[16..24] != [0; 8]
        {
            return Err("Invalid ZIP64 end record");
        }
        let at64 = |p| u64::from_le_bytes(record[p..p + 8].try_into().unwrap());
        if at64(24) != at64(32) {
            return Err("Multi-volume ZIP is unsupported");
        }
        (at64(32), at64(40), at64(48))
    } else {
        if at16(pos + 10) == u16::MAX || at32(pos + 12) == u32::MAX || at32(pos + 16) == u32::MAX {
            return Err("ZIP64 locator missing");
        }
        (
            u64::from(at16(pos + 10)),
            u64::from(at32(pos + 12)),
            u64::from(at32(pos + 16)),
        )
    };
    if count == 0
        || count > 100_000
        || size > 32 * 1024 * 1024
        || offset
            .checked_add(size)
            .is_none_or(|end| end > archive_size)
    {
        return Err("ZIP admission metadata exceeds bounds");
    }
    Ok((offset, size as usize))
}
pub fn zip_declaration(central: &[u8]) -> Result<(u64, u32), &'static str> {
    let mut pos = 0;
    let mut bytes = 0_u64;
    let mut files = 0_u32;
    while pos < central.len() {
        let record = central
            .get(pos..pos + 46)
            .ok_or("ZIP central directory truncated")?;
        if !record.starts_with(b"PK\x01\x02") {
            return Err("Invalid ZIP central directory");
        }
        let at16 = |p| u16::from_le_bytes([record[p], record[p + 1]]) as usize;
        let mut size = u64::from(u32::from_le_bytes(record[24..28].try_into().unwrap()));
        let name_len = at16(28);
        let extra = at16(30);
        let comment = at16(32);
        let end = pos
            .checked_add(46 + name_len + extra + comment)
            .ok_or("ZIP metadata overflow")?;
        let name = central
            .get(pos + 46..pos + 46 + name_len)
            .ok_or("ZIP central directory truncated")?;
        if end > central.len() {
            return Err("Truncated ZIP central directory");
        }
        if size == u64::from(u32::MAX) {
            let fields = &central[pos + 46 + name_len..pos + 46 + name_len + extra];
            let mut cursor = 0;
            let mut found = None;
            while cursor + 4 <= fields.len() {
                let id = u16::from_le_bytes(fields[cursor..cursor + 2].try_into().unwrap());
                let length =
                    u16::from_le_bytes(fields[cursor + 2..cursor + 4].try_into().unwrap()) as usize;
                let data = fields
                    .get(cursor + 4..cursor + 4 + length)
                    .ok_or("Truncated ZIP extra field")?;
                if id == 1 {
                    found = Some(u64::from_le_bytes(
                        data.get(..8)
                            .ok_or("ZIP64 size missing")?
                            .try_into()
                            .unwrap(),
                    ));
                    break;
                }
                cursor += 4 + length;
            }
            size = found.ok_or("ZIP64 size missing")?;
        }
        if !name.ends_with(b"/") && size > 512 * 1024 * 1024 {
            return Err("ZIP individual file exceeds 512 MiB");
        }
        if !name.ends_with(b"/") {
            files += 1;
            bytes = bytes.checked_add(size).ok_or("ZIP admission overflow")?;
        }
        if files > 100_000 || bytes > 20 * 1024 * 1024 * 1024 {
            return Err("ZIP library exceeds admission bounds");
        }
        pos = end;
    }
    if files == 0 || bytes == 0 {
        return Err("ZIP library is empty");
    }
    Ok((bytes, files))
}
#[cfg(test)]
mod zip_tests {
    use super::*;
    #[test]
    fn zip_admission_counts_expanded_regular_files_and_rejects_truncation() {
        let mut central = vec![0_u8; 47];
        central[..4].copy_from_slice(b"PK\x01\x02");
        central[24] = 7;
        central[28] = 1;
        central[46] = b'a';
        let mut second = central.clone();
        second[24] = 5;
        second[46] = b'b';
        central.extend(second);
        assert_eq!(zip_declaration(&central).unwrap(), (12, 2));
        assert!(zip_declaration(&central[..90]).is_err());
        central[0] = 0;
        assert!(zip_declaration(&central).is_err());
        assert!(zip_declaration(&[]).is_err());
        let mut end = vec![0_u8; 22];
        end[..4].copy_from_slice(b"PK\x05\x06");
        end[8] = 2;
        end[10] = 2;
        end[12] = 94;
        end[16] = 12;
        assert_eq!(zip_extent(&end, 128).unwrap(), (12, 94));
        assert!(zip_extent(&end, 100).is_err());
        end[10] = 1;
        assert!(zip_extent(&end, 128).is_err());
        end[8] = 255;
        end[9] = 255;
        end[10] = 255;
        end[11] = 255;
        assert!(zip_extent(&end, 128).is_err());
        assert!(zip_extent(&[], 0).is_err());
    }
}
#[cfg(test)]
mod zip64_tests {
    use super::*;
    #[test]
    fn zip64_admission_reads_small_structural_fixture_and_bounded_extras() {
        let mut tail = vec![0_u8; 42];
        tail[..4].copy_from_slice(b"PK\x06\x07");
        tail[16] = 1;
        tail[20..24].copy_from_slice(b"PK\x05\x06");
        tail[28..32].fill(255);
        tail[32..40].fill(255);
        let mut record = vec![0_u8; 56];
        record[..4].copy_from_slice(b"PK\x06\x06");
        record[4] = 44;
        record[24] = 2;
        record[32] = 2;
        record[40] = 94;
        record[48] = 12;
        assert_eq!(zip64_record_offset(&tail, 256).unwrap(), Some(0));
        let mut long_comment = tail.clone();
        long_comment[40] = 255;
        long_comment[41] = 255;
        long_comment.resize(65577, 0);
        assert_eq!(zip64_record_offset(&long_comment, 70000).unwrap(), Some(0));
        assert_eq!(
            zip_extent_with_record(&long_comment, 70000, Some(&record)).unwrap(),
            (12, 94)
        );
        assert_eq!(
            zip_extent_with_record(&tail, 256, Some(&record)).unwrap(),
            (12, 94)
        );
        assert!(zip_extent(&tail, 256).is_err());
        record[16] = 1;
        assert!(zip_extent_with_record(&tail, 256, Some(&record)).is_err());
        let mut central = vec![0_u8; 59];
        central[..4].copy_from_slice(b"PK\x01\x02");
        central[24..28].fill(255);
        central[28] = 1;
        central[30] = 12;
        central[46] = b'a';
        central[47] = 1;
        central[49] = 8;
        central[51] = 7;
        assert_eq!(zip_declaration(&central).unwrap(), (7, 1));
        central[47] = 2;
        assert!(zip_declaration(&central).is_err());
        central[47] = 1;
        central[58] = 255;
        assert!(zip_declaration(&central).is_err());
        tail[16] = 2;
        assert!(zip64_record_offset(&tail, 256).is_err());
    }
}

/// Bound retained relative-path metadata before initiating any upload.
pub fn path_metadata(total: usize, path: &str) -> Result<usize, &'static str> {
    path_hex(path)?;
    let next = total
        .checked_add(path.len())
        .ok_or("Path metadata overflow")?;
    if next > 32 * 1024 * 1024 {
        return Err("Selected paths exceed the 32 MiB metadata budget");
    }
    Ok(next)
}
#[cfg(test)]
mod metadata_tests {
    use super::*;
    #[test]
    fn many_short_files_cannot_amplify_path_metadata() {
        let mut bytes = 0;
        let path = "a".repeat(4096);
        for _ in 0..8192 {
            bytes = path_metadata(bytes, &path).unwrap();
        }
        assert_eq!(bytes, 32 * 1024 * 1024);
        assert!(path_metadata(bytes, "a").is_err());
        assert!(path_metadata(usize::MAX, "a").is_err());
    }
}

/// Validate generated derivative bytes against the authenticated typed response metadata.
pub fn verified_derivative(
    bytes: &[u8],
    derivative: &lytronaut_contracts::FocusDerivative,
) -> bool {
    bytes.len() as u64 == derivative.encoded_bytes
        && format!("{:x}", Sha256::digest(bytes)) == String::from(derivative.content_sha256.clone())
}

#[cfg(test)]
mod derivative_integrity_tests {
    use super::*;
    #[test]
    fn generated_derivative_length_and_digest_reject_corruption_and_truncation() {
        let bytes = b"independent generated JPEG response stand-in";
        let derivative:lytronaut_contracts::FocusDerivative=serde_json::from_value(serde_json::json!({"version":1,"representation":"jpeg_focus_plane","id":"synthetic","plane_index":0,"lambda":0.0,"source_dimensions":{"width":360,"height":360},"output_dimensions":{"width":360,"height":360},"orientation":1,"encoded_bytes":bytes.len(),"content_sha256":format!("{:x}",Sha256::digest(bytes)),"color_space":"unknown","provenance":"Lossy synthetic derivative"})).unwrap();
        assert!(verified_derivative(bytes, &derivative));
        assert!(!verified_derivative(&bytes[..bytes.len() - 1], &derivative));
        let mut corrupt = bytes.to_vec();
        corrupt[0] ^= 1;
        assert!(!verified_derivative(&corrupt, &derivative));
    }
}

/// Only currently available selected controls may supersede a capture request.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) fn selected_control_available(
    video: bool,
    focus_available: bool,
    video_available: bool,
) -> bool {
    if video {
        video_available
    } else {
        focus_available
    }
}

/// One running browse operation and one overwritten, not-yet-started intent.
/// The runner remains owned until its current promises settle, including abort.
#[cfg(any(target_arch = "wasm32", test))]
pub(crate) struct LatestRequest<T> {
    pending: Option<T>,
    running: bool,
}
#[cfg(any(target_arch = "wasm32", test))]
impl<T> Default for LatestRequest<T> {
    fn default() -> Self {
        Self {
            pending: None,
            running: false,
        }
    }
}
#[cfg(any(target_arch = "wasm32", test))]
impl<T> LatestRequest<T> {
    /// True only for the activation which must schedule the single runner.
    pub(crate) fn submit(&mut self, request: T) -> bool {
        self.pending = Some(request);
        if self.running {
            false
        } else {
            self.running = true;
            true
        }
    }
    /// Called only after the previous operation settled; never replace its future.
    pub(crate) fn take_after_settlement(&mut self) -> Option<T> {
        let next = self.pending.take();
        if next.is_none() {
            self.running = false;
        }
        next
    }
    pub(crate) fn clear_pending(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod browse_lifetime_tests {
    use super::{LatestRequest, selected_control_available};
    use std::{cell::Cell, rc::Rc};
    struct Intent {
        index: usize,
        live: Rc<Cell<usize>>,
    }
    impl Intent {
        fn new(index: usize, live: &Rc<Cell<usize>>) -> Self {
            live.set(live.get() + 1);
            Self {
                index,
                live: live.clone(),
            }
        }
    }
    impl Drop for Intent {
        fn drop(&mut self) {
            self.live.set(self.live.get() - 1);
        }
    }
    #[test]
    fn unavailable_focus_and_video_burst_preserves_pending_capture() {
        let mut lane = LatestRequest::default();
        assert!(lane.submit("capture"));
        for _ in 0..10000 {
            if selected_control_available(false, false, false) {
                lane.submit("focus");
            }
            if selected_control_available(true, false, false) {
                lane.submit("video");
            }
        }
        assert_eq!(lane.take_after_settlement(), Some("capture"));
        assert!(lane.take_after_settlement().is_none());
        assert!(selected_control_available(false, true, false));
        assert!(selected_control_available(true, false, true));
        assert!(!selected_control_available(false, false, true));
        assert!(!selected_control_available(true, true, false));
    }
    #[test]
    fn synchronous_burst_has_one_runner_and_drops_superseded_intents() {
        let live = Rc::new(Cell::new(0));
        let mut lane = LatestRequest::default();
        assert!(lane.submit(Intent::new(0, &live)));
        for index in 1..10000 {
            assert!(!lane.submit(Intent::new(index, &live)));
            assert_eq!(live.get(), 1);
        }
        assert_eq!(lane.take_after_settlement().unwrap().index, 9999);
        assert_eq!(live.get(), 0);
        assert!(lane.take_after_settlement().is_none());
        assert!(lane.submit(Intent::new(10000, &live)));
    }
    #[test]
    fn stalled_active_operation_retains_only_latest_pending_until_settlement() {
        let live = Rc::new(Cell::new(0));
        let mut lane = LatestRequest::default();
        assert!(lane.submit(Intent::new(0, &live)));
        let active = lane.take_after_settlement().unwrap();
        for index in 1..10000 {
            assert!(!lane.submit(Intent::new(index, &live)));
            assert_eq!(live.get(), 2);
        }
        assert_eq!(active.index, 0);
        drop(active);
        let replacement = lane.take_after_settlement().unwrap();
        assert_eq!(replacement.index, 9999);
        drop(replacement);
        assert!(lane.take_after_settlement().is_none());
        assert_eq!(live.get(), 0);
    }
    #[test]
    fn disposal_removes_pending_without_releasing_a_stalled_runner() {
        let mut lane = LatestRequest::default();
        assert!(lane.submit(1));
        let active = lane.take_after_settlement();
        assert_eq!(active, Some(1));
        assert!(!lane.submit(2));
        lane.clear_pending();
        // A replacement mount reuses the same draining slot, not a fresh runner.
        assert!(!lane.submit(3));
        assert_eq!(lane.take_after_settlement(), Some(3));
        assert!(lane.take_after_settlement().is_none());
        assert!(lane.submit(4));
    }
}
