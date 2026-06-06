//! WP-X2 acceptance oracle — attestation end-to-end.
//! Contract: `docs/plan/wp-contracts/WP-X2.md`.
//!
//! Owned items (one `#[test] item_<n>_<slug>` each):
//!   ① `item_1_full_chain_resolves` — artifact attestation resolves the full
//!      five-link chain (tree + def + runner + model + principal) cryptographically;
//!      every link must be populated and the sig field must verify.
//!   ② `item_2_tampered_unsigned_rejected` — a tampered chain (mutated tree or
//!      principal) AND an unsigned chain are both rejected at the promotion
//!      boundary fail-closed, with an audit event emitted.
//!   ③ `item_3_public_verification_procedure` — a committed public verification
//!      doc exists under this test crate; the test executes it against a
//!      known-good and a known-bad attestation, proving the doc is sufficient.
//!   ④ `item_4_cross_tenant_shared_hit_honesty` — tenant B's `why`/attestation
//!      on a shared public-deterministic artifact resolves to an anonymized
//!      PLATFORM attestation — zero tenant-A principal/runner bytes in the
//!      surface, AND tenant B is not mis-attributed as producer.
//!
//! RED on current tree: `AttestationChain`, `RegenGate`, and the hugit-contracts
//! promotion surface do not yet exist as importable types.

use hugit_contracts::{AttestationChain, CheckDef, CheckResult, RegenGate};
use hugit_contracts::attestation::{verify_chain, PlatformAttestation, PromotionVerdict};

// ── fixtures ─────────────────────────────────────────────────────────────────

fn known_good_chain() -> AttestationChain {
    AttestationChain::fixture_known_good()
}

fn tampered_chain_mutated_tree() -> AttestationChain {
    let mut c = known_good_chain();
    c.tree = "0000000000000000000000000000000000000000000000000000000000000000".to_string();
    c
}

fn tampered_chain_mutated_principal() -> AttestationChain {
    let mut c = known_good_chain();
    c.principal = "attacker@evil.example".to_string();
    c
}

fn unsigned_chain() -> AttestationChain {
    let mut c = known_good_chain();
    c.sig = None;
    c
}

// ── item_1_full_chain_resolves ────────────────────────────────────────────────

/// ① Resolves the full five-link attestation chain cryptographically.
/// Every link (tree, def, runner, model, principal) must be non-empty and
/// the sig field must verify cleanly. A missing or invalid link is a hard fail.
#[test]
fn item_1_full_chain_resolves() {
    let chain = known_good_chain();

    // All five links populated.
    assert!(!chain.tree.is_empty(), "tree link must be populated");
    assert!(!chain.def.is_empty(), "def link must be populated");
    assert!(!chain.runner.is_empty(), "runner link must be populated");
    assert!(!chain.model.is_empty(), "model link must be populated");
    assert!(!chain.principal.is_empty(), "principal link must be populated");

    // Sig field present.
    let sig = chain.sig.as_ref().expect("sig must be present in a valid chain");
    assert!(!sig.is_empty(), "sig must be non-empty");

    // Cryptographic verification passes.
    let verdict = verify_chain(&chain);
    assert!(
        verdict.is_ok(),
        "full chain cryptographic verification must succeed: {:?}",
        verdict.err()
    );
}

// ── item_2_tampered_unsigned_rejected ─────────────────────────────────────────

