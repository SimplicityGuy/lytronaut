//! FIFO working admission shared by the page's retained task owners.
use crate::model::{Admission, VIEWER_BUDGET};
use std::{
    cell::RefCell,
    collections::VecDeque,
    future::Future,
    pin::Pin,
    rc::Rc,
    task::{Context, Poll, Waker},
};

// Six browse runners and four action guards each await at most one lease.
// Remounts reuse those owners. Refuse overflow even if a future caller violates
// that invariant; neither pending work nor cancellation listeners may grow.
pub(crate) const MAX_WAITERS: usize = 10;

#[derive(Default)]
struct WakeSlot(RefCell<Option<Waker>>);
impl WakeSlot {
    fn wake(&self) {
        let wake = self.0.borrow_mut().take();
        if let Some(wake) = wake {
            wake.wake();
        }
    }
}
#[derive(Clone)]
pub(crate) struct AdmissionWake(Rc<WakeSlot>);
impl AdmissionWake {
    pub(crate) fn wake(&self) {
        self.0.wake();
    }
}
struct Entry {
    id: u64,
    wake: Rc<WakeSlot>,
}
#[derive(Default)]
struct State {
    active: Admission,
    next: u64,
    pending: VecDeque<Entry>,
}
#[derive(Clone, Default)]
pub(crate) struct AdmissionQueue(Rc<RefCell<State>>);
impl AdmissionQueue {
    pub(crate) fn wait<F: Fn() -> bool>(
        &self,
        bytes: u64,
        cancelled: F,
    ) -> Result<Acquire<F>, &'static str> {
        if bytes > VIEWER_BUDGET {
            return Err("Request exceeds browser memory budget");
        }
        if cancelled() {
            return Err("Cancelled");
        }
        let wake = Rc::new(WakeSlot::default());
        let id = {
            let mut state = self.0.borrow_mut();
            if state.pending.len() >= MAX_WAITERS {
                return Err("Browser admission queue is full");
            }
            let id = state.next;
            state.next = state
                .next
                .checked_add(1)
                .ok_or("Browser admission identity exhausted")?;
            state.pending.push_back(Entry {
                id,
                wake: wake.clone(),
            });
            id
        };
        Ok(Acquire {
            queue: self.clone(),
            ticket: Some(id),
            bytes,
            cancelled,
            wake,
        })
    }
    fn remove(&self, id: u64) {
        let next = {
            let mut state = self.0.borrow_mut();
            if let Some(index) = state.pending.iter().position(|entry| entry.id == id) {
                state.pending.remove(index);
            }
            state.pending.front().map(|entry| entry.wake.clone())
        };
        if let Some(next) = next {
            next.wake();
        }
    }
}
pub(crate) struct Acquire<F> {
    queue: AdmissionQueue,
    ticket: Option<u64>,
    bytes: u64,
    cancelled: F,
    wake: Rc<WakeSlot>,
}
impl<F> Unpin for Acquire<F> {}
impl<F> Acquire<F> {
    pub(crate) fn wake_handle(&self) -> AdmissionWake {
        AdmissionWake(self.wake.clone())
    }
}
impl<F: Fn() -> bool> Future for Acquire<F> {
    type Output = Result<AdmissionLease, &'static str>;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        // Clone/drop arbitrary wakers and run cancellation callbacks outside all
        // queue borrows. The AbortSignal listener is already installed. Recheck
        // cancellation before atomically registering this waker and checking
        // FIFO position/capacity; no JS callback or await occurs in that borrow.
        let new_wake = cx.waker().clone();
        if (this.cancelled)() {
            if let Some(id) = this.ticket.take() {
                this.queue.remove(id);
            }
            return Poll::Ready(Err("Cancelled"));
        }
        let (granted, next, old_wake) = {
            let mut state = this.queue.0.borrow_mut();
            let old_wake = this.wake.0.replace(Some(new_wake));
            let head = state.pending.front().map(|entry| entry.id);
            if this.ticket.is_some() && head == this.ticket && state.active.acquire(this.bytes) {
                state.pending.pop_front();
                (
                    true,
                    state.pending.front().map(|entry| entry.wake.clone()),
                    old_wake,
                )
            } else {
                (false, None, old_wake)
            }
        };
        if !granted {
            drop(old_wake);
            if (this.cancelled)() {
                if let Some(id) = this.ticket.take() {
                    this.queue.remove(id);
                }
                return Poll::Ready(Err("Cancelled"));
            }
            return Poll::Pending;
        }
        this.ticket = None;
        // Capacity always has an owner before any reentrant wake. No reservation
        // is made for an unpolled future or delivered by a cancellation callback.
        let lease = AdmissionLease {
            queue: this.queue.clone(),
            bytes: this.bytes,
        };
        drop(old_wake);
        if let Some(next) = next {
            next.wake();
        }
        if (this.cancelled)() {
            return Poll::Ready(Err("Cancelled"));
        }
        Poll::Ready(Ok(lease))
    }
}

