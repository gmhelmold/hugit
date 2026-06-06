//! The audited event emitter for the experiment gate.
//!
//! Every consequential gate action — a promotion, an ingestion rejection, a
//! verdict invalidation — emits one hash-chained [`EventRecord`] (contract
//! ⑥/⑧: "the promotion event itself is audited", "rejected at INGESTION …
//! audited"). The hash chain reuses the FROZEN formula from `EventRecord`:
//! `H(prev_hash ‖ kind ‖ principal_chain ‖ payload ‖ seq)`.

use hugit_contracts::EventRecord;
use sha2::{Digest, Sha256};

/// The kinds of audited experiment-gate events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExperimentEvent {
    /// A promotion was authorized (a feature flipped on a genuine PASS).
    PromotionAuthorized,
    /// A promotion attempt was refused (fail-closed: not PASS / insufficient /
    /// degraded / forged report).
    PromotionRefused,
    /// A datapoint was rejected at ingestion (focus-gate-ineligible source, or
    /// a degraded-window contribution that was not honestly marked).
    IngestionRejected,
    /// The sealed corpus was found tampered → the verdict is invalidated.
    VerdictInvalidated,
}

impl ExperimentEvent {
    /// The stable event-kind discriminator string written into the log.
    pub fn kind(self) -> &'static str {
        match self {
            ExperimentEvent::PromotionAuthorized => "experiment.promotion.authorized",
            ExperimentEvent::PromotionRefused => "experiment.promotion.refused",
            ExperimentEvent::IngestionRejected => "experiment.ingestion.rejected",
            ExperimentEvent::VerdictInvalidated => "experiment.verdict.invalidated",
        }
    }
}

/// Emit one audited [`EventRecord`] onto the caller-provided log.
///
/// The event is appended in place and also returned. `payload` is an opaque
/// JSON string carrying the reason / subject of the event so the decision is
/// fully reconstructable from the log alone.
pub fn emit_event(
    log: &mut Vec<EventRecord>,
    event: ExperimentEvent,
    principal: &str,
    payload: &str,
) -> EventRecord {
    let seq = log.len() as u64;
    let prev_hash = log
        .last()
        .map(|e| e.this_hash.clone())
        .unwrap_or_else(|| "0".repeat(64));

    let kind = event.kind();
    let this_hash = compute_event_hash(&prev_hash, kind, principal, payload, seq);

    let record = EventRecord {
        seq,
        prev_hash,
        this_hash,
        kind: kind.to_string(),
        principal_chain: vec![principal.to_string()],
        payload: payload.to_string(),
        recorded_at: 0, // deterministic for tests; callers may overwrite
    };

    log.push(record.clone());
    record
}

/// The FROZEN per-record hash:
/// `H(prev_hash ‖ kind ‖ principal_chain ‖ payload ‖ seq)`, each string field
/// length-prefixed (4-byte BE u32), `seq` as 8-byte BE u64.
fn compute_event_hash(
    prev_hash: &str,
    kind: &str,
    principal: &str,
    payload: &str,
    seq: u64,
) -> String {
    let mut hasher = Sha256::new();
    for field in &[prev_hash, kind, principal, payload] {
        let bytes = field.as_bytes();
        hasher.update((bytes.len() as u32).to_be_bytes());
        hasher.update(bytes);
    }
    hasher.update(seq.to_be_bytes());
    hex::encode(hasher.finalize())
}
