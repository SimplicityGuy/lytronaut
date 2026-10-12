//! Opt-in partial buffer observations, never an allocator or total-memory proof.
//! Fixed stage names and numeric capacities only; no source identities or contents.
use std::cell::RefCell;
const CAP: usize = 1024;
#[derive(Clone, Copy, serde::Serialize)]
pub struct Observation {
    pub stage: &'static str,
    pub bytes: [usize; 4],
}
#[derive(serde::Serialize)]
pub struct Report {
    pub observations: Vec<Observation>,
    pub dropped: usize,
    pub complete: bool,
    pub observer_storage_bytes: usize,
}
struct Buffer {
    enabled: bool,
    entries: [Option<Observation>; CAP],
    count: usize,
    dropped: usize,
}
thread_local! {
    static BUFFER: RefCell<Buffer> = const { RefCell::new(Buffer {
        enabled: false, entries: [None; CAP], count: 0, dropped: 0,
    }) };
}
/// Constant-time guard: disabled observers must not walk metadata or its strings.
pub fn enabled() -> bool {
    BUFFER.with(|b| b.borrow().enabled)
}
/// Start explicitly: ordinary feature builds do not record until requested.
pub fn start() {
    BUFFER.with(|b| {
        *b.borrow_mut() = Buffer {
            enabled: true,
            entries: [None; CAP],
            count: 0,
            dropped: 0,
        }
    });
}
pub fn record(stage: &'static str, bytes: [usize; 4]) {
    BUFFER.with(|b| {
        let mut b = b.borrow_mut();
        if !b.enabled {
            return;
        }
        let index = b.count;
        if index == CAP {
            b.dropped = b.dropped.saturating_add(1);
        } else {
            b.entries[index] = Some(Observation { stage, bytes });
            b.count += 1;
        }
    });
}
/// Arithmetic overflow in a partial observer is incomplete evidence, never zero.
pub fn invalidate() {
    BUFFER.with(|b| {
        let mut b = b.borrow_mut();
        if b.enabled {
            b.dropped = b.dropped.saturating_add(1);
        }
    });
}
/// Stop before allocating the report. Overflow explicitly invalidates completeness.
pub fn take() -> Report {
    BUFFER.with(|b| {
        let mut b = b.borrow_mut();
        b.enabled = false;
        Report {
            observations: b.entries[..b.count].iter().flatten().copied().collect(),
            dropped: b.dropped,
            complete: b.dropped == 0,
            observer_storage_bytes: std::mem::size_of::<Buffer>(),
        }
    })
}

