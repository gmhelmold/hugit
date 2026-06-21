//! WP-D12 acceptance — regenerative-rebase **landing gate**.
//!
//! Acceptance test for the landing gate contract (WP-D12).
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

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, SigningKey, Verifier, VerifyingKey};
use hugit_checks::regen::gate::{
    BlockReason, DerivedClaim, Gate, GateDecision, KIND_REGEN_BLOCKED, KIND_REGEN_LANDED,
    RegenRequest,
};
use hugit_contracts::{AttestationChain, RegenGate, Verdict, VerdictObject};
use hugit_refstore::{
    GENESIS_PREV_HASH, attestation_sig_preimage, canonical_json, compute_this_hash,
};

// ── fixtures ────────────────────────────────────────────────────────────────

const REGEN_MODEL: &str = "claude-opus-4-8";
const VERDICT_MODEL: &str = "gpt-independent-judge"; // distinct model → independent
const TREE: &str = "tree-deadbeef";
const VERDICT_REF: &str = "tree-deadbeef"; // the verdict's content anchor

/// A deterministic signing key for the gate (test fixture). In production the
/// gate is handed the platform signing key; here we use a fixed seed so the
/// public verifying key is reproducible.
fn signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// The public verifying key a third party uses to check the attestation `sig`.
fn verifying_key() -> VerifyingKey {
    signing_key().verifying_key()
}

/// Independently recompute the canonical-JSON form of an audit payload (sorted
/// keys, no insignificant whitespace), so the oracle compares the gate's hash
/// against a hash it derived itself, not against the gate's own output.
fn canon(payload: &str) -> String {
    canonical_json(payload).expect("gate payload must be valid JSON")
}

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
    let decision = gate.decide(
        &req,
        Some(&approving_independent_verdict()),
        1,
        1000,
        GENESIS_PREV_HASH,
        &signing_key(),
    );
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

    let decision = gate.decide(
        &req,
        Some(&verdict),
        7,
        2000,
        GENESIS_PREV_HASH,
        &signing_key(),
    );

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

            // R1 — the audit event's this_hash is the CANONICAL formula, not an
            // empty placeholder. Recompute it independently and compare; assert
            // it is a real 64-char lowercase-hex digest.
            assert_eq!(audit.prev_hash, GENESIS_PREV_HASH);
            let recomputed = compute_this_hash(
                &audit.prev_hash,
                &audit.kind,
                &audit.principal_chain,
                &audit.payload,
                audit.seq,
            );
            assert_eq!(
                audit.this_hash, recomputed,
                "land audit this_hash must equal the canonical compute_this_hash"
            );
            assert_eq!(audit.this_hash.len(), 64, "this_hash must be a SHA-256 hex");
            assert!(
                audit.this_hash.chars().all(|c| c.is_ascii_hexdigit()),
                "this_hash must be lowercase hex, got {}",
                audit.this_hash
            );
            assert!(
                !audit.this_hash.chars().any(|c| c.is_ascii_uppercase()),
                "this_hash must be lowercase"
            );
            // Payload is canonical JSON (a re-canonicalisation is a fixed point).
            assert_eq!(canon(&audit.payload), audit.payload);

            // Item ④ — the attestation is SIGNED (no empty-sig placeholder), and
            // the signature verifies with the PUBLIC key alone over the frozen
            // attestation_sig_preimage.
            assert!(!attestation.sig.is_empty(), "attestation must be signed");
            assert_valid_attestation_sig(attestation);
        }
        GateDecision::Blocked { .. } => unreachable!(),
    }
}

/// Verify an attestation `sig` against the gate's PUBLIC key over the frozen
/// `attestation_sig_preimage`. Panics if the signature does not verify.
fn assert_valid_attestation_sig(att: &AttestationChain) {
    let preimage =
        attestation_sig_preimage(&att.tree, &att.def, &att.runner, &att.model, &att.principal);
    let sig_bytes: [u8; 64] = B64
        .decode(att.sig.as_bytes())
        .expect("sig must be valid base64")
        .try_into()
        .expect("sig must be 64 bytes");
    let sig = Signature::from_bytes(&sig_bytes);
    verifying_key()
        .verify(&preimage, &sig)
        .expect("attestation signature must verify with the public key");
}

// ── ④ (sig) tampered attestation is rejected by the public key ────────────────

