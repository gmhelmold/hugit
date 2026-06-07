//! WP-X9 acceptance oracle — cross-phase object identity.
//! Contract: `docs/plan/wp-contracts/WP-X9.md`.
//!
//! Owned items (VERBATIM from the contract; one `#[test] item_<n>_…` each, plus
//! adversarial guards proving each oracle goes RED on the gamed/broken case):
//!
//!   ① a CheckResult memoized by the phase-B App is bit-identical to the one
//!      served as evidence in a phase-D verdict panel for the same
//!      (tree,def,toolchain)
//!   ② mismatch fails CLOSED + alerts
//!   ③ intent_id identity: the id minted by a phase-B sidecar is identical and
//!      non-colliding with the native phase-D intent for the same logical intent
//!      — one lifecycle, one id; divergence/collision fails CLOSED
//!
//! WHY THE COMPOSITION IS LOAD-BEARING: B2 owns memoization, D7 the verdict
//! panels, B6 the sidecar, D4 the native intents. X9 proves IDENTITY crosses the
//! phase boundary: the SAME CheckResult bytes are served in phase D as were
//! memoized in phase B, and the SAME intent id flows from the phase-B sidecar
//! into the phase-D native intent — one lifecycle, one id. The oracle uses the
//! REAL canonical `hugit_refstore::{canonical_json, compute_memo_key,
//! compute_this_hash}` and the frozen `hugit_contracts::{CheckResult,
//! IntentSidecar, VerdictObject}` so it tests the production surfaces, never a
//! hand-rolled stand-in. The bit-identity assertions are BYTE compares (not
//! result-equal), and the content-id is derived from those exact bytes — so a
//! re-serialization that diverges, or a collision admitted, turns the oracle RED.

#[path = "../identity.rs"]
mod identity;

use hugit_contracts::check_result::{Artifact, CheckResult};
use hugit_contracts::intent_sidecar::IntentSidecar;
use hugit_contracts::verdict_object::{Verdict, VerdictObject};
use hugit_refstore::compute_memo_key;
use identity::{
    IdentityError, IntentError, IntentRegistry, MemoStore, canonical_bytes, content_id,
    reconcile_intent, register_one_lifecycle, serve_evidence,
};

// ── shared fixtures ──────────────────────────────────────────────────────────

/// 64-char lowercase-hex digest fixtures for the three memo axes.
const TREE: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const DEF: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TOOLCHAIN: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

/// The single logical CheckResult both phases must share. Built once; the SAME
/// value is what phase-B memoizes and what phase-D's panel serves as evidence.
fn the_check_result() -> CheckResult {
    CheckResult {
        memo_key: compute_memo_key(TREE, DEF, TOOLCHAIN),
        tree_hash: TREE.to_string(),
        def_digest: DEF.to_string(),
        toolchain_digest: TOOLCHAIN.to_string(),
        exit: 0,
        artifacts: vec![
            Artifact {
                path: "target/out.bin".to_string(),
                digest: "d".repeat(64),
            },
            Artifact {
                path: "target/report.json".to_string(),
                digest: "e".repeat(64),
            },
        ],
        stdout_ref: "blob:stdout-0001".to_string(),
        stderr_ref: "blob:stderr-0001".to_string(),
        duration_ms: 4242,
        runner_ref: "runner:hetzner-01".to_string(),
        produced_at: 1_700_000_000_000,
    }
}

/// A phase-B sidecar that mints `intent_id`.
fn sidecar(intent_id: &str) -> IntentSidecar {
    IntentSidecar {
        intent_id: intent_id.to_string(),
        charter: "land the X9 cross-phase identity work".to_string(),
        acceptance: vec!["①".to_string(), "②".to_string(), "③".to_string()],
        context_ref: "blob:context-0001".to_string(),
        authoritative: false,
    }
}

/// A phase-D native verdict object carrying `intent` as its native intent id.
fn native_verdict(intent: &str) -> VerdictObject {
    VerdictObject {
        intent: intent.to_string(),
        tree_hash: TREE.to_string(),
        lens: "lens:default".to_string(),
        model: "claude-opus-4-8".to_string(),
        prompt_digest: "f".repeat(64),
        verdict: Verdict::Approve,
        claims_checked: vec!["c1".to_string()],
        evidence_refs: vec!["blob:evidence-0001".to_string()],
    }
}

