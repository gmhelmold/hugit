// WP-D12 acceptance oracle — regen gate.
// Each test corresponds to one owned acceptance item from WP-D12.md.
// RED on current tree: the implementation module does not exist yet.
// Implementation target: crates/hugit-checks/regen/gate/

use hugit_checks::regen::gate::{
    classify_derived, optin_scope, regen_gate_decision, GateInput, GateVerdict,
};

/// ① regen only on opt-in scope; non-opted repo never regens.
/// Fixture: non-opted repo → assert zero regen actions emitted.
#[test]
fn item_1_optin_scope_only() {
    let non_opted = optin_scope::Repo::non_opted();
    let result = regen_gate_decision(&non_opted, &GateInput::any());
    assert!(
        result.regen_actions().is_empty(),
        "non-opted repo must emit zero regen actions; got: {:?}",
        result.regen_actions()
    );
}

/// ② regen lands only if acceptance re-passes AND fresh independent adversarial verdict approves.
/// Fixture: re-pass=true, verdict=Approved → expect Land.
#[test]
fn item_2_both_preconditions_land() {
    let input = GateInput::builder()
        .opted_in(true)
        .acceptance_repass(true)
        .independent_verdict(GateVerdict::Approved)
        .build();
    let result = regen_gate_decision(&optin_scope::Repo::opted_in(), &input);
    assert_eq!(
        result.outcome(),
        hugit_checks::regen::gate::Outcome::Land,
        "both preconditions met must produce Land; got: {:?}",
        result.outcome()
    );
}

/// ③ missing/failing either → blocked + reported.
/// Fixture A: re-pass=false → Blocked; Fixture B: verdict=Missing → Blocked.
/// Both must also emit a blocking report.
#[test]
fn item_3_missing_either_blocked_reported() {
    let repo = optin_scope::Repo::opted_in();

    // Failing re-pass, verdict present
    let a = GateInput::builder()
        .opted_in(true)
        .acceptance_repass(false)
        .independent_verdict(GateVerdict::Approved)
        .build();
    let ra = regen_gate_decision(&repo, &a);
    assert_eq!(ra.outcome(), hugit_checks::regen::gate::Outcome::Blocked);
    assert!(ra.report().is_some(), "blocked gate must emit a report (fixture A)");

    // Re-pass ok, verdict missing
    let b = GateInput::builder()
        .opted_in(true)
        .acceptance_repass(true)
        .independent_verdict(GateVerdict::Missing)
        .build();
    let rb = regen_gate_decision(&repo, &b);
    assert_eq!(rb.outcome(), hugit_checks::regen::gate::Outcome::Blocked);
    assert!(rb.report().is_some(), "blocked gate must emit a report (fixture B)");
}

/// ④ every regen auditable as its own revision; AttestationChain records gate-verdict ref (provenance closure).
/// Attestation fixture: assert gate-verdict ref is present and resolves.
#[test]
fn item_4_regen_auditable_verdict_ref() {
    let input = GateInput::builder()
        .opted_in(true)
        .acceptance_repass(true)
        .independent_verdict(GateVerdict::Approved)
        .verdict_ref("report-v42".to_string())
        .build();
    let result = regen_gate_decision(&optin_scope::Repo::opted_in(), &input);
    let attestation = result
        .attestation()
        .expect("Land outcome must produce an AttestationChain");
    assert_eq!(
        attestation.gate_verdict_ref(),
        "report-v42",
        "AttestationChain must record the authorizing gate-verdict ref"
    );
    assert!(
        attestation.resolves(),
        "gate-verdict ref in AttestationChain must resolve"
    );
}

/// ⑤ anti-smuggling: file not provably derived CANNOT be classified derived.
/// False "derived" declaration is blocked + audited (EventRecord emitted).
#[test]
fn item_5_false_derived_blocked_audited() {
    // Fixture: file whose regen command does NOT deterministically produce it
    let false_claim = classify_derived::Claim::unverifiable("src/handwritten.rs");
    let result = classify_derived::evaluate(&false_claim);
    assert!(
        result.is_blocked(),
        "false derived declaration must be blocked; got: {:?}",
        result
    );
    assert!(
        result.audit_event().is_some(),
        "false derived block must emit an audit EventRecord"
    );
}
