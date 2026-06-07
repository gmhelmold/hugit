//! Serialization + back-pressure around the single-writer append point.
//!
//! # The single serialization point
//!
//! In production exactly **one Durable Object per repo** owns the [`EventLog`]
//! and is the only writer. Concurrent operations are ordered *through* that one
//! append point — there is no second writer to reconcile, so serialization is
//! **structural**, not a lock-juggle across competing writers. This module
//! models that single-writer discipline in-process: a [`Serializer`] wraps one
//! [`EventLog`] behind a single mutex, and every operation — however many call
//! concurrently — passes through that one critical section, one at a time, in a
//! total order. The hash chain that D1a sealed is exactly what makes the order
//! observable and tamper-evident.
//!
//! # Zero loss under contention
//!
//! Every **accepted** operation produces exactly one [`EventRecord`] on the
//! chain: `records_appended == ops_accepted`, always. When the writer is
//! saturated this module applies **back-pressure, never silent drop** — an
//! admission gate ([`Serializer::with_capacity`]) bounds the number of
//! operations queued at the writer; an operation over that bound is **rejected
//! explicitly** with [`SubmitError::Backpressure`] (the caller learns it was
//! not accepted) rather than disappearing. An op is therefore in exactly one of
//! two terminal states — accepted-and-chained, or explicitly-rejected — and is
//! never lost in between.
//!
//! # Latency budget
//!
//! The append critical section is O(1) on top of one SHA-256 over the record's
//! length-prefixed pre-image, so the per-op wait is bounded by the number of
//! ops ahead of it in the serial order. At 100 concurrent ops this stays well
//! under the 500ms p99 budget the acceptance harness asserts.
//!
//! This module adds **no new event semantics** — it loads and measures the
//! append/hash-chain primitive D1a sealed. Any correctness regression observed
//! here is a defect in those primitives, fixed at root, never bypassed here.

use crate::log::EventLog;
use hugit_contracts::event_record::EventRecord;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// One operation submitted to the serializer: the inputs of a single
/// [`EventLog::append`], decided by the caller before admission.
///
/// The serializer assigns `seq`, `prev_hash`, and `this_hash` at the moment it
/// holds the writer — the caller never picks a position in the chain, which is
/// what makes concurrent submission safe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    /// Event kind / type discriminator (e.g. `"ref.update"`).
    pub kind: String,
    /// Ordered chain of principals that produced this operation.
    pub principal_chain: Vec<String>,
    /// Opaque JSON payload, serialized as a string (mirrors [`EventRecord`]).
    pub payload: String,
    /// Unix epoch milliseconds the caller stamped the operation.
    pub recorded_at: u64,
}

impl Op {
    /// Construct an operation from its parts.
    pub fn new(
        kind: impl Into<String>,
        principal_chain: Vec<String>,
        payload: impl Into<String>,
        recorded_at: u64,
    ) -> Self {
        Self {
            kind: kind.into(),
            principal_chain,
            payload: payload.into(),
            recorded_at,
        }
    }
}

/// Why a submission was not accepted onto the chain.
///
/// Every non-acceptance is **explicit** — an operation that returns an error
/// here is provably not on the chain, which is the other half of the zero-loss
/// invariant (an op is either chained or it carries one of these).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitError {
    /// The writer is saturated: the number of in-flight admitted operations is
    /// already at capacity, so this operation is rejected rather than queued
    /// unboundedly or silently dropped. The caller should retry/queue it.
    Backpressure {
        /// The admission capacity that was reached.
        capacity: usize,
    },
    /// The shared writer mutex was poisoned by a panic in another submitter.
    /// Surfaced (not swallowed) so a corrupt critical section can never be
    /// mistaken for a successful, lost-then-forgotten append.
    WriterPoisoned,
}

impl std::fmt::Display for SubmitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SubmitError::Backpressure { capacity } => write!(
                f,
                "writer saturated at capacity {capacity}: operation rejected (back-pressure, not dropped)"
            ),
            SubmitError::WriterPoisoned => {
                write!(f, "writer mutex poisoned by a panicking submitter")
            }
        }
    }
}

impl std::error::Error for SubmitError {}

/// The single serialization point in front of one repository's [`EventLog`].
///
/// Cloneable and `Send + Sync`: every clone shares the *same* underlying writer
/// (one [`EventLog`] behind one [`Mutex`]), so handing a clone to each of N
/// threads models N concurrent submitters fanning into the one Durable-Object
/// writer. Operations are serialized through the single mutex into a total
/// order and chained by D1a's append primitive.
#[derive(Clone)]
pub struct Serializer {
    inner: Arc<Inner>,
}