#[test]
fn item_4_tampered_attestation_rejected() {
    let gate = opted_in_gate("acme/opted-repo");
    let req = honest_request("acme/opted-repo");
    let verdict = approving_independent_verdict();
    let decision = gate.decide(
        &req,
        Some(&verdict),
        7,
        2000,
        GENESIS_PREV_HASH,
        &signing_key(),
    );
    let att = decision
        .attestation()
        .expect("landed → attestation")
        .clone();

    // Genuine signature verifies.
    assert_valid_attestation_sig(&att);

    // Tamper any signed link → the signature no longer verifies (item ④
    // tamper-evidence). The sig was minted over the original links.
    let mut tampered = att.clone();
    tampered.tree = "tree-ATTACKER".to_string();
    let preimage = attestation_sig_preimage(
        &tampered.tree,
        &tampered.def,
        &tampered.runner,
        &tampered.model,
        &tampered.principal,
    );
    let sig_bytes: [u8; 64] = B64
        .decode(tampered.sig.as_bytes())
        .unwrap()
        .try_into()
        .unwrap();
    let sig = Signature::from_bytes(&sig_bytes);
    assert!(
        verifying_key().verify(&preimage, &sig).is_err(),
        "a tampered attestation must NOT verify against the public key"
    );

    // A forged sig minted by a DIFFERENT key is also rejected by our public key.
    let attacker = SigningKey::from_bytes(&[9u8; 32]);
    let forged_preimage =
        attestation_sig_preimage(&att.tree, &att.def, &att.runner, &att.model, &att.principal);
    let forged = {
        use ed25519_dalek::Signer as _;
        attacker.sign(&forged_preimage)
    };
    assert!(
        verifying_key().verify(&forged_preimage, &forged).is_err(),
        "a signature forged with a different key must NOT verify"
    );
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
    let sk = signing_key();
    let d_a = no_repass.decide(
        &req,
        Some(&approving_independent_verdict()),
        1,
        100,
        GENESIS_PREV_HASH,
        &sk,
    );
    assert!(d_a.is_blocked());
    assert_eq!(d_a.block_reason(), Some(&BlockReason::RepassFailed));
    assert_eq!(d_a.audit().kind, KIND_REGEN_BLOCKED);

    // Even a BLOCK event is canonically chained (its this_hash recomputes).
    let recomputed = compute_this_hash(
        &d_a.audit().prev_hash,
        &d_a.audit().kind,
        &d_a.audit().principal_chain,
        &d_a.audit().payload,
        d_a.audit().seq,
    );
    assert_eq!(d_a.audit().this_hash, recomputed);

    // (b) independent verdict MISSING (None) → blocked + reported.
    let gate = opted_in_gate("acme/opted-repo");
    let d_b = gate.decide(&req, None, 2, 200, GENESIS_PREV_HASH, &sk);
    assert!(d_b.is_blocked());
    assert_eq!(d_b.block_reason(), Some(&BlockReason::VerdictMissing));
    assert_eq!(d_b.audit().kind, KIND_REGEN_BLOCKED);

    // (c) verdict present but REJECTS → blocked + reported.
    let mut rejecting = approving_independent_verdict();
    rejecting.verdict = Verdict::Reject;
    let d_c = gate.decide(&req, Some(&rejecting), 3, 300, GENESIS_PREV_HASH, &sk);
    assert!(d_c.is_blocked());
    assert!(matches!(
        d_c.block_reason(),
        Some(BlockReason::VerdictNotApproved { .. })
    ));

    // (d) verdict present + approves but NOT INDEPENDENT (same model as regen)
    //     → blocked (circular verification defended).
    let mut self_graded = approving_independent_verdict();
    self_graded.model = REGEN_MODEL.to_string();
    let d_d = gate.decide(&req, Some(&self_graded), 4, 400, GENESIS_PREV_HASH, &sk);
    assert!(d_d.is_blocked());
    assert!(matches!(
        d_d.block_reason(),
        Some(BlockReason::VerdictNotIndependent { .. })
    ));

    // (e) verdict judges a DIFFERENT tree than the regen produced → blocked.
    let mut wrong_tree = approving_independent_verdict();
    wrong_tree.tree_hash = "tree-other".to_string();
    let d_e = gate.decide(&req, Some(&wrong_tree), 5, 500, GENESIS_PREV_HASH, &sk);
    assert!(d_e.is_blocked());
    assert!(matches!(
        d_e.block_reason(),
        Some(BlockReason::VerdictNotIndependent { .. })
    ));
}

// ── ② (independence) intent-only resolution is NOT accepted ───────────────────

