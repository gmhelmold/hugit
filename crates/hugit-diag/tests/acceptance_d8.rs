//! WP-D8 acceptance tests — experiment harness + gate binding.
//!
//! Acceptance test for the experiment-harness contract (WP-D8).
//! One `#[test] item_<n>_<slug>` per owned item ①–⑨.
//! Gate-binding items ⑥⑧⑨ are state-machine + attestation fixture proofs (local).

use hugit_contracts::{AttestationChain, RegenGate};
use hugit_diag::experiment::{
    Corpus, Dashboard, EvaluatorHealth, EventRecord, ExperimentGate, GateError, GateReport,
    IngestError, Promotion, PromotionAttempt, RegenOutcome, ReportError, ReportVerdict,
    SourceOrigin, WaveContribution, ingest,
};
use hugit_refstore::compute_this_hash;

// ── fixtures ────────────────────────────────────────────────────────────────

/// A genuine X2-class attestation chain for a report.
fn attestation() -> AttestationChain {
    AttestationChain {
        tree: "tree-cas-ref".into(),
        def: "checkdef-cas-ref".into(),
        runner: "runner-cas-ref".into(),
        model: "claude-opus-4-8".into(),
        principal: vec!["experiment-evaluator".into()],
        sig: "base64-genuine-signature".into(),
    }
}

/// Ingest `n` PASS-shaped (disjoint + regen-agree) hugit-eligible datapoints
/// into a fresh corpus through the real ingestion path (①). Returns the corpus
/// buffer and the audit log.
fn passing_corpus(n: usize) -> (Corpus, Vec<EventRecord>) {
    let mut buffer = Vec::new();
    let mut log: Vec<EventRecord> = Vec::new();
    for i in 0..n {
        let c = WaveContribution::from_wave(
            format!("wave-{i}"),
            SourceOrigin::Hugit,
            true,
            RegenOutcome::Agree,
            false,
        );
        ingest(&mut buffer, &mut log, c, 0).expect("eligible hugit wave ingests");
    }
    (Corpus::from_buffer(buffer), log)
}

// ── ① every wave auto-contributes datapoints ────────────────────────────────

#[test]
fn item_1_wave_auto_contributes() {
    let mut buffer = Vec::new();
    let mut log = Vec::new();

    // Two real waves each produce a contribution derived from their outcome.
    let w1 = WaveContribution::from_wave(
        "wave-a",
        SourceOrigin::Hugit,
        true,
        RegenOutcome::Agree,
        false,
    );
    let w2 = WaveContribution::from_wave(
        "wave-b",
        SourceOrigin::Hugit,
        false,
        RegenOutcome::Disagree,
        false,
    );

    ingest(&mut buffer, &mut log, w1, 0).unwrap();
    ingest(&mut buffer, &mut log, w2, 0).unwrap();

    // Both waves auto-contributed; the buffer reflects the real outcomes.
    assert_eq!(buffer.len(), 2, "every wave auto-contributes a datapoint");
    assert_eq!(buffer[0].wave_id, "wave-a");
    assert_eq!(buffer[1].wave_id, "wave-b");
    assert!(buffer[0].claims_disjoint);
    assert!(!buffer[1].claims_disjoint);
}

// ── ② dashboard: disjointness %, regen agree/disagree, n ────────────────────

#[test]
fn item_2_dashboard_fields() {
    let mut buffer = Vec::new();
    let mut log = Vec::new();
    // 3 disjoint+agree, 1 non-disjoint+disagree  → 75% disjoint, 3/1 regen, n=4.
    for (id, disj, regen) in [
        ("w0", true, RegenOutcome::Agree),
        ("w1", true, RegenOutcome::Agree),
        ("w2", true, RegenOutcome::Agree),
        ("w3", false, RegenOutcome::Disagree),
    ] {
        ingest(
            &mut buffer,
            &mut log,
            WaveContribution::from_wave(id, SourceOrigin::Hugit, disj, regen, false),
            0,
        )
        .unwrap();
    }
    let sealed = Corpus::from_buffer(buffer).seal();
    let dash = Dashboard::from_sealed(&sealed);

    assert_eq!(dash.n, 4, "n reported");
    assert_eq!(dash.disjoint_count, 3);
    assert!(
        (dash.disjointness_pct() - 75.0).abs() < 1e-9,
        "disjointness %"
    );
    assert_eq!(dash.regen_agree, 3, "regen agree count");
    assert_eq!(dash.regen_disagree, 1, "regen disagree count");
}