/// A callback invoked by [`Serializer::submit`] *after* a submission has taken
/// its admission permit but *before* it attempts the writer lock — i.e. exactly
/// at the admit/lock split, with the permit provably held and counted in-flight.
///
/// This is the seam a deterministic test uses to *observe* saturation without
/// timing: each admitted submitter signals through this hook the instant it is
/// counted in-flight, so a test can await exactly `capacity` such signals (and
/// know the surplus was refused) instead of spin-sampling [`Serializer::in_flight`].
/// Production never installs a hook, so [`Serializer::submit`] is unchanged on
/// the real path.
type AdmissionHook = dyn Fn() + Send + Sync;

struct Inner {
    /// The one writer. Exactly one submitter holds this lock at a time; that is
    /// the serialization point.
    log: Mutex<EventLog>,
    /// Optional observer fired at the admit/lock split (permit held, lock not yet
    /// taken). `None` in production; a test installs one to make saturation
    /// deterministically observable. See [`AdmissionHook`].
    on_admitted: Option<Box<AdmissionHook>>,
    /// Admission gate. A non-blocking counting semaphore of `capacity` permits:
    /// every accepted submission holds exactly one permit from *before* it takes
    /// the writer lock until *after* that lock drops, so the number of admitted
    /// in-flight operations is, by construction, the number of permits checked
    /// out — and that can never exceed `capacity`. Over-capacity submissions are
    /// refused (no permit) rather than queued or dropped. Pairing the permit's
    /// lifetime with the lock's is what makes the bound hold under concurrent
    /// admission: there is no window where admission is granted but uncounted.
    gate: Semaphore,
    /// Admission bound for back-pressure. `usize::MAX` means unbounded admission
    /// (pure serialization with no rejection).
    capacity: usize,
}

impl Serializer {
    /// A serializer over a fresh, empty log with **unbounded admission** — every
    /// submission is accepted and serialized; no operation is ever rejected.
    pub fn new() -> Self {
        Self::from_log(EventLog::new(), usize::MAX)
    }

    /// A serializer with a bounded admission gate of `capacity` in-flight
    /// operations. Beyond the bound, submissions are rejected with
    /// [`SubmitError::Backpressure`] (explicit back-pressure, never silent drop).
    ///
    /// `capacity` is clamped to at least 1 so a single submitter can always make
    /// progress.
    pub fn with_capacity(capacity: usize) -> Self {
        Self::from_log(EventLog::new(), capacity.max(1))
    }

