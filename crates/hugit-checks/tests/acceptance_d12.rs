//! WP-D12 acceptance — regenerative-rebase **landing gate**.
//!
//! Contract: docs/plan/wp-contracts/WP-D12.md
//! Oracle: tests/acceptance/wp-d12/run.sh
//!
//! Owned items (one `#[test] item_<n>_<slug>` each):
//!   ① regen only on opt-in scope; non-opted repo never regens
//!   ② regen lands only if acceptance re-passes AND a fresh independent
//!      adversarial verdict approves
//!   ③ missing/failing either → blocked + reported
//!   ④ every regen is its own auditable revision whose AttestationChain
//!      RECORDS the authorising gate-verdict ref (provenance closure)
//!   ⑤ (R5) anti-smuggling: a file not provably derived CANNOT be classified
//!      derived — bypass via a false "derived" declaration is blocked + audited
//!
//! Gate-binding items ②③④⑤ are proven via state-machine + attestation fixture
//! proofs (local, deterministic) per the oracle.

use std::path::Path;

use hugit_checks::regen::gate::{
    BlockReason, DerivedClaim, Gate, GateDecision, KIND_REGEN_BLOCKED, KIND_REGEN_LANDED,
    RegenRequest,
};
use hugit_contracts::{RegenGate, Verdict, VerdictObject};

// ── fixtures ────────────────────────────────────────────────────────────────

const REGEN_MODEL: &str = "claude-opus-4-8";
const VERDICT_MODEL: &str = "gpt-independent-judge"; // distinct model → independent
const TREE: &str = "tree-deadbeef";
const VERDICT_REF: &str = "tree-deadbeef"; // the verdict's content anchor

/// A gate opted-in for `scope`, re-passed, bound to the verdict ref.
fn opted_in_gate(scope: &str) -> Gate {
    Gate::new(RegenGate {
        optin_scope: scope.to_string(),
        repass: true,
        indep_verdict: VERDICT_REF.to_string(),
    })
}

/// A FRESH INDEPENDENT adversarial verdict that APPROVES, judging `TREE`.
fn approving_independent_verdict() -> VerdictObject {
    VerdictObject {
        intent: "intent-regen-1".to_string(),
        tree_hash: TREE.to_string(),
        lens: "adversarial-regen-panel".to_string(),
        model: VERDICT_MODEL.to_string(),
        prompt_digest: "a".repeat(64),
        verdict: Verdict::Approve,
        claims_checked: vec!["regen-reproduces-sources".to_string()],
        evidence_refs: vec!["evidence-1".to_string()],
    }
}

/// A regen request whose single derived file is a real, deterministically
/// reproduced lockfile (passes anti-smuggling).
fn honest_request(scope: &str) -> RegenRequest<'static> {
    RegenRequest {
        requested_scope: scope.to_string(),
        regen_model: REGEN_MODEL.to_string(),
        regen_tree: TREE.to_string(),
        derived_claims: vec![DerivedClaim {
            path: Path::new("Cargo.lock"),
            deterministically_reproduced: true,
        }],
    }
}

// ── ① opt-in scope only; non-opted repo never regens ─────────────────────────

#[test]
fn item_1_optin_scope_only() {
    let gate = opted_in_gate("acme/opted-repo");

    // Opted repo: scope check passes.
    assert!(gate.is_opted_in("acme/opted-repo"));

    // Non-opted repo: NEVER regens — decision is a block with NotOptedIn, even
    // when every other precondition would have passed.
    assert!(!gate.is_opted_in("acme/other-repo"));
    let req = honest_request("acme/other-repo");
    let decision = gate.decide(&req, Some(&approving_independent_verdict()), 1, 1000);
    assert!(
        decision.is_blocked(),
        "non-opted repo must never land a regen"
    );
    assert!(matches!(
        decision.block_reason(),
        Some(BlockReason::NotOptedIn { .. })
    ));
    assert_eq!(decision.audit().kind, KIND_REGEN_BLOCKED);

    // A "*" scope opts in everything.
    let star = opted_in_gate("*");
    assert!(star.is_opted_in("anyone/anything"));
}

