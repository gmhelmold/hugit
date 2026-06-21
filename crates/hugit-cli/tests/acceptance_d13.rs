//! WP-D13 acceptance tests — tournament: N candidates, judge panel, budget-bounded.
//!
//! Acceptance test for the tournament contract (WP-D13).
//! Oracle:   tests/acceptance/wp-d13/run.sh.
//!
//! Naming convention (binding, pre-decided by the lead):
//!   item_<n>_<slug>
//!
//! Owned items:
//!   ① `-n N` produces N independent candidates
//!   ② judge panel selects per documented criteria (fixture w/ known-best)
//!   ③ losers remain addressable as evidence
//!   ④ (R7) budget-bounded fan-out: N is policy-capped; respects per-tenant
//!      caps + fairness (C7); ZERO overage under flat plan.
//!
//! STRUCTURAL / fixture proofs only — there are NO live model API calls.
//! The judge panel uses D7 dispatch with injected deterministic reviewers.
//! Budget assertions use the C7 BudgetManager directly (consumed read-only).

use hugit_cli::tournament::{
    FanOutOutcome, MAX_N_POLICY, UNITS_PER_CANDIDATE, assert_fairness_constants, attempt_fan_out,
    fan_out_event, produce_candidates, resolve_loser, select, selection_event,
};
use hugit_cli::verdict::fixtures::lens_isolation::{
    ApprovingReviewer, CONTRACTS_PROMPT, SECURITY_PROMPT,
};
use hugit_cli::verdict::panel_dispatch::{Lens, Panel, ServedGroundTruth};
use hugit_contracts::IntentSidecar;
use hugit_queue::budget::BudgetManager;
use std::sync::Arc;

// ── helpers ───────────────────────────────────────────────────────────────────

fn test_intent(id: &str) -> IntentSidecar {
    IntentSidecar {
        intent_id: id.to_string(),
        charter: "tournament acceptance fixture".to_string(),
        acceptance: vec![],
        context_ref: "blob://ctx-d13".to_string(),
        authoritative: false,
    }
}

/// A two-model, diverse panel (consumes D7 dispatch — no live calls).
fn diverse_panel() -> Panel {
    let reviewer: Arc<dyn hugit_cli::verdict::panel_dispatch::Reviewer> =
        Arc::new(ApprovingReviewer);
    Panel::new(vec![
        Lens::new(
            "security",
            SECURITY_PROMPT,
            "model-alpha",
            Arc::clone(&reviewer),
        ),
        Lens::new(
            "contracts",
            CONTRACTS_PROMPT,
            "model-beta",
            Arc::clone(&reviewer),
        ),
    ])
}

fn ground_truth(intent_id: &str, evidence_refs: Vec<String>) -> ServedGroundTruth {
    ServedGroundTruth::from_served(intent_id, "tree-d13", vec![], vec![], vec![], evidence_refs)
}

fn budget_manager_with(tenant: &str, capacity: u64) -> BudgetManager {
    let mut m = BudgetManager::default();
    m.register_tenant(tenant, capacity);
    m
}

// ─── item ①: -n N produces N independent candidates ─────────────────────────

/// ① `-n N` produces exactly N independent candidates: count equals N and every
/// candidate carries a distinct content-addressed ref (no shared mutation).
#[test]
fn item_1_n_independent_candidates() {
    let intent = test_intent("intent-d13-item1");
    let strategies = ["strategy-0", "strategy-1", "strategy-2", "strategy-3"];
    let n = strategies.len();

    let candidates = produce_candidates(&intent, &strategies);

    // Count equals N.
    assert_eq!(
        candidates.len(),
        n,
        "produce_candidates must produce exactly N candidates"
    );

    // All candidate refs are distinct (independence by construction).
    let refs: std::collections::HashSet<_> = candidates.iter().map(|c| &c.candidate_ref).collect();
    assert_eq!(
        refs.len(),
        n,
        "all N candidate refs must be distinct (no shared mutation)"
    );

    // Each candidate knows its own index.
    for (expected_idx, c) in candidates.iter().enumerate() {
        assert_eq!(
            c.index, expected_idx,
            "candidate {expected_idx} must carry its own index"
        );
    }

    // No shared mutation: marking one selected does not affect siblings.
    let first_selected = candidates[0].clone().mark_selected();
    assert!(first_selected.selected, "marked candidate is selected");
    for other in candidates.iter().skip(1) {
        assert!(
            !other.selected,
            "marking one candidate must not affect siblings"
        );
    }

    // Fan-out event is recorded.
    let genesis_hash = "0".repeat(64);
    let event = fan_out_event(&candidates, 1, &genesis_hash);
    assert_eq!(event.kind, "tournament.fan_out");
    assert!(
        event.payload.contains(&format!("\"n\":{n}")),
        "fan-out event payload must record N"
    );
}

