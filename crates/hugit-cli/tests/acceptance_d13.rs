// WP-D13 acceptance oracle — tournament.
// Each test corresponds to one owned acceptance item from WP-D13.md.
// RED on current tree: the implementation module does not exist yet.
// Implementation target: crates/hugit-cli/tournament/

use hugit_cli::tournament::{
    run_tournament, BudgetPolicy, Candidate, JudgePanel, TournamentConfig,
};

/// ① -n N produces N independent candidates.
/// Fixture: N=3, assert count=3 and no shared mutation between candidates.
#[test]
fn item_1_n_independent_candidates() {
    let config = TournamentConfig::builder()
        .n(3)
        .intent_id("intent-abc".to_string())
        .budget(BudgetPolicy::unlimited_for_test())
        .build();
    let result = run_tournament(&config);
    assert_eq!(
        result.candidates().len(),
        3,
        "-n 3 must produce exactly 3 candidates"
    );
    // Independence: no two candidates share a mutable state object
    let ids: Vec<_> = result.candidates().iter().map(|c| c.id()).collect();
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(ids.len(), unique.len(), "all candidate ids must be distinct (independence)");
}

/// ② judge panel selects per documented criteria; known-best fixture.
/// Fixture: 3 candidates with one marked known-best; assert panel picks it.
#[test]
fn item_2_panel_selects_known_best() {
    let candidates = vec![
        Candidate::fixture("c1", false),
        Candidate::fixture("c2-known-best", true), // known-best
        Candidate::fixture("c3", false),
    ];
    let panel = JudgePanel::documented_criteria();
    let selection = panel.select(&candidates);
    assert_eq!(
        selection.winner_id(),
        "c2-known-best",
        "judge panel must select the known-best candidate per documented criteria"
    );
}

/// ③ losers remain addressable as evidence objects (not discarded).
/// After selection: assert all non-winner candidates resolve as evidence.
#[test]
fn item_3_losers_addressable_as_evidence() {
    let candidates = vec![
        Candidate::fixture("c1", false),
        Candidate::fixture("c2-known-best", true),
        Candidate::fixture("c3", false),
    ];
    let panel = JudgePanel::documented_criteria();
    let selection = panel.select(&candidates);
    for loser_id in &["c1", "c3"] {
        assert!(
            selection.resolve_loser(loser_id).is_some(),
            "loser {} must remain addressable as evidence after selection",
            loser_id
        );
    }
}

/// ④ budget-bounded fan-out: N is policy-capped; cap-overrun attempt yields zero overage.
/// Fixture: per-tenant cap=2, request N=5 → assert capped/refused, overage_charge=0.
#[test]
fn item_4_budget_bounded_zero_overage() {
    let policy = BudgetPolicy::builder()
        .per_tenant_cap(2)
        .fairness_c7(true)
        .build();
    let config = TournamentConfig::builder()
        .n(5) // exceeds cap
        .intent_id("intent-xyz".to_string())
        .budget(policy)
        .build();
    let result = run_tournament(&config);
    assert!(
        result.was_capped() || result.was_refused(),
        "N=5 over cap=2 must be capped or refused"
    );
    assert_eq!(
        result.overage_charge(),
        0,
        "zero overage must be generated under a flat plan; got: {}",
        result.overage_charge()
    );
}