    /// Wrap an existing (possibly rehydrated) log with the given admission bound.
    pub fn from_log(log: EventLog, capacity: usize) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Arc::new(Inner {
                log: Mutex::new(log),
                on_admitted: None,
                gate: Semaphore::new(capacity),
                capacity,
            }),
        }
    }

    /// A serializer with a bounded admission gate and an observer fired at the
    /// admit/lock split (permit held + counted in-flight, writer lock not yet
    /// taken). Test-only: lets a test deterministically await exactly `capacity`
    /// admitted-and-blocked submitters instead of spin-sampling [`in_flight`].
    ///
    /// [`in_flight`]: Serializer::in_flight
    #[doc(hidden)]
    pub fn with_capacity_and_admission_hook(
        capacity: usize,
        on_admitted: impl Fn() + Send + Sync + 'static,
    ) -> Self {
        let capacity = capacity.max(1);
        Self {
            inner: Arc::new(Inner {
                log: Mutex::new(EventLog::new()),
                on_admitted: Some(Box::new(on_admitted)),
                gate: Semaphore::new(capacity),
                capacity,
            }),
        }
    }

    /// The admission capacity (max in-flight operations before back-pressure).
    pub fn capacity(&self) -> usize {
        self.inner.capacity
    }

    /// The number of operations currently admitted (holding an admission permit):
    /// granted before the writer lock and released after it drops. Invariant:
    /// `in_flight() <= capacity()` at every instant. Exposed for load-test
    /// instrumentation of the back-pressure bound.
    pub fn in_flight(&self) -> usize {
        self.inner.gate.in_use()
    }

    /// The peak number of simultaneously-admitted operations observed so far
    /// (high-water mark of [`Serializer::in_flight`]). A correct admission gate
    /// keeps this `<= capacity()`; a load test asserts exactly that to catch any
    /// transient breach of the bound under contention.
    pub fn peak_in_flight(&self) -> usize {
        self.inner.gate.peak()
    }

    /// Number of records on the chain (taken under the writer lock).
    ///
    /// Returns [`SubmitError::WriterPoisoned`] if a submitter panicked while
    /// holding the writer.
    pub fn len(&self) -> Result<usize, SubmitError> {
        let log = self
            .inner
            .log
            .lock()
            .map_err(|_| SubmitError::WriterPoisoned)?;
        Ok(log.len())
    }

    /// Whether the chain is empty.
    pub fn is_empty(&self) -> Result<bool, SubmitError> {
        Ok(self.len()? == 0)
    }

    /// Submit one operation. On acceptance it is serialized through the single
    /// writer and appended to the chain; the produced [`EventRecord`] (with its
    /// assigned `seq`/`prev_hash`/`this_hash`) is returned.
    ///
    /// # Serialization
    ///
    /// Concurrent calls contend for one mutex; the lock imposes a total order,
    /// so the records land in a single, gap-free, hash-linked sequence whatever
    /// the arrival interleaving.
    ///
    /// # Back-pressure (zero loss)
    ///
    /// Admission is checked *before* taking the writer: if `capacity` operations
    /// are already in flight, this call returns [`SubmitError::Backpressure`]
    /// without touching the chain — explicitly rejected, never silently dropped.
    /// Otherwise the operation is counted in-flight, appended, then counted out,
    /// guaranteeing `records_appended == submissions_accepted`.
    pub fn submit(&self, op: Op) -> Result<EventRecord, SubmitError> {
        // ── Admission gate: take one permit or refuse (back-pressure).
        // The permit is the *only* way to be in-flight, and it is held until the
        // end of this call — see below. Because a permit cannot be checked out
        // beyond `capacity`, the in-flight count is bounded by construction with
        // no admit/lock split for an interleaving to slip through.
        let _permit = match self.inner.gate.try_acquire() {
            Some(permit) => permit,
            None => {
                return Err(SubmitError::Backpressure {
                    capacity: self.inner.capacity,
                });
            }
        };

        // Admit/lock split observer: the permit is held and counted in-flight,
        // the writer lock is not yet taken. A test installs a hook here to make
        // saturation deterministically observable (await exactly `capacity`
        // signals); production installs none, so this is a no-op on the real path.
        if let Some(hook) = self.inner.on_admitted.as_deref() {
            hook();
        }

        // The permit is now held for the whole critical section and is released
        // (`Drop`) only *after* the writer lock below has dropped, because `log`
        // is declared after `_permit` and locals drop in reverse declaration
        // order. So "permit held" ⊇ "lock held": admission can never be granted
        // for an operation that is not yet counted, nor released for one still at
        // the writer. The bound therefore holds under concurrent admission.
        let mut log = self
            .inner
            .log
            .lock()
            .map_err(|_| SubmitError::WriterPoisoned)?;
        let record = log.append(op.kind, op.principal_chain, op.payload, op.recorded_at);
        Ok(record)
        // `log` drops first (lock released), then `_permit` (in-flight count
        // decremented) — every path, including the `?` error and any panic.
    }

    /// Run `f` against the chain's records under the writer lock — e.g. to verify
    /// the chain or replay it after a load test, with no concurrent writer able
    /// to mutate mid-read.
    pub fn with_records<R>(&self, f: impl FnOnce(&[EventRecord]) -> R) -> Result<R, SubmitError> {
        let log = self
            .inner
            .log
            .lock()
            .map_err(|_| SubmitError::WriterPoisoned)?;
        Ok(f(log.records()))
    }

    /// Take a snapshot clone of the underlying [`EventLog`] (under the lock).
    /// Useful for off-writer replay/verification after a load run.
    pub fn snapshot(&self) -> Result<EventLog, SubmitError> {
        let log = self
            .inner
            .log
            .lock()
            .map_err(|_| SubmitError::WriterPoisoned)?;
        Ok(log.clone())
    }
}

impl Default for Serializer {
    fn default() -> Self {
        Self::new()
    }
}

