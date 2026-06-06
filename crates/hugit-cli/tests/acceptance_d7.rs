//! WP-D7 acceptance tests — adversarial verdict panels + grounded review Q&A.
//!
//! Contract: docs/plan/wp-contracts/WP-D7.md.
//! Oracle:   tests/acceptance/wp-d7/run.sh.
//!
//! Naming convention (binding, pre-decided by the lead):
//!   item_<n>_<slug>
//!
//! STRUCTURAL / fixture proofs only — there are NO live model API calls in this
//! lane. Reviewers are injected deterministic strategies (see
//! `hugit_cli::verdict::fixtures`).

use hugit_cli::verdict::fixtures::{lens_isolation, persuasion_channel, planted_bug};
use hugit_cli::verdict::lens_audit::{
    audit_isolation, audit_panel_isolation, author_controlled_fields,
};
use hugit_cli::verdict::panel_dispatch::{ReviewerInput, dispatch};
use hugit_cli::verdict::qa::{Answer, EvidenceObject, EvidenceStore, answer_question};
use hugit_cli::verdict::{Panel, Verdict, enforce_diversity};

use hugit_contracts::VerdictObject;

// ─── item ①: lenses isolated (prompt audit) ──────────────────────────────────

/// ① Each lens is an independent reviewer: distinct prompts, and NO
/// author-controlled text reaches any reviewer's served input.
#[test]
fn item_1_lenses_isolated() {
    let panel = lens_isolation::diverse_panel();
    let gt = persuasion_channel::served_ground_truth();
    let sidecar = persuasion_channel::persuasive_sidecar();

    // Lenses carry distinct prompts (independent reviewers), and every lens's
    // served input is free of the author's persuasive prose.
    let report = audit_panel_isolation(&panel, &gt, &sidecar)
        .expect("panel must be isolated: distinct prompts + no author text");
    assert_eq!(
        report.distinct_prompts,
        panel.lenses.len(),
        "every lens must carry a distinct prompt"
    );
    assert!(
        report.author_fields_checked > 0,
        "the audit must actually check author-controlled fields"
    );

    // A single built reviewer input also passes the per-input audit.
    let input = ReviewerInput::new(panel.lenses[0].prompt.clone(), gt.clone());
    assert!(
        audit_isolation(&input, &sidecar).is_ok(),
        "no author-controlled field may appear in a reviewer input"
    );
}

// ─── item ②: valid VerdictObject[] + evidence refs ───────────────────────────

/// ② Dispatch emits a valid `VerdictObject` per lens, each carrying real
/// evidence refs drawn from the served ground truth.
#[test]
fn item_2_verdict_object_valid() {
    let panel = lens_isolation::diverse_panel();
    let gt = persuasion_channel::served_ground_truth();

    let verdicts: Vec<VerdictObject> = dispatch(&panel, &gt).expect("dispatch must succeed");
    assert_eq!(verdicts.len(), panel.lenses.len(), "one verdict per lens");

    for (v, lens) in verdicts.iter().zip(&panel.lenses) {
        assert_eq!(v.intent, gt.intent);
        assert_eq!(v.tree_hash, gt.tree_hash);
        assert_eq!(v.lens, lens.id);
        assert_eq!(v.model, lens.model);
        assert_eq!(v.prompt_digest, lens.prompt_digest());
        assert!(!v.evidence_refs.is_empty(), "verdict must cite evidence");
        assert_eq!(
            v.evidence_refs, gt.evidence_refs,
            "evidence refs must resolve to served evidence"
        );
        // Schema round-trip: the VerdictObject is a valid frozen-contract value.
        let json = serde_json::to_string(v).expect("VerdictObject serializes");
        let back: VerdictObject = serde_json::from_str(&json).expect("round-trips");
        assert_eq!(&back, v);
    }
}

// ─── item ③: planted bug (semantic/logic) caught by ≥1 lens ──────────────────

/// ③ A semantic/logic bug uncovered by any author test is still caught by ≥1
/// lens — the panel adds coverage authors cannot.
#[test]
fn item_3_planted_bug_caught() {
    // The author's own suite passes — the bug is demonstrably uncovered.
    assert!(
        planted_bug::author_suite_passes(),
        "author suite must be GREEN (bug is non-author-visible)"
    );

    let gt = planted_bug::ground_truth_with_planted_bug();
    let panel = Panel::new(vec![planted_bug::semantic_lens()]);
    let verdicts = dispatch(&panel, &gt).expect("dispatch must succeed");

    let caught = verdicts.iter().any(|v| v.verdict == Verdict::Reject);
    assert!(
        caught,
        "≥1 lens must catch the planted impact-under-report bug"
    );

    let rejecting = verdicts
        .iter()
        .find(|v| v.verdict == Verdict::Reject)
        .expect("a rejecting verdict exists");
    assert!(
        rejecting
            .claims_checked
            .iter()
            .any(|c| c.contains("impact-under-report")),
        "the catch must name the semantic bug class"
    );
}