// ─── item ②: judge panel selects per documented criteria; known-best fixture ──

/// ② The judge panel selects the known-best candidate per WRITTEN criteria
/// (evidence completeness → approval count → index tiebreak). In the fixture
/// the known-best candidate carries the most evidence refs and is planted at
/// index 1 (not index 0) to prove the criteria drive selection, not position.
#[test]
fn item_2_panel_selects_known_best() {
    let intent = test_intent("intent-d13-item2");
    let strategies = ["strat-a", "strat-b", "strat-c"];

    // Build candidates and attach evidence refs.
    let mut candidates = produce_candidates(&intent, &strategies);

    // Plant the known-best at index 1 (most evidence refs — criterion ①).
    // Index 0 and 2 have fewer refs.
    candidates[0] = candidates[0].clone().with_evidence(vec!["ev-a0".into()]);
    candidates[1] =
        candidates[1]
            .clone()
            .with_evidence(vec!["ev-b0".into(), "ev-b1".into(), "ev-b2".into()]); // known-best
    candidates[2] = candidates[2]
        .clone()
        .with_evidence(vec!["ev-c0".into(), "ev-c1".into()]);

    let known_best_ref = candidates[1].candidate_ref.clone();

    // Ground truths: each candidate's served evidence matches its evidence_refs.
    let gts: Vec<ServedGroundTruth> = candidates
        .iter()
        .map(|c| ground_truth(&c.intent_id, c.evidence_refs.clone()))
        .collect();

    let panel = diverse_panel();
    let result = select(candidates, &panel, &gts).expect("selection must succeed");

    // The known-best is selected.
    assert_eq!(
        result.winner.candidate_ref, known_best_ref,
        "judge panel must select the known-best candidate (most evidence refs)"
    );
    assert!(result.winner.selected, "winner must be marked as selected");

    // The winner score reflects criterion ①: highest evidence count.
    assert_eq!(
        result.winner_score.evidence_count, 3,
        "winner evidence count must be 3 (criterion ①)"
    );

    // Two losers remain.
    assert_eq!(result.losers.len(), 2, "two losers must remain");

    // Selection event is emitted.
    let loser_refs: Vec<String> = result
        .losers
        .iter()
        .map(|c| c.candidate_ref.clone())
        .collect();
    let genesis_hash = "0".repeat(64);
    let event = selection_event(&result.winner, &loser_refs, 2, &genesis_hash);
    assert_eq!(event.kind, "tournament.selection");
    assert!(
        event.payload.contains(&known_best_ref),
        "selection event must name the winner ref"
    );
}

// ─── item ③: losers remain addressable as evidence objects (not discarded) ────

/// ③ After selection, all losing candidates are retained as addressable evidence:
/// they remain resolvable by their `candidate_ref` and are not discarded.
#[test]
fn item_3_losers_addressable_as_evidence() {
    let intent = test_intent("intent-d13-item3");
    let strategies = ["strat-x", "strat-y", "strat-z", "strat-w"];
    let mut candidates = produce_candidates(&intent, &strategies);

    // Known-best: index 0, most evidence refs.
    candidates[0] =
        candidates[0]
            .clone()
            .with_evidence(vec!["ev-0a".into(), "ev-0b".into(), "ev-0c".into()]);
    candidates[1] = candidates[1].clone().with_evidence(vec!["ev-1a".into()]);
    candidates[2] = candidates[2].clone().with_evidence(vec!["ev-2a".into()]);
    candidates[3] = candidates[3].clone().with_evidence(vec!["ev-3a".into()]);

    // Record loser refs BEFORE selection.
    let expected_loser_refs: Vec<String> = candidates[1..]
        .iter()
        .map(|c| c.candidate_ref.clone())
        .collect();

    let gts: Vec<ServedGroundTruth> = candidates
        .iter()
        .map(|c| ground_truth(&c.intent_id, c.evidence_refs.clone()))
        .collect();

    let panel = diverse_panel();
    let result = select(candidates, &panel, &gts).expect("selection must succeed");

    // All 3 losers are retained.
    assert_eq!(
        result.losers.len(),
        3,
        "all losing candidates must be retained"
    );

    // Each loser is resolvable by candidate_ref (addressable as evidence).
    for expected_ref in &expected_loser_refs {
        let resolved = resolve_loser(&result, expected_ref);
        assert!(
            resolved.is_some(),
            "loser with ref '{expected_ref}' must be resolvable after selection"
        );
        let loser = resolved.unwrap();
        assert_eq!(
            &loser.candidate_ref, expected_ref,
            "resolved loser must carry its original ref"
        );
        // The loser's evidence_refs are preserved (retained as evidence objects).
        assert!(
            !loser.evidence_refs.is_empty(),
            "loser evidence refs must be preserved"
        );
    }

    // Losers are NOT marked as selected.
    for loser in &result.losers {
        assert!(!loser.selected, "losers must not be marked as selected");
    }

    // Resolve by a non-existent ref → None (no phantom evidence).
    let phantom = resolve_loser(&result, "not-a-real-ref");
    assert!(
        phantom.is_none(),
        "resolve_loser must return None for unknown refs"
    );
}