/// A non-blocking counting semaphore: the admission gate for back-pressure.
///
/// `try_acquire` either checks out one of `capacity` permits or returns `None`
/// (caller turns that into [`SubmitError::Backpressure`]) — it never blocks and
/// never over-issues. A checked-out [`Permit`] returns its count on `Drop`, so
/// the live permit count (`in_use`) is exactly the number of admitted in-flight
/// operations and is bounded by `capacity` at every instant.
struct Semaphore {
    /// Permits currently checked out. Bounded in `[0, capacity]` by the
    /// compare-exchange in [`Semaphore::try_acquire`].
    in_use: AtomicUsize,
    /// High-water mark of `in_use`, for load-test instrumentation of the bound.
    peak: AtomicUsize,
    /// Total permits. `usize::MAX` means effectively unbounded admission.
    capacity: usize,
}

impl Semaphore {
    fn new(capacity: usize) -> Self {
        Self {
            in_use: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            capacity,
        }
    }

    /// Check out one permit, or `None` if all `capacity` are already in use.
    ///
    /// The bound is enforced atomically: the increment only commits while
    /// `cur < capacity`, so the live count can never exceed `capacity` — there
    /// is no read-then-act window for an interleaving to exploit.
    fn try_acquire(&self) -> Option<Permit<'_>> {
        let mut cur = self.in_use.load(Ordering::Acquire);
        loop {
            if cur >= self.capacity {
                return None;
            }
            match self.in_use.compare_exchange_weak(
                cur,
                cur + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    // Record the high-water mark for the bound assertion. `cur + 1`
                    // is the count this acquisition just established.
                    self.peak.fetch_max(cur + 1, Ordering::AcqRel);
                    return Some(Permit { sem: self });
                }
                Err(observed) => cur = observed,
            }
        }
    }

    /// Permits currently checked out (= admitted in-flight operations).
    fn in_use(&self) -> usize {
        self.in_use.load(Ordering::Acquire)
    }

    /// High-water mark of [`Semaphore::in_use`] observed since construction.
    fn peak(&self) -> usize {
        self.peak.load(Ordering::Acquire)
    }
}

/// RAII permit: returns its count to the [`Semaphore`] when a submission leaves
/// the critical section by any path (success, error, or panic), so the
/// back-pressure bound can never be permanently consumed by a lost op.
struct Permit<'a> {
    sem: &'a Semaphore,
}

impl Drop for Permit<'_> {
    fn drop(&mut self) {
        // Saturating: a permit is returned exactly once, but guard the counter
        // so a (bug-induced) double return can never underflow `in_use` to
        // `usize::MAX` and wedge admission shut.
        let mut cur = self.sem.in_use.load(Ordering::Acquire);
        loop {
            let next = cur.saturating_sub(1);
            match self.sem.in_use.compare_exchange_weak(
                cur,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(observed) => cur = observed,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Defect 2 regression: an extra/double permit return must NOT underflow the
    /// in-flight counter to `usize::MAX` (which would wedge admission shut). The
    /// saturating decrement clamps at 0. A naive `fetch_sub(1)` would have made
    /// this assert `usize::MAX`.
    #[test]
    fn permit_return_saturates_never_underflows() {
        let sem = Semaphore::new(2);
        // One genuine permit out, then dropped: count back to 0.
        drop(sem.try_acquire().expect("permit available"));
        assert_eq!(sem.in_use(), 0);

        // Simulate a spurious extra return on the already-empty counter (the
        // double-drop the original `fetch_sub` would have underflowed on).
        let phantom = Permit { sem: &sem };
        drop(phantom);
        assert_eq!(
            sem.in_use(),
            0,
            "an over-return must saturate at 0, never wrap to usize::MAX"
        );

        // Admission still works after the would-be underflow.
        assert!(
            sem.try_acquire().is_some(),
            "admission must remain open — counter not wedged at usize::MAX"
        );
    }

    /// The atomic bound holds: never more than `capacity` permits checked out at
    /// once, and the surplus acquisition is refused.
    #[test]
    fn try_acquire_bounds_at_capacity() {
        let sem = Semaphore::new(2);
        let p1 = sem.try_acquire().expect("1st permit");
        let p2 = sem.try_acquire().expect("2nd permit");
        assert_eq!(sem.in_use(), 2);
        assert!(sem.try_acquire().is_none(), "3rd over capacity is refused");
        assert_eq!(sem.peak(), 2, "peak high-water mark equals capacity");
        drop(p1);
        drop(p2);
        assert_eq!(sem.in_use(), 0, "all permits returned");
    }
}
