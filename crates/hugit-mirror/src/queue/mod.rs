//! Durable outage queue core (WP-E1a item ⑩).
//!
//! The outbound mirror writer is fed by this queue: a **bounded, ordered,
//! durable FIFO** of landed-ref events awaiting replication to the GitHub
//! mirror. Its invariants are the trust contract for one-way sync:
//!
//! - **Stated capacity bound** — the queue holds at most [`QUEUE_CAPACITY`]
//!   entries. The bound is a pinned constant (config surface), documented in
//!   the SEAL, never an unbounded buffer that silently grows.
//! - **Backpressure on overflow, never drop** — a push that would exceed the
//!   capacity bound is rejected with [`EnqueueError::Backpressure`] AND raises
//!   an incident ([`Incident`]). The producer is told to slow down; an entry is
//!   NEVER dropped and NEVER silently discarded.
//! - **Strict landing-order preservation (FIFO)** — entries drain in the exact
//!   order they were enqueued. The queue never reorders.
//! - **Durable** — the queue state can be snapshotted and restored, so it
//!   survives a writer restart with both contents and order intact.
//!
//! The *failure-injection* semantics over this queue (outage drain, bounded
//! backoff, recovery) are E1b④'s claim; what is proven HERE is the capacity
//! bound (⑩), the overflow→backpressure+incident behavior, and the ordering
//! guarantee.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

/// The stated capacity bound of the durable outage queue (item ⑩).
///
/// This is a **pinned constant** — the queue holds at most this many pending
/// entries. It is intentionally a single, documented number rather than an
/// unbounded buffer: overflow is a backpressure-and-incident event, not a
/// silent memory blow-up. Sized for a sustained-outage burst of landed refs
/// while keeping a hard ceiling; tune via [`OutageQueue::with_capacity`] in
/// tests, but production pins to this value.
pub const QUEUE_CAPACITY: usize = 4096;

/// One pending outbound replication entry: a landed ref destined for the
/// GitHub mirror, carried with the sequence number that fixes its landing
/// order.
///
/// `seq` mirrors the source `EventRecord::seq` — the queue preserves *this*
/// order and never reorders relative to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueueEntry {
    /// Landing sequence number (from the source event log). Fixes FIFO order.
    pub seq: u64,
    /// The fully-qualified ref name being mirrored (e.g. `refs/heads/main`).
    pub ref_name: String,
    /// The git object id (40-hex SHA-1) the ref points at, to be pushed.
    pub oid: String,
}

impl QueueEntry {
    /// Construct a pending entry.
    pub fn new(seq: u64, ref_name: impl Into<String>, oid: impl Into<String>) -> Self {
        Self {
            seq,
            ref_name: ref_name.into(),
            oid: oid.into(),
        }
    }
}

/// An incident raised when the queue cannot accept a push within its capacity
/// bound. Surfaced to the producer alongside backpressure — the entry is NOT
/// dropped; the producer must retry/slow. (E1b owns alarm routing; E1a raises.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incident {
    /// Stable kind discriminator.
    pub kind: String,
    /// The capacity bound that was hit.
    pub capacity: usize,
    /// The seq of the entry that triggered backpressure (preserved, not dropped).
    pub rejected_seq: u64,
    /// Human-readable description for the incident log.
    pub detail: String,
}

/// Error returned by [`OutageQueue::enqueue`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnqueueError {
    /// The queue is at its stated capacity bound. This is **backpressure**:
    /// the entry was NOT accepted and NOT dropped — the producer must slow and
    /// retry. An [`Incident`] is attached for the incident log.
    Backpressure {
        /// The incident describing the overflow.
        incident: Incident,
    },
}

impl std::fmt::Display for EnqueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EnqueueError::Backpressure { incident } => write!(
                f,
                "queue backpressure at capacity {} (seq {} rejected, not dropped): {}",
                incident.capacity, incident.rejected_seq, incident.detail
            ),
        }
    }
}

impl std::error::Error for EnqueueError {}

/// A durable, ordered, capacity-bounded FIFO outage queue (item ⑩).
///
/// Ordering is strict FIFO over enqueue order. The capacity bound is a stated
/// constant ([`QUEUE_CAPACITY`] by default). On overflow the push is rejected
/// with backpressure + an incident; nothing is ever dropped or reordered. The
/// full state is serializable for durability across writer restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutageQueue {
    capacity: usize,
    entries: VecDeque<QueueEntry>,
}