// ── ② both preconditions → land (re-pass AND independent verdict approve) ─────

#[test]
fn item_2_both_preconditions_land() {
    let gate = opted_in_gate("acme/opted-repo");
    let req = honest_request("acme/opted-repo");
    let verdict = approving_independent_verdict();

    let decision = gate.decide(&req, Some(&verdict), 7, 2000);

    // Both preconditions met → LAND.
    assert!(
        decision.is_landed(),
        "re-pass + independent approving verdict must land"
    );
    match &decision {
        GateDecision::Land { attestation, audit } => {
            assert_eq!(audit.kind, KIND_REGEN_LANDED);
            assert_eq!(attestation.tree, TREE);
            assert_eq!(attestation.model, REGEN_MODEL);
        }
        GateDecision::Blocked { .. } => unreachable!(),
    }
}

// ── ③ missing/failing EITHER precondition → blocked + reported ────────────────

#[test]
fn item_3_missing_either_blocked_reported() {
    // (a) acceptance re-pass FAILS → blocked + reported.
    let no_repass = Gate::new(RegenGate {
        optin_scope: "acme/opted-repo".to_string(),
        repass: false,
        indep_verdict: VERDICT_REF.to_string(),
    });
    let req = honest_request("acme/opted-repo");
    let d_a = no_repass.decide(&req, Some(&approving_independent_verdict()), 1, 100);
    assert!(d_a.is_blocked());
    assert_eq!(d_a.block_reason(), Some(&BlockReason::RepassFailed));
    assert_eq!(d_a.audit().kind, KIND_REGEN_BLOCKED);

    // (b) independent verdict MISSING (None) → blocked + reported.
    let gate = opted_in_gate("acme/opted-repo");
    let d_b = gate.decide(&req, None, 2, 200);
    assert!(d_b.is_blocked());
    assert_eq!(d_b.block_reason(), Some(&BlockReason::VerdictMissing));
    assert_eq!(d_b.audit().kind, KIND_REGEN_BLOCKED);

    // (c) verdict present but REJECTS → blocked + reported.
    let mut rejecting = approving_independent_verdict();
    rejecting.verdict = Verdict::Reject;
    let d_c = gate.decide(&req, Some(&rejecting), 3, 300);
    assert!(d_c.is_blocked());
    assert!(matches!(
        d_c.block_reason(),
        Some(BlockReason::VerdictNotApproved { .. })
    ));

    // (d) verdict present + approves but NOT INDEPENDENT (same model as regen)
    //     → blocked (circular verification defended).
    let mut self_graded = approving_independent_verdict();
    self_graded.model = REGEN_MODEL.to_string();
    let d_d = gate.decide(&req, Some(&self_graded), 4, 400);
    assert!(d_d.is_blocked());
    assert!(matches!(
        d_d.block_reason(),
        Some(BlockReason::VerdictNotIndependent { .. })
    ));

    // (e) verdict judges a DIFFERENT tree than the regen produced → blocked.
    let mut wrong_tree = approving_independent_verdict();
    wrong_tree.tree_hash = "tree-other".to_string();
    let d_e = gate.decide(&req, Some(&wrong_tree), 5, 500);
    assert!(d_e.is_blocked());
    assert!(matches!(
        d_e.block_reason(),
        Some(BlockReason::VerdictNotIndependent { .. })
    ));
}

// ── ④ provenance closure: per-regen attestation RECORDS gate-verdict ref ──────

