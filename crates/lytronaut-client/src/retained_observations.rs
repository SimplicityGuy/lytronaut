//! Feature-only borrowed inventory. Not an allocator, framework-state or heap probe.
use lytronaut_contracts::*;
use std::mem::size_of;

/// Slots: known inline/Vec-slot bytes, nested heap capacities, UTF16 units, string owners.
/// Inline signal values include their headers once; nested items occupy Vec slots.
/// BTree nodes, signal infrastructure and allocation rounding are not measured.
#[derive(Default)]
pub(crate) struct Inventory {
    bytes: [usize; 4],
    overflow: bool,
}
impl Inventory {
    fn add(&mut self, slot: usize, amount: usize) {
        if let Some(total) = self.bytes[slot].checked_add(amount) {
            self.bytes[slot] = total;
        } else {
            self.overflow = true;
        }
    }
    pub(crate) fn value<T: Owned>(&mut self, value: &T) {
        self.add(0, size_of::<T>());
        value.visit(self);
    }
    pub(crate) fn string(&mut self, value: &String) {
        self.add(1, value.capacity());
        self.add(2, value.encode_utf16().count());
        self.add(3, 1);
    }
    fn id(&mut self, value: &OpaqueId) {
        self.add(1, value.harness_capacity());
        self.add(2, value.as_str().len()); // Valid IDs are ASCII.
        self.add(3, 1);
    }
    pub(crate) fn slots<T>(&mut self, value: &Vec<T>) {
        if let Some(bytes) = value.capacity().checked_mul(size_of::<T>()) {
            self.add(0, bytes);
        } else {
            self.overflow = true;
        }
    }
    pub(crate) fn vec<T: Owned>(&mut self, value: &Vec<T>) {
        self.slots(value);
        for item in value {
            item.visit(self);
        }
    }
    pub(crate) fn option<T: Owned>(&mut self, value: &Option<T>) {
        if let Some(value) = value {
            value.visit(self);
        }
    }
    pub(crate) fn emit(&self, stage: &'static str) {
        if self.overflow {
            lytronaut_format::capacity_observations::invalidate();
        }
        lytronaut_format::capacity_observations::record(stage, self.bytes);
    }
}
pub(crate) trait Owned {
    fn visit(&self, inventory: &mut Inventory);
}
impl<T: Owned> Owned for Vec<T> {
    fn visit(&self, i: &mut Inventory) {
        i.vec(self);
    }
}
impl<T: Owned> Owned for Option<T> {
    fn visit(&self, i: &mut Inventory) {
        i.option(self);
    }
}
impl Owned for f64 {
    fn visit(&self, _i: &mut Inventory) {}
}
impl<T: Owned> Owned for std::collections::BTreeMap<String, T> {
    fn visit(&self, i: &mut Inventory) {
        for (key, value) in self {
            i.string(key);
            value.visit(i);
        }
    }
}
impl Owned for String {
    fn visit(&self, i: &mut Inventory) {
        i.string(self);
    }
}
impl Owned for OpaqueId {
    fn visit(&self, i: &mut Inventory) {
        i.id(self);
    }
}
impl Owned for LibrarySummary {
    fn visit(&self, i: &mut Inventory) {
        i.id(&self.id);
    }
}
impl Owned for AlbumSummary {
    fn visit(&self, i: &mut Inventory) {
        i.id(&self.id);
        i.string(&self.name);
    }
}
impl Owned for CaptureSummary {
    fn visit(&self, i: &mut Inventory) {
        i.id(&self.id);
        i.string(&self.name);
    }
}
impl Owned for PreservedFile {
    fn visit(&self, i: &mut Inventory) {
        i.id(&self.id);
    }
}
impl Owned for CaptureDetail {
    fn visit(&self, i: &mut Inventory) {
        self.capture.visit(i);
        i.string(&self.caption);
        i.option(&self.preview);
        i.slots(&self.item_errors); // Errors/capabilities contain only inline enums.
        i.vec(&self.originals);
    }
}
impl Owned for FocusStack {
    fn visit(&self, i: &mut Inventory) {
        i.id(&self.id);
        i.slots(&self.lambdas);
        i.string(&self.orientation_policy);
    }
}
impl Owned for FocusDerivative {
    fn visit(&self, i: &mut Inventory) {
        i.id(&self.id);
        i.string(&self.provenance);
        i.add(1, self.content_sha256.harness_capacity());
        i.add(2, 64); // Valid content digests are exactly64 ASCII characters.
        i.add(3, 1);
    }
}
impl Owned for ExportProgress {
    fn visit(&self, i: &mut Inventory) {
        i.id(&self.job.id);
    }
}
/// Target-dependent layout, not allocator rounding or BTree node layout.
pub(crate) fn record_layout() {
    use lytronaut_format::{Plane, Processed, Reference, Section};
    for (stage, bytes) in [
        (
            "retained_layout_strings",
            [
                size_of::<usize>(),
                size_of::<String>(),
                size_of::<Vec<u8>>(),
                size_of::<std::collections::BTreeMap<String, String>>(),
            ],
        ),
        (
            "retained_layout_lists",
            [
                size_of::<LibrarySummary>(),
                size_of::<AlbumSummary>(),
                size_of::<CaptureSummary>(),
                size_of::<PreservedFile>(),
            ],
        ),
        (
            "retained_layout_detail",
            [
                size_of::<CaptureDetail>(),
                size_of::<FocusStack>(),
                size_of::<FocusDerivative>(),
                size_of::<ExportProgress>(),
            ],
        ),
        (
            "retained_layout_metadata",
            [
                size_of::<serde_json::Value>(),
                size_of::<Plane<'_>>(),
                size_of::<Processed<'_>>(),
                size_of::<Section<'_>>(),
            ],
        ),
        (
            "retained_layout_options",
            [
                size_of::<Option<CaptureDetail>>(),
                size_of::<Option<FocusStack>>(),
                size_of::<Reference>(),
                size_of::<ItemError>(),
            ],
        ),
    ] {
        lytronaut_format::capacity_observations::record(stage, bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn borrowed_inventory_counts_reserved_slots_and_distinct_string_owners() {
        let mut name = String::with_capacity(4096);
        name.push_str("public");
        let mut values = Vec::with_capacity(17);
        values.push(name);
        let mut i = Inventory::default();
        i.vec(&values);
        let selected = Some(values[0].clone());
        i.option(&selected); // Same contents but a distinct actual allocation.
        assert_eq!(i.bytes[0], values.capacity() * size_of::<String>());
        assert_eq!(
            i.bytes[1],
            values[0].capacity() + selected.as_ref().unwrap().capacity()
        );
        assert_eq!(i.bytes[2], 12);
        assert_eq!(i.bytes[3], 2);
        assert!(!i.overflow);
    }
    #[test]
    fn empty_options_and_reserved_empty_vectors_keep_their_actual_ownership() {
        let mut i = Inventory::default();
        i.option(&None::<String>);
        let values = Vec::<String>::with_capacity(9);
        i.vec(&values);
        assert_eq!(i.bytes, [values.capacity() * size_of::<String>(), 0, 0, 0]);
        let mut text = String::with_capacity(128);
        text.push('🦀');
        i.string(&text);
        assert_eq!(i.bytes[1], text.capacity());
        assert_eq!(i.bytes[2], 2);
    }
    #[test]
    fn overflow_invalidates_partial_evidence_instead_of_wrapping() {
        lytronaut_format::capacity_observations::start();
        let mut i = Inventory::default();
        i.add(1, usize::MAX);
        i.add(1, 1);
        i.emit("retained_overflow_test");
        let report = lytronaut_format::capacity_observations::take();
        assert!(!report.complete);
        assert_eq!(report.dropped, 1);
        assert_eq!(report.observations[0].bytes[1], usize::MAX);
    }
    #[test]
    fn target_layout_and_root_headers_are_recorded_without_contents() {
        lytronaut_format::capacity_observations::start();
        record_layout();
        let mut i = Inventory::default();
        i.value(&None::<String>);
        i.emit("retained_empty_root_test");
        let report = lytronaut_format::capacity_observations::take();
        assert!(report.complete);
        assert_eq!(report.observations.len(), 6);
        assert_eq!(report.observations[0].bytes[0], size_of::<usize>());
        assert_eq!(report.observations[0].bytes[1], size_of::<String>());
        assert_eq!(
            report.observations[5].bytes,
            [size_of::<Option<String>>(), 0, 0, 0]
        );
    }
    #[test]
    fn private_wrappers_report_capacity_without_exposing_contents() {
        let mut id = String::with_capacity(1024);
        id.push_str("public-id");
        let id = OpaqueId::try_from(id).unwrap();
        assert!(id.harness_capacity() >= 1024);
        let mut hash = String::with_capacity(2048);
        hash.push_str(&"0".repeat(64));
        let hash = ContentDigest::try_from(hash).unwrap();
        assert!(hash.harness_capacity() >= 2048);
    }
}
