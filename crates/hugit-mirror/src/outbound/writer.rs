//! The outbound mirror sync writer + push driver (WP-E1a items ① ③).
//!
//! The writer drains the durable outage queue in landing order and replicates
//! each landed ref to the GitHub mirror, then **content-hash verifies** the
//! push per the [`crate::verify`] contract. It is the soak instrument whose
//! continuous uptime IS the trust metric.
//!
//! Decided forks (zero-decision):
//! - Auth = GitHub App installation token ([`crate::outbound::auth`]).
//! - Every push carries the expected content hash; verification is **per
//!   push**, fail-CLOSED on mismatch (raise divergence, never mark synced).
//! - One-way only: this writer NEVER reads GitHub state as truth. There is no
//!   reverse-sync path here (E1b⑦ proves reverse writes are divergence).
//! - SLA = land-event → verified-on-mirror within [`SLA_BOUND_MS`] (item ①);
//!   the soak (③) asserts 100% verified over the soak window.

use std::time::Duration;

use crate::queue::{OutageQueue, QueueEntry};
use crate::verify::{ContentHash, DivergenceSignal, VerifyOutcome, verify_push};

/// The <60s SLA bound, in milliseconds: land-event → verified-on-mirror
/// (item ①). A push whose verified latency exceeds this is an SLA breach.
pub const SLA_BOUND_MS: u64 = 60_000;

/// The mirror push surface: pushes a ref to the GitHub mirror and reports the
/// object hash the mirror holds for that ref after the push.
///
/// This is the seam the live GitHub App client implements; tests supply an
/// in-process fixture. The writer NEVER reads GitHub state as a source of
/// truth — it only re-reads the *just-pushed* ref's hash to verify byte
/// identity. (One-way enforcement: no inbound/reverse path; writes flow only
/// forge → GitHub, never the other direction.)
pub trait MirrorPushTarget {
    /// Push `oid` to `ref_name` on the mirror, returning the object hash the
    /// mirror reports for that ref after the push (for per-push verification).
    fn push_ref(&mut self, ref_name: &str, oid: &ContentHash) -> Result<ContentHash, PushError>;
}

/// Errors from a push attempt.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PushError {
    /// The mirror rejected the push (transport / API).
    #[error("mirror push failed for {ref_name}: {detail}")]
    Rejected {
        /// The ref that failed.
        ref_name: String,
        /// Non-secret failure detail.
        detail: String,
    },
}

/// The result of replicating one queue entry to the mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PushReport {
    /// The ref replicated.
    pub ref_name: String,
    /// Landing sequence number (preserves order across the soak log).
    pub seq: u64,
    /// The verify outcome (Verified or fail-CLOSED Diverged).
    pub verify: VerifyOutcome,
    /// Measured land→verified latency in milliseconds.
    pub latency_ms: u64,
}

impl PushReport {
    /// Whether the push verified to byte-identity within the SLA bound.
    pub fn verified_within_sla(&self) -> bool {
        self.verify.is_verified() && self.latency_ms <= SLA_BOUND_MS
    }

    /// The divergence signal, if the push failed verification (fail-CLOSED).
    pub fn divergence(&self) -> Option<&DivergenceSignal> {
        self.verify.divergence()
    }
}

/// An in-process [`MirrorPushTarget`] for fixture proofs.
///
/// Faithful mirror: it echoes back the pushed oid as the observed hash, so a
/// correct push verifies. It can be told to corrupt specific refs (to exercise
/// the fail-CLOSED divergence path) or to reject pushes.
#[derive(Debug, Default, Clone)]
pub struct FixtureMirror {
    corrupt_ref: Option<(String, ContentHash)>,
    reject_ref: Option<String>,
    pushed: Vec<(String, ContentHash)>,
}

impl FixtureMirror {
    /// A faithful mirror that verifies every well-formed push.
    pub fn faithful() -> Self {
        Self::default()
    }

    /// Make the mirror report a different hash for `ref_name` (divergence).
    pub fn corrupting(mut self, ref_name: impl Into<String>, observed: ContentHash) -> Self {
        self.corrupt_ref = Some((ref_name.into(), observed));
        self
    }

    /// Make the mirror reject pushes to `ref_name`.
    pub fn rejecting(mut self, ref_name: impl Into<String>) -> Self {
        self.reject_ref = Some(ref_name.into());
        self
    }

    /// The refs pushed so far, in push order.
    pub fn pushed_order(&self) -> Vec<String> {
        self.pushed.iter().map(|(r, _)| r.clone()).collect()
    }
}

impl MirrorPushTarget for FixtureMirror {
    fn push_ref(&mut self, ref_name: &str, oid: &ContentHash) -> Result<ContentHash, PushError> {
        if self.reject_ref.as_deref() == Some(ref_name) {
            return Err(PushError::Rejected {
                ref_name: ref_name.to_string(),
                detail: "fixture: configured rejection".to_string(),
            });
        }
        self.pushed.push((ref_name.to_string(), oid.clone()));
        match &self.corrupt_ref {
            Some((r, observed)) if r == ref_name => Ok(observed.clone()),
            // Faithful: a content-addressed mirror reports back the same oid.
            _ => Ok(oid.clone()),
        }
    }
}

/// The outbound sync writer (items ① ③).
///
/// Drains the durable queue in FIFO landing order, pushes each ref via the
/// [`MirrorPushTarget`], verifies per-push (fail-CLOSED), and produces a
/// [`PushReport`] per entry. One-way only — never reads GitHub as truth.
pub struct OutboundWriter<T: MirrorPushTarget> {
    target: T,
}