impl<F> Drop for Acquire<F> {
    fn drop(&mut self) {
        if let Some(id) = self.ticket.take() {
            self.queue.remove(id);
        }
    }
}
pub(crate) struct AdmissionLease {
    queue: AdmissionQueue,
    bytes: u64,
}
impl Drop for AdmissionLease {
    fn drop(&mut self) {
        let next = {
            let mut state = self.queue.0.borrow_mut();
            state.active.release(self.bytes);
            state.pending.front().map(|entry| entry.wake.clone())
        };
        if let Some(next) = next {
            next.wake();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::Cell,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::Wake,
    };
    struct Counter(AtomicUsize);
    impl Wake for Counter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    fn context() -> (Arc<Counter>, Waker) {
        let count = Arc::new(Counter(AtomicUsize::new(0)));
        (count.clone(), Waker::from(count))
    }
    fn poll<F: Fn() -> bool>(
        future: &mut Acquire<F>,
        wake: &Waker,
    ) -> Poll<Result<AdmissionLease, &'static str>> {
        Pin::new(future).poll(&mut Context::from_waker(wake))
    }
    fn grant<F: Fn() -> bool>(future: &mut Acquire<F>, wake: &Waker) -> AdmissionLease {
        match poll(future, wake) {
            Poll::Ready(Ok(lease)) => lease,
            _ => panic!("expected grant"),
        }
    }
    #[test]
    fn selection_cannot_be_starved_by_continuous_new_thumbnail_arrivals() {
        // The old memoryless policy repeatedly admits the newest thumbnail
        // while selection is asleep, despite capacity becoming free.
        let mut old = Admission::default();
        for _ in 0..24 {
            assert!(old.acquire(VIEWER_BUDGET));
            old.release(VIEWER_BUDGET);
        }
        let q = AdmissionQueue::default();
        let (_, w) = context();
        let active = grant(&mut q.wait(VIEWER_BUDGET, || false).unwrap(), &w);
        let mut selected = q.wait(VIEWER_BUDGET, || false).unwrap();
        assert!(poll(&mut selected, &w).is_pending());
        drop(active);
        for _ in 0..24 {
            let mut newer = q.wait(1, || false).unwrap();
            assert!(poll(&mut newer, &w).is_pending());
        }
        let selected = grant(&mut selected, &w);
        assert!(!q.0.borrow_mut().active.acquire(1));
        drop(selected);
        assert!(q.0.borrow_mut().active.acquire(VIEWER_BUDGET));
    }
    #[test]
    fn fifo_blocks_compatible_younger_request_behind_full_budget_head() {
        let q = AdmissionQueue::default();
        let (_, w) = context();
        let active = grant(&mut q.wait(1, || false).unwrap(), &w);
        let mut head = q.wait(VIEWER_BUDGET, || false).unwrap();
        let mut younger = q.wait(1, || false).unwrap();
        assert!(poll(&mut head, &w).is_pending());
        assert!(poll(&mut younger, &w).is_pending());
        drop(active);
        assert!(poll(&mut younger, &w).is_pending());
        let head = grant(&mut head, &w);
        drop(head);
        drop(grant(&mut younger, &w));
    }
    #[test]
    fn two_slots_and_bytes_stay_owned_until_real_lease_drop() {
        let q = AdmissionQueue::default();
        let (_, w) = context();
        let a = grant(&mut q.wait(VIEWER_BUDGET - 1, || false).unwrap(), &w);
        let b = grant(&mut q.wait(1, || false).unwrap(), &w);
        let cancelled = Rc::new(Cell::new(false));
        let flag = cancelled.clone();
        let mut next = q.wait(0, move || flag.get()).unwrap();
        assert!(poll(&mut next, &w).is_pending());
        cancelled.set(true);
        assert!(matches!(poll(&mut next, &w), Poll::Ready(Err("Cancelled"))));
        let mut other = q.wait(1, || false).unwrap();
        assert!(poll(&mut other, &w).is_pending());
        drop(b);
        drop(grant(&mut other, &w));
        drop(a);
        drop(grant(&mut q.wait(VIEWER_BUDGET, || false).unwrap(), &w));
    }
    #[test]
    fn bound_and_oversize_refuse_before_enqueuing_or_consuming_identity() {
        let q = AdmissionQueue::default();
        assert!(q.wait(VIEWER_BUDGET + 1, || false).is_err());
        assert!(q.wait(1, || true).is_err());
        assert_eq!(q.0.borrow().next, 0);
        let waiters = (0..MAX_WAITERS)
            .map(|_| q.wait(1, || false).unwrap())
            .collect::<Vec<_>>();
        assert!(q.wait(1, || false).is_err());
        assert_eq!(q.0.borrow().pending.len(), MAX_WAITERS);
        drop(waiters);
        assert!(q.0.borrow().pending.is_empty());
    }
    #[test]
    fn head_and_middle_drop_wake_successor_with_zero_active_leases() {
        let q = AdmissionQueue::default();
        let (count, w) = context();
        let head = q.wait(1, || false).unwrap();
        let middle = q.wait(1, || false).unwrap();
        let mut tail = q.wait(1, || false).unwrap();
        assert!(poll(&mut tail, &w).is_pending());
        drop(middle);
        drop(head);
        assert_eq!(count.0.load(Ordering::Relaxed), 1);
        drop(grant(&mut tail, &w));
    }
    #[test]
    fn deferred_cancel_wakes_pending_head_and_never_releases_active_drain() {
        let q = AdmissionQueue::default();
        let (count, w) = context();
        let active = grant(&mut q.wait(VIEWER_BUDGET, || false).unwrap(), &w);
        let flag = Rc::new(Cell::new(false));
        let cancelled = flag.clone();
        let mut head = q.wait(1, move || cancelled.get()).unwrap();
        assert!(poll(&mut head, &w).is_pending());
        let native_abort = head.wake_handle();
        flag.set(true); // Rust supersession precedes native abort's microtask.
        native_abort.wake();
        native_abort.wake(); // repeated/stale wake is harmless.
        assert_eq!(count.0.load(Ordering::Relaxed), 1);
        assert!(matches!(poll(&mut head, &w), Poll::Ready(Err("Cancelled"))));
        let mut tail = q.wait(1, || false).unwrap();
        assert!(poll(&mut tail, &w).is_pending());
        drop(active);
        drop(grant(&mut tail, &w));
    }
    #[test]
    fn abort_after_registration_before_first_poll_and_drop_after_grant_do_not_leak() {
        let q = AdmissionQueue::default();
        let (_, w) = context();
        let flag = Rc::new(Cell::new(false));
        let cancelled = flag.clone();
        let mut head = q.wait(VIEWER_BUDGET, move || cancelled.get()).unwrap();
        flag.set(true);
        head.wake_handle().wake();
        assert!(matches!(poll(&mut head, &w), Poll::Ready(Err("Cancelled"))));
        let mut next = q.wait(VIEWER_BUDGET, || false).unwrap();
        // Dropping the ready output before a caller uses it releases exactly once.
        drop(poll(&mut next, &w));
        drop(next);
        drop(grant(&mut q.wait(VIEWER_BUDGET, || false).unwrap(), &w));
    }

