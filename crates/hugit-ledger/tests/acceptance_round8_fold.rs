//! Round 8 — Wave L, WP L-D: the ledger fold is reject-STICKY WITHIN a record
//! (C5-F1), at the source-of-truth granularity.
//!
//! The C5-F1 attack laundered a sticky reject by carrying the SAME lens twice in
//! ONE `verdict.recorded` record's `claims_checked` vector
//! (`["security:reject","security:approve"]`). The old fold did a plain
//! `per_lens.insert(lens, outcome)` = last-wins, so the trailing approve
//! overwrote the reject for that lens and the intent projected `proven:1,
//! rejected:0`.
//!
//! The recorder now refuses that conflicting input at the door, but the FOLD is
//! the source of truth — these tests drive it DIRECTLY (constructing the record
//! a recorder bug, a legacy record, or a hand-forged log could produce) and pin
//! that within-record stickiness holds regardless of what the recorder writes.
//! Cross-record clearing (the legit same-lens re-approval in a LATER record) is
//! pinned too.

use hugit_contracts::event_record::EventRecord;
use hugit_contracts::verdict_object::{Verdict, VerdictObject};
use hugit_ledger::ledger::Ledger;

fn build_log(events: &[(&str, String)]) -> Vec<EventRecord> {
    use hugit_refstore::log::EventLog;
    let mut log = EventLog::new();
    let mut records = Vec::new();
    for (i, (kind, payload)) in events.iter().enumerate() {
        let r = log.append_for_test(
            *kind,
            vec!["agent".to_string()],
            payload.clone(),
            1_000_000 + i as u64,
        );
        records.push(r);
    }
    records
}

fn intent_landed(id: &str, campaign: &str) -> String {
    serde_json::json!({
        "intent_id": id,
        "campaign": campaign,
        "charter": format!("charter for {id}"),
        "ref": "refs/heads/main",
        "target": "sha:abc",
        "deep_link_target": id,
    })
    .to_string()
}

/// A `verdict.recorded` payload carrying an explicit `claims_checked` vector of
/// `lens:result` entries — the exact shape the recorder writes and the fold
/// reads. `aggregate` is REJECT iff any entry is non-approve (the panel rule).
fn verdict_with_claims(id: &str, claims: &[&str]) -> String {
    let any_non_approve = claims.iter().any(|c| !c.ends_with(":approve"));
    let aggregate = if any_non_approve {
        Verdict::Reject
    } else {
        Verdict::Approve
    };
    let vo = VerdictObject {
        intent: id.to_string(),
        tree_hash: "abc123".to_string(),
        lens: "panel".to_string(),
        model: "test-model".to_string(),
        prompt_digest: "deadbeef".to_string(),
        verdict: aggregate,
        claims_checked: claims.iter().map(|c| c.to_string()).collect(),
        evidence_refs: vec![],
    };
    serde_json::to_string(&vo).unwrap()
}

/// C5-F1 (fold, the attack): a SINGLE record whose `claims_checked` is
/// `["security:reject","security:approve"]` must NOT launder the reject. The fold
/// is reject-sticky within the record → `rejected:1, proven:0`.
#[test]
fn within_record_same_lens_reject_then_approve_stays_rejected() {
    let id = "i-launder";
    let camp = "camp-f1";
    let records = build_log(&[
        ("intent.landed", intent_landed(id, camp)),
        (
            "verdict.recorded",
            verdict_with_claims(id, &["security:reject", "security:approve"]),
        ),
    ]);
    let ledger = Ledger::from_records(&records);
    let e = ledger.by_campaign(camp).next().expect("one intent");
    assert!(
        e.rejected && !e.proven,
        "within-record same-lens reject→approve must stay REJECTED (no launder); \
         got proven={}, rejected={}",
        e.proven,
        e.rejected
    );
    assert_eq!(ledger.proven(camp), 0);
    assert_eq!(ledger.rejected(camp), 1);
}

/// Order-independence: `["security:approve","security:reject"]` in one record is
/// also rejected (a reject anywhere in the record wins).
#[test]
fn within_record_same_lens_approve_then_reject_is_rejected() {
    let id = "i-order";
    let camp = "camp-f1b";
    let records = build_log(&[
        ("intent.landed", intent_landed(id, camp)),
        (
            "verdict.recorded",
            verdict_with_claims(id, &["security:approve", "security:reject"]),
        ),
    ]);
    let ledger = Ledger::from_records(&records);
    let e = ledger.by_campaign(camp).next().expect("one intent");
    assert!(
        e.rejected && !e.proven,
        "a reject anywhere in the record wins"
    );
}

/// A clean within-record approve (no reject anywhere) is proven — the sticky
/// merge does not over-reach and reject a genuinely-approved intent.
#[test]
fn within_record_all_approve_is_proven() {
    let id = "i-clean";
    let camp = "camp-f1c";
    let records = build_log(&[
        ("intent.landed", intent_landed(id, camp)),
        (
            "verdict.recorded",
            verdict_with_claims(id, &["security:approve", "contracts:approve"]),
        ),
    ]);
    let ledger = Ledger::from_records(&records);
    let e = ledger.by_campaign(camp).next().expect("one intent");
    assert!(e.proven && !e.rejected, "all-approve record is proven");
}

/// Cross-record clear is PRESERVED: a within-record reject in record A, then a
/// same-lens approve in a LATER record B, clears (the K-VERDICT cross-record
/// behavior the within-record fix must not break).
#[test]
fn cross_record_same_lens_approve_clears_earlier_reject() {
    let id = "i-clear";
    let camp = "camp-f1d";
    let records = build_log(&[
        ("intent.landed", intent_landed(id, camp)),
        (
            "verdict.recorded",
            verdict_with_claims(id, &["security:reject"]),
        ),
        (
            "verdict.recorded",
            verdict_with_claims(id, &["security:approve"]),
        ),
    ]);
    let ledger = Ledger::from_records(&records);
    let e = ledger.by_campaign(camp).next().expect("one intent");
    assert!(
        e.proven && !e.rejected,
        "a same-lens approve in a LATER record clears the earlier reject; \
         got proven={}, rejected={}",
        e.proven,
        e.rejected
    );
}

/// Cross-lens stickiness is unaffected: a reject under `security` in record A is
/// NOT cleared by an approve under a DIFFERENT lens `style` in record B.
#[test]
fn cross_record_different_lens_approve_does_not_clear_reject() {
    let id = "i-diff";
    let camp = "camp-f1e";
    let records = build_log(&[
        ("intent.landed", intent_landed(id, camp)),
        (
            "verdict.recorded",
            verdict_with_claims(id, &["security:reject"]),
        ),
        (
            "verdict.recorded",
            verdict_with_claims(id, &["style:approve"]),
        ),
    ]);
    let ledger = Ledger::from_records(&records);
    let e = ledger.by_campaign(camp).next().expect("one intent");
    assert!(
        e.rejected && !e.proven,
        "a different-lens approve must NOT clear an outstanding reject"
    );
}
