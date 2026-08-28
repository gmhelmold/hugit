//! WP-B9 acceptance tests — exit telemetry + the money gate.
//! Contract: the work-package contract
//!
//! Tests:
//!   item_1_week3_retention_computable_vs_40pct
//!   item_2_unprompted_vs_prompted_classifier
//!   item_3_exit_report_generated_auditable
//!   item_4_cohort_window_guards_insufficient_not_pass
//!   item_5_gate_pass_fail_2_unprompted_fail_3_pass
//!   item_6_money_gate_blocks_billing_until_pass

use hugit_app_exit::{
    classifier::{FeedbackKind, FeedbackSignal, count_unprompted},
    cohort::{CohortGuardResult, CohortState, evaluate_cohort_guards},
    gate::{GateEvaluatorState, MoneyGate},
    report::{ExitReportStatus, UNPROMPTED_GATE, generate_report},
    retention::{ActivityEvent, RETENTION_THRESHOLD, RetentionResult, compute_retention},
};

// ─── helpers ────────────────────────────────────────────────────────────────

fn valid_cohort() -> CohortState {
    CohortState {
        team_count: 10,
        min_weeks_real_use: 3,
        days_since_anchor: Some(30),
    }
}

fn mk_activity(install_id: &str, week: u32, active: bool) -> ActivityEvent {
    ActivityEvent {
        install_id: install_id.to_string(),
        week,
        active,
    }
}

fn mk_signal(install_id: &str, kind: FeedbackKind) -> FeedbackSignal {
    FeedbackSignal {
        install_id: install_id.to_string(),
        kind,
        text: "I'd pay for this".to_string(),
    }
}

/// Build 10 installs each with a week-3 active event; retention = 100%.
fn events_full_retention() -> Vec<ActivityEvent> {
    (0..10)
        .map(|i| mk_activity(&format!("install-{}", i), 3, true))
        .collect()
}

/// Build 10 installs: 4 active at week-3, 6 inactive → 40% retention.
fn events_40pct_retention() -> Vec<ActivityEvent> {
    let mut evs: Vec<ActivityEvent> = (0..4)
        .map(|i| mk_activity(&format!("install-{}", i), 3, true))
        .collect();
    evs.extend((4..10).map(|i| mk_activity(&format!("install-{}", i), 3, false)));
    evs
}

/// Build events with exactly 3 unprompted signals.
fn three_unprompted() -> Vec<FeedbackSignal> {
    vec![
        mk_signal("install-0", FeedbackKind::Unprompted),
        mk_signal("install-1", FeedbackKind::Unprompted),
        mk_signal("install-2", FeedbackKind::Unprompted),
    ]
}

// ─── item_1: week-3 retention computable vs ≥40% threshold ─────────────────

#[test]
fn item_1_week3_retention_computable_vs_40pct() {
    // RETENTION_THRESHOLD is 0.40
    assert_eq!(RETENTION_THRESHOLD, 0.40, "threshold must be 0.40");

    // 10/10 active → Pass
    let result = compute_retention(&events_full_retention());
    assert!(
        matches!(result, RetentionResult::Pass { .. }),
        "10/10 active at week-3 must Pass; got {:?}",
        result
    );

    // 4/10 active → Pass (exactly 40%)
    let result = compute_retention(&events_40pct_retention());
    assert!(
        matches!(result, RetentionResult::Pass { .. }),
        "4/10 active at week-3 (40%) must Pass; got {:?}",
        result
    );

    // 3/10 active → Fail (30%)
    let mut low: Vec<ActivityEvent> = (0..3)
        .map(|i| mk_activity(&format!("i-{}", i), 3, true))
        .collect();
    low.extend((3..10).map(|i| mk_activity(&format!("i-{}", i), 3, false)));
    let result = compute_retention(&low);
    assert!(
        matches!(result, RetentionResult::Fail { .. }),
        "3/10 active (30%) must Fail; got {:?}",
        result
    );

    // No week-3 events → Insufficient
    let no_w3: Vec<ActivityEvent> = (0..5)
        .map(|i| mk_activity(&format!("i-{}", i), 1, true))
        .collect();
    let result = compute_retention(&no_w3);
    assert_eq!(
        result,
        RetentionResult::Insufficient,
        "no week-3 events must be Insufficient"
    );
}

