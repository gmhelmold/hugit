//! Deterministic fixtures for the WP-D4 acceptance oracle (`acceptance_d4.rs`).
//!
//! Two fixtures, both built purely from D1a's frozen append API so the event log
//! is the single source of truth and every projection is reproducible:
//!
//! - [`build_50_intent_log`] — 50 landed intents (the two-altitude consistency
//!   fixture, ②).
//! - [`build_mixed_log`] — intents and raw pushes interleaved on one ref (the
//!   externals-stay-external fixture, ④).

use hugit_refstore::log::EventLog;

/// A deterministic, parseable `intent.landed` payload for intent `i`.
pub fn intent_payload(i: u64) -> String {
    serde_json::json!({
        "intent_id": format!("intent-{i:04}"),
        "ref": "refs/heads/main",
        "target": format!("oid-{i:064x}"),
        "charter": format!("land change number {i}"),
    })
    .to_string()
}

/// A deterministic raw-push (`ref.update`) payload — provenance-free.
pub fn raw_push_payload(i: u64) -> String {
    serde_json::json!({
        "ref": "refs/heads/main",
        "target": format!("rawoid-{i:064x}"),
    })
    .to_string()
}

/// **② fixture:** a 50-intent log. Every event is a landed intent; the two
/// altitudes must be provably consistent over it.
pub fn build_50_intent_log() -> EventLog {
    let mut log = EventLog::new();
    for i in 0..50u64 {
        let principal_chain = vec![format!("agent:runner-{:02}", i % 4), "user:gustavo".into()];
        log.append(
            "intent.landed",
            principal_chain,
            intent_payload(i),
            1_717_000_000_000 + i,
        );
    }
    log
}

/// **④ fixture:** intents and raw pushes interleaved on one ref, with inert
/// events sprinkled in. Returns `(log, intent_count, raw_push_count)`.
///
/// Pattern over 60 events: every 3rd event is a raw push, every 7th is inert,
/// the rest are landed intents — so all three event classes interleave on the
/// same `refs/heads/main`.
pub fn build_mixed_log() -> (EventLog, usize, usize) {
    let mut log = EventLog::new();
    let mut intent_count = 0usize;
    let mut raw_push_count = 0usize;
    for i in 0..60u64 {
        let principal_chain = vec![format!("agent:runner-{:02}", i % 4), "user:gustavo".into()];
        let recorded_at = 1_717_000_000_000 + i;
        if i % 3 == 0 {
            // raw push — external change, NEVER an intent.
            log.append(
                "ref.update",
                principal_chain,
                raw_push_payload(i),
                recorded_at,
            );
            raw_push_count += 1;
        } else if i % 7 == 0 {
            // inert event — advances the chain, no altitude row.
            log.append(
                "checkpoint.noted",
                principal_chain,
                format!(r#"{{"note":"cp-{i}"}}"#),
                recorded_at,
            );
        } else {
            log.append(
                "intent.landed",
                principal_chain,
                intent_payload(i),
                recorded_at,
            );
            intent_count += 1;
        }
    }
    (log, intent_count, raw_push_count)
}