// ── ③ gate report generated, never hand-written ─────────────────────────────

#[test]
fn item_3_report_generated_not_handwritten() {
    let (corpus, _log) = passing_corpus(30);
    let sealed = corpus.seal();
    let report = GateReport::generate(&sealed, attestation());

    // The verdict is DERIVED from the data, not asserted by a constructor:
    // a 30-strong all-PASS sample yields PASS.
    assert_eq!(report.verdict(), ReportVerdict::Pass);
    // The report self-verifies against the corpus it was generated from —
    // i.e. it carries a recomputable body digest, not a hand-set value.
    assert!(report.verify(&sealed).is_ok(), "generated report verifies");
    // There is no public constructor that takes a verdict directly — the only
    // path is `generate`. (Compile-time guarantee: `GateReport` fields are
    // private; this test exercises the single generation path.)
}

// ── ④ corpus pre-registered + SEALED before evaluation; auditable ───────────

#[test]
fn item_4_corpus_presealed_auditable() {
    let (corpus, _log) = passing_corpus(30);
    let sealed = corpus.seal();

    // The seal pins an auditable sample-selection digest over the exact set.
    let digest = sealed.seal_digest().to_string();
    assert!(!digest.is_empty(), "seal digest recorded");
    assert_eq!(sealed.n(), 30, "sealed sample size auditable");

    // The seal verifies against its own pinned, untouched sample.
    assert!(sealed.verify().is_ok(), "untouched sealed corpus verifies");
}

// ── ⑤ post-hoc removal detected → invalidates the verdict ───────────────────

#[test]
fn item_5_posthoc_removal_invalidates() {
    let (corpus, _log) = passing_corpus(30);
    let sealed = corpus.seal();
    let report = GateReport::generate(&sealed, attestation());
    assert_eq!(report.verdict(), ReportVerdict::Pass);

    // An attacker silently removes one sealed member after evaluation.
    let tampered = sealed.with_member_removed(0);

    // Detected at the corpus seal …
    assert_eq!(
        tampered.verify(),
        Err(hugit_diag::experiment::CorpusError::SealBroken),
        "post-hoc removal detected"
    );
    // … and it invalidates the verdict at the report-verify layer.
    assert_eq!(
        report.verify(&tampered),
        Err(ReportError::CorpusTampered),
        "post-hoc removal invalidates the verdict"
    );

    // And the binding gate refuses to promote on the tampered corpus.
    let mut gate = ExperimentGate::new();
    let mut log = Vec::new();
    let res = gate.attempt_promotion(
        &mut log,
        PromotionAttempt {
            promotion: Promotion::RegenPromotion,
            report: &report,
            corpus: &tampered,
            evaluator: EvaluatorHealth::Healthy,
            principal: "operator",
        },
        0,
    );
    assert_eq!(
        res,
        Err(GateError::CorpusTampered),
        "tampered corpus cannot promote"
    );
    assert!(gate.is_regen_blocked(), "regen stays blocked after tamper");
}

// ── ⑥ THE GATE BINDS: fail-closed control ───────────────────────────────────