// ─── item_2: UNPROMPTED vs PROMPTED classifier ──────────────────────────────

#[test]
fn item_2_unprompted_vs_prompted_classifier() {
    let unprompted = mk_signal("a", FeedbackKind::Unprompted);
    let prompted = mk_signal("b", FeedbackKind::Prompted);

    // Only unprompted counts toward gate.
    assert!(
        unprompted.counts_toward_gate(),
        "Unprompted must count toward gate"
    );
    assert!(
        !prompted.counts_toward_gate(),
        "Prompted must NOT count toward gate"
    );

    // Mixed batch: 2 unprompted + 3 prompted → only 2 count.
    let signals = vec![
        mk_signal("a", FeedbackKind::Unprompted),
        mk_signal("b", FeedbackKind::Prompted),
        mk_signal("c", FeedbackKind::Unprompted),
        mk_signal("d", FeedbackKind::Prompted),
        mk_signal("e", FeedbackKind::Prompted),
    ];
    assert_eq!(
        count_unprompted(&signals),
        2,
        "2 unprompted in mixed batch of 5"
    );

    // All prompted → 0 count.
    let all_prompted: Vec<FeedbackSignal> = (0..5)
        .map(|i| mk_signal(&format!("{}", i), FeedbackKind::Prompted))
        .collect();
    assert_eq!(count_unprompted(&all_prompted), 0, "all prompted → 0");
}

// ─── item_3: exit report generated from data, auditable ─────────────────────

#[test]
fn item_3_exit_report_generated_auditable() {
    let cohort = valid_cohort();
    let events = events_full_retention();
    let signals = three_unprompted();

    let report = generate_report(&cohort, &events, &signals);

    // Report must be PASS with all data present.
    assert!(
        report.status.is_pass(),
        "full-passing data must yield Pass report; got {:?}",
        report.status
    );

    // Audit trail: all source fields present.
    assert_eq!(report.team_count, 10);
    assert_eq!(report.min_weeks_real_use, 3);
    assert_eq!(report.days_since_anchor, Some(30));
    assert_eq!(report.unprompted_count, 3);
    assert_eq!(report.total_signals, 3);
    assert!(
        report.retention_rate.is_some(),
        "retention_rate must be Some"
    );
    assert!(
        report.retention_rate.unwrap() >= RETENTION_THRESHOLD,
        "retention_rate must be ≥ 40%"
    );

    // Report must be serializable (auditable = can be persisted).
    let serialized = serde_json::to_string(&report).expect("report must serialize to JSON");
    assert!(
        !serialized.is_empty(),
        "serialized report must not be empty"
    );
    // Round-trip check.
    let _: hugit_app_exit::report::ExitReport =
        serde_json::from_str(&serialized).expect("report must round-trip from JSON");
}

// ─── item_4: cohort/window guards — insufficient/out-of-window, never pass ──

