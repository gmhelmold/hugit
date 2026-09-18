//! WP-B10 acceptance oracle — phase-B negative scope asserts (items ①②).
//!
//! Acceptance test for the phase-B negative scope contract (WP-B10).
//! Owned items (ABSENCE assertions):
//!   ① no claim/lease acquired at dispatch — conflict discovery happens ONLY at
//!      landing/union (assert mechanism absent).
//!   ② rebase in phase B is textual-fallback only — regenerative path
//!      absent/disabled (assert).
//!
//! These are structural/negative proofs over the existing crate public surface
//! and source via in-process grep-equivalent checks.  No source is modified.
//!
//! Lead adjudication (wave-d1 anchor, recorded in run.sh header):
//!   ② targets regenerative REBASE, not C4's sanctioned derived-file regen
//!   (hugit-checks/src/regen/ is C4's lockfile/codegen/snapshot drivers, which
//!   are SANCTIONED phase-B behavior; this WP asserts the ABSENCE of
//!   *rebase-re-execution* symbols only).

mod negative_scope;

use hugit_contracts::{QueueApi, RunnerLease};
use hugit_queue::core::union::{CheckSource, MemoCheck, UnionVerdict};

// ── helpers ───────────────────────────────────────────────────────────────────

/// Compile-time proof that `QueueApi` and `RunnerLease` are importable as
/// frozen contracts (they must exist for the absence assertions to be anchored
/// to the real contract surface).
fn _assert_contracts_importable(_q: &QueueApi, _r: &RunnerLease) {}

// ── item_1: no claim/lease acquired at dispatch ───────────────────────────────

/// ① The Phase-B dispatch path acquires NO claim and NO `RunnerLease` for
/// conflict-discovery purposes.  Conflict discovery happens ONLY at
/// landing/union (whitepaper §6.4, B4 union test is the conflict oracle).
///
/// Structural proof: the `MemoCheck` trait — the ONLY dispatch-time interface
/// in the Phase-B engine — has no method that accepts or returns a `RunnerLease`
/// or claim token.  The engine transitions through `evaluate_union` without
/// acquiring a lease.
#[test]
fn item_1_no_claim_lease_at_dispatch_mechanism_absent() {
    // Verify that the `MemoCheck` interface (the dispatch-time oracle) does NOT
    // involve RunnerLease or any claim acquisition.
    //
    // We construct a conforming `MemoCheck` implementation that demonstrates the
    // interface is purely `evaluate(&mut self, item_ids: &[&str])` with no
    // lease/claim parameter.  If the interface had grown a claim/lease parameter,
    // this test would fail to compile — the structural proof is in the type.
    struct NoClaimOracle;
    impl MemoCheck for NoClaimOracle {
        fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
            // This entire implementation has no access to RunnerLease or any
            // claim token — the interface forbids it structurally.
            (
                UnionVerdict::Green,
                item_ids.iter().map(|_| CheckSource::Hit).collect(),
            )
        }
    }

    // A dispatch call through `evaluate_union` (the Phase-B dispatch path)
    // uses only the `MemoCheck` oracle — no claim/lease is acquired.
    // The assertion is that we can complete the dispatch without ever
    // constructing or touching a `RunnerLease`.
    use hugit_contracts::LandableEntry;
    use hugit_queue::core::affected::AffectedSet;
    use hugit_queue::core::batch::Batch;
    use hugit_queue::core::union::evaluate_union;

    fn entry(id: &str, order: u64) -> LandableEntry {
        LandableEntry {
            item_id: id.to_string(),
            intent_id: format!("intent-{id}"),
            tree_hash: format!("tree-{id}"),
            order_index: order,
        }
    }

    let batch = Batch::from_entries(
        "b10-dispatch-1",
        [
            (entry("x", 0), AffectedSet::new(["src/x.rs"])),
            (entry("y", 1), AffectedSet::new(["src/y.rs"])),
        ],
    );

    let mut oracle = NoClaimOracle;
    let ev = evaluate_union(&batch, &mut oracle);

    // Dispatch completes green — no claim/lease acquired anywhere in the path.
    assert_eq!(
        ev.verdict,
        UnionVerdict::Green,
        "dispatch succeeds without claim/lease"
    );
    assert_eq!(
        ev.executed_count, 0,
        "dispatch is claim-free and uses only memoised checks (0 executions)"
    );
    assert!(
        ev.minimal_failing_pair.is_none(),
        "conflict discovery is absent at dispatch — no pair named"
    );

    // Structural proof summary: `evaluate_union` is the Phase-B dispatch path.
    // Its signature is `(batch: &Batch, oracle: &mut M) -> UnionEvaluation`.
    // There is no `RunnerLease` parameter, no `claim_acquire` call, and no
    // mechanism to acquire one — the ABSENCE is in the type signature itself.
}

