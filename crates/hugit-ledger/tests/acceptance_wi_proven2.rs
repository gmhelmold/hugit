//! WP-WI-PROVEN2 acceptance suite — latest-verdict-wins for proven/rejected.
//!
//! Defect (opus1-authz, Round 5): the ledger folded `verdict.recorded` events
//! as a MONOTONIC OR — Approve → proven=true, Reject → rejected=true, neither
//! ever CLEARED the other.  So approve-then-REJECT left `proven:1, rejected:1`
//! even though the LATEST verdict was reject.  Fix: the LAST `verdict.recorded`
//! event for an intent is authoritative; `proven` and `rejected` are MUTUALLY
//! EXCLUSIVE.
//!
//! # Tests
//! 1. **approve-then-reject** → proven:0, rejected:1  (latest=reject wins)
//! 2. **reject-then-approve** → proven:1, rejected:0  (latest=approve wins)
//! 3. **single approve** → proven:1, rejected:0  (non-regression WH-PROVEN/B3B4)
//! 4. **single reject** → proven:0, rejected:1  (non-regression WH-PROVEN)
//! 5. **multiple revisions, last wins** → only the final outcome governs
//! 6. **mutually exclusive invariant** — for every entry, proven XOR rejected
//!    (proven && rejected is never simultaneously true)

use hugit_contracts::event_record::EventRecord;
use hugit_contracts::verdict_object::{Verdict, VerdictObject};
use hugit_ledger::ledger::Ledger;

// ── fixture helpers ──────────────────────────────────────────────────────────

/// Build a well-formed EventLog from (kind, payload) tuples with sequential seqs.
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

fn verdict_recorded(id: &str, v: Verdict) -> String {
    let vo = VerdictObject {
        intent: id.to_string(),
        tree_hash: "abc123".to_string(),
        lens: "security".to_string(),
        model: "test-model".to_string(),
        prompt_digest: "deadbeef".to_string(),
        verdict: v,
        claims_checked: vec!["c1".to_string()],
        evidence_refs: vec![],
    };
    serde_json::to_string(&vo).unwrap()
}

// ── 1. approve-then-reject → proven:0, rejected:1 ───────────────────────────

/// WI-PROVEN2 primary: approve-then-reject — the LATEST verdict is reject.
/// Before the fix: proven=true, rejected=true (stuck proven on revision).
/// After the fix: proven=false, rejected=true (mutually exclusive, latest wins).
#[test]
fn approve_then_reject_latest_reject_wins() {
    let id = "i-revision-a2r";
    let camp = "camp-wi";

    let records = build_log(&[
        ("intent.landed", intent_landed(id, camp)),
        ("verdict.recorded", verdict_recorded(id, Verdict::Approve)),
        ("verdict.recorded", verdict_recorded(id, Verdict::Reject)),
    ]);

    let ledger = Ledger::from_records(&records);
    let entries: Vec<_> = ledger.by_campaign(camp).collect();
    assert_eq!(entries.len(), 1, "one intent in campaign");
    let e = &entries[0];

    assert!(
        !e.proven,
        "approve-then-REJECT: proven must be false (latest verdict is reject); got proven={}, rejected={}",
        e.proven, e.rejected
    );
    assert!(
        e.rejected,
        "approve-then-REJECT: rejected must be true (latest verdict is reject); got proven={}, rejected={}",
        e.proven, e.rejected
    );

    // Mutually exclusive: never both true simultaneously.
    assert!(
        !(e.proven && e.rejected),
        "proven and rejected must be mutually exclusive; got proven={}, rejected={}",
        e.proven,
        e.rejected
    );

    // Ledger counts are coherent.
    assert_eq!(ledger.proven(camp), 0, "proven count must be 0");
    assert_eq!(ledger.rejected(camp), 1, "rejected count must be 1");
}

// ── 2. reject-then-approve → proven:1, rejected:0 ───────────────────────────

