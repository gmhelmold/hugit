//! Branch-protection guard (⑥): a protected / required-review PR is HELD and
//! reported, NEVER force-merged.
//!
//! GitHub branch protection (required reviews, required status checks, required
//! signatures, restricted pushers) expresses repository policy that the queue
//! MUST honor. When a PR sits under unmet protection, the queue does not — and
//! structurally *cannot* — force it through: there is no force-merge path in
//! this crate. Instead it produces a HOLD, records it as an
//! [`hugit_contracts::EventRecord`], and surfaces the status. The held PR lands
//! only once GitHub itself reports protection satisfied.

use hugit_contracts::EventRecord;

/// The branch-protection facts for a PR, as reported by GitHub. This is the
/// *observed* protection state — the queue never edits or bypasses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectionStatus {
    /// Branch protection is enabled on the target branch.
    pub protection_enabled: bool,
    /// A review is required and has not yet been approved.
    pub required_review_pending: bool,
    /// A required status check has not yet succeeded.
    pub required_check_pending: bool,
}

impl ProtectionStatus {
    /// An unprotected, ready PR — nothing to hold for.
    pub fn clear() -> Self {
        Self {
            protection_enabled: false,
            required_review_pending: false,
            required_check_pending: false,
        }
    }
}

/// Why a PR is being held. A hold is reported, never silently dropped and
/// never overridden by a force path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HoldReason {
    /// A required review has not been approved.
    RequiredReviewPending,
    /// A required status check has not succeeded.
    RequiredCheckPending,
}

impl HoldReason {
    /// Stable string used in the recorded event payload.
    fn as_str(&self) -> &'static str {
        match self {
            HoldReason::RequiredReviewPending => "required_review_pending",
            HoldReason::RequiredCheckPending => "required_check_pending",
        }
    }
}

/// Evaluate branch protection for one PR.
///
/// Returns the (possibly empty) set of hold reasons, plus an
/// [`EventRecord`]-shaped audit event when the PR is held. There is no return
/// value, and no code path anywhere in this crate, that lets a held PR merge:
/// the hold is the terminal decision until GitHub reports protection satisfied.
///
/// `seq` / `prev_hash` thread the append-only event log (the real hash-chain is
/// computed by the ledger; here we transcribe the held decision into the frozen
/// envelope so the hold is auditable end-to-end).
pub fn evaluate_protection(
    item_id: &str,
    status: &ProtectionStatus,
    seq: u64,
    prev_hash: &str,
    recorded_at: u64,
) -> (Vec<HoldReason>, Option<EventRecord>) {
    if !status.protection_enabled {
        return (Vec::new(), None);
    }
    let mut reasons = Vec::new();
    if status.required_review_pending {
        reasons.push(HoldReason::RequiredReviewPending);
    }
    if status.required_check_pending {
        reasons.push(HoldReason::RequiredCheckPending);
    }
    if reasons.is_empty() {
        // Protection enabled but fully satisfied → not held. The PR may land
        // through the normal engine path; protection was honored by waiting.
        return (Vec::new(), None);
    }

    let payload = format!(
        "{{\"item_id\":\"{}\",\"held\":true,\"reasons\":[{}]}}",
        item_id,
        reasons
            .iter()
            .map(|r| format!("\"{}\"", r.as_str()))
            .collect::<Vec<_>>()
            .join(",")
    );
    let event = EventRecord {
        seq,
        prev_hash: prev_hash.to_string(),
        // The ledger recomputes the real chained digest; this records the
        // held decision into the frozen envelope. Never a force-merge marker.
        this_hash: String::new(),
        kind: "queue.protection_hold".to_string(),
        principal_chain: vec!["hugit-queue/github".to_string()],
        payload,
        recorded_at,
    };
    (reasons, Some(event))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unprotected_pr_is_not_held() {
        let (reasons, event) =
            evaluate_protection("pr-1", &ProtectionStatus::clear(), 1, &"0".repeat(64), 0);
        assert!(reasons.is_empty());
        assert!(event.is_none());
    }

    #[test]
    fn protected_satisfied_pr_is_not_held() {
        let status = ProtectionStatus {
            protection_enabled: true,
            required_review_pending: false,
            required_check_pending: false,
        };
        let (reasons, event) = evaluate_protection("pr-1", &status, 1, &"0".repeat(64), 0);
        assert!(reasons.is_empty());
        assert!(event.is_none());
    }

    #[test]
    fn required_review_pending_holds_and_records() {
        let status = ProtectionStatus {
            protection_enabled: true,
            required_review_pending: true,
            required_check_pending: false,
        };
        let (reasons, event) = evaluate_protection("pr-7", &status, 42, &"a".repeat(64), 123);
        assert_eq!(reasons, vec![HoldReason::RequiredReviewPending]);
        let ev = event.expect("a held PR records an EventRecord");
        assert_eq!(ev.kind, "queue.protection_hold");
        assert_eq!(ev.seq, 42);
        assert!(ev.payload.contains("pr-7"));
        assert!(ev.payload.contains("required_review_pending"));
        assert!(ev.payload.contains("\"held\":true"));
    }

    #[test]
    fn multiple_pending_requirements_all_reported() {
        let status = ProtectionStatus {
            protection_enabled: true,
            required_review_pending: true,
            required_check_pending: true,
        };
        let (reasons, event) = evaluate_protection("pr-9", &status, 1, &"0".repeat(64), 0);
        assert_eq!(
            reasons,
            vec![
                HoldReason::RequiredReviewPending,
                HoldReason::RequiredCheckPending
            ]
        );
        assert!(event.is_some());
    }
}
