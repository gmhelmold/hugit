//! The SHARED terminal-seal precondition (C5-F2).
//!
//! A sealed campaign (`campaign.closed` on the log) is **terminal**: no
//! campaign-scoped mutation may append into it. Round 8 (Class 5, F2) showed
//! the K-VERDICT post-seal guard was point-local to the `verdict` verb only —
//! `intent new --log` and every `pr` mutation still appended into a closed
//! campaign, mutating its projection AFTER the immutable seal and diverging the
//! durable `sealed_with_rejected` fact from the live ledger.
//!
//! This module is the ONE chokepoint every campaign-scoped mutation verb routes
//! through BEFORE it appends. The seal-detection logic is defined exactly once
//! here (matching [`crate::campaign::world::World::campaign_closed`]) and reused
//! by `intent new`, `pr open/land/settle/abandon`, and `verdict` — so a future
//! verb that forgets the guard is a single missing call, not a re-implemented
//! rule that drifts.
//!
//! The guard is intentionally pure over the in-memory [`EventLog`] the caller
//! already loaded (no separate `World` load), so each verb can call it under the
//! advisory lock it already holds, with no extra I/O and no deadlock.

use hugit_refstore::EventLog;
use serde_json::Value;

use super::world::KIND_CAMPAIGN_CLOSED;

/// The stable error class a terminal-seal violation surfaces — IDENTICAL to the
/// `campaign_sealed` kind K-VERDICT introduced on the `verdict` verb, so every
/// verb refuses a post-seal append with the same machine-parseable kind/exit.
pub const SEALED_KIND: &str = "campaign_sealed";

/// A structured terminal-seal violation, carried as plain
/// `kind`/`message`/`fix` strings so each verb can render it into its own
/// porcelain error type (`crate::porcelain::PorcelainError`,
/// `crate::intent::error::PorcelainError`, `crate::pr::PrError`, …) without this
/// module depending on any one of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealViolation {
    /// The campaign key that is sealed (already scrubbed by the caller's
    /// surrounding redaction posture; this module never echoes raw secrets).
    pub campaign: String,
}

impl SealViolation {
    /// The stable error class (`campaign_sealed`).
    pub fn kind(&self) -> &'static str {
        SEALED_KIND
    }

    /// The operator-facing message naming the sealed campaign.
    pub fn message(&self) -> String {
        format!(
            "campaign '{}' is sealed (campaign.closed on log): a sealed campaign is \
             terminal — no further appends (intent, pr, or verdict) are allowed",
            self.campaign
        )
    }

    /// The remediation hint.
    pub fn fix(&self) -> &'static str {
        "open or create a new campaign for further work; a closed campaign is an \
         immutable audit-trail fact and cannot be mutated"
    }
}

/// Whether a `campaign.closed` record naming `campaign_key` exists on the log.
///
/// This is the single source of truth for "is this campaign terminal?" — it
/// matches [`crate::campaign::world::World::campaign_closed`] (same kind, same
/// payload `campaign` field), so the verbs and the campaign projections never
/// disagree on the seal state.
pub fn campaign_is_sealed(log: &EventLog, campaign_key: &str) -> bool {
    log.records().iter().any(|r| {
        r.kind == KIND_CAMPAIGN_CLOSED
            && serde_json::from_str::<Value>(&r.payload)
                .ok()
                .as_ref()
                .and_then(|v| v.get("campaign"))
                .and_then(Value::as_str)
                == Some(campaign_key)
    })
}

/// The SHARED precondition: refuse a campaign-scoped append if the campaign is
/// sealed. Every mutation verb calls this BEFORE appending (C5-F2 chokepoint).
///
/// Returns `Ok(())` when the campaign is open (or has no `campaign.closed`
/// record at all — permissive when the seal vocabulary is absent, consistent
/// with K-VERDICT), and `Err(SealViolation)` when it is terminal.
pub fn guard_not_sealed(log: &EventLog, campaign_key: &str) -> Result<(), SealViolation> {
    if campaign_is_sealed(log, campaign_key) {
        Err(SealViolation {
            campaign: campaign_key.to_string(),
        })
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};

    fn closed_campaign_log(key: &str) -> EventLog {
        let mut log = EventLog::new();
        let payload = format!("{{\"campaign\":\"{key}\"}}");
        log.append_authorized(
            PrincipalClass::Human,
            Endpoint::Policy,
            KIND_CAMPAIGN_CLOSED,
            vec!["user:owner".to_string()],
            payload,
            0,
        )
        .unwrap();
        log
    }

    #[test]
    fn open_campaign_passes_guard() {
        let log = EventLog::new();
        assert!(guard_not_sealed(&log, "camp-x").is_ok());
        assert!(!campaign_is_sealed(&log, "camp-x"));
    }

    #[test]
    fn sealed_campaign_is_refused() {
        let log = closed_campaign_log("camp-x");
        assert!(campaign_is_sealed(&log, "camp-x"));
        let err = guard_not_sealed(&log, "camp-x").unwrap_err();
        assert_eq!(err.kind(), "campaign_sealed");
        assert!(err.message().contains("camp-x"));
    }

    #[test]
    fn other_campaign_seal_does_not_leak() {
        // A close of `camp-x` must NOT seal a DIFFERENT campaign `camp-y`.
        let log = closed_campaign_log("camp-x");
        assert!(guard_not_sealed(&log, "camp-y").is_ok());
    }
}
