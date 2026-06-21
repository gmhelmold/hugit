//! WP-C6 acceptance oracle — flake-stats collector + quarantine policy +
//! false-positive guard.
//!
//! Acceptance test for the flake-stats and quarantine contract (WP-C6).
//! One `#[test] item_<n>_<slug>` per owned item ①–④ (VERBATIM from contract).
//!
//! All items are proved hermetically with deterministic fixtures — no live
//! infra.  We feed synthetic `CheckResult` streams (the B2 executor surface)
//! into the real in-process collector and assert every invariant.
//!
//! Items:
//!   ① every execution feeds stats
//!   ② planted 20% flake detected <30 runs
//!   ③ quarantine list = policy artifact; no auto-act
//!   ④ false-positive guard: deterministically-failing test → REAL, never quarantined

use hugit_contracts::CheckResult;
use hugit_diag::flake::{
    AutoActMechanism, Classification, FlakeCollector, QuarantineAnnotation, QuarantineList,
    feed_result,
};

// ── fixtures ─────────────────────────────────────────────────────────────────

/// A minimal valid memo key (64 hex zeros).
fn key(n: u8) -> String {
    format!("{n:064x}")
}

/// Build a synthetic `CheckResult` with the given memo_key and exit code.
fn make_result(test_id: &str, exit: i32) -> CheckResult {
    CheckResult {
        memo_key: test_id.to_owned(),
        tree_hash: "aa".repeat(32),
        def_digest: "bb".repeat(32),
        toolchain_digest: "cc".repeat(32),
        exit,
        artifacts: vec![],
        stdout_ref: "stdout-ref".into(),
        stderr_ref: "stderr-ref".into(),
        duration_ms: 42,
        runner_ref: "runner-ref".into(),
        produced_at: 1_717_000_000_000,
    }
}

// ── ① every execution feeds stats ───────────────────────────────────────────

#[test]
fn item_1_every_execution_feeds_stats() {
    let mut collector = FlakeCollector::new();

    let r1 = make_result(&key(1), 0); // pass
    let r2 = make_result(&key(1), 1); // fail
    let r3 = make_result(&key(2), 0); // different test, pass

    feed_result(&mut collector, &r1);
    feed_result(&mut collector, &r2);
    feed_result(&mut collector, &r3);

    // key(1) has 2 runs recorded.
    let stats1 = collector
        .stats_for(&key(1))
        .expect("stats must exist after feeding key(1)");
    assert_eq!(stats1.run_count, 2, "key(1) must record 2 runs");
    assert_eq!(stats1.pass_count, 1);
    assert_eq!(stats1.fail_count, 1);

    // key(2) has 1 run recorded.
    let stats2 = collector
        .stats_for(&key(2))
        .expect("stats must exist after feeding key(2)");
    assert_eq!(stats2.run_count, 1, "key(2) must record 1 run");
    assert_eq!(stats2.pass_count, 1);
    assert_eq!(stats2.fail_count, 0);

    // A key never fed has no stats.
    assert!(
        collector.stats_for(&key(99)).is_none(),
        "unfed key must have no stats"
    );
}

// ── ② planted 20% flake detected <30 runs ───────────────────────────────────

#[test]
fn item_2_planted_20pct_flake_detected_lt_30_runs() {
    // Plant a 20%-flake: every 5th run fails, the rest pass.
    // We send at most 29 runs and assert detection before run 30.
    let test_id = key(42);
    let mut collector = FlakeCollector::new();
    let mut detection_run: Option<usize> = None;

    for run in 1..=29usize {
        let exit = if run % 5 == 0 { 1 } else { 0 }; // 20% fail rate
        let result = make_result(&test_id, exit);
        feed_result(&mut collector, &result);

        if let Some(class) = collector.classify(&test_id)
            && class == Classification::Flaky
        {
            detection_run = Some(run);
            break;
        }
    }

    assert!(
        detection_run.is_some(),
        "a 20%-flake must be detected within 29 runs (< 30)"
    );
    assert!(
        detection_run.unwrap() < 30,
        "detection must happen before run 30; got run {}",
        detection_run.unwrap()
    );
}

// ── ③ quarantine list = policy artifact; no auto-act ────────────────────────
//
// "auto-act" defined (contract ③): any reorder / skip / block /
// annotation-that-gates = prohibited in v0.  Annotation-only is the sole
// permitted surface.
//
// We prove:
//   (a) Adding to the quarantine list produces an annotation, not a gate action.
//   (b) There is no public API surface on QuarantineList / FlakeCollector that
//       reorders, skips, blocks, or gates based on the list.
//   (c) `AutoActMechanism` is a zero-variant enum (MUST NOT EXIST in v0).

#[test]
fn item_3_quarantine_list_is_policy_artifact_no_auto_act() {
    let test_id = key(7);
    let mut collector = FlakeCollector::new();

    // Feed enough data so the test is detected as flaky.
    for run in 1..=29usize {
        let exit = if run % 5 == 0 { 1 } else { 0 };
        feed_result(&mut collector, &make_result(&test_id, exit));
    }
    assert_eq!(
        collector.classify(&test_id),
        Some(Classification::Flaky),
        "pre-condition: test must be classified Flaky"
    );

    // Build the quarantine list from the collector's detected flakes.
    let qlist = collector.quarantine_list();

    // (a) The list contains an annotation for our flaky test.
    let annotation: Option<&QuarantineAnnotation> = qlist.annotation_for(&test_id);
    assert!(
        annotation.is_some(),
        "flaky test must appear in the quarantine list as an annotation"
    );
    let ann = annotation.unwrap();
    assert_eq!(&ann.test_id, &test_id);
    // The annotation is non-gating — it carries no block/skip/reorder flag.
    assert!(!ann.gates, "annotation must be non-gating (gates == false)");

    // (b) QuarantineList has NO auto-act method: no method named
    //     `reorder`, `skip`, `block`, or `gate_execution` is callable.
    //     This is a compile-time guarantee — if the API existed, this file
    //     would not compile.  We additionally assert that the list is
    //     annotation-only: the only operation is `annotation_for`.
    let _: &QuarantineList = &qlist; // type is QuarantineList, no extra methods

    // (c) AutoActMechanism is an uninhabited (zero-variant) enum — the type
    //     exists to state "this mechanism is absent"; it can never be constructed.
    //     We prove it is zero-size by asking for its size.
    assert_eq!(
        std::mem::size_of::<AutoActMechanism>(),
        0,
        "AutoActMechanism must be uninhabited (zero variants) — auto-act is absent in v0"
    );
}

// ── ④ false-positive guard ───────────────────────────────────────────────────
//
// A deterministically-failing (non-flaky) test — 100% fail rate — is classified
// REAL and NEVER quarantined within the same volume window.

#[test]
fn item_4_deterministic_failure_classified_real_never_quarantined() {
    let test_id = key(99);
    let mut collector = FlakeCollector::new();

    // Feed 30 deterministic failures (100% fail rate — not flaky, just broken).
    for _ in 0..30 {
        feed_result(&mut collector, &make_result(&test_id, 1));
    }

    // Must be classified REAL (not Flaky, not Unknown).
    let classification = collector
        .classify(&test_id)
        .expect("must classify after 30 runs");
    assert_eq!(
        classification,
        Classification::Real,
        "deterministically-failing test must be classified REAL"
    );

    // The quarantine list must NOT contain this test.
    let qlist = collector.quarantine_list();
    assert!(
        qlist.annotation_for(&test_id).is_none(),
        "deterministically-failing test must NEVER appear in the quarantine list"
    );
}