#[test]
fn item_4_regen_auditable_verdict_ref() {
    let gate = opted_in_gate("acme/opted-repo");
    let req = honest_request("acme/opted-repo");
    let verdict = approving_independent_verdict();

    let decision = gate.decide(&req, Some(&verdict), 42, 9000);
    let attestation = decision
        .attestation()
        .expect("a landed regen must carry its own attestation");

    // The attestation is this regen's OWN auditable revision: it anchors to the
    // regen tree and is produced by the gate runner.
    assert_eq!(attestation.tree, TREE);
    assert_eq!(attestation.runner, "hugit-checks/regen/gate");

    // Provenance closure: the authorising gate-verdict ref is RECORDED in the
    // principal chain ("this regen was permitted because report-vX passed"),
    // and it RESOLVES to the bound verdict ref.
    let expected_ref = format!("gate-verdict:{VERDICT_REF}");
    assert!(
        attestation.principal.iter().any(|p| p == &expected_ref),
        "attestation must record the authorising gate-verdict ref; got {:?}",
        attestation.principal
    );
    assert_eq!(gate.contract().indep_verdict, VERDICT_REF);

    // The land audit event also references the verdict (auditable revision).
    let audit = decision.audit();
    assert_eq!(audit.kind, KIND_REGEN_LANDED);
    assert!(
        audit.payload.contains(VERDICT_REF),
        "land audit payload must reference the gate-verdict ref; got {}",
        audit.payload
    );

    // Distinct revisions get distinct sequence numbers (each regen its own
    // auditable revision).
    let d2 = gate.decide(&req, Some(&verdict), 43, 9001);
    assert_eq!(d2.audit().seq, 43);
    assert_ne!(audit.seq, d2.audit().seq);
}

// ── ⑤ anti-smuggling: false "derived" declaration blocked + audited ───────────

#[test]
fn item_5_false_derived_blocked_audited() {
    let gate = opted_in_gate("acme/opted-repo");
    let verdict = approving_independent_verdict();

    // (a) A non-derived source file FALSELY declared "derived" — does not
    //     classify into any derived class → blocked + audited (the gate is NOT
    //     bypassed by the false declaration).
    let smuggle_unclassified = RegenRequest {
        requested_scope: "acme/opted-repo".to_string(),
        regen_model: REGEN_MODEL.to_string(),
        regen_tree: TREE.to_string(),
        derived_claims: vec![DerivedClaim {
            path: Path::new("src/main.rs"),     // hand-written source, NOT derived
            deterministically_reproduced: true, // even if the witness lies
        }],
    };
    let d_a = gate.decide(&smuggle_unclassified, Some(&verdict), 1, 10);
    assert!(
        d_a.is_blocked(),
        "a non-derived file declared derived must be blocked"
    );
    assert!(matches!(
        d_a.block_reason(),
        Some(BlockReason::FalseDerivedDeclaration { .. })
    ));
    assert_eq!(
        d_a.audit().kind,
        KIND_REGEN_BLOCKED,
        "the false-derived declaration must be AUDITED"
    );
    assert!(d_a.audit().payload.contains("false_derived_declaration"));

    // (b) A real derived class (lockfile) but NOT deterministically reproduced
    //     from sources → still a false "derived" claim → blocked + audited.
    let smuggle_nondeterministic = RegenRequest {
        requested_scope: "acme/opted-repo".to_string(),
        regen_model: REGEN_MODEL.to_string(),
        regen_tree: TREE.to_string(),
        derived_claims: vec![DerivedClaim {
            path: Path::new("Cargo.lock"),
            deterministically_reproduced: false, // not provably derived
        }],
    };
    let d_b = gate.decide(&smuggle_nondeterministic, Some(&verdict), 2, 20);
    assert!(d_b.is_blocked());
    assert!(matches!(
        d_b.block_reason(),
        Some(BlockReason::FalseDerivedDeclaration { .. })
    ));
    assert_eq!(d_b.audit().kind, KIND_REGEN_BLOCKED);

    // Anti-smuggling is checked BEFORE the verdict path: even with every other
    // precondition perfect, the smuggling attempt cannot reach a land.
    assert!(!d_a.is_landed() && !d_b.is_landed());
}