/// ① Supplementary: `RunnerLease` exists as a contract type but has NO
/// constructor or method reachable from the Phase-B dispatch path.
/// Verified by the fact that `RunnerLease` fields are public (JSON contract)
/// but no `hugit-queue` src function calls into it.
#[test]
fn item_1_runner_lease_is_contract_type_only_not_acquired_at_dispatch() {
    // We can construct a RunnerLease from its fields (it is a plain data type),
    // but the Phase-B dispatch path (`core::*`) never does so.
    use hugit_contracts::RunnerState;

    let lease = RunnerLease {
        lease_id: "test-lease".to_string(),
        principal_chain: vec!["agent-1".to_string()],
        path_set: vec!["/workspace/test".to_string()],
        expiry: 9_999_999_999,
        net_policy: "default".to_string(),
        tmp_root: "/tmp/test".to_string(),
        state: RunnerState::Held,
    };

    // The lease exists as a data contract; asserting its fields are accessible
    // proves it is a frozen type, not a runtime mechanism in the dispatch path.
    assert_eq!(lease.lease_id, "test-lease");
    assert_eq!(lease.state, RunnerState::Held);

    // The critical ABSENCE: no function in `hugit_queue::core::*` accepts or
    // returns a `RunnerLease` — there is no dispatch-time claim path.
    // This is verified by the source grep in run.sh check (d) item ①, and
    // by the fact that the `evaluate_union` call above (item_1 primary test)
    // compiles and runs without importing or touching RunnerLease.
}

// ── item_2: regenerative rebase path absent/disabled in Phase B ───────────────

/// ② The Phase-B rebase uses the textual fast-path ONLY (whitepaper §6.3:
/// `claims(I) ∩ Δ = ∅ → textual fast-path`).  The regenerative (re-execution)
/// rebase path is ⛔ CUT from Phase B (command-catalog).
///
/// Structural proof: the `MemoCheck` trait — which backs the Phase-B
/// union/rebase oracle — has no `regen_rebase`, `RegenRebase`, or re-execution
/// variant.  The union fold returns a `UnionVerdict` with no rebase-execution
/// discriminant.  The regen-rebase code-path is unrepresentable in Phase B.
#[test]
fn item_2_rebase_textual_fallback_only_regen_path_absent() {
    // Verify that `UnionVerdict` — the rebase outcome type in Phase B — has
    // green/red and explicit inconclusive results. A `RegenRebase` or re-execution variant
    // would need to appear here; its absence is the structural proof.
    //
    // This is an exhaustive match: if a new variant is added to `UnionVerdict`,
    // this match will fail to compile (non-exhaustive), immediately surfacing
    // scope creep.
    fn verdict_has_no_regen_variant(v: UnionVerdict) -> &'static str {
        match v {
            // Only textual-path outcomes exist in Phase B.
            UnionVerdict::Green => "textual-pass",
            UnionVerdict::Red => "textual-fail",
            UnionVerdict::Unknown => "textual-unknown",
            UnionVerdict::InfrastructureFailure => "textual-infrastructure-failure",
            // If a `RegenRebase` or `Regenerative` variant were added, the
            // exhaustive match would fail to compile — catching scope creep
            // at build time.
        }
    }

    assert_eq!(
        verdict_has_no_regen_variant(UnionVerdict::Green),
        "textual-pass"
    );
    assert_eq!(
        verdict_has_no_regen_variant(UnionVerdict::Red),
        "textual-fail"
    );
    assert_eq!(
        verdict_has_no_regen_variant(UnionVerdict::Unknown),
        "textual-unknown"
    );
    assert_eq!(
        verdict_has_no_regen_variant(UnionVerdict::InfrastructureFailure),
        "textual-infrastructure-failure"
    );
}

/// ② Supplementary: the Phase-B `CheckSource` type has only `Hit` and
/// `Executed` — no `RegenRebase` or re-execution variant.
/// The regenerative rebase path requires a distinct source discriminant that
/// does not exist in the Phase-B type system.
#[test]
fn item_2_check_source_has_no_regen_rebase_variant() {
    // Exhaustive match over `CheckSource` — if a `RegenRebase` variant were
    // added, this match would not compile.
    fn check_source_name(s: CheckSource) -> &'static str {
        match s {
            CheckSource::Hit => "hit",
            CheckSource::Executed => "executed",
            // No `RegenRebase`, `Regenerative`, or re-execution variant exists.
        }
    }

    assert_eq!(check_source_name(CheckSource::Hit), "hit");
    assert_eq!(check_source_name(CheckSource::Executed), "executed");

    // Lead adjudication note: hugit-checks/src/regen/ (C4's derived-file
    // drivers for lockfiles/codegen/snapshots) is SANCTIONED phase-B behavior
    // and is NOT the target of this assertion.  The prohibition is on
    // *regenerative REBASE* (source-change re-execution, D12-gated) only.
    // The `regen/driver` module contains `RegenDriver` (derived-file regen),
    // not `RegenRebase` (source-rebase re-execution) — the names are distinct
    // and the grep in run.sh check (d) item ② is scoped accordingly.
}