#[test]
fn item_6_gate_binds_fail_closed() {
    // Defaults are structurally pinned.
    let mut gate = ExperimentGate::new();
    assert!(
        gate.is_claims_oracle_pinned(),
        "claims-as-oracle starts advisory/OFF"
    );
    assert!(gate.is_regen_blocked(), "regen promotion starts BLOCKED");

    // (a) A genuine PASS report flips the targeted feature — and ONLY it.
    let (corpus, _l) = passing_corpus(30);
    let sealed = corpus.seal();
    let pass = GateReport::generate(&sealed, attestation());
    assert_eq!(pass.verdict(), ReportVerdict::Pass);

    let mut log: Vec<EventRecord> = Vec::new();
    let v = gate
        .attempt_promotion(
            &mut log,
            PromotionAttempt {
                promotion: Promotion::RegenPromotion,
                report: &pass,
                corpus: &sealed,
                evaluator: EvaluatorHealth::Healthy,
                principal: "operator",
            },
            0,
        )
        .expect("genuine PASS authorizes regen promotion");
    assert!(!gate.is_regen_blocked(), "regen promoted on genuine PASS");
    assert!(
        gate.is_claims_oracle_pinned(),
        "claims untouched by regen promotion"
    );
    assert_eq!(
        v.event.kind, "experiment.promotion.authorized",
        "promotion audited"
    );
    assert_eq!(log.len(), 1, "exactly one audited promotion event");

    // (b) A FAIL report CANNOT flip a feature.
    let mut fbuf = Vec::new();
    let mut flog = Vec::new();
    for i in 0..30 {
        // one disagree → not all-pass → FAIL
        let regen = if i == 0 {
            RegenOutcome::Disagree
        } else {
            RegenOutcome::Agree
        };
        ingest(
            &mut fbuf,
            &mut flog,
            WaveContribution::from_wave(format!("f{i}"), SourceOrigin::Hugit, true, regen, false),
            0,
        )
        .unwrap();
    }
    let fsealed = Corpus::from_buffer(fbuf).seal();
    let fail = GateReport::generate(&fsealed, attestation());
    assert_eq!(fail.verdict(), ReportVerdict::Fail);
    let mut gate_b = ExperimentGate::new();
    let mut blog = Vec::new();
    let res = gate_b.attempt_promotion(
        &mut blog,
        PromotionAttempt {
            promotion: Promotion::ClaimsOracle,
            report: &fail,
            corpus: &fsealed,
            evaluator: EvaluatorHealth::Healthy,
            principal: "operator",
        },
        0,
    );
    assert_eq!(res, Err(GateError::NotPass(ReportVerdict::Fail)));
    assert!(
        gate_b.is_claims_oracle_pinned(),
        "FAIL cannot flip claims-oracle"
    );
    assert_eq!(blog.last().unwrap().kind, "experiment.promotion.refused");

    // (c) An insufficient-n report CANNOT flip a feature.
    let (small, _s) = passing_corpus(5);
    let ssealed = small.seal();
    let insuff = GateReport::generate(&ssealed, attestation());
    assert_eq!(insuff.verdict(), ReportVerdict::Insufficient);
    let mut gate_c = ExperimentGate::new();
    let mut clog = Vec::new();
    let res = gate_c.attempt_promotion(
        &mut clog,
        PromotionAttempt {
            promotion: Promotion::RegenPromotion,
            report: &insuff,
            corpus: &ssealed,
            evaluator: EvaluatorHealth::Healthy,
            principal: "operator",
        },
        0,
    );
    assert_eq!(res, Err(GateError::NotPass(ReportVerdict::Insufficient)));
    assert!(gate_c.is_regen_blocked(), "insufficient-n cannot promote");

    // (d) A DEGRADED evaluator = insufficient → fails CLOSED even on a PASS report.
    let mut gate_d = ExperimentGate::new();
    let mut dlog = Vec::new();
    let res = gate_d.attempt_promotion(
        &mut dlog,
        PromotionAttempt {
            promotion: Promotion::RegenPromotion,
            report: &pass,
            corpus: &sealed,
            evaluator: EvaluatorHealth::Degraded,
            principal: "operator",
        },
        0,
    );
    assert_eq!(res, Err(GateError::EvaluatorDegraded));
    assert!(
        gate_d.is_regen_blocked(),
        "degraded evaluator cannot promote"
    );
    assert_eq!(
        dlog.last().unwrap().kind,
        "experiment.promotion.refused",
        "refusal audited"
    );

    // (e) The consumed RegenGate.repass only flips through a genuine promotion.
    let base = RegenGate {
        optin_scope: "*".into(),
        repass: false,
        indep_verdict: "v".into(),
    };
    let blocked_gate = ExperimentGate::new();
    assert!(
        !blocked_gate.apply_regen_promotion(base.clone()).repass,
        "blocked → repass stays false"
    );
    assert!(
        gate.apply_regen_promotion(base).repass,
        "promoted gate → repass true"
    );
}

// ── ⑦ degradation honesty ────────────────────────────────────────────────────

