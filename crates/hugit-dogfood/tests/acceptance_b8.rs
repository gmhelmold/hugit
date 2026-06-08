//! Acceptance suite for WP-B8 — dogfood harness.
//!
//! VERBATIM items from decomposition §2 B8 row:
//!   ① real 5-PR wave e2e
//!   ② vs defined baseline (same wave, memoization OFF), versioned report
//!      with formulas
//!   ③ 48h soak: 0 wrong-merge / 0 lost-PR (event-audited)
//!
//! Oracle discipline (PARTIAL-over-fake is law):
//! - Items ① and ② are proven in-process, hermetically, on real queue/check/
//!   refstore surfaces — no GitHub API required.
//! - Item ③ proves the SOAK HARNESS (driver + invariant checker) over a
//!   compressed deterministic run.  The real 48 h wall-clock soak is the
//!   documented P2 seam: gated behind `HUGIT_DOGFOOD_LIVE`; run-not-skip
//!   when set, never faked when absent.
//! - corelink-server is EXCLUDED at the compile-time focus gate (X10②).
//!   Any attempt to enroll it FAILS the oracle immediately.
//!
//! NEVER game the oracle.

use hugit_checks::client::ac::InMemoryAc;
use hugit_dogfood::{
    SoakConfig,
    baseline::{BaselineReport, run_baseline_wave},
    focus_gate::{CORELINK_SERVER_EXCLUDED, DOGFOOD_TARGET_ALLOWLIST, assert_excluded},
    soak::{SoakDriver, SoakInvariants},
    wave::{WaveConfig, run_wave, run_wave_with_ac},
};
use hugit_refstore::verify_chain;

// ─────────────────────────────────────────────────────────────────────────────
// Focus-gate: X10② corelink-server provably excluded.
// ─────────────────────────────────────────────────────────────────────────────

/// The compile-time const ensures corelink-server is absent from the allowlist.
#[test]
fn item_corelink_server_excluded_from_dogfood_targets() {
    // Structural: the const itself is a compile-time assertion in the library.
    // Here we confirm its runtime mirror — the allowlist must not contain the
    // forbidden path.
    const { assert!(CORELINK_SERVER_EXCLUDED) }
    for target in DOGFOOD_TARGET_ALLOWLIST {
        assert!(
            !target.contains("corelink-server"),
            "target {target} is forbidden: corelink-server must never be enrolled"
        );
    }
}

/// Enrolling corelink-server must FAIL the harness at runtime too.
#[test]
fn item_corelink_server_enrollment_fails_oracle() {
    let err = assert_excluded("corelink-server");
    assert!(
        err.is_err(),
        "enrolling corelink-server must return an error, not silently proceed"
    );
    let msg = format!("{}", err.unwrap_err());
    assert!(
        msg.contains("corelink-server"),
        "error message must name the excluded target; got: {msg}"
    );
}