impl<T: MirrorPushTarget> OutboundWriter<T> {
    /// Create a writer over a push target.
    pub fn new(target: T) -> Self {
        Self { target }
    }

    /// Borrow the underlying target (e.g. to inspect push order in tests).
    pub fn target(&self) -> &T {
        &self.target
    }

    /// Replicate one entry to the mirror and content-hash verify it (item ①).
    ///
    /// `elapsed` is the measured land→push-issued latency; combined with the
    /// verify it yields the land→verified latency for the SLA check. On a push
    /// transport failure the entry is reported with a synthesised divergence so
    /// the caller never marks it synced (fail-CLOSED on every non-verified
    /// path).
    pub fn replicate(&mut self, entry: &QueueEntry, elapsed: Duration) -> PushReport {
        let expected = ContentHash::new(&entry.oid);
        let latency_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        match self.target.push_ref(&entry.ref_name, &expected) {
            Ok(observed) => {
                let verify = verify_push(&entry.ref_name, &expected, &observed);
                PushReport {
                    ref_name: entry.ref_name.clone(),
                    seq: entry.seq,
                    verify,
                    latency_ms,
                }
            }
            Err(PushError::Rejected { ref_name, detail }) => {
                // A failed push can never be "synced" — surface as divergence
                // (fail-CLOSED). Observed = empty hash (nothing landed).
                let verify = VerifyOutcome::Diverged(DivergenceSignal {
                    ref_name: ref_name.clone(),
                    expected: expected.clone(),
                    observed: ContentHash::new(""),
                    detail: format!("push rejected (fail-CLOSED, not synced): {detail}"),
                });
                PushReport {
                    ref_name,
                    seq: entry.seq,
                    verify,
                    latency_ms,
                }
            }
        }
    }

    /// Drain the whole queue in FIFO order, replicating + verifying each entry.
    ///
    /// Returns one [`PushReport`] per drained entry, in landing order. Each
    /// report records `per_entry_latency` as the land→verified latency for that
    /// entry (the caller supplies the per-entry elapsed via `latency_for`).
    pub fn drain(
        &mut self,
        queue: &mut OutageQueue,
        latency_for: impl Fn(&QueueEntry) -> Duration,
    ) -> Vec<PushReport> {
        let mut reports = Vec::with_capacity(queue.len());
        while let Some(entry) = queue.dequeue() {
            let elapsed = latency_for(&entry);
            reports.push(self.replicate(&entry, elapsed));
        }
        reports
    }
}

/// Aggregate verified-fraction + SLA summary over a run of reports — the soak
/// metric (item ③: 100% verified over the soak window).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SoakSummary {
    /// Total entries replicated.
    pub total: usize,
    /// Entries verified to byte-identity within the SLA bound.
    pub verified_within_sla: usize,
    /// Entries that diverged (fail-CLOSED) — must be zero for a healthy soak.
    pub diverged: usize,
}

impl SoakSummary {
    /// Summarise a run of push reports.
    pub fn from_reports(reports: &[PushReport]) -> Self {
        let verified_within_sla = reports.iter().filter(|r| r.verified_within_sla()).count();
        let diverged = reports.iter().filter(|r| r.divergence().is_some()).count();
        Self {
            total: reports.len(),
            verified_within_sla,
            diverged,
        }
    }

    /// Whether the soak is 100% verified-within-SLA with zero divergence.
    pub fn all_verified(&self) -> bool {
        self.diverged == 0 && self.total > 0 && self.verified_within_sla == self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(seq: u64, name: &str) -> QueueEntry {
        QueueEntry::new(seq, name, format!("{seq:040x}"))
    }

    #[test]
    fn faithful_push_verifies_within_sla() {
        let mut w = OutboundWriter::new(FixtureMirror::faithful());
        let r = w.replicate(&entry(1, "refs/heads/main"), Duration::from_millis(10));
        assert!(r.verified_within_sla());
        assert!(r.divergence().is_none());
    }

    #[test]
    fn corrupt_mirror_is_fail_closed_divergence() {
        let bad = ContentHash::new(format!("{:040x}", 999));
        let mut w = OutboundWriter::new(FixtureMirror::faithful().corrupting("refs/heads/x", bad));
        let r = w.replicate(&entry(1, "refs/heads/x"), Duration::from_millis(5));
        assert!(!r.verified_within_sla());
        assert!(r.divergence().is_some());
    }

    #[test]
    fn sla_breach_not_counted_verified() {
        let mut w = OutboundWriter::new(FixtureMirror::faithful());
        let r = w.replicate(
            &entry(1, "refs/heads/main"),
            Duration::from_millis(SLA_BOUND_MS + 1),
        );
        assert!(r.verify.is_verified());
        assert!(!r.verified_within_sla());
    }

    #[test]
    fn drain_preserves_order() {
        let mut q = OutageQueue::with_capacity(8);
        for s in 0..4 {
            q.enqueue(entry(s, &format!("refs/heads/b{s}"))).unwrap();
        }
        let mut w = OutboundWriter::new(FixtureMirror::faithful());
        let reports = w.drain(&mut q, |_| Duration::from_millis(1));
        let seqs: Vec<u64> = reports.iter().map(|r| r.seq).collect();
        assert_eq!(seqs, vec![0, 1, 2, 3]);
        assert_eq!(
            w.target().pushed_order(),
            vec![
                "refs/heads/b0",
                "refs/heads/b1",
                "refs/heads/b2",
                "refs/heads/b3"
            ]
        );
    }
}