#[test]
fn item_4_cohort_window_guards_insufficient_not_pass() {
    // (a) Too few teams → InsufficientOrOutOfWindow, not pass.
    let too_few_teams = CohortState {
        team_count: 9,
        min_weeks_real_use: 3,
        days_since_anchor: Some(30),
    };
    let guard = evaluate_cohort_guards(&too_few_teams);
    assert!(
        !guard.is_satisfied(),
        "9 teams must not satisfy guard; got {:?}",
        guard
    );
    assert!(
        matches!(guard, CohortGuardResult::InsufficientOrOutOfWindow(_)),
        "9 teams must be InsufficientOrOutOfWindow"
    );

    // (b) Insufficient weeks → InsufficientOrOutOfWindow.
    let too_few_weeks = CohortState {
        team_count: 10,
        min_weeks_real_use: 2,
        days_since_anchor: Some(30),
    };
    let guard = evaluate_cohort_guards(&too_few_weeks);
    assert!(
        matches!(guard, CohortGuardResult::InsufficientOrOutOfWindow(_)),
        "2 weeks must be InsufficientOrOutOfWindow"
    );

    // (c) Anchor event not yet occurred → out-of-window.
    let no_anchor = CohortState {
        team_count: 10,
        min_weeks_real_use: 3,
        days_since_anchor: None,
    };
    let guard = evaluate_cohort_guards(&no_anchor);
    assert!(
        matches!(guard, CohortGuardResult::InsufficientOrOutOfWindow(_)),
        "no anchor must be InsufficientOrOutOfWindow"
    );

    // (d) Beyond 90 days → out-of-window.
    let beyond_window = CohortState {
        team_count: 10,
        min_weeks_real_use: 3,
        days_since_anchor: Some(91),
    };
    let guard = evaluate_cohort_guards(&beyond_window);
    assert!(
        matches!(guard, CohortGuardResult::InsufficientOrOutOfWindow(_)),
        "91 days must be InsufficientOrOutOfWindow"
    );

    // (e) All guards satisfied → Satisfied.
    let good = valid_cohort();
    let guard = evaluate_cohort_guards(&good);
    assert!(
        guard.is_satisfied(),
        "valid cohort must be Satisfied; got {:?}",
        guard
    );

    // (f) Report generated with invalid cohort → InsufficientOrOutOfWindow status, never Pass.
    let report = generate_report(
        &too_few_teams,
        &events_full_retention(),
        &three_unprompted(),
    );
    assert!(
        matches!(
            report.status,
            ExitReportStatus::InsufficientOrOutOfWindow(_)
        ),
        "report with insufficient cohort must be InsufficientOrOutOfWindow; got {:?}",
        report.status
    );
    assert!(
        !report.status.is_pass(),
        "must never be Pass with invalid cohort"
    );
}

// ─── item_5: ≥3 gate pass/fail — 2 → FAIL, 3 → PASS ────────────────────────

#[test]
fn item_5_gate_pass_fail_2_unprompted_fail_3_pass() {
    assert_eq!(UNPROMPTED_GATE, 3, "gate threshold must be 3");

    let cohort = valid_cohort();
    let good_events = events_full_retention();

    // (a) Exactly 2 unprompted → FAIL, even with ≥40% retention.
    let two_unprompted = vec![
        mk_signal("a", FeedbackKind::Unprompted),
        mk_signal("b", FeedbackKind::Unprompted),
        mk_signal("c", FeedbackKind::Prompted),
        mk_signal("d", FeedbackKind::Prompted),
    ];
    let report = generate_report(&cohort, &good_events, &two_unprompted);
    assert!(
        matches!(report.status, ExitReportStatus::Fail(_)),
        "2 unprompted + ≥40% retention must Fail; got {:?}",
        report.status
    );
    assert!(!report.status.is_pass(), "2 unprompted must not Pass");

    // (b) Exactly 3 unprompted + ≥40% retention → PASS.
    let report = generate_report(&cohort, &good_events, &three_unprompted());
    assert!(
        report.status.is_pass(),
        "3 unprompted + ≥40% retention must Pass; got {:?}",
        report.status
    );

    // (c) 3 unprompted but retention < 40% → FAIL.
    let low_retention: Vec<ActivityEvent> = {
        let mut v: Vec<ActivityEvent> = (0..3)
            .map(|i| mk_activity(&format!("i-{}", i), 3, true))
            .collect();
        v.extend((3..10).map(|i| mk_activity(&format!("i-{}", i), 3, false)));
        v
    };
    let report = generate_report(&cohort, &low_retention, &three_unprompted());
    assert!(
        matches!(report.status, ExitReportStatus::Fail(_)),
        "3 unprompted but <40% retention must Fail; got {:?}",
        report.status
    );

    // (d) 0 unprompted, all prompted → FAIL.
    let all_prompted: Vec<FeedbackSignal> = (0..5)
        .map(|i| mk_signal(&format!("{}", i), FeedbackKind::Prompted))
        .collect();
    let report = generate_report(&cohort, &good_events, &all_prompted);
    assert!(
        matches!(report.status, ExitReportStatus::Fail(_)),
        "0 unprompted must Fail; got {:?}",
        report.status
    );
}

// ─── item_6: THE MONEY GATE BINDS ────────────────────────────────────────────