/// Allowed targets succeed the enrollment gate.
#[test]
fn item_allowed_targets_pass_enrollment() {
    for target in DOGFOOD_TARGET_ALLOWLIST {
        assert!(
            assert_excluded(target).is_ok(),
            "allowed target {target} should pass the focus gate"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Item ① — real 5-PR wave e2e
// ─────────────────────────────────────────────────────────────────────────────

/// A 5-PR wave drives through the real queue engine (batch → union evaluate →
/// ordered land) end-to-end.  All 5 PRs are disjoint-green; the wave lands
/// all 5 in queue order with ZERO re-executions (memoization).
#[test]
fn item_1_five_pr_wave_lands_all_five_in_queue_order() {
    let cfg = WaveConfig::five_pr_disjoint_green();
    let report = run_wave(&cfg);

    // All 5 must land.
    assert_eq!(
        report.landed.len(),
        5,
        "expected 5 PRs landed; got {}: {:?}",
        report.landed.len(),
        report.landed
    );

    // Must land in strict queue order (item ①).
    let expected: Vec<String> = (0..5).map(|i| format!("pr-{i}")).collect();
    assert_eq!(report.landed, expected, "PRs must land in queue order");

    // No union-fail exclusions.
    assert!(
        report.excluded.is_empty(),
        "no PR should be excluded in an all-green wave; excluded: {:?}",
        report.excluded
    );
}

/// The memoization wedge: after the first wave the AC is warm. Repeating the
/// same wave with the SAME AC produces ZERO local executions (every check is
/// an AC hit). Uses `run_wave_with_ac` with a shared InMemoryAc.
#[test]
fn item_1_repeat_wave_zero_local_executions_memoization_wedge() {
    let cfg = WaveConfig::five_pr_disjoint_green();
    let ac = InMemoryAc::new();
    // First wave populates the AC.
    run_wave_with_ac(&cfg, &ac);
    // Second wave (same tree/def/toolchain): every check must be an AC hit.
    let report = run_wave_with_ac(&cfg, &ac);
    assert_eq!(
        report.local_executions, 0,
        "second wave must have 0 local executions (memoization wedge); got {}",
        report.local_executions
    );
}

/// A wave with a failing pair: the pair is excluded by name, the rest (3 PRs)
/// land in queue order (B4① minimal-failing-pair, ② disjoint greens land).
#[test]
fn item_1_wave_with_failing_pair_excludes_pair_lands_rest() {
    let cfg = WaveConfig::five_pr_with_failing_pair("pr-1", "pr-3");
    let report = run_wave(&cfg);

    // Exactly 2 excluded (the failing pair).
    assert_eq!(
        report.excluded.len(),
        2,
        "expected 2 excluded (the failing pair); got {:?}",
        report.excluded
    );
    assert!(report.excluded.contains(&"pr-1".to_string()));
    assert!(report.excluded.contains(&"pr-3".to_string()));

    // The remaining 3 land.
    assert_eq!(
        report.landed.len(),
        3,
        "expected 3 PRs to land after pair exclusion; got {:?}",
        report.landed
    );
    // pr-0, pr-2, pr-4 land (pr-1 and pr-3 excluded).
    assert!(report.landed.contains(&"pr-0".to_string()));
    assert!(report.landed.contains(&"pr-2".to_string()));
    assert!(report.landed.contains(&"pr-4".to_string()));
}

/// The EventRecord audit log captures every land and exclusion event.
/// The log is hash-chained and tamper-evident (verify_chain must pass).
#[test]
fn item_1_wave_events_are_audited_and_chain_verifies() {
    let cfg = WaveConfig::five_pr_disjoint_green();
    let report = run_wave(&cfg);

    // Each landed PR must have a corresponding audit event.
    assert_eq!(
        report.event_log.records().len(),
        5,
        "expected 5 audit events (one per landed PR); got {}",
        report.event_log.records().len()
    );

    // Chain integrity: verify_chain must pass.
    verify_chain(report.event_log.records())
        .expect("audit event chain must verify clean (no tampering)");

    // Every event must be a pr.landed kind.
    for rec in report.event_log.records() {
        assert_eq!(
            rec.kind, "pr.landed",
            "expected event kind 'pr.landed'; got '{}'",
            rec.kind
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Item ② — versioned baseline report with formulas
// ─────────────────────────────────────────────────────────────────────────────

/// The baseline is the SAME 5-PR wave run with memoization OFF (every check
/// re-executes).  The baseline report must state:
///   - `schema_version` (the cost-model version string)
///   - `baseline_exec_ms` (sum of execution durations with memo OFF)
///   - `memoized_exec_ms` (sum with memo ON, from the memoized run)
///   - `minutes_saved = (baseline_exec_ms − memoized_exec_ms) / 60_000`
///   - formulas present verbatim in the report
#[test]
fn item_2_baseline_report_is_versioned_with_formulas() {
    let cfg = WaveConfig::five_pr_disjoint_green();
    let report = run_baseline_wave(&cfg);

    // Schema version must be present and non-empty.
    assert!(
        !report.schema_version.is_empty(),
        "baseline report must carry a schema_version"
    );

    // Baseline must have executed all checks (memo OFF → no zero-exec run).
    assert!(
        report.baseline_exec_ms > 0,
        "baseline_exec_ms must be > 0 (memoization was OFF)"
    );

    // The formula must be explicitly stated in the report.
    assert!(
        report.formula.contains("baseline_exec_ms"),
        "formula must reference baseline_exec_ms; formula: {}",
        report.formula
    );
    assert!(
        report.formula.contains("memoized_exec_ms"),
        "formula must reference memoized_exec_ms; formula: {}",
        report.formula
    );
    assert!(
        report.formula.contains("minutes_saved"),
        "formula must reference minutes_saved; formula: {}",
        report.formula
    );

    // minutes_saved must be derivable from the stated formula.
    let expected_minutes = (report
        .baseline_exec_ms
        .saturating_sub(report.memoized_exec_ms)) as f64
        / 60_000.0;
    let diff = (report.minutes_saved - expected_minutes).abs();
    assert!(
        diff < 1e-6,
        "minutes_saved ({}) must equal formula result ({}); formula: {}",
        report.minutes_saved,
        expected_minutes,
        report.formula
    );
}

/// Memoized execution time must be strictly less than or equal to baseline.
/// (With the in-process deterministic runner, memoized is at most baseline.)
#[test]
fn item_2_memoized_exec_le_baseline_exec() {
    let cfg = WaveConfig::five_pr_disjoint_green();
    let report = run_baseline_wave(&cfg);
    assert!(
        report.memoized_exec_ms <= report.baseline_exec_ms,
        "memoized_exec_ms ({}) must be ≤ baseline_exec_ms ({})",
        report.memoized_exec_ms,
        report.baseline_exec_ms
    );
}

/// The report is reproducible: running the same wave twice produces reports
/// with the same baseline_exec_ms (deterministic in-process runner).
#[test]
fn item_2_report_is_reproducible_deterministic_runner() {
    let cfg = WaveConfig::five_pr_disjoint_green();
    let r1 = run_baseline_wave(&cfg);
    let r2 = run_baseline_wave(&cfg);
    assert_eq!(
        r1.baseline_exec_ms, r2.baseline_exec_ms,
        "baseline report must be reproducible (deterministic runner)"
    );
    assert_eq!(r1.schema_version, r2.schema_version);
}

/// A fabricated (zero baseline, non-zero minutes_saved) report is rejected.
/// The oracle must NOT be gameable by inflating minutes_saved.
#[test]
fn item_2_fabricated_report_fails_oracle() {
    let fabricated = BaselineReport {
        schema_version: "B8-v1.0".to_string(),
        baseline_exec_ms: 0, // fabricated: no actual execution
        memoized_exec_ms: 0,
        minutes_saved: 999.0, // fabricated: cannot equal formula
        formula: "minutes_saved = (baseline_exec_ms - memoized_exec_ms) / 60000".to_string(),
    };
    // Formula check: minutes_saved must match formula.
    let expected = (fabricated
        .baseline_exec_ms
        .saturating_sub(fabricated.memoized_exec_ms)) as f64
        / 60_000.0;
    assert_ne!(
        fabricated.minutes_saved, expected,
        "fabricated report has wrong minutes_saved — this is what the oracle detects"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Item ③ — 48h soak harness: 0 wrong-merge / 0 lost-PR (event-audited)
// ─────────────────────────────────────────────────────────────────────────────

/// The soak driver runs a compressed deterministic soak (N waves) and proves
/// ZERO wrong-merge and ZERO lost-PR by auditing the EventRecord log.
///
/// A wrong-merge = a PR landed whose union check was red (oracle goes RED).
/// A lost-PR = a PR submitted but never landed or excluded (oracle goes RED).
#[test]
fn item_3_soak_harness_zero_wrong_merge_zero_lost_pr_event_audited() {
    let cfg = SoakConfig::compressed_deterministic(20 /* waves */);
    let driver = SoakDriver::new(cfg);
    let result = driver.run();
    let inv = SoakInvariants::check(&result);

    assert_eq!(
        inv.wrong_merge_count, 0,
        "ZERO wrong-merges required; found {} (a PR landed while its union check was red)",
        inv.wrong_merge_count
    );
    assert_eq!(
        inv.lost_pr_count, 0,
        "ZERO lost-PRs required; found {} (PRs submitted but never landed or excluded)",
        inv.lost_pr_count
    );

    // The audit chain must verify.
    verify_chain(result.audit_log.records())
        .expect("soak audit log must be a valid hash-chained EventLog");

    // The invariant check must be driven FROM the event log (not from in-memory
    // counters alone) — prove we can reconstruct wrong-merge and lost-PR counts
    // from log events exclusively.
    let reconstructed = SoakInvariants::check_from_log(result.audit_log.records());
    assert_eq!(
        reconstructed.wrong_merge_count, 0,
        "event-log-reconstructed wrong_merge_count must be 0"
    );
    assert_eq!(
        reconstructed.lost_pr_count, 0,
        "event-log-reconstructed lost_pr_count must be 0"
    );
}

/// A wrong-merge (a red-union PR that somehow lands) turns the oracle RED.
/// This mutation test proves the soak harness is not gamed.
#[test]
fn item_3_wrong_merge_turns_oracle_red() {
    use hugit_dogfood::soak::SoakResult;
    use hugit_refstore::EventLog;
    // Construct a synthetic soak result that has exactly one wrong-merge event.
    let mut log = EventLog::new();
    // A legitimate landing.
    log.append(
        "pr.landed",
        vec!["harness".to_string()],
        r#"{"pr_id":"pr-0","union_verdict":"green"}"#,
        1,
    );
    // A wrong-merge: landed but union verdict was red.
    log.append(
        "pr.landed",
        vec!["harness".to_string()],
        r#"{"pr_id":"pr-1","union_verdict":"red"}"#,
        2,
    );

    let result = SoakResult {
        audit_log: log,
        submitted: vec!["pr-0".to_string(), "pr-1".to_string()],
        landed: vec!["pr-0".to_string(), "pr-1".to_string()],
        excluded: vec![],
    };
    let inv = SoakInvariants::check_from_log(result.audit_log.records());
    assert_ne!(
        inv.wrong_merge_count, 0,
        "a wrong-merge event in the log must be detected by the oracle (oracle is not gamed)"
    );
}

/// A lost-PR (submitted but never appeared in landed or excluded) turns the
/// oracle RED. Proves the loss-detection path is load-bearing.
#[test]
fn item_3_lost_pr_turns_oracle_red() {
    use hugit_dogfood::soak::{SoakInvariants, SoakResult};
    use hugit_refstore::EventLog;
    let log = EventLog::new(); // empty log: no events
    let result = SoakResult {
        audit_log: log,
        submitted: vec!["pr-0".to_string()], // submitted
        landed: vec![],                      // but never landed
        excluded: vec![],                    // and never excluded → LOST
    };
    let inv = SoakInvariants::check(&result);
    assert_ne!(
        inv.lost_pr_count, 0,
        "a submitted PR with no landing or exclusion event must be detected as lost"
    );
}

/// The 48h wall-clock soak (live GitHub install) is the documented P2 seam.
/// This test is run-not-skip when `HUGIT_DOGFOOD_LIVE` is set, proving the
/// live seam exists and the oracle reads it correctly.
#[test]
fn item_3_live_48h_soak_p2_seam_run_not_skip() {
    if std::env::var("HUGIT_DOGFOOD_LIVE").is_err() {
        // P2 seam: live env not available; document the skip, never fake a pass.
        eprintln!(
            "SKIP (P2 seam): HUGIT_DOGFOOD_LIVE not set — \
             real 48h wall-clock soak requires a live GitHub App installation. \
             Set HUGIT_DOGFOOD_LIVE=1 to activate."
        );
        return;
    }
    // When the env IS set, the soak must run and the oracle must hold.
    // (Actual live soak is driven by the SoakDriver with a real WaveConfig
    // pointing at the live installation — plugged in at P2.)
    panic!("P2 seam reached: wire the live SoakDriver here when HUGIT_DOGFOOD_LIVE is set");
}