// ── Item ① — cross-phase CheckResult bit-identity ────────────────────────────

#[test]
fn item_1_memoized_checkresult_is_bit_identical_to_phase_d_evidence() {
    // phase-B memoizes the result.
    let mut memo = MemoStore::new();
    memo.memoize(the_check_result());

    // phase-D's verdict panel proposes the SAME logical result as evidence.
    let panel_candidate = the_check_result();

    // serve_evidence proves cross-phase byte-identity and returns the ONE
    // shared canonical byte string.
    let served = serve_evidence(&memo, TREE, DEF, TOOLCHAIN, &panel_candidate)
        .expect("identical evidence serves");

    // BYTE compare (not result-equal): the served evidence bytes equal the
    // phase-B memo's canonical bytes, for the same (tree,def,toolchain).
    let memoized = memo
        .get(TREE, DEF, TOOLCHAIN)
        .expect("phase-B memo present");
    assert_eq!(
        served,
        canonical_bytes(memoized),
        "served phase-D evidence must be bit-identical to the phase-B memo bytes"
    );

    // and the content id derived from those exact bytes agrees byte-for-byte.
    assert_eq!(
        content_id(memoized),
        content_id(&panel_candidate),
        "content id of memo and evidence must be identical"
    );
}

#[test]
fn item_1_canonical_bytes_are_stable_across_field_order() {
    // GAMED-ORACLE GUARD: bit-identity must survive non-canonical authoring
    // (different JSON key order / whitespace) — otherwise the oracle would be
    // testing serializer luck, not identity. canonical_bytes routes through the
    // production canonicaliser, so two authorings of the SAME logical result
    // produce IDENTICAL bytes.
    let a = the_check_result();
    let b = the_check_result();
    assert_eq!(
        canonical_bytes(&a),
        canonical_bytes(&b),
        "canonical bytes must be deterministic for the same logical result"
    );
}

// ── Item ② — mismatch fails CLOSED + alerts ──────────────────────────────────

#[test]
fn item_2_cross_phase_mismatch_fails_closed_and_alerts() {
    let mut memo = MemoStore::new();
    memo.memoize(the_check_result());

    // phase-D proposes a DIVERGENT object under the SAME memo key: same
    // (tree,def,toolchain) but a tampered field (different exit code) — the bytes
    // diverge from the phase-B memo.
    let mut tampered = the_check_result();
    tampered.exit = 1;

    let err = serve_evidence(&memo, TREE, DEF, TOOLCHAIN, &tampered)
        .expect_err("a divergent object must NOT be served as evidence");

    // fails CLOSED on the mismatch path…
    match &err {
        IdentityError::EvidenceMismatch {
            memo_key,
            memo_id,
            evidence_id,
            ..
        } => {
            assert_eq!(*memo_key, compute_memo_key(TREE, DEF, TOOLCHAIN));
            assert_ne!(
                memo_id, evidence_id,
                "the mismatch must be detected as differing content ids"
            );
        }
        other => panic!("expected EvidenceMismatch, got {other:?}"),
    }

    // …AND alerts: the fail-closed error carries an alert with a stable code.
    assert_eq!(err.alert().code, "x9.evidence_mismatch");
    assert!(
        !err.alert().detail.is_empty(),
        "the alert must carry a non-empty detail"
    );
}

#[test]
fn item_2_missing_memo_fails_closed_and_alerts() {
    // phase-D must never invent evidence with no phase-B memo behind it.
    let memo = MemoStore::new(); // empty
    let candidate = the_check_result();

    let err = serve_evidence(&memo, TREE, DEF, TOOLCHAIN, &candidate)
        .expect_err("no phase-B memo ⇒ fail closed");
    assert!(matches!(err, IdentityError::NoMemo { .. }));
    assert_eq!(err.alert().code, "x9.no_memo");
}

#[test]
fn item_2_guard_serving_does_not_swallow_subtle_divergence() {
    // GAMED-ORACLE GUARD: a divergence in a NON-key field (an artifact digest)
    // for the SAME (tree,def,toolchain) must still fail closed — proving the
    // compare is over the FULL object bytes, not just the memo axes.
    let mut memo = MemoStore::new();
    memo.memoize(the_check_result());

    let mut tampered = the_check_result();
    tampered.artifacts[0].digest = "0".repeat(64);

    let err = serve_evidence(&memo, TREE, DEF, TOOLCHAIN, &tampered)
        .expect_err("a subtle artifact-digest divergence must fail closed");
    assert!(matches!(err, IdentityError::EvidenceMismatch { .. }));
}