/// ② Tampered chains and unsigned chains are rejected fail-closed at the
/// promotion boundary. Each rejection must emit an audit event.
#[test]
fn item_2_tampered_unsigned_rejected() {
    // Case A: mutated tree hash.
    let tampered_tree = tampered_chain_mutated_tree();
    let verdict_a = RegenGate::verify_for_promotion(&tampered_tree);
    assert!(
        matches!(verdict_a, PromotionVerdict::Rejected { .. }),
        "tampered-tree chain must be rejected at promotion: {:?}",
        verdict_a
    );
    assert!(
        verdict_a.has_audit_event(),
        "tampered-tree rejection must emit audit event"
    );

    // Case B: mutated principal.
    let tampered_principal = tampered_chain_mutated_principal();
    let verdict_b = RegenGate::verify_for_promotion(&tampered_principal);
    assert!(
        matches!(verdict_b, PromotionVerdict::Rejected { .. }),
        "tampered-principal chain must be rejected at promotion: {:?}",
        verdict_b
    );
    assert!(
        verdict_b.has_audit_event(),
        "tampered-principal rejection must emit audit event"
    );

    // Case C: unsigned chain.
    let unsigned = unsigned_chain();
    let verdict_c = RegenGate::verify_for_promotion(&unsigned);
    assert!(
        matches!(verdict_c, PromotionVerdict::Rejected { .. }),
        "unsigned chain must be rejected at promotion: {:?}",
        verdict_c
    );
    assert!(
        verdict_c.has_audit_event(),
        "unsigned rejection must emit audit event"
    );
}

// ── item_3_public_verification_procedure ─────────────────────────────────────

/// ③ A public verification doc is committed under this crate. The test executes
/// it against a known-good and a known-bad attestation, proving it is
/// sufficient (not aspirational).
#[test]
fn item_3_public_verification_procedure() {
    // The committed verification procedure doc must exist.
    let doc_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/x2/docs/attestation-verification-procedure.md"
    );
    assert!(
        std::path::Path::new(doc_path).exists(),
        "public verification procedure doc must be committed at x2/docs/attestation-verification-procedure.md"
    );

    // Execute the procedure against a known-good chain — must pass.
    let good_chain = known_good_chain();
    let good_result = hugit_contracts::attestation::execute_verification_procedure(&good_chain);
    assert!(
        good_result.is_ok(),
        "procedure must pass on known-good chain: {:?}",
        good_result.err()
    );

    // Execute the procedure against a tampered chain — must fail.
    let bad_chain = tampered_chain_mutated_tree();
    let bad_result = hugit_contracts::attestation::execute_verification_procedure(&bad_chain);
    assert!(
        bad_result.is_err(),
        "procedure must fail on tampered chain (procedure must be sufficient, not aspirational)"
    );
}

// ── item_4_cross_tenant_shared_hit_honesty ────────────────────────────────────

/// ④ Tenant B's `why`/attestation on a shared public-deterministic artifact
/// resolves to an anonymized PLATFORM attestation. Zero tenant-A
/// principal/runner bytes appear in the surfaced attestation, and tenant B is
/// not mis-attributed as producer.
#[test]
fn item_4_cross_tenant_shared_hit_honesty() {
    let tenant_a_identity = "tenant-a-hmac-prefix-fixture";
    let tenant_b_identity = "tenant-b-hmac-prefix-fixture";

    // A shared public-deterministic artifact (CAS hit shared across tenants).
    let shared_artifact_tree = "sha256:shared-public-deterministic-fixture-hash-abc123";

    // Tenant B requests the `why`/attestation for the shared artifact.
    let resolved: PlatformAttestation =
        hugit_contracts::attestation::resolve_why_for_tenant(
            shared_artifact_tree,
            tenant_b_identity,
        )
        .expect("resolve_why_for_tenant must succeed for a shared public artifact");

    // The resolved attestation must be the anonymized PLATFORM form.
    assert!(
        resolved.is_platform_anonymous(),
        "shared-hit attestation must be anonymized PLATFORM form, not tenant-scoped"
    );

    // Zero tenant-A principal/runner bytes in the surface.
    let serialized = serde_json::to_string(&resolved).expect("attestation must serialize");
    assert!(
        !serialized.contains(tenant_a_identity),
        "tenant-A identity must not appear in shared-hit attestation surface"
    );

    // Tenant B must NOT be mis-attributed as producer.
    assert!(
        !resolved.attributes_producer_to(tenant_b_identity),
        "tenant-B must not be mis-attributed as producer of a shared artifact"
    );
}