/// Completed owned JSON only; serde scratch, map nodes and growth temporaries are absent.
#[derive(Default)]
struct MetadataInventory {
    storage: [usize; 4],
    counts: [usize; 4],
    invalid: bool,
}
impl MetadataInventory {
    fn add(slot: &mut usize, value: usize, invalid: &mut bool) {
        if let Some(total) = slot.checked_add(value) {
            *slot = total;
        } else {
            *invalid = true;
        }
    }
    fn multiply(value: usize, size: usize, invalid: &mut bool) -> usize {
        match value.checked_mul(size) {
            Some(bytes) => bytes,
            None => {
                *invalid = true;
                0
            }
        }
    }
    fn string(&mut self, value: &String) {
        Self::add(&mut self.storage[1], value.capacity(), &mut self.invalid);
        Self::add(&mut self.counts[3], 1, &mut self.invalid);
    }
    fn walk(&mut self, value: &serde_json::Value, depth: usize) {
        if depth > 64 {
            self.invalid = true;
            return;
        }
        self.storage[3] = self.storage[3].max(depth);
        Self::add(&mut self.counts[0], 1, &mut self.invalid);
        match value {
            serde_json::Value::String(value) => self.string(value),
            serde_json::Value::Array(items) => {
                let slots = Self::multiply(
                    items.capacity(),
                    std::mem::size_of::<serde_json::Value>(),
                    &mut self.invalid,
                );
                Self::add(&mut self.storage[0], slots, &mut self.invalid);
                Self::add(&mut self.counts[2], 1, &mut self.invalid);
                for item in items {
                    self.walk(item, depth + 1);
                }
            }
            serde_json::Value::Object(items) => {
                // Logical occupied key/value slots, NOT BTree allocation/node slack.
                let slots = Self::multiply(
                    items.len(),
                    std::mem::size_of::<String>() + std::mem::size_of::<serde_json::Value>(),
                    &mut self.invalid,
                );
                Self::add(&mut self.storage[2], slots, &mut self.invalid);
                Self::add(&mut self.counts[1], items.len(), &mut self.invalid);
                for (key, item) in items {
                    self.string(key);
                    self.walk(item, depth + 1);
                }
            }
            _ => {}
        }
    }
    fn emit(&self) {
        if self.invalid {
            invalidate();
        }
        record("metadata_json_storage", self.storage);
        record("metadata_json_counts", self.counts);
    }
}
/// Only visits borrowed metadata when explicitly enabled. No contents are emitted.
pub fn metadata_json(value: &serde_json::Value) {
    if !enabled() {
        return;
    }
    let mut inventory = MetadataInventory::default();
    inventory.storage[0] = std::mem::size_of::<serde_json::Value>();
    inventory.walk(value, 0);
    inventory.emit();
}
/// Exact validated reference-list lifetime; no retained references into plane parsing.
pub fn metadata_references(refs: &Vec<crate::Reference>) {
    if !enabled() {
        return;
    }
    let mut invalid = false;
    let slots = MetadataInventory::multiply(
        refs.capacity(),
        std::mem::size_of::<crate::Reference>(),
        &mut invalid,
    );
    let mut pointer_capacity = 0;
    let mut identity_capacity = 0;
    for item in refs {
        MetadataInventory::add(&mut pointer_capacity, item.pointer.capacity(), &mut invalid);
        MetadataInventory::add(
            &mut identity_capacity,
            item.identity.capacity(),
            &mut invalid,
        );
    }
    if invalid {
        invalidate();
    }
    record(
        "metadata_references",
        [slots, pointer_capacity, identity_capacity, refs.len()],
    );
}
/// Plane descriptors own their Vec slots; payload slices and identities alias source.
pub fn metadata_planes(planes: &Vec<crate::Plane<'_>>, maps: &Vec<crate::Plane<'_>>) {
    if !enabled() {
        return;
    }
    let mut invalid = false;
    let plane_slots = MetadataInventory::multiply(
        planes.capacity(),
        std::mem::size_of::<crate::Plane<'_>>(),
        &mut invalid,
    );
    let map_slots = MetadataInventory::multiply(
        maps.capacity(),
        std::mem::size_of::<crate::Plane<'_>>(),
        &mut invalid,
    );
    if invalid {
        invalidate();
    }
    record(
        "metadata_planes",
        [plane_slots, map_slots, planes.len(), maps.len()],
    );
}
/// Immediately before parser-owner drops. Source length is borrowed, not a capacity.
pub fn metadata_export(source: &[u8], sections: &Vec<crate::Section<'_>>, lambdas: &Vec<f64>) {
    if !enabled() {
        return;
    }
    let mut invalid = false;
    let section_slots = MetadataInventory::multiply(
        sections.capacity(),
        std::mem::size_of::<crate::Section<'_>>(),
        &mut invalid,
    );
    let lambda_slots =
        MetadataInventory::multiply(lambdas.capacity(), std::mem::size_of::<f64>(), &mut invalid);
    if invalid {
        invalidate();
    }
    record(
        "metadata_export",
        [source.len(), section_slots, lambda_slots, lambdas.len()],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn explicit_start_reset_and_stop_preserve_evidence_scope() {
        start();
        take();
        record("ignored", [1; 4]);
        assert!(take().observations.is_empty());
        start();
        record("source", [8, 4, 0, 0]);
        let report = take();
        assert_eq!(report.observations.len(), 1);
        assert_eq!(report.observations[0].bytes, [8, 4, 0, 0]);
        assert!(report.complete);
        record("late", [2; 4]);
        assert_eq!(take().observations.len(), 1);
        start();
        assert!(take().observations.is_empty());
    }
    #[test]
    fn overflow_is_bounded_and_invalidates_completeness() {
        start();
        for _ in 0..CAP + 3 {
            record("strip", [128; 4]);
        }
        let report = take();
        assert_eq!(report.observations.len(), CAP);
        assert_eq!(report.dropped, 3);
        assert!(!report.complete);
        assert_eq!(report.observer_storage_bytes, std::mem::size_of::<Buffer>());
        start();
        assert!(take().complete);
    }
}

#[cfg(test)]
mod metadata_tests {
    use super::*;
    use crate::{FormatError, Limits, Reference, ReferenceLocation, bounded_json};
    use serde_json::{Map, Value};
    use std::mem::size_of;

    #[test]
    fn reserved_nested_json_counts_actual_capacity_and_never_contents() {
        let mut text = String::with_capacity(1024);
        text.push_str("public-content-not-in-report");
        let mut key = String::with_capacity(512);
        key.push_str("public-key-not-in-report");
        let mut nested = String::with_capacity(256);
        nested.push('x');
        let expected_strings = text.capacity() + key.capacity() + nested.capacity();
        let mut object = Map::new();
        object.insert(key, Value::String(nested));
        let mut array = Vec::with_capacity(17);
        array.push(Value::String(text));
        array.push(Value::Object(object));
        let expected_slots = size_of::<Value>() + array.capacity() * size_of::<Value>();
        let value = Value::Array(array);
        start();
        metadata_json(&value);
        let report = take();
        assert!(report.complete);
        assert_eq!(
            report.observations[0].bytes,
            [
                expected_slots,
                expected_strings,
                size_of::<String>() + size_of::<Value>(),
                2
            ]
        );
        assert_eq!(report.observations[1].bytes, [4, 1, 1, 3]);
        let encoded = serde_json::to_string(&report).unwrap();
        assert!(!encoded.contains("public-content-not-in-report"));
        assert!(!encoded.contains("public-key-not-in-report"));
    }
    #[test]
    fn reserved_reference_plane_and_export_tables_exclude_payload_aliases() {
        let mut pointer = String::with_capacity(512);
        pointer.push_str("/public");
        let mut identity = String::with_capacity(256);
        identity.push_str("public");
        let pointer_capacity = pointer.capacity();
        let identity_capacity = identity.capacity();
        let mut refs = Vec::with_capacity(9);
        refs.push(Reference {
            pointer,
            identity,
            location: ReferenceLocation::Local,
        });
        let source = [0; 8192];
        let mut planes = Vec::with_capacity(13);
        for frame_index in 0..2 {
            planes.push(crate::Plane {
                block_index: 0,
                frame_index,
                geometry: crate::Geometry {
                    width: 32,
                    height: 32,
                },
                lambda: Some(frame_index as f64),
                representation: crate::Representation::H264,
                payload: &source,
                section_identity: "public-borrowed-identity",
            });
        }
        let maps = Vec::with_capacity(7);
        let sections = Vec::with_capacity(3);
        let lambdas = Vec::with_capacity(11);
        start();
        metadata_references(&refs);
        metadata_planes(&planes, &maps);
        metadata_export(b"public-borrowed-source", &sections, &lambdas);
        let report = take();
        assert!(report.complete);
        assert_eq!(
            report.observations[0].bytes,
            [
                refs.capacity() * size_of::<Reference>(),
                pointer_capacity,
                identity_capacity,
                1
            ]
        );
        assert_eq!(
            report.observations[1].bytes,
            [
                planes.capacity() * size_of::<crate::Plane<'_>>(),
                maps.capacity() * size_of::<crate::Plane<'_>>(),
                2,
                0
            ]
        );
        assert_eq!(
            report.observations[2].bytes,
            [
                b"public-borrowed-source".len(),
                sections.capacity() * size_of::<crate::Section<'_>>(),
                lambdas.capacity() * size_of::<f64>(),
                0
            ]
        );
    }
    #[test]
    fn wide_and_escaped_bounded_json_is_observed_without_altering_it() {
        let mut object = Map::new();
        for index in 0..32 {
            object.insert(
                format!("public-{index}"),
                Value::String("line\nquote\"backslash\\".into()),
            );
        }
        let encoded = serde_json::to_vec(&Value::Object(object)).unwrap();
        let parsed = bounded_json(&encoded, Limits::default()).unwrap();
        start();
        metadata_json(&parsed);
        let report = take();
        assert!(report.complete);
        assert_eq!(report.observations[1].bytes, [33, 32, 0, 64]);
        assert_eq!(serde_json::to_vec(&parsed).unwrap(), encoded);
    }
    #[test]
    fn helper_depth_is_bounded_and_disabled_observers_emit_nothing() {
        let mut value = Value::Null;
        for _ in 0..64 {
            value = Value::Array(vec![value]);
        }
        start();
        metadata_json(&value);
        assert!(take().complete);
        let overdeep = Value::Array(vec![value]);
        start();
        metadata_json(&overdeep);
        let invalid = take();
        assert!(!invalid.complete);
        assert_eq!(invalid.dropped, 1);
        start();
        take();
        assert!(!enabled());
        metadata_json(&overdeep);
        metadata_references(&Vec::new());
        metadata_planes(&Vec::new(), &Vec::new());
        metadata_export(&[], &Vec::new(), &Vec::new());
        let disabled = take();
        assert!(disabled.complete);
        assert!(disabled.observations.is_empty());
    }
    #[test]
    fn existing_byte_node_and_depth_rejections_are_unchanged() {
        let limits = Limits::default();
        let exact = b"[0]";
        let admitted = Limits {
            json_bytes: exact.len(),
            ..limits
        };
        assert!(bounded_json(exact, admitted).is_ok());
        assert_eq!(
            bounded_json(
                exact,
                Limits {
                    json_bytes: exact.len() - 1,
                    ..limits
                }
            ),
            Err(FormatError::Budget)
        );
        assert!(
            bounded_json(
                b"[0,0,0]",
                Limits {
                    json_nodes: 3,
                    ..limits
                }
            )
            .is_ok()
        );
        assert_eq!(
            bounded_json(
                b"[0,0,0,0]",
                Limits {
                    json_nodes: 3,
                    ..limits
                }
            ),
            Err(FormatError::Budget)
        );
        let accepted = format!("{}0{}", "[".repeat(64), "]".repeat(64));
        assert!(bounded_json(accepted.as_bytes(), limits).is_ok());
        let rejected = format!("[{accepted}]");
        assert_eq!(
            bounded_json(rejected.as_bytes(), limits),
            Err(FormatError::Budget)
        );
    }
    #[test]
    fn checked_arithmetic_overflow_makes_the_metadata_report_incomplete() {
        start();
        let mut inventory = MetadataInventory::default();
        inventory.storage[1] = usize::MAX;
        MetadataInventory::add(&mut inventory.storage[1], 1, &mut inventory.invalid);
        assert_eq!(
            MetadataInventory::multiply(usize::MAX, 2, &mut inventory.invalid),
            0
        );
        inventory.emit();
        let report = take();
        assert!(!report.complete);
        assert_eq!(report.dropped, 1);
        assert_eq!(report.observations[0].bytes[1], usize::MAX);
    }
}