// ─── item ④: budget-bounded fan-out: policy cap + zero overage ───────────────

/// ④ N is POLICY-CAPPED. An N-way tournament respects per-tenant caps + C7
/// fairness and generates ZERO overage under a flat plan.
///
/// Fixture:
/// - Cap-overrun attempt (N > MAX_N_POLICY) → refused, zero units consumed.
/// - Budget-overrun attempt (N > remaining) → refused, zero units consumed.
/// - Valid fan-out (N within cap AND budget) → approved, budget decremented.
/// - C7 fairness constants are within the stated contract bounds.
#[test]
fn item_4_budget_bounded_zero_overage() {
    const TENANT: &str = "tenant-d13";

    // ── Sub-fixture A: policy cap overrun ──────────────────────────────────────
    {
        let cap_overrun_n = MAX_N_POLICY + 1;
        let mut mgr = budget_manager_with(TENANT, 1000); // ample budget
        let mut events = vec![];

        let outcome = attempt_fan_out(&mut mgr, TENANT, cap_overrun_n, 0, &mut events);
        assert!(
            outcome.is_cap_refused(),
            "N={cap_overrun_n} > policy cap={MAX_N_POLICY} must be refused"
        );

        // Zero units consumed (zero overage).
        assert_eq!(
            mgr.budget(TENANT).unwrap().remaining,
            1000,
            "cap-overrun attempt must consume ZERO units (zero overage)"
        );
        assert!(
            events.is_empty(),
            "no budget events must be emitted on a cap refusal"
        );

        // Inspect the refused outcome.
        if let FanOutOutcome::NExceedsCap { requested, cap } = outcome {
            assert_eq!(requested, cap_overrun_n);
            assert_eq!(cap, MAX_N_POLICY);
        } else {
            panic!("expected NExceedsCap outcome");
        }
    }

    // ── Sub-fixture B: budget overrun ──────────────────────────────────────────
    {
        let n = 5;
        let budget = 3; // 3 < 5*UNITS_PER_CANDIDATE
        let mut mgr = budget_manager_with(TENANT, budget);
        let mut events = vec![];

        let outcome = attempt_fan_out(&mut mgr, TENANT, n, 0, &mut events);
        assert!(
            outcome.is_budget_refused(),
            "N={n} requesting {units} units > remaining={budget} must be refused",
            units = n as u64 * UNITS_PER_CANDIDATE,
        );

        // Zero units consumed (zero overage).
        assert_eq!(
            mgr.budget(TENANT).unwrap().remaining,
            budget,
            "budget-overrun attempt must consume ZERO units (zero overage)"
        );
    }

    // ── Sub-fixture C: valid fan-out within cap and budget ─────────────────────
    {
        let n = 4;
        let initial_budget = 10;
        let mut mgr = budget_manager_with(TENANT, initial_budget);
        let mut events = vec![];

        let outcome = attempt_fan_out(&mut mgr, TENANT, n, 0, &mut events);
        assert!(
            outcome.is_approved(),
            "N={n} within cap={MAX_N_POLICY} and budget={initial_budget} must be approved"
        );

        let expected_consumed = n as u64 * UNITS_PER_CANDIDATE;
        let expected_remaining = initial_budget - expected_consumed;
        assert_eq!(
            mgr.budget(TENANT).unwrap().remaining,
            expected_remaining,
            "budget must decrease by exactly N*UNITS_PER_CANDIDATE"
        );

        if let FanOutOutcome::Approved {
            n: approved_n,
            remaining_after,
        } = outcome
        {
            assert_eq!(approved_n, n);
            assert_eq!(remaining_after, expected_remaining);
        } else {
            panic!("expected Approved outcome");
        }
    }

    // ── Sub-fixture D: C7 fairness constants are within contract bounds ────────
    // Confirms that D13's consumed C7 surface hasn't drifted.
    assert_fairness_constants();

    // ── Sub-fixture E: N at exact policy cap is approved ──────────────────────
    {
        let n = MAX_N_POLICY;
        let mut mgr = budget_manager_with(TENANT, 1000);
        let mut events = vec![];
        let outcome = attempt_fan_out(&mut mgr, TENANT, n, 0, &mut events);
        assert!(
            outcome.is_approved(),
            "N=MAX_N_POLICY={MAX_N_POLICY} must be approved (boundary)"
        );
    }
}