#[test]
fn item_6_money_gate_blocks_billing_until_pass() {
    // (a) PASS report → gate allows billing + produces audited EventRecord.
    let pass_report = generate_report(
        &valid_cohort(),
        &events_full_retention(),
        &three_unprompted(),
    );
    assert!(
        pass_report.status.is_pass(),
        "precondition: pass_report must be Pass"
    );

    let gate = MoneyGate::new();
    let decision = gate.evaluate(&pass_report);
    assert!(
        decision.is_allow(),
        "PASS report must Allow billing; got {:?}",
        decision
    );

    let event_result =
        gate.try_enable_billing(&pass_report, "owner@example.com", &"0".repeat(64), 1);
    assert!(
        event_result.is_ok(),
        "PASS report must produce audited event; got {:?}",
        event_result
    );
    let event = event_result.unwrap();
    assert_eq!(
        event.kind, "hugit.billing.enable",
        "event kind must be hugit.billing.enable"
    );
    assert!(!event.this_hash.is_empty(), "event must have a hash");
    assert!(
        !event.principal_chain.is_empty(),
        "event must have principal chain"
    );

    // R1 ORACLE: the enable-billing this_hash MUST be byte-identical to the
    // canonical single-source formula (hugit_refstore::compute_this_hash) over
    // the (non-empty) principal_chain. RED on the old hand-rolled hash that
    // hashed the principal as one plain LP field with no VEC element-count
    // prefix; GREEN once routed to canonical.
    let expected_hash = hugit_refstore::compute_this_hash(
        &event.prev_hash,
        &event.kind,
        &event.principal_chain,
        &event.payload,
        event.seq,
    );
    assert_eq!(
        event.this_hash, expected_hash,
        "enable-billing this_hash must equal the canonical compute_this_hash"
    );
    // ...and a genesis-slot copy MUST verify against the canonical verifier.
    let genesis_event = gate
        .try_enable_billing(&pass_report, "owner@example.com", &"0".repeat(64), 0)
        .expect("genesis-seq enable-billing event must build");
    let mut log = hugit_refstore::EventLog::new();
    log.push_record(genesis_event)
        .expect("genesis-seq enable-billing record must push");
    hugit_refstore::verify_chain(log.records())
        .expect("enable-billing record must verify against the canonical chain");

    // (b) FAIL report → gate blocks billing.
    let two_unprompted = vec![
        mk_signal("a", FeedbackKind::Unprompted),
        mk_signal("b", FeedbackKind::Unprompted),
    ];
    let fail_report = generate_report(&valid_cohort(), &events_full_retention(), &two_unprompted);
    assert!(
        matches!(fail_report.status, ExitReportStatus::Fail(_)),
        "precondition: fail_report"
    );

    let decision = gate.evaluate(&fail_report);
    assert!(!decision.is_allow(), "FAIL report must Block billing");

    let err = gate.try_enable_billing(&fail_report, "owner@example.com", &"0".repeat(64), 2);
    assert!(err.is_err(), "FAIL report must not enable billing");

    // (c) InsufficientOrOutOfWindow report → gate blocks billing.
    let insuff_cohort = CohortState {
        team_count: 5,
        min_weeks_real_use: 3,
        days_since_anchor: Some(30),
    };
    let insuff_report = generate_report(
        &insuff_cohort,
        &events_full_retention(),
        &three_unprompted(),
    );
    assert!(
        matches!(
            insuff_report.status,
            ExitReportStatus::InsufficientOrOutOfWindow(_)
        ),
        "precondition: insuff_report"
    );

    let decision = gate.evaluate(&insuff_report);
    assert!(
        !decision.is_allow(),
        "insufficient report must Block billing"
    );

    let err = gate.try_enable_billing(&insuff_report, "owner@example.com", &"0".repeat(64), 3);
    assert!(err.is_err(), "insufficient report must not enable billing");

    // (d) DEGRADED gate evaluator → fails CLOSED (cannot enable) regardless of report.
    let degraded_gate = MoneyGate::with_state(GateEvaluatorState::Degraded);
    let decision = degraded_gate.evaluate(&pass_report);
    assert!(
        !decision.is_allow(),
        "DEGRADED gate must Block billing even on PASS report; got {:?}",
        decision
    );

    let err =
        degraded_gate.try_enable_billing(&pass_report, "owner@example.com", &"0".repeat(64), 4);
    assert!(
        err.is_err(),
        "DEGRADED gate must not enable billing even on PASS report"
    );

    // (e) The enable-billing event is audited: verify EventRecord fields.
    // Re-enable with healthy gate on pass report.
    let gate2 = MoneyGate::new();
    let event = gate2
        .try_enable_billing(&pass_report, "audit-principal", &"0".repeat(64), 10)
        .expect("healthy gate + pass report must succeed");
    assert_eq!(event.seq, 10);
    assert_eq!(event.principal_chain, vec!["audit-principal"]);
    assert!(
        event.payload.contains("enable_billing"),
        "payload must contain enable_billing"
    );
    assert!(event.payload.contains("PASS"), "payload must contain PASS");
    assert!(event.recorded_at > 0, "event must have a timestamp");
}

