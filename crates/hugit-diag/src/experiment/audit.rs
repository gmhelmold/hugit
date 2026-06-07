//! The audited event emitter for the experiment gate.
//!
//! Every consequential gate action — a promotion, an ingestion rejection, a
//! verdict invalidation — emits one hash-chained [`EventRecord`] (contract
//! ⑥/⑧: "the promotion event itself is audited", "rejected at INGESTION …
//! audited"). The hash chain uses the single canonical formula from
//! `hugit_refstore::log::compute_this_hash` (WP-00 / R0):
//! `H(LP(prev_hash) ‖ LP(kind) ‖ VEC(principal_chain) ‖ LP(payload) ‖ u64_be(seq))`.

use hugit_contracts::EventRecord;
use hugit_refstore::compute_this_hash;

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
/// fully reconstructable from the log alone. `recorded_at` is a caller-supplied
/// wall-clock timestamp (Unix seconds or millis); it is stored on the record but
/// deliberately excluded from the hash pre-image (unauthenticated annotation —
/// see `hugit_refstore::log` module docs).
pub fn emit_event(
    log: &mut Vec<EventRecord>,
    event: ExperimentEvent,
    principal: &str,
    payload: &str,
    recorded_at: u64,
) -> EventRecord {
    let seq = log.len() as u64;
    let prev_hash = log
        .last()
        .map(|e| e.this_hash.clone())
        .unwrap_or_else(|| "0".repeat(64));

    let kind = event.kind();
    let principal_chain = vec![principal.to_string()];
    let this_hash = compute_this_hash(&prev_hash, kind, &principal_chain, payload, seq);

    let record = EventRecord {
        seq,
        prev_hash,
        this_hash,
        kind: kind.to_string(),
        principal_chain,
        payload: payload.to_string(),
        recorded_at,
    };

    log.push(record.clone());
    record
}
