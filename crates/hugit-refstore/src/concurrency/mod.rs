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

struct Inner {
    /// The one writer. Exactly one submitter holds this lock at a time; that is
    /// the serialization point.
    log: Mutex<EventLog>,
    /// Operations currently admitted but not yet returned from the critical
    /// section. The admission gate keeps this `<= capacity`.
    in_flight: AtomicUsize,
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
        Self {
            inner: Arc::new(Inner {
                log: Mutex::new(log),
                in_flight: AtomicUsize::new(0),
                capacity: capacity.max(1),
            }),
        }
    }

    /// The admission capacity (max in-flight operations before back-pressure).
    pub fn capacity(&self) -> usize {
        self.inner.capacity
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
        // ── Admission gate: reserve an in-flight slot or refuse (back-pressure).
        // CAS loop so the bound is honoured exactly under contention.
        let cap = self.inner.capacity;
        loop {
            let cur = self.inner.in_flight.load(Ordering::Acquire);
            if cur >= cap {
                return Err(SubmitError::Backpressure { capacity: cap });
            }
            if self
                .inner
                .in_flight
                .compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                break;
            }
        }

        // From here the slot is reserved; release it on every path so a refused
        // or panicking writer cannot leak admission capacity.
        let _slot = InFlightSlot {
            counter: &self.inner.in_flight,
        };

        // ── The single serialization point: one writer at a time.
        let mut log = self
            .inner
            .log
            .lock()
            .map_err(|_| SubmitError::WriterPoisoned)?;
        let record = log.append(op.kind, op.principal_chain, op.payload, op.recorded_at);
        Ok(record)
        // `log` then `_slot` drop here: lock released, in-flight decremented.
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

/// RAII guard that decrements the in-flight admission counter when a submission
/// leaves the critical section by any path (success, error, or panic), so the
/// back-pressure bound can never be permanently consumed by a lost op.
struct InFlightSlot<'a> {
    counter: &'a AtomicUsize,
}

impl Drop for InFlightSlot<'_> {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::AcqRel);
    }
}