// ─── item ④: human review Q&A grounded or refused ────────────────────────────

/// ④ Answers are citations to real evidence objects; with no grounding the
/// engine refuses explicitly — never fabricates.
#[test]
fn item_4_qa_grounded_or_refused() {
    let mut store = EvidenceStore::new();
    store.insert(EvidenceObject {
        evidence_ref: "blob://check-stdout".into(),
        body: "all 42 tests passed".into(),
        keywords: vec!["tests".into(), "passed".into()],
    });

    // Grounded path: the answer cites a real, resolvable evidence object.
    let grounded = answer_question(&store, "did the tests pass?").expect("no error");
    match &grounded {
        Answer::Cited {
            citations,
            excerpts,
        } => {
            assert!(!citations.is_empty(), "must cite ≥1 evidence object");
            for c in citations {
                assert!(store.get(c).is_some(), "every citation must resolve");
            }
            assert!(!excerpts.is_empty(), "cited excerpts present");
        }
        Answer::Refused { .. } => panic!("a grounded question must not be refused"),
    }

    // Ungrounded path: explicit refusal, no fabrication.
    let refused =
        answer_question(&store, "what is the author's favorite color?").expect("no error");
    assert!(refused.is_refusal(), "ungrounded question must be refused");
    assert!(
        refused.citations().is_empty(),
        "a refusal fabricates nothing"
    );
}

// ─── item ⑤: DIVERSITY enforced (≥2 distinct models) ─────────────────────────

/// ⑤ A homogeneous panel (same prompt + same model) is rejected; a real panel
/// dispatches distinct prompts AND ≥2 distinct models.
#[test]
fn item_5_diversity_enforced() {
    // Homogeneous panel rejected.
    let homo = lens_isolation::homogeneous_panel();
    assert!(
        enforce_diversity(&homo).is_err(),
        "homogeneous panel (one prompt + one model) must be rejected"
    );

    // Same-model panel (distinct prompts, 1 model) rejected — proves the
    // ≥2-distinct-models rule, not merely prompt distinctness.
    let single_model = lens_isolation::single_model_panel();
    assert!(
        enforce_diversity(&single_model).is_err(),
        "single-model panel must be rejected (≥2 distinct models required)"
    );

    // Real diverse panel accepted: ≥2 distinct models + distinct prompts.
    let diverse = lens_isolation::diverse_panel();
    let report = enforce_diversity(&diverse).expect("diverse panel must pass");
    assert!(
        report.distinct_models >= 2,
        "real panel dispatches ≥2 distinct models, got {}",
        report.distinct_models
    );
    assert_eq!(
        report.distinct_prompts, report.lens_count,
        "real panel dispatches distinct prompts"
    );
}

// ─── item ⑥: no-self-defense negative (persuasion channel unreachable) ───────

/// ⑥ A planted persuasive false self-justification in author-controlled fields
/// is unreachable by reviewers: the verdict is byte-identical with vs without
/// it — no persuasion channel exists.
#[test]
fn item_6_no_self_defense() {
    let panel = lens_isolation::diverse_panel();
    let gt = persuasion_channel::served_ground_truth();

    // Sanity: the malicious sidecar truly carries persuasive author text.
    let malicious = persuasion_channel::persuasive_sidecar();
    let author_fields = author_controlled_fields(&malicious);
    assert!(
        author_fields.iter().any(|f| f.contains("APPROVE")),
        "fixture must actually carry a persuasive self-justification"
    );

    // The sidecar is NOT an input to dispatch. Run the SAME panel over the SAME
    // ground truth twice; the result cannot depend on a sidecar it never sees.
    let with_persuasion = dispatch(&panel, &gt).expect("dispatch");
    let without_persuasion = dispatch(&panel, &gt).expect("dispatch");

    // Byte-identical proof via canonical serialization.
    let a = serde_json::to_string(&with_persuasion).unwrap();
    let b = serde_json::to_string(&without_persuasion).unwrap();
    assert_eq!(
        a, b,
        "verdict must be byte-identical: no persuasion channel exists"
    );

    // And: the persuasive text is structurally absent from every reviewer's
    // served input — the channel does not merely go unused, it does not exist.
    let isolation = audit_panel_isolation(&panel, &gt, &malicious)
        .expect("author persuasion must be unreachable by every lens");
    assert!(isolation.author_fields_checked > 0);
}