impl Default for OutageQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl OutageQueue {
    /// Create a queue with the pinned [`QUEUE_CAPACITY`] bound.
    pub fn new() -> Self {
        Self::with_capacity(QUEUE_CAPACITY)
    }

    /// Create a queue with an explicit capacity bound (test surface).
    ///
    /// Production uses [`OutageQueue::new`] / [`QUEUE_CAPACITY`]; this exists so
    /// the overflow/backpressure invariant can be exercised at small sizes.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            capacity,
            entries: VecDeque::new(),
        }
    }

    /// The stated capacity bound of this queue.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current number of pending entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the queue holds no pending entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether the queue is at its capacity bound (next enqueue → backpressure).
    pub fn is_full(&self) -> bool {
        self.entries.len() >= self.capacity
    }

    /// Enqueue a landed-ref entry at the tail (FIFO).
    ///
    /// On success the entry is appended in landing order. If the queue is at
    /// its capacity bound the push is rejected with
    /// [`EnqueueError::Backpressure`] carrying an [`Incident`]; the entry is
    /// NOT dropped and the queue is left unchanged (never reordered, never
    /// truncated).
    pub fn enqueue(&mut self, entry: QueueEntry) -> Result<(), EnqueueError> {
        if self.is_full() {
            // Overflow → backpressure + incident, NEVER drop, NEVER reorder.
            return Err(EnqueueError::Backpressure {
                incident: Incident {
                    kind: "queue_overflow_backpressure".to_string(),
                    capacity: self.capacity,
                    rejected_seq: entry.seq,
                    detail: format!(
                        "outage queue at capacity bound {}; entry seq {} held back \
                         (producer must apply backpressure and retry)",
                        self.capacity, entry.seq
                    ),
                },
            });
        }
        self.entries.push_back(entry);
        Ok(())
    }

    /// Dequeue the head entry (FIFO drain order). `None` if empty.
    pub fn dequeue(&mut self) -> Option<QueueEntry> {
        self.entries.pop_front()
    }

    /// Peek the head entry without removing it.
    pub fn peek(&self) -> Option<&QueueEntry> {
        self.entries.front()
    }

    /// The pending entries in drain (FIFO) order, for inspection/proof.
    pub fn pending(&self) -> Vec<QueueEntry> {
        self.entries.iter().cloned().collect()
    }

    /// Serialize the full queue state for durability (survives writer restart).
    pub fn snapshot(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Restore a queue from a snapshot — contents AND order preserved.
    pub fn restore(snapshot: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capacity_bound_is_stated_constant() {
        assert_eq!(OutageQueue::new().capacity(), QUEUE_CAPACITY);
        const { assert!(QUEUE_CAPACITY > 0) };
    }

    #[test]
    fn fifo_order_preserved() {
        let mut q = OutageQueue::with_capacity(8);
        for seq in 0..5u64 {
            q.enqueue(QueueEntry::new(
                seq,
                "refs/heads/main",
                format!("{seq:040}"),
            ))
            .unwrap();
        }
        let drained: Vec<u64> = std::iter::from_fn(|| q.dequeue()).map(|e| e.seq).collect();
        assert_eq!(drained, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn overflow_backpressures_never_drops() {
        let mut q = OutageQueue::with_capacity(2);
        q.enqueue(QueueEntry::new(0, "r", "a")).unwrap();
        q.enqueue(QueueEntry::new(1, "r", "b")).unwrap();
        let err = q.enqueue(QueueEntry::new(2, "r", "c")).unwrap_err();
        match err {
            EnqueueError::Backpressure { incident } => {
                assert_eq!(incident.rejected_seq, 2);
                assert_eq!(incident.capacity, 2);
            }
        }
        // Not dropped from the queue, not truncated: still exactly the two
        // accepted entries, in order.
        assert_eq!(q.len(), 2);
        assert_eq!(q.peek().unwrap().seq, 0);
    }

    #[test]
    fn snapshot_restore_round_trips_order() {
        let mut q = OutageQueue::with_capacity(4);
        q.enqueue(QueueEntry::new(7, "r", "x")).unwrap();
        q.enqueue(QueueEntry::new(8, "r", "y")).unwrap();
        let snap = q.snapshot().unwrap();
        let restored = OutageQueue::restore(&snap).unwrap();
        assert_eq!(restored.pending(), q.pending());
        assert_eq!(restored.capacity(), q.capacity());
    }
}
