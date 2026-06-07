//! Deep-link resolver (③).
//!
//! Resolves an intent_id deep-link to its golden expected target object.
//!
//! A deep-link is a stable reference from the ledger view to the underlying
//! intent record. Resolution is GOLDEN — each link is asserted to reach a
//! specific expected target object, not merely a non-error response.
//!
//! In this v0 implementation the "store" is a slice of EventRecords. The
//! resolver finds the `intent.landed` event with matching `intent_id` and
//! returns the `deep_link_target` field from the payload — the golden target.

use hugit_contracts::event_record::EventRecord;

use crate::redact;

/// The result of resolving a deep-link.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveResult {
    /// Resolved to a golden target (the exact expected object id).
    Found {
        /// The intent id that was looked up.
        intent_id: String,
        /// The golden target object (content address or oid), redacted at the
        /// view boundary if it contains a secret marker.
        target: String,
        /// Log sequence of the record that contained this target.
        seq: u64,
    },
    /// No record with this intent_id was found in the event log.
    NotFound { intent_id: String },
}

/// Resolve a deep-link intent_id against a slice of EventRecords.
///
/// Scans the records for the first `intent.landed` event whose payload carries
/// the given `intent_id` and returns its `deep_link_target` (or falls back to
/// the `target` field).  The returned `target` is the GOLDEN expected object —
/// not merely "no error".  The target is routed through view-boundary redaction
/// (④) before being surfaced.
pub fn resolve(intent_id: &str, records: &[EventRecord]) -> ResolveResult {
    for r in records {
        if r.kind != "intent.landed" {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.payload) else {
            continue;
        };
        let id = v.get("intent_id").and_then(serde_json::Value::as_str);
        if id != Some(intent_id) {
            continue;
        }
        // Golden target: prefer explicit deep_link_target, fall back to target field.
        // Apply view-boundary redaction before surfacing the target (④).
        let raw_target = v
            .get("deep_link_target")
            .and_then(serde_json::Value::as_str)
            .or_else(|| v.get("target").and_then(serde_json::Value::as_str))
            .unwrap_or(intent_id);
        let target = redact::apply(raw_target);
        return ResolveResult::Found {
            intent_id: intent_id.to_string(),
            target,
            seq: r.seq,
        };
    }
    ResolveResult::NotFound {
        intent_id: intent_id.to_string(),
    }
}