#[test]
fn item_7_degradation_honesty() {
    let mut buffer = Vec::new();
    let mut log = Vec::new();
    // 30 healthy + 2 degraded-window waves, all eligible & ingested.
    for i in 0..30 {
        ingest(
            &mut buffer,
            &mut log,
            WaveContribution::from_wave(
                format!("h{i}"),
                SourceOrigin::Hugit,
                true,
                RegenOutcome::Agree,
                false,
            ),
            0,
        )
        .unwrap();
    }
    for i in 0..2 {
        ingest(
            &mut buffer,
            &mut log,
            // degraded waves are marked, and would have skewed the result if counted
            WaveContribution::from_wave(
                format!("d{i}"),
                SourceOrigin::Hugit,
                false,
                RegenOutcome::Disagree,
                true,
            ),
            0,
        )
        .unwrap();
    }
    let sealed = Corpus::from_buffer(buffer).seal();

    // Degraded waves are EXCLUDED from the evaluated sample and recorded
    // explicitly — never silently biasing the data.
    assert_eq!(
        sealed.n(),
        30,
        "degraded waves excluded from evaluated sample"
    );
    assert_eq!(
        sealed.excluded().len(),
        2,
        "excluded degraded waves recorded explicitly"
    );
    assert!(
        sealed.excluded().iter().all(|d| d.degraded),
        "only degraded waves excluded"
    );

    // With degraded data correctly excluded, the verdict is the honest PASS,
    // not a biased FAIL.
    let report = GateReport::generate(&sealed, attestation());
    assert_eq!(report.verdict(), ReportVerdict::Pass);
}

// ── ⑧ source-eligibility wired to focus gate: ineligible rejected at ingestion ─

#[test]
fn item_8_ineligible_source_rejected() {
    let mut buffer = Vec::new();
    let mut log = Vec::new();

    // A corelink-server change attempts to enter the corpus.
    let attempt = WaveContribution::from_wave(
        "wave-corelink",
        SourceOrigin::CorelinkServer,
        true,
        RegenOutcome::Agree,
        false,
    );
    let res = ingest(&mut buffer, &mut log, attempt, 0);

    // Rejected at INGESTION, fail-closed + audited; never becomes a datapoint.
    assert_eq!(
        res,
        Err(IngestError::IneligibleSource("corelink-server".into())),
        "focus-gate-ineligible source rejected at ingestion"
    );
    assert!(
        buffer.is_empty(),
        "ineligible change never enters the corpus"
    );
    assert_eq!(log.len(), 1, "ingestion rejection audited");
    assert_eq!(log[0].kind, "experiment.ingestion.rejected");

    // Eligibility predicate IS the X10② exclusion: only hugit is eligible.
    assert!(SourceOrigin::Hugit.is_focus_eligible());
    assert!(!SourceOrigin::CorelinkServer.is_focus_eligible());
    assert!(!SourceOrigin::OtherExcluded("anything".into()).is_focus_eligible());
}

// ── ⑨ gate report is attested, tamper-evident; forged PASS cannot promote ────

#[test]
fn item_9_report_attested_tamper_evident() {
    let (corpus, _l) = passing_corpus(5); // small → genuine verdict = Insufficient
    let sealed = corpus.seal();
    let genuine = GateReport::generate(&sealed, attestation());
    assert_eq!(genuine.verdict(), ReportVerdict::Insufficient);
    // The genuine report carries the X2-class attestation chain.
    assert_eq!(genuine.attestation().model, "claude-opus-4-8");

    // An attacker forges a PASS verdict onto the report without re-evaluating.
    let forged = genuine.forged_pass();
    assert_eq!(forged.verdict(), ReportVerdict::Pass, "forgery claims PASS");

    // The forgery is detected by report verification …
    assert_eq!(
        forged.verify(&sealed),
        Err(ReportError::Forged),
        "forged report rejected"
    );

    // … and the binding surface refuses to promote on it: a forged PASS cannot
    // enable promotion through any path other than a genuine evaluation.
    let mut gate = ExperimentGate::new();
    let mut log = Vec::new();
    let res = gate.attempt_promotion(
        &mut log,
        PromotionAttempt {
            promotion: Promotion::RegenPromotion,
            report: &forged,
            corpus: &sealed,
            evaluator: EvaluatorHealth::Healthy,
            principal: "attacker",
        },
        0,
    );
    assert_eq!(
        res,
        Err(GateError::ForgedReport),
        "forged PASS rejected at gate"
    );
    assert!(gate.is_regen_blocked(), "forged PASS cannot promote");
    assert_eq!(
        log.last().unwrap().kind,
        "experiment.promotion.refused",
        "refusal audited"
    );
}

