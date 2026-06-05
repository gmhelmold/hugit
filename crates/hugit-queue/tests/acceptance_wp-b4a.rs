//! WP-B4a acceptance oracle — items ①②⑤ of B4 (the pure union-queue engine).
//!
//! Contract: docs/plan/wp-contracts/WP-B4a.md.
//! Owned items:
//!   ① A+B-red pair excluded + NAMED (minimal_failing_pair).
//!   ② 5 disjoint greens land in parallel lanes with 0 check re-runs.
//!   ⑤ lands in queue order; out-of-order landing structurally prevented.
//!
//! These tests are author-side; the standalone bash suite
//! (tests/acceptance/wp-b4a/run.sh) drives `cargo test --test acceptance_wp-b4a`
//! and adds the structural/boundary assertions.

use hugit_contracts::LandableEntry;
use hugit_queue::core::affected::AffectedSet;
use hugit_queue::core::batch::Batch;
use hugit_queue::core::order::{LandingError, land_in_order, landed_in_order};
use hugit_queue::core::state::{TransitionError, UnionOutcome};
use hugit_queue::core::union::{
    CheckSource, MemoCheck, UnionVerdict, disjoint_lanes, evaluate_union,
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn landable(id: &str, order: u64) -> LandableEntry {
    LandableEntry {
        item_id: id.to_string(),
        intent_id: format!("intent-{id}"),
        tree_hash: format!("tree-{id}"),
        order_index: order,
    }
}

/// Memo oracle whose union is red exactly when both `bad_a` and `bad_b` are
/// present together. Every evaluation is served from the AC (a hit) — so the
/// executed count is always zero, modelling the "all memoised" scenario.
struct AllHitsPairFails {
    bad_a: &'static str,
    bad_b: &'static str,
}

impl MemoCheck for AllHitsPairFails {
    fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
        let red = item_ids.contains(&self.bad_a) && item_ids.contains(&self.bad_b);
        let verdict = if red {
            UnionVerdict::Red
        } else {
            UnionVerdict::Green
        };
        let sources = item_ids.iter().map(|_| CheckSource::Hit).collect();
        (verdict, sources)
    }
}

/// Memo oracle that is always green and always a cache hit (0 executions).
struct AllHitsGreen;
impl MemoCheck for AllHitsGreen {
    fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
        (
            UnionVerdict::Green,
            item_ids.iter().map(|_| CheckSource::Hit).collect(),
        )
    }
}

// ── ① minimal failing pair: excluded + named ─────────────────────────────────

#[test]
fn item_1_minimal_failing_pair_excluded_and_named() {
    // Batch A,B,C,D where the union is red iff A and B are both present.
    let batch = Batch::from_entries(
        "batch-1",
        [
            (landable("A", 0), AffectedSet::new(["a"])),
            (landable("B", 1), AffectedSet::new(["b"])),
            (landable("C", 2), AffectedSet::new(["c"])),
            (landable("D", 3), AffectedSet::new(["d"])),
        ],
    );
    let mut oracle = AllHitsPairFails {
        bad_a: "A",
        bad_b: "B",
    };
    let ev = evaluate_union(&batch, &mut oracle);

    // Red union → a minimal failing pair is isolated and BOTH members named.
    assert_eq!(ev.verdict, UnionVerdict::Red);
    let pair = ev
        .minimal_failing_pair
        .expect("minimal_failing_pair must be named on a red union");
    assert_eq!(pair.item_a, "A", "first member named");
    assert_eq!(pair.item_b, "B", "second member named");

    // The pair is EXCLUDED from the batch; the rest proceeds.
    assert_eq!(
        ev.proceeding,
        vec!["C".to_string(), "D".to_string()],
        "the A+B pair is excluded; C and D proceed"
    );
    // Bisection is over memoised checks (≈ free): zero executions.
    assert_eq!(ev.executed_count, 0, "bisection is memoised — 0 re-runs");
}

// ── ② 5 disjoint greens land, 0 re-runs ──────────────────────────────────────

#[test]
fn item_2_five_disjoint_greens_land_zero_reruns() {
    // Five changes with pairwise-disjoint affected-sets → one parallel lane.
    let batch = Batch::from_entries(
        "batch-2",
        [
            (landable("g1", 0), AffectedSet::new(["src/p1.rs"])),
            (landable("g2", 1), AffectedSet::new(["src/p2.rs"])),
            (landable("g3", 2), AffectedSet::new(["src/p3.rs"])),
            (landable("g4", 3), AffectedSet::new(["src/p4.rs"])),
            (landable("g5", 4), AffectedSet::new(["src/p5.rs"])),
        ],
    );

    // Disjointness: all five are mutually disjoint → a single lane carries all.
    let lanes = disjoint_lanes(&batch);
    assert_eq!(lanes.len(), 1, "all five disjoint → one parallel lane");
    assert_eq!(
        lanes[0],
        vec![
            "g1".to_string(),
            "g2".to_string(),
            "g3".to_string(),
            "g4".to_string(),
            "g5".to_string(),
        ]
    );

    // The union is green and every check is an AC hit → ZERO re-runs.
    let mut oracle = AllHitsGreen;
    let ev = evaluate_union(&batch, &mut oracle);
    assert_eq!(ev.verdict, UnionVerdict::Green);
    assert_eq!(ev.executed_count, 0, "all greens are AC hits — 0 re-runs");

    // All five land.
    let mut batch = batch;
    let steps = land_in_order(
        &mut batch,
        &[
            ("g1", UnionOutcome::Green),
            ("g2", UnionOutcome::Green),
            ("g3", UnionOutcome::Green),
            ("g4", UnionOutcome::Green),
            ("g5", UnionOutcome::Green),
        ],
    )
    .expect("all disjoint greens land");
    assert_eq!(steps.len(), 5);
    assert_eq!(
        landed_in_order(&batch),
        vec!["g1", "g2", "g3", "g4", "g5"],
        "all five disjoint greens land"
    );
}