/// WI-PROVEN2: reject-then-approve — the LATEST verdict is approve.
/// The reject is revised to approve: proven=true, rejected=false.
#[test]
fn reject_then_approve_latest_approve_wins() {
    let id = "i-revision-r2a";
    let camp = "camp-wi-r2a";

    let records = build_log(&[
        ("intent.landed", intent_landed(id, camp)),
        ("verdict.recorded", verdict_recorded(id, Verdict::Reject)),
        ("verdict.recorded", verdict_recorded(id, Verdict::Approve)),
    ]);

    let ledger = Ledger::from_records(&records);
    let entries: Vec<_> = ledger.by_campaign(camp).collect();
    assert_eq!(entries.len(), 1, "one intent in campaign");
    let e = &entries[0];

    assert!(
        e.proven,
        "reject-then-APPROVE: proven must be true (latest verdict is approve); got proven={}, rejected={}",
        e.proven, e.rejected
    );
    assert!(
        !e.rejected,
        "reject-then-APPROVE: rejected must be false (latest verdict is approve); got proven={}, rejected={}",
        e.proven, e.rejected
    );

    // Mutually exclusive.
    assert!(
        !(e.proven && e.rejected),
        "proven and rejected must be mutually exclusive; got proven={}, rejected={}",
        e.proven,
        e.rejected
    );

    assert_eq!(ledger.proven(camp), 1, "proven count must be 1");
    assert_eq!(ledger.rejected(camp), 0, "rejected count must be 0");
}

// ── 3. single approve → proven:1, rejected:0 (non-regression WH-PROVEN/B3B4) ─

/// WH-PROVEN non-regression: a single approve verdict must still produce proven=1.
/// This is the happy path that must not be broken by the latest-wins fix.
#[test]
fn single_approve_proven_non_regression() {
    let id = "i-single-approve";
    let camp = "camp-wi-approve";

    let records = build_log(&[
        ("intent.landed", intent_landed(id, camp)),
        ("verdict.recorded", verdict_recorded(id, Verdict::Approve)),
    ]);

    let ledger = Ledger::from_records(&records);
    let entries: Vec<_> = ledger.by_campaign(camp).collect();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];

    assert!(
        e.proven,
        "single APPROVE: proven must be true; got proven={}, rejected={}",
        e.proven, e.rejected
    );
    assert!(
        !e.rejected,
        "single APPROVE: rejected must be false; got proven={}, rejected={}",
        e.proven, e.rejected
    );

    assert_eq!(
        ledger.proven(camp),
        1,
        "proven count must be 1 (non-regression)"
    );
    assert_eq!(
        ledger.rejected(camp),
        0,
        "rejected count must be 0 (non-regression)"
    );
}

// ── 4. single reject → proven:0, rejected:1 (non-regression WH-PROVEN) ──────

/// WH-PROVEN non-regression: a single reject verdict must still produce
/// proven=0, rejected=1.
#[test]
fn single_reject_rejected_non_regression() {
    let id = "i-single-reject";
    let camp = "camp-wi-reject";

    let records = build_log(&[
        ("intent.landed", intent_landed(id, camp)),
        ("verdict.recorded", verdict_recorded(id, Verdict::Reject)),
    ]);

    let ledger = Ledger::from_records(&records);
    let entries: Vec<_> = ledger.by_campaign(camp).collect();
    assert_eq!(entries.len(), 1);
    let e = &entries[0];

    assert!(
        !e.proven,
        "single REJECT: proven must be false; got proven={}, rejected={}",
        e.proven, e.rejected
    );
    assert!(
        e.rejected,
        "single REJECT: rejected must be true; got proven={}, rejected={}",
        e.proven, e.rejected
    );

    assert_eq!(
        ledger.proven(camp),
        0,
        "proven count must be 0 (non-regression)"
    );
    assert_eq!(
        ledger.rejected(camp),
        1,
        "rejected count must be 1 (non-regression)"
    );
}

// ── 5. multiple revisions — last one wins ────────────────────────────────────