    thread_local! {
        static REENTER: RefCell<Option<Box<dyn FnOnce()>>> = RefCell::new(None);
    }
    struct Reentrant;
    impl Wake for Reentrant {
        fn wake(self: Arc<Self>) {
            let callback = REENTER.with(|slot| slot.borrow_mut().take());
            if let Some(callback) = callback {
                callback();
            }
        }
    }
    #[test]
    fn cancellation_during_grant_wakeup_releases_once_without_held_borrows() {
        let q = AdmissionQueue::default();
        let (_, w) = context();
        let flag = Rc::new(Cell::new(false));
        let cancelled = flag.clone();
        let mut head = q.wait(VIEWER_BUDGET, move || cancelled.get()).unwrap();
        let mut tail = q.wait(VIEWER_BUDGET, || false).unwrap();
        let reentrant = Waker::from(Arc::new(Reentrant));
        assert!(poll(&mut tail, &reentrant).is_pending());
        let observed = q.clone();
        REENTER.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                // A grant's successor wake may re-enter cancellation immediately.
                assert!(observed.0.try_borrow_mut().is_ok());
                flag.set(true);
            }))
        });
        assert!(matches!(poll(&mut head, &w), Poll::Ready(Err("Cancelled"))));
        drop(head);
        drop(grant(&mut tail, &w));
        drop(grant(&mut q.wait(VIEWER_BUDGET, || false).unwrap(), &w));
    }

    #[test]
    fn cancellation_between_enqueue_and_first_poll_removes_head() {
        let q = AdmissionQueue::default();
        let (_, w) = context();
        let calls = Cell::new(0);
        let mut first = q
            .wait(1, || {
                calls.set(calls.get() + 1);
                calls.get() > 1
            })
            .unwrap();
        assert!(matches!(
            poll(&mut first, &w),
            Poll::Ready(Err("Cancelled"))
        ));
        assert!(q.0.borrow().pending.is_empty());
        drop(grant(&mut q.wait(VIEWER_BUDGET, || false).unwrap(), &w));
    }

    struct DropWake;
    impl Wake for DropWake {
        fn wake(self: Arc<Self>) {
            panic!("drop-only waker unexpectedly invoked");
        }
    }
    impl Drop for DropWake {
        fn drop(&mut self) {
            let callback = REENTER.with(|slot| slot.borrow_mut().take());
            if let Some(callback) = callback {
                callback();
            }
        }
    }
    #[test]
    fn replaced_waker_drop_cancels_pending_head_outside_all_borrows() {
        let q = AdmissionQueue::default();
        let (_, w) = context();
        let active = grant(&mut q.wait(VIEWER_BUDGET, || false).unwrap(), &w);
        let flag = Rc::new(Cell::new(false));
        let cancelled = flag.clone();
        let mut head = q.wait(1, move || cancelled.get()).unwrap();
        let old = Waker::from(Arc::new(DropWake));
        assert!(poll(&mut head, &old).is_pending());
        drop(old);
        let observed = q.clone();
        REENTER.with(|slot| {
            *slot.borrow_mut() = Some(Box::new(move || {
                assert!(observed.0.try_borrow_mut().is_ok());
                flag.set(true);
            }))
        });
        assert!(matches!(poll(&mut head, &w), Poll::Ready(Err("Cancelled"))));
        let mut next = q.wait(VIEWER_BUDGET, || false).unwrap();
        assert!(poll(&mut next, &w).is_pending());
        drop(active);
        drop(grant(&mut next, &w));
    }
}