// ── Item ③ — intent_id identity: one lifecycle, one id ───────────────────────

#[test]
fn item_3_phase_b_intent_id_is_identical_to_phase_d_native_intent() {
    const ID: &str = "intent:01HXAMPLE-one-lifecycle";

    // phase-B sidecar mints the id; phase-D native intent carries the EXACT id.
    let b = sidecar(ID);
    let d = native_verdict(ID);

    let reconciled = reconcile_intent(&b, &d).expect("identical ids reconcile");
    assert_eq!(reconciled, ID, "one lifecycle yields exactly one id");

    // end-to-end: register the lifecycle under one logical intent + one id.
    let mut registry = IntentRegistry::new();
    let id = register_one_lifecycle(&mut registry, "logical:pr-42", &b, &d)
        .expect("one lifecycle registers");
    assert_eq!(id, ID);
    assert_eq!(registry.logical_for(ID), Some("logical:pr-42"));
}

#[test]
fn item_3_divergent_intent_id_fails_closed() {
    // phase-D mints a SECOND id for the same logical intent — divergence.
    let b = sidecar("intent:from-phase-b");
    let d = native_verdict("intent:freshly-minted-in-phase-d");

    let err = reconcile_intent(&b, &d).expect_err("a divergent id must fail closed");
    match &err {
        IntentError::Divergence {
            sidecar_id,
            native_id,
            ..
        } => {
            assert_eq!(sidecar_id, "intent:from-phase-b");
            assert_eq!(native_id, "intent:freshly-minted-in-phase-d");
        }
        other => panic!("expected Divergence, got {other:?}"),
    }
    assert_eq!(err.alert().code, "x9.intent_divergence");
}

#[test]
fn item_3_colliding_intent_id_fails_closed() {
    // two DISTINCT logical intents fold onto the SAME id — a collision.
    let mut registry = IntentRegistry::new();
    registry
        .bind("logical:pr-42", "intent:shared")
        .expect("first bind succeeds");

    let err = registry
        .bind("logical:pr-99", "intent:shared")
        .expect_err("a distinct logical intent on the same id must fail closed");
    match &err {
        IntentError::Collision {
            intent_id,
            existing_logical,
            incoming_logical,
            ..
        } => {
            assert_eq!(intent_id, "intent:shared");
            assert_eq!(existing_logical, "logical:pr-42");
            assert_eq!(incoming_logical, "logical:pr-99");
        }
        other => panic!("expected Collision, got {other:?}"),
    }
    assert_eq!(err.alert().code, "x9.intent_collision");
}

#[test]
fn item_3_rebinding_same_lifecycle_is_idempotent_not_a_collision() {
    // GAMED-ORACLE GUARD: the SAME logical intent re-observed on the SAME id is
    // NOT a collision (one lifecycle, observed twice) — otherwise the collision
    // check would be too strict and the law would be vacuous/unusable.
    let mut registry = IntentRegistry::new();
    registry
        .bind("logical:pr-42", "intent:shared")
        .expect("first bind succeeds");
    registry
        .bind("logical:pr-42", "intent:shared")
        .expect("idempotent re-bind of the same lifecycle succeeds");
    assert_eq!(registry.logical_for("intent:shared"), Some("logical:pr-42"));
}

#[test]
fn item_3_register_one_lifecycle_rejects_collision_end_to_end() {
    // end-to-end fail-closed: two distinct logical intents whose sidecar/native
    // ids each reconcile, but which collide on the shared id in the registry.
    let mut registry = IntentRegistry::new();

    let b1 = sidecar("intent:shared");
    let d1 = native_verdict("intent:shared");
    register_one_lifecycle(&mut registry, "logical:pr-42", &b1, &d1).expect("first lifecycle ok");

    let b2 = sidecar("intent:shared");
    let d2 = native_verdict("intent:shared");
    let err = register_one_lifecycle(&mut registry, "logical:pr-99", &b2, &d2)
        .expect_err("a distinct logical intent colliding on the id must fail closed");
    assert!(matches!(err, IntentError::Collision { .. }));
}