// ─── PR-8: forge-check — a hand-built `Pass` literal without a matching
// body_digest is fail-closed at the gate (Block, not Allow).
#[test]
fn pr8_forged_pass_literal_is_blocked_by_body_digest_check() {
    use hugit_app_exit::report::ExitReport;
    use hugit_app_exit::retention::RetentionMetrics;
    // A report built by hand, with status=Pass and a deliberately WRONG
    // body_digest. The gate's evaluate() must verify BEFORE honoring and
    // return Block(forge check failed). This is the regression for the
    // "field is private but a hand literal still passes" failure mode.
    let hand = ExitReport {
        status: ExitReportStatus::Pass,
        team_count: 5,
        min_weeks_real_use: 3,
        days_since_anchor: Some(30),
        cohort_guard: CohortGuardResult::Satisfied,
        retention_rate: Some(0.5),
        retention_result: RetentionResult::Pass {
            metrics: RetentionMetrics {
                total_installs_at_week3: 10,
                active_at_week3: 5,
                retention_rate: Some(0.5),
            },
        },
        unprompted_count: 3,
        total_signals: 3,
        body_digest: "0".repeat(64), // WRONG — a real report would have a real SHA-256
        attestation: None,
    };
    let gate = MoneyGate::new();
    let decision = gate.evaluate(&hand);
    assert!(!decision.is_allow(), "forged digest must Block, got {:?}", decision);
    let s = format!("{:?}", decision);
    assert!(s.contains("forge check failed"), "expected forge check, got: {s}");
}

// ─── PR-8: a generated Pass report carries a valid body_digest and verify() passes.
#[test]
fn pr8_generated_pass_report_passes_own_verify() {
    let pass_report = generate_report(
        &valid_cohort(),
        &events_full_retention(),
        &three_unprompted(),
    );
    assert!(!pass_report.body_digest.is_empty(), "body_digest must be set");
    assert!(
        pass_report.verify("").is_ok(),
        "generated report's body_digest must verify against empty corpus_seal; got {:?}",
        pass_report.verify("")
    );
}

// ─── PR-8 v12: corpus_seal actually affects the digest (regression for
// the v11 bug where verify and compute_body_digest had different hashes).
#[test]
fn pr8_corpus_seal_affects_digest() {
    use sha2::{Digest, Sha256};
    let pass_report = generate_report(
        &valid_cohort(),
        &events_full_retention(),
        &three_unprompted(),
    );
    // Manually hash with two different corpus_seals (verify and compute_body_digest
    // are pub(crate); tests must not bypass the tripwire).
    let hash_with_seal = |seal: &[u8]| -> String {
        let mut hasher = Sha256::new();
        hasher.update(serde_json::to_string(&pass_report.status).unwrap().as_bytes());
        hasher.update(pass_report.team_count.to_le_bytes());
        hasher.update(pass_report.min_weeks_real_use.to_le_bytes());
        hasher.update(
            pass_report.days_since_anchor.map(u64::to_le_bytes).unwrap_or([0u8; 8]).as_ref(),
        );
        hasher.update(pass_report.unprompted_count.to_le_bytes());
        hasher.update(pass_report.total_signals.to_le_bytes());
        hasher.update(seal);
        format!("{:x}", hasher.finalize())
    };
    let sealed_empty = hash_with_seal(b"");
    let sealed_deploy = hash_with_seal(b"deploy-seal-2026");
    assert_ne!(sealed_empty, sealed_deploy, "corpus_seal must change the digest");
    // generate_report uses empty seal → verify("") should pass
    assert!(pass_report.verify("").is_ok());
    // A different seal must fail (DigestMismatch)
    let err = pass_report.verify("deploy-seal-2026");
    assert!(
        matches!(err, Err(hugit_app_exit::report::VerifyError::DigestMismatch { .. })),
        "verify with wrong seal must DigestMismatch, got {:?}",
        err
    );
}