// ── REMEDIATION ORACLE TESTS (RED → GREEN) ───────────────────────────────────

/// (a) Audit hash must match canonical hugit_refstore formula exactly.
///
/// RED on the bespoke hasher (which omits the VEC count prefix for
/// principal_chain).  GREEN once audit.rs routes through compute_this_hash.
#[test]
fn rdiag_a_audit_hash_matches_canonical_formula() {
    // Emit an ingestion-rejected event so log has one entry.
    let mut buffer = Vec::new();
    let mut log: Vec<EventRecord> = Vec::new();
    let rejected = WaveContribution::from_wave(
        "wave-ineligible",
        SourceOrigin::CorelinkServer,
        true,
        RegenOutcome::Agree,
        false,
    );
    ingest(&mut buffer, &mut log, rejected, 0).unwrap_err();

    // Also emit a refused-promotion event to get two chain links.
    let (small_corpus, _) = {
        let mut buf2 = Vec::new();
        let mut lg2: Vec<EventRecord> = Vec::new();
        let c = WaveContribution::from_wave(
            "wave-small",
            SourceOrigin::Hugit,
            true,
            RegenOutcome::Agree,
            false,
        );
        ingest(&mut buf2, &mut lg2, c, 0).unwrap();
        (Corpus::from_buffer(buf2), lg2)
    };
    let sealed_small = small_corpus.seal();
    let insuff = GateReport::generate(&sealed_small, attestation());
    let mut gate = ExperimentGate::new();
    gate.attempt_promotion(
        &mut log,
        PromotionAttempt {
            promotion: Promotion::RegenPromotion,
            report: &insuff,
            corpus: &sealed_small,
            evaluator: EvaluatorHealth::Healthy,
            principal: "test-principal",
        },
        0,
    )
    .unwrap_err();

    assert_eq!(log.len(), 2, "exactly two events in log");

    // ── Hard-coded expected values for both events (from known test inputs) ──
    //
    // Record 0: ingest rejection for "wave-ineligible" (CorelinkServer)
    //   principal = "experiment-harness" (hardcoded in emit_event for ingest)
    //   kind      = "experiment.ingestion.rejected"
    //   seq       = 0
    //   prev_hash = genesis (no predecessor)
    //
    // Record 1: promotion refused for "test-principal" (insufficient-n)
    //   principal = "test-principal"
    //   kind      = "experiment.promotion.refused"
    //   seq       = 1
    //   prev_hash = record[0].this_hash

    let genesis = "0".repeat(64);

    // Record 0 — ingest rejection.
    let r0_principal_chain = vec!["experiment-harness".to_string()];
    let r0_kind = "experiment.ingestion.rejected";
    assert_eq!(
        log[0].principal_chain, r0_principal_chain,
        "record 0: principal_chain must be [experiment-harness]"
    );
    assert_eq!(log[0].kind, r0_kind, "record 0: kind mismatch");
    assert_eq!(log[0].seq, 0, "record 0: seq must be 0");
    assert_eq!(
        log[0].prev_hash, genesis,
        "record 0: prev_hash must be genesis"
    );
    let r0_expected_hash =
        compute_this_hash(&genesis, r0_kind, &r0_principal_chain, &log[0].payload, 0);
    assert_eq!(
        log[0].this_hash, r0_expected_hash,
        "record 0: this_hash diverges from canonical formula"
    );

    // Record 1 — promotion refused.
    let r1_principal_chain = vec!["test-principal".to_string()];
    let r1_kind = "experiment.promotion.refused";
    assert_eq!(
        log[1].principal_chain, r1_principal_chain,
        "record 1: principal_chain must be [test-principal]"
    );
    assert_eq!(log[1].kind, r1_kind, "record 1: kind mismatch");
    assert_eq!(log[1].seq, 1, "record 1: seq must be 1");
    assert_eq!(
        log[1].prev_hash, log[0].this_hash,
        "record 1: prev_hash must chain to record 0"
    );
    let r1_expected_hash = compute_this_hash(
        &log[0].this_hash,
        r1_kind,
        &r1_principal_chain,
        &log[1].payload,
        1,
    );
    assert_eq!(
        log[1].this_hash, r1_expected_hash,
        "record 1: this_hash diverges from canonical formula"
    );
}