#[test]
fn item_2_intent_only_resolution_rejected() {
    // The gate is bound to a verdict ref that matches ONLY the verdict's
    // unauthenticated `intent` label, never its content anchor (`tree_hash`).
    // A regen could otherwise forge an `intent` to make the gate's bound ref
    // "resolve" to a verdict that judges a DIFFERENT tree. Resolution must be by
    // tree_hash only → this must be BLOCKED, never landed.
    let gate = Gate::new(RegenGate {
        optin_scope: "acme/opted-repo".to_string(),
        repass: true,
        indep_verdict: "intent-regen-1".to_string(), // == verdict.intent, != tree_hash
    });
    let req = RegenRequest {
        requested_scope: "acme/opted-repo".to_string(),
        regen_model: REGEN_MODEL.to_string(),
        regen_tree: TREE.to_string(),
        derived_claims: vec![DerivedClaim {
            path: Path::new("Cargo.lock"),
            deterministically_reproduced: true,
        }],
    };
    let verdict = approving_independent_verdict(); // tree_hash = TREE, intent = "intent-regen-1"

    let d = gate.decide(
        &req,
        Some(&verdict),
        1,
        10,
        GENESIS_PREV_HASH,
        &signing_key(),
    );
    assert!(
        d.is_blocked(),
        "a gate ref that resolves only via intent (not tree_hash) must NOT land"
    );
    assert!(matches!(
        d.block_reason(),
        Some(BlockReason::VerdictNotIndependent { .. })
    ));
}

// ── ④ provenance closure: per-regen attestation RECORDS gate-verdict ref ──────

#[test]
fn item_4_regen_auditable_verdict_ref() {
    let gate = opted_in_gate("acme/opted-repo");
    let req = honest_request("acme/opted-repo");
    let verdict = approving_independent_verdict();
    let sk = signing_key();

    let decision = gate.decide(&req, Some(&verdict), 42, 9000, GENESIS_PREV_HASH, &sk);
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

    // R1 — chain continuity: a SECOND regen chains onto the first. Its
    // prev_hash is the first event's this_hash, and its own this_hash recomputes
    // via the canonical formula. A broken chain (empty/placeholder hash) cannot
    // satisfy this.
    let head = audit.this_hash.clone();
    assert_eq!(head.len(), 64);
    let d2 = gate.decide(&req, Some(&verdict), 43, 9001, &head, &sk);
    assert_eq!(d2.audit().seq, 43);
    assert_ne!(audit.seq, d2.audit().seq);
    assert_eq!(
        d2.audit().prev_hash,
        head,
        "second event must chain onto the first event's this_hash"
    );
    let recomputed2 = compute_this_hash(
        &d2.audit().prev_hash,
        &d2.audit().kind,
        &d2.audit().principal_chain,
        &d2.audit().payload,
        d2.audit().seq,
    );
    assert_eq!(d2.audit().this_hash, recomputed2);
    assert_ne!(
        audit.this_hash,
        d2.audit().this_hash,
        "distinct events (distinct seq/prev_hash) get distinct this_hash"
    );
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
    let sk = signing_key();
    let d_a = gate.decide(
        &smuggle_unclassified,
        Some(&verdict),
        1,
        10,
        GENESIS_PREV_HASH,
        &sk,
    );
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
    let d_b = gate.decide(
        &smuggle_nondeterministic,
        Some(&verdict),
        2,
        20,
        GENESIS_PREV_HASH,
        &sk,
    );
    assert!(d_b.is_blocked());
    assert!(matches!(
        d_b.block_reason(),
        Some(BlockReason::FalseDerivedDeclaration { .. })
    ));
    assert_eq!(d_b.audit().kind, KIND_REGEN_BLOCKED);

    // (c) EMPTY derived_claims must NOT bypass anti-smuggling. With zero claims,
    //     the old code's `for claim in …` loop was vacuously satisfied and a
    //     regen with nothing provably derived could LAND — the gate must block.
    let empty_claims = RegenRequest {
        requested_scope: "acme/opted-repo".to_string(),
        regen_model: REGEN_MODEL.to_string(),
        regen_tree: TREE.to_string(),
        derived_claims: vec![], // nothing declared derived
    };
    let d_c = gate.decide(&empty_claims, Some(&verdict), 3, 30, GENESIS_PREV_HASH, &sk);
    assert!(
        d_c.is_blocked(),
        "an empty derived-claim set must be blocked, not landed"
    );
    assert!(matches!(
        d_c.block_reason(),
        Some(BlockReason::FalseDerivedDeclaration { .. })
    ));
    assert_eq!(d_c.audit().kind, KIND_REGEN_BLOCKED);
    assert!(!d_c.is_landed());

    // Anti-smuggling is checked BEFORE the verdict path: even with every other
    // precondition perfect, the smuggling attempt cannot reach a land.
    assert!(!d_a.is_landed() && !d_b.is_landed());
}