// ─── PR-8 v12: attestation = Some(_) returns AttestationInvalid (regression
// for the v11 gap where the AttestationInvalid branch was untested).
#[test]
fn pr8_attestation_some_is_rejected() {
    use hugit_app_exit::report::ExitReport;
    use hugit_app_exit::retention::RetentionMetrics;
    use hugit_app_exit::cohort::CohortGuardResult;
    use hugit_app_exit::report::{ExitReportStatus, VerifyError};
    use sha2::{Digest, Sha256};
    // Build a report with a body_digest that matches verify's expectation
    // (corpus_seal = "") so the digest check passes and the attestation check
    // is the one that fires.
    let body_digest = {
        let r = ExitReport {
            status: ExitReportStatus::Pass,
            team_count: 5,
            min_weeks_real_use: 3,
            days_since_anchor: Some(30),
            cohort_guard: CohortGuardResult::Satisfied,
            retention_rate: Some(0.5),
            retention_result: RetentionResult::Pass {
                metrics: RetentionMetrics { total_installs_at_week3: 10, active_at_week3: 5, retention_rate: Some(0.5) },
            },
            unprompted_count: 3,
            total_signals: 3,
            body_digest: String::new(),
            attestation: None,
        };
        let mut hasher = Sha256::new();
        hasher.update(serde_json::to_string(&r.status).unwrap().as_bytes());
        hasher.update(r.team_count.to_le_bytes());
        hasher.update(r.min_weeks_real_use.to_le_bytes());
        hasher.update(r.days_since_anchor.map(u64::to_le_bytes).unwrap_or([0u8; 8]).as_ref());
        hasher.update(r.unprompted_count.to_le_bytes());
        hasher.update(r.total_signals.to_le_bytes());
        hasher.update(b"");
        format!("{:x}", hasher.finalize())
    };
    let report = ExitReport {
        status: ExitReportStatus::Pass,
        team_count: 5,
        min_weeks_real_use: 3,
        days_since_anchor: Some(30),
        cohort_guard: CohortGuardResult::Satisfied,
        retention_rate: Some(0.5),
        retention_result: RetentionResult::Pass {
            metrics: RetentionMetrics { total_installs_at_week3: 10, active_at_week3: 5, retention_rate: Some(0.5) },
        },
        unprompted_count: 3,
        total_signals: 3,
        body_digest: body_digest.clone(),
        attestation: Some("not-a-real-sig".to_string()),
    };
    assert_eq!(report.verify(""), Err(VerifyError::AttestationInvalid));
}

// ─── PR-8 v12: EmptyCohort (team_count=0) returns EmptyCohort (regression
// for the v11 gap where this branch was untested).
#[test]
fn pr8_empty_cohort_is_rejected() {
    use hugit_app_exit::report::ExitReport;
    use hugit_app_exit::retention::RetentionMetrics;
    use hugit_app_exit::cohort::CohortGuardResult;
    use hugit_app_exit::report::{ExitReportStatus, VerifyError};
    let report = ExitReport {
        status: ExitReportStatus::Pass,
        team_count: 0, // ← empty cohort
        min_weeks_real_use: 3,
        days_since_anchor: Some(30),
        cohort_guard: CohortGuardResult::Satisfied,
        retention_rate: Some(0.5),
        retention_result: RetentionResult::Pass {
            metrics: RetentionMetrics { total_installs_at_week3: 10, active_at_week3: 5, retention_rate: Some(0.5) },
        },
        unprompted_count: 3,
        total_signals: 3,
        body_digest: "f".repeat(64), // body_digest value doesn't matter — EmptyCohort is checked first
        attestation: None,
    };
    assert_eq!(report.verify(""), Err(VerifyError::EmptyCohort));
}