/// WI-PROVEN2: multiple revisions (approve, reject, approve, reject, approve)
/// — only the FINAL outcome governs.  Here the sequence ends on Approve.
#[test]
fn multiple_revisions_last_wins() {
    let id = "i-multi-rev";
    let camp = "camp-wi-multi";

    let records = build_log(&[
        ("intent.landed", intent_landed(id, camp)),
        ("verdict.recorded", verdict_recorded(id, Verdict::Approve)),
        ("verdict.recorded", verdict_recorded(id, Verdict::Reject)),
        ("verdict.recorded", verdict_recorded(id, Verdict::Approve)),
        ("verdict.recorded", verdict_recorded(id, Verdict::Reject)),
        ("verdict.recorded", verdict_recorded(id, Verdict::Approve)), // LAST = approve
    ]);

    let ledger = Ledger::from_records(&records);
    let entries: Vec<_> = ledger.by_campaign(camp).collect();
    let e = &entries[0];

    assert!(
        e.proven,
        "5-revision sequence ending on Approve: proven must be true; got proven={}, rejected={}",
        e.proven, e.rejected
    );
    assert!(
        !e.rejected,
        "5-revision sequence ending on Approve: rejected must be false; got proven={}, rejected={}",
        e.proven, e.rejected
    );
    assert_eq!(ledger.proven(camp), 1);
    assert_eq!(ledger.rejected(camp), 0);

    // Same fixture but last verdict is Reject.
    let camp2 = "camp-wi-multi2";
    let id2 = "i-multi-rev2";
    let records2 = build_log(&[
        ("intent.landed", intent_landed(id2, camp2)),
        ("verdict.recorded", verdict_recorded(id2, Verdict::Approve)),
        ("verdict.recorded", verdict_recorded(id2, Verdict::Reject)),
        ("verdict.recorded", verdict_recorded(id2, Verdict::Approve)),
        ("verdict.recorded", verdict_recorded(id2, Verdict::Reject)), // LAST = reject
    ]);
    let ledger2 = Ledger::from_records(&records2);
    let entries2: Vec<_> = ledger2.by_campaign(camp2).collect();
    let e2 = &entries2[0];
    assert!(
        !e2.proven,
        "4-revision sequence ending on Reject: proven must be false; got proven={}, rejected={}",
        e2.proven, e2.rejected
    );
    assert!(
        e2.rejected,
        "4-revision sequence ending on Reject: rejected must be true; got proven={}, rejected={}",
        e2.proven, e2.rejected
    );
}

// ── 6. mutually exclusive invariant across a mixed campaign ──────────────────

/// WI-PROVEN2 invariant: for EVERY entry in any campaign, proven XOR rejected
/// holds — proven && rejected is never simultaneously true.
///
/// Fixture: 4 intents with different verdict histories.
#[test]
fn mutually_exclusive_proven_rejected_invariant() {
    let camp = "camp-wi-invariant";

    let records = build_log(&[
        // intent A: no verdict yet — neither proven nor rejected.
        ("intent.landed", intent_landed("ia", camp)),
        // intent B: single approve.
        ("intent.landed", intent_landed("ib", camp)),
        ("verdict.recorded", verdict_recorded("ib", Verdict::Approve)),
        // intent C: approve then reject (the broken case pre-fix).
        ("intent.landed", intent_landed("ic", camp)),
        ("verdict.recorded", verdict_recorded("ic", Verdict::Approve)),
        ("verdict.recorded", verdict_recorded("ic", Verdict::Reject)),
        // intent D: reject then approve.
        ("intent.landed", intent_landed("id", camp)),
        ("verdict.recorded", verdict_recorded("id", Verdict::Reject)),
        ("verdict.recorded", verdict_recorded("id", Verdict::Approve)),
    ]);

    let ledger = Ledger::from_records(&records);
    let entries: Vec<_> = ledger.by_campaign(camp).collect();
    assert_eq!(entries.len(), 4, "four intents in campaign");

    // Invariant: proven && rejected must NEVER be simultaneously true.
    for e in &entries {
        assert!(
            !(e.proven && e.rejected),
            "intent '{}': proven and rejected must be mutually exclusive; \
             got proven={}, rejected={}",
            e.intent_id,
            e.proven,
            e.rejected
        );
    }

    // Spot-check the expected outcomes.
    let by_id: std::collections::HashMap<_, _> =
        entries.iter().map(|e| (e.intent_id.as_str(), e)).collect();

    let ia = by_id["ia"];
    assert!(
        !ia.proven && !ia.rejected,
        "ia: no verdict → neither proven nor rejected"
    );

    let ib = by_id["ib"];
    assert!(
        ib.proven && !ib.rejected,
        "ib: single approve → proven only"
    );

    let ic = by_id["ic"];
    assert!(
        !ic.proven && ic.rejected,
        "ic: approve-then-reject → rejected only (latest wins)"
    );

    let id_entry = by_id["id"];
    assert!(
        id_entry.proven && !id_entry.rejected,
        "id: reject-then-approve → proven only (latest wins)"
    );

    // Counts.
    assert_eq!(ledger.proven(camp), 2, "proven count: ib + id");
    assert_eq!(ledger.rejected(camp), 1, "rejected count: ic only");
}
