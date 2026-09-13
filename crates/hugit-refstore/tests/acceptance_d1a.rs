//! WP-D1a acceptance oracle — hugit-refstore event-log core.
//!
//! Owned items (VERBATIM from decomposition v2.0 — D1):
//!   ① 10k-event replay identical
//!   ② tamper detected
//!
//! Driven by `tests/acceptance/wp-d1a/run.sh`.

use hugit_refstore::log::EventLog;
use hugit_refstore::replay::{replay, replay_unchecked};
use hugit_refstore::tamper::verify_chain;

/// Deterministically build a 10k-event log of ref mutations interleaved with
/// inert events, so the projected ref state is non-trivial and the chain is long.
fn build_10k_log() -> EventLog {
    let mut log = EventLog::new();
    let n = 10_000u64;
    for i in 0..n {
        let principal_chain = vec![format!("agent:runner-{:02}", i % 7), "user:gustavo".into()];
        let recorded_at = 1_717_000_000_000 + i;
        match i % 5 {
            // ref.update — set refs/heads/branch-<bucket> to a rolling oid.
            0..=2 => {
                let bucket = i % 64;
                let payload =
                    format!(r#"{{"ref":"refs/heads/branch-{bucket}","target":"oid-{i:064x}"}}"#);
                log.append_for_test("ref.update", principal_chain, payload, recorded_at);
            }
            // ref.delete — occasionally drop a ref (may be a no-op; still chained).
            3 => {
                let bucket = i % 64;
                let payload = format!(r#"{{"ref":"refs/heads/branch-{bucket}"}}"#);
                log.append_for_test("ref.delete", principal_chain, payload, recorded_at);
            }
            // inert event — advances the chain, does not touch ref state.
            _ => {
                let payload = format!(r#"{{"note":"checkpoint-{i}"}}"#);
                log.append_for_test("checkpoint.noted", principal_chain, payload, recorded_at);
            }
        }
    }
    log
}

/// ① 10k-event replay identical.
///
/// A 10k-event log, replayed twice, must project to the byte-for-byte identical
/// ref state. Replay is pure and deterministic.
#[test]
fn item_1_replay_identical() {
    let log = build_10k_log();
    assert_eq!(log.len(), 10_000, "log must hold all 10k events");

    // Chain is intact, so replay succeeds.
    let state_a = replay(&log).expect("replay must succeed on an intact 10k chain");
    let state_b = replay(&log).expect("second replay must succeed");

    // Determinism: equal logs → equal ref states → identical canonical bytes.
    assert_eq!(
        state_a, state_b,
        "two replays of the same log must be equal"
    );
    assert_eq!(
        state_a.canonical_bytes(),
        state_b.canonical_bytes(),
        "two replays must be byte-for-byte identical"
    );

    // Replaying a freshly-rebuilt-but-identical log must also match, proving the
    // projection depends only on the record sequence, not on object identity.
    let log2 = build_10k_log();
    let state_c = replay(&log2).expect("replay of an equal log must succeed");
    assert_eq!(
        state_a.canonical_bytes(),
        state_c.canonical_bytes(),
        "an independently-built identical log must project identically"
    );

    // The projection must be non-trivial (the proof would be vacuous otherwise).
    assert!(!state_a.is_empty(), "10k ref ops must leave live refs");

    // The unchecked fold over the same records agrees with the checked replay.
    let state_unchecked =
        replay_unchecked(log.records()).expect("unchecked fold of intact records succeeds");
    assert_eq!(state_a, state_unchecked);
}

/// ② tamper detected.
///
/// Each tamper class — altering a record's content, splicing the chain,
/// dropping a record, inserting/reordering — must break chain verification and,
/// fail-closed, refuse replay. An intact chain must verify.
#[test]
fn item_2_tamper_detected() {
    // Baseline intact chain verifies and replays.
    let log = build_10k_log();
    verify_chain(log.records()).expect("intact chain must verify");
    assert!(replay(&log).is_ok(), "intact chain must replay");

    // (a) ALTERED content: flip a payload byte in the middle without fixing the
    // hash → this_hash mismatch.
    {
        let mut records = log.records().to_vec();
        records[5_000].payload.push('X');
        assert!(
            verify_chain(&records).is_err(),
            "altered payload must be detected"
        );
        let mut tampered = EventLog::new();
        // push_record enforces seq monotonicity but not hashes; replay must still
        // refuse the broken chain.
        for r in records {
            tampered.push_record(r).expect("seq stays monotonic");
        }
        assert!(
            replay(&tampered).is_err(),
            "replay must fail closed on altered content"
        );
    }

    // (b) ALTERED this_hash directly → mismatch with recomputed formula.
    {
        let mut records = log.records().to_vec();
        records[1_234].this_hash =
            "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".to_string();
        assert!(
            verify_chain(&records).is_err(),
            "forged this_hash must be detected"
        );
    }

    // (c) DROPPED record: remove one from the middle → seq gap (and prev_hash
    // break) detected.
    {
        let mut records = log.records().to_vec();
        records.remove(2_000);
        assert!(
            verify_chain(&records).is_err(),
            "dropped record must be detected"
        );
    }

    // (d) INSERTED / reordered record: swap two adjacent records → seq out of
    // order and/or prev_hash break.
    {
        let mut records = log.records().to_vec();
        records.swap(7_000, 7_001);
        assert!(
            verify_chain(&records).is_err(),
            "reordered records must be detected"
        );
    }

    // (e) SPLICED prev_hash: rewrite one prev_hash link → linkage break.
    {
        let mut records = log.records().to_vec();
        records[8_888].prev_hash =
            "0000000000000000000000000000000000000000000000000000000000000000".to_string();
        assert!(
            verify_chain(&records).is_err(),
            "spliced prev_hash must be detected"
        );
    }

    // Control: an untouched copy still verifies (no false positive).
    let pristine = log.records().to_vec();
    verify_chain(&pristine).expect("untouched chain must remain valid");
}

/// D1 regression (user walkthrough): a captured CHECKOUT (`ref.update` with
/// `checkout:true` and no `ref`/`target`) is an INERT event for the ref view —
/// the projection must NOT fail closed on it. Before this fix, `export` on any
/// repo that had ever captured a checkout died with "malformed payload for
/// ref.update event".
#[test]
fn checkout_ref_update_is_inert_not_malformed() {
    let mut log = EventLog::new();
    // A normal commit capture advances refs/heads/main.
    log.append_for_test(
        "ref.update",
        vec!["orchestrator:hugit-hook".to_string()],
        r#"{"ref":"refs/heads/main","target":"1111111111111111111111111111111111111111"}"#
            .to_string(),
        1,
    );
    // Current checkout schema keeps old/new alongside legacy from/to. It remains
    // an inert observation, so undo/replay ref projection cannot reinterpret it.
    log.append_for_test(
        "ref.update",
        vec!["orchestrator:hugit-hook".to_string()],
        r#"{"checkout":{"fact":"nonzero_old_oid","truth":"branch_checkout_observed"},"from":"1111111111111111111111111111111111111111","to":"2222222222222222222222222222222222222222","old":"1111111111111111111111111111111111111111","new":"2222222222222222222222222222222222222222","branch":"feat/x"}"#.to_string(),
        2,
    );
    // Another commit on the other branch — still inert skip of the checkout.
    log.append_for_test(
        "ref.update",
        vec!["orchestrator:hugit-hook".to_string()],
        r#"{"ref":"refs/heads/feat/x","target":"3333333333333333333333333333333333333333"}"#
            .to_string(),
        3,
    );

    let state = replay(&log).expect("checkout must not make replay fail");
    assert_eq!(
        state.get("refs/heads/main"),
        Some("1111111111111111111111111111111111111111"),
        "main ref survives the inert checkout"
    );
    assert_eq!(
        state.get("refs/heads/feat/x"),
        Some("3333333333333333333333333333333333333333"),
        "the post-checkout commit ref is the projected value"
    );
}

/// A captured PUSH ATTEMPT (`ref.update` with `attempt:true` and no
/// `ref`/`target`) is also inert. A pre-push hook records the proposed local
/// shas for provenance, but it must not make replay/export reject the log.
#[test]
fn push_attempt_ref_update_is_inert_not_malformed() {
    let mut log = EventLog::new();
    log.append_for_test(
        "ref.update",
        vec!["orchestrator:hugit-hook".to_string()],
        r#"{"ref":"refs/heads/main","target":"1111111111111111111111111111111111111111"}"#
            .to_string(),
        1,
    );
    log.append_for_test(
        "ref.update",
        vec!["orchestrator:hugit-hook".to_string()],
        r#"{"attempt":true,"refspecs":"refs/heads/main 1111111111111111111111111111111111111111 refs/heads/main 1111111111111111111111111111111111111111","shas":["1111111111111111111111111111111111111111"]}"#
            .to_string(),
        2,
    );

    let state = replay(&log).expect("push attempt must not make replay fail");
    assert_eq!(
        state.get("refs/heads/main"),
        Some("1111111111111111111111111111111111111111")
    );
}
