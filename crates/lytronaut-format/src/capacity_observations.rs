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