#[test]
fn item_2_overlapping_changes_split_into_separate_lanes() {
    // Two changes that touch the same key are NOT disjoint → separate lanes.
    let batch = Batch::from_entries(
        "batch-2b",
        [
            (landable("x", 0), AffectedSet::new(["shared", "x_only"])),
            (landable("y", 1), AffectedSet::new(["shared", "y_only"])),
            (landable("z", 2), AffectedSet::new(["z_only"])),
        ],
    );
    let lanes = disjoint_lanes(&batch);
    // x and y overlap on "shared" → different lanes; z disjoint → joins lane 0.
    assert_eq!(lanes.len(), 2);
    assert_eq!(lanes[0], vec!["x".to_string(), "z".to_string()]);
    assert_eq!(lanes[1], vec!["y".to_string()]);
}

// ── ⑤ lands in queue order; out-of-order structurally prevented ──────────────

#[test]
fn item_5_lands_in_queue_order() {
    // Provide entries out of queue order; the batch sorts to queue order and
    // landing follows that order.
    let batch = Batch::from_entries(
        "batch-5",
        [
            (landable("third", 2), AffectedSet::new(["3"])),
            (landable("first", 0), AffectedSet::new(["1"])),
            (landable("second", 1), AffectedSet::new(["2"])),
        ],
    );
    assert!(
        batch.is_queue_ordered(),
        "batch is held in strict queue (order_index) order"
    );

    let mut batch = batch;
    let steps = land_in_order(
        &mut batch,
        &[
            ("first", UnionOutcome::Green),
            ("second", UnionOutcome::Green),
            ("third", UnionOutcome::Green),
        ],
    )
    .expect("greens land in order");
    let order: Vec<&str> = steps.iter().map(|s| s.item_id.as_str()).collect();
    assert_eq!(
        order,
        vec!["first", "second", "third"],
        "landing applies in queue order regardless of input order"
    );
}

#[test]
fn item_5_out_of_order_landing_structurally_prevented() {
    // The first entry fails (excluded). A later green entry must NOT land
    // ahead of the gap — the state machine has no edge that lands it, so the
    // driver surfaces a refusal rather than an out-of-order landing.
    let batch = Batch::from_entries(
        "batch-5b",
        [
            (landable("p", 0), AffectedSet::new(["p"])),
            (landable("q", 1), AffectedSet::new(["q"])),
        ],
    );
    let mut batch = batch;
    let err = land_in_order(
        &mut batch,
        &[
            ("p", UnionOutcome::FailingPairMember),
            ("q", UnionOutcome::Green),
        ],
    )
    .expect_err("q cannot land ahead of failed predecessor p");
    assert_eq!(
        err,
        LandingError::Refused("q".to_string(), TransitionError::PredecessorNotLanded),
        "out-of-order landing is structurally prevented, not silently allowed"
    );
    // q did NOT land.
    assert!(
        landed_in_order(&batch).is_empty(),
        "no entry landed when the predecessor failed"
    );
}

#[test]
fn item_5_ordered_landing_is_idempotent_under_replay() {
    // Pure transitions are idempotent: re-driving a fully-landed batch never
    // re-lands or duplicates (terminal states do not transition).
    let batch = Batch::from_entries(
        "batch-5c",
        [
            (landable("a", 0), AffectedSet::new(["a"])),
            (landable("b", 1), AffectedSet::new(["b"])),
        ],
    );
    let mut batch = batch;
    land_in_order(
        &mut batch,
        &[("a", UnionOutcome::Green), ("b", UnionOutcome::Green)],
    )
    .unwrap();
    let first = landed_in_order(&batch);

    // Replay: every entry already terminal → refused, no state change.
    let replay = land_in_order(
        &mut batch,
        &[("a", UnionOutcome::Green), ("b", UnionOutcome::Green)],
    );
    assert!(matches!(
        replay,
        Err(LandingError::Refused(_, TransitionError::AlreadyTerminal))
    ));
    assert_eq!(landed_in_order(&batch), first, "replay does not re-land");
}