/// (rdiag_c) recorded_at must be the caller-supplied value, not the hardcoded 0.
///
/// RED on the current always-0 call sites. GREEN once recorded_at (now_ms) is
/// plumbed from the caller (attempt_promotion / ingest) down to emit_event.
#[test]
fn rdiag_c_recorded_at_is_caller_supplied() {
    const NOW: u64 = 1_700_000_000_000; // non-zero fixed epoch-ms

    // ── gate path: authorized promotion ─────────────────────────────────────
    let (corpus, _) = passing_corpus(30);
    let sealed = corpus.seal();
    let pass_report = GateReport::generate(&sealed, attestation());

    let mut gate = ExperimentGate::new();
    let mut log: Vec<EventRecord> = Vec::new();

    let verdict = gate
        .attempt_promotion(
            &mut log,
            PromotionAttempt {
                promotion: Promotion::RegenPromotion,
                report: &pass_report,
                corpus: &sealed,
                evaluator: EvaluatorHealth::Healthy,
                principal: "operator",
            },
            NOW,
        )
        .expect("genuine PASS authorizes");
    assert_eq!(
        verdict.event.recorded_at, NOW,
        "authorized event recorded_at must equal caller-supplied NOW (not 0)"
    );
    assert_ne!(verdict.event.recorded_at, 0, "recorded_at must not be 0");

    // ── gate path: refused promotion ─────────────────────────────────────────
    let (small_c, _) = passing_corpus(5);
    let small_sealed = small_c.seal();
    let insuff = GateReport::generate(&small_sealed, attestation());
    let mut gate2 = ExperimentGate::new();
    let mut log2: Vec<EventRecord> = Vec::new();
    gate2
        .attempt_promotion(
            &mut log2,
            PromotionAttempt {
                promotion: Promotion::RegenPromotion,
                report: &insuff,
                corpus: &small_sealed,
                evaluator: EvaluatorHealth::Healthy,
                principal: "operator",
            },
            NOW,
        )
        .unwrap_err();
    assert_eq!(
        log2[0].recorded_at, NOW,
        "refused event recorded_at must equal caller-supplied NOW"
    );
    assert_ne!(log2[0].recorded_at, 0, "refused recorded_at must not be 0");

    // ── ingest path: rejection audit ─────────────────────────────────────────
    let mut buf = Vec::new();
    let mut ilog: Vec<EventRecord> = Vec::new();
    let ineligible = WaveContribution::from_wave(
        "wave-bad",
        SourceOrigin::CorelinkServer,
        true,
        RegenOutcome::Agree,
        false,
    );
    ingest(&mut buf, &mut ilog, ineligible, NOW).unwrap_err();
    assert_eq!(
        ilog[0].recorded_at, NOW,
        "ingestion-rejected event recorded_at must equal caller-supplied NOW"
    );
    assert_ne!(
        ilog[0].recorded_at, 0,
        "ingestion recorded_at must not be 0"
    );
}

/// (b) A corpus-swap (different corpus presented to verify) must return
/// CorpusTampered, not Forged.
///
/// RED on the current report.rs line 140 which returns Forged for this case.
/// GREEN once report.rs returns CorpusTampered for corpus_seal mismatch.
#[test]
fn rdiag_b_corpus_swap_yields_corpus_tampered() {
    // Report generated from corpus A (30-point PASS corpus).
    let (corpus_a, _) = passing_corpus(30);
    let sealed_a = corpus_a.seal();
    let report = GateReport::generate(&sealed_a, attestation());
    assert_eq!(report.verdict(), ReportVerdict::Pass);

    // Corpus B: a different sealed corpus (different seal digest).
    let (corpus_b, _) = passing_corpus(31);
    let sealed_b = corpus_b.seal();

    // The report's body_digest is intact (no forgery), but the report was
    // generated from corpus A and the presented corpus is B → seal mismatch.
    // Contract: must return CorpusTampered (not Forged).
    assert_eq!(
        report.verify(&sealed_b),
        Err(ReportError::CorpusTampered),
        "corpus swap must return CorpusTampered, not Forged"
    );
}
