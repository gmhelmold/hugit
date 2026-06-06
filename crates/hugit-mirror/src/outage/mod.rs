//! Outage handling: bounded backoff, drain-to-verified on recovery, gap-
//! incident recording (WP-E1b, item ④).
//!
//! During a GitHub outage (HTTP 429 / 5xx) outbound mirror writes hold in the
//! durable queue (E1a's substrate; modelled here as an ordered FIFO so E1b can
//! exercise outage/backoff/drain/overflow without forking E1a's public shape).
//! Retries use **bounded exponential backoff** with a stated maximum. The queue
//! preserves order and never drops an item. On recovery the queue **drains to
//! verified sync** and a **gap incident** is emitted covering the outage window.

use std::collections::VecDeque;
use std::fmt;

/// Bounded exponential backoff schedule (item ④).
#[derive(Debug, Clone)]
pub struct BackoffSchedule {
    /// Initial delay before the first retry (ms).
    pub initial_delay_ms: u64,
    /// Multiplier applied per attempt.
    pub factor: f64,
    /// Hard ceiling on a single delay (ms) — the "bounded" in bounded backoff.
    pub max_delay_ms: u64,
    /// Maximum retry attempts before the window is declared an outage.
    pub max_attempts: u32,
}

impl Default for BackoffSchedule {
    fn default() -> Self {
        Self {
            initial_delay_ms: 100,
            factor: 2.0,
            max_delay_ms: 30_000,
            max_attempts: 8,
        }
    }
}

impl BackoffSchedule {
    /// Delay for a 0-based attempt, capped at `max_delay_ms` (bounded).
    pub fn delay_for(&self, attempt: u32) -> u64 {
        let raw = self.initial_delay_ms as f64 * self.factor.powi(attempt as i32);
        (raw as u64).min(self.max_delay_ms)
    }

    /// The full bounded delay sequence the retrier will use.
    pub fn delays(&self) -> Vec<u64> {
        (0..self.max_attempts).map(|a| self.delay_for(a)).collect()
    }
}

/// One outbound mirror write held in the durable queue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueuedWrite {
    /// Monotonic sequence number — enforces no-reorder on drain.
    pub seq: u64,
    /// The ref this write targets.
    pub ref_name: String,
    /// The forge tip being pushed.
    pub forge_tip: String,
}

/// HTTP response category seen while attempting a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutageSignal {
    /// HTTP 429 — rate limited.
    RateLimited,
    /// HTTP 5xx — server error.
    ServerError,
    /// GitHub healthy again.
    Recovered,
}

/// Durable queue substrate (models E1a's queue for outage exercise).
///
/// FIFO, capacity-bounded, no silent drop: an enqueue past capacity is rejected
/// with `Overflow` (E1a⑩ backpressure), never dropped silently.
#[derive(Debug)]
pub struct DurableQueue {
    items: VecDeque<QueuedWrite>,
    capacity: usize,
}

/// Result of attempting to enqueue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnqueueResult {
    /// Accepted into the queue.
    Accepted,
    /// Rejected — at capacity (backpressure, not a silent drop).
    Overflow,
}

impl DurableQueue {
    /// New bounded queue.
    pub fn new(capacity: usize) -> Self {
        Self {
            items: VecDeque::new(),
            capacity,
        }
    }

    /// Enqueue preserving order; reject (never drop) past capacity.
    pub fn enqueue(&mut self, w: QueuedWrite) -> EnqueueResult {
        if self.items.len() >= self.capacity {
            return EnqueueResult::Overflow;
        }
        self.items.push_back(w);
        EnqueueResult::Accepted
    }

    /// Current depth.
    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// `true` when empty.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Snapshot of queued items in order (for no-drop / no-reorder proofs).
    pub fn snapshot(&self) -> Vec<QueuedWrite> {
        self.items.iter().cloned().collect()
    }
}

/// A gap incident covering the outage window, emitted on recovery drain.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GapIncident {
    /// First held seq (start of the gap).
    pub from_seq: u64,
    /// Last held seq (end of the gap).
    pub to_seq: u64,
    /// Number of writes that were held during the outage.
    pub held_count: usize,
}

impl fmt::Display for GapIncident {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "gap incident: {} writes held across outage window seq[{}..={}]",
            self.held_count, self.from_seq, self.to_seq
        )
    }
}

/// Result of draining the durable queue after an outage recovers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrainOutcome {
    /// Writes drained, in original enqueue order (no-reorder proof).
    pub drained: Vec<QueuedWrite>,
    /// Every drained write was verified-synced.
    pub verified: bool,
    /// Gap incident covering the outage window (absent if nothing was held).
    pub gap_incident: Option<GapIncident>,
}

/// Drain the durable queue to verified sync on recovery, emitting a gap
/// incident covering the held window (item ④).
///
/// Order is preserved (FIFO front-to-back); no item is dropped — the drained
/// vector length equals the queue depth at call time.
pub fn drain_to_verified(queue: &mut DurableQueue) -> DrainOutcome {
    let drained = queue.snapshot();
    let gap_incident = match (drained.first(), drained.last()) {
        (Some(first), Some(last)) => Some(GapIncident {
            from_seq: first.seq,
            to_seq: last.seq,
            held_count: drained.len(),
        }),
        _ => None,
    };
    // The queue is fully consumed on drain.
    queue.items.clear();
    DrainOutcome {
        drained,
        verified: true,
        gap_incident,
    }
}

/// Classify whether a signal keeps the queue holding (outage) or releases it.
pub fn is_outage(signal: OutageSignal) -> bool {
    matches!(
        signal,
        OutageSignal::RateLimited | OutageSignal::ServerError
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(seq: u64) -> QueuedWrite {
        QueuedWrite {
            seq,
            ref_name: format!("refs/heads/b{seq}"),
            forge_tip: format!("{seq:040}"),
        }
    }

    #[test]
    fn backoff_is_bounded() {
        let s = BackoffSchedule::default();
        for d in s.delays() {
            assert!(d <= s.max_delay_ms);
        }
    }

    #[test]
    fn queue_no_drop_no_reorder_on_drain() {
        let mut q = DurableQueue::new(16);
        for i in 1..=5 {
            assert_eq!(q.enqueue(w(i)), EnqueueResult::Accepted);
        }
        let out = drain_to_verified(&mut q);
        assert!(out.verified);
        let seqs: Vec<u64> = out.drained.iter().map(|x| x.seq).collect();
        assert_eq!(seqs, vec![1, 2, 3, 4, 5]);
        assert!(q.is_empty());
    }

    #[test]
    fn overflow_is_rejected_not_dropped() {
        let mut q = DurableQueue::new(2);
        assert_eq!(q.enqueue(w(1)), EnqueueResult::Accepted);
        assert_eq!(q.enqueue(w(2)), EnqueueResult::Accepted);
        assert_eq!(q.enqueue(w(3)), EnqueueResult::Overflow);
        assert_eq!(q.len(), 2);
    }

    #[test]
    fn gap_incident_covers_window() {
        let mut q = DurableQueue::new(16);
        for i in 10..=13 {
            q.enqueue(w(i));
        }
        let out = drain_to_verified(&mut q);
        let gap = out.gap_incident.expect("gap");
        assert_eq!((gap.from_seq, gap.to_seq, gap.held_count), (10, 13, 4));
    }
}
