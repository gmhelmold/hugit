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
use hugit_queue::core::batch::{Batch, BatchError};
use hugit_queue::core::order::{land_in_order, landed_in_order};
use hugit_queue::core::state::{EntryState, TransitionError, UnionOutcome, transition};
use hugit_queue::core::union::{
    CheckSource, FailureLocus, MemoCheck, UnionVerdict, disjoint_lanes, evaluate_union,
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

/// Memo oracle where one named item is INDIVIDUALLY red (broken on its own).
/// Any union containing it is red; no pair interaction is involved.
struct AllHitsItemFails {
    bad: &'static str,
}
impl MemoCheck for AllHitsItemFails {
    fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
        let red = item_ids.contains(&self.bad);
        let verdict = if red {
            UnionVerdict::Red
        } else {
            UnionVerdict::Green
        };
        (verdict, item_ids.iter().map(|_| CheckSource::Hit).collect())
    }
}

/// Memo oracle red only when ALL of a,b,c are present (a ≥3-way interaction):
/// no single item and no pair is red, so bisection cannot localise it.
struct AllHitsTripleFails;
impl MemoCheck for AllHitsTripleFails {
    fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
        let all3 = ["a", "b", "c"].iter().all(|x| item_ids.contains(x));
        let verdict = if all3 {
            UnionVerdict::Red
        } else {
            UnionVerdict::Green
        };
        (verdict, item_ids.iter().map(|_| CheckSource::Hit).collect())
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

/// THE WEDGE PROPERTY, END-TO-END (defect 1): "exclude the failing pair, the
/// rest proceeds" must actually LAND the innocent entries — not merely compute
/// a `proceeding` list that the landing layer then blocks.
///
/// Batch A,B,C,D at positions 0,1,2,3. The failing pair is (A,B) at the FRONT;
/// C,D are innocent and sit BEHIND the excluded pair. After exclusion, C and D
/// MUST end in `Landed`. On `main` before remediation, the UnionFail of A (a
/// predecessor) closed the ordering gate and C,D were refused with
/// PredecessorNotLanded forever — proving the wedge had no end-to-end impl.
#[test]
fn item_1_excluded_pair_lets_innocent_successors_land_end_to_end() {
    let mut batch = Batch::from_entries(
        "batch-1-e2e",
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
    assert_eq!(ev.verdict, UnionVerdict::Red);
    assert_eq!(
        ev.failure,
        Some(FailureLocus::Pair(hugit_contracts::MinimalFailingPair {
            item_a: "A".to_string(),
            item_b: "B".to_string(),
        }))
    );

    // Bridge the evaluation into per-entry landing outcomes and DRIVE the land.
    let owned_ids: Vec<String> = batch
        .entries()
        .iter()
        .map(|e| e.item_id().to_string())
        .collect();
    let ids: Vec<&str> = owned_ids.iter().map(String::as_str).collect();
    let outcomes = ev.outcomes_for_landing(&ids);
    let steps =
        land_in_order(&mut batch, &outcomes).expect("the rest proceeds despite the excluded pair");

    // A,B excluded; C,D LAND — even though they sit behind the failing pair.
    let by_id = |id: &str| steps.iter().find(|s| s.item_id == id).unwrap().state;
    assert_eq!(by_id("A"), EntryState::UnionFail, "A excluded");
    assert_eq!(by_id("B"), EntryState::UnionFail, "B excluded");
    assert_eq!(by_id("C"), EntryState::Landed, "C lands (innocent)");
    assert_eq!(by_id("D"), EntryState::Landed, "D lands (innocent)");
    assert_eq!(
        landed_in_order(&batch),
        vec!["C", "D"],
        "exclude the failing pair, the rest LANDS — end to end"
    );
}

/// Defect 3 (minimal pair): a batch where item0 is INDIVIDUALLY red must NOT
/// name an innocent neighbour as a pair member. The failure is a single item.
#[test]
fn item_1_single_item_failure_is_not_a_false_pair() {
    let batch = Batch::from_entries(
        "batch-1-single",
        [
            (landable("A", 0), AffectedSet::new(["a"])),
            (landable("B", 1), AffectedSet::new(["b"])),
            (landable("C", 2), AffectedSet::new(["c"])),
        ],
    );
    // A is individually red (broken on its own), not a pair interaction.
    let mut oracle = AllHitsItemFails { bad: "A" };
    let ev = evaluate_union(&batch, &mut oracle);
    assert_eq!(ev.verdict, UnionVerdict::Red);
    assert!(
        ev.minimal_failing_pair.is_none(),
        "an individually-red item is never disguised as a pair"
    );
    assert_eq!(
        ev.failure,
        Some(FailureLocus::SingleItem("A".to_string())),
        "the locus is the single failing item, named explicitly"
    );
    assert_eq!(ev.proceeding, vec!["B".to_string(), "C".to_string()]);
}

/// Defect 4 (silent drop): a red union that bisection cannot localise to a
/// single item OR a pair must surface an EXPLICIT `Unlocalised`, not a silent
/// empty `proceeding` masquerading as success.
#[test]
fn item_1_unlocalised_red_union_is_explicit_not_silent_empty() {
    let batch = Batch::from_entries(
        "batch-1-triple",
        [
            (landable("a", 0), AffectedSet::new(["x"])),
            (landable("b", 1), AffectedSet::new(["y"])),
            (landable("c", 2), AffectedSet::new(["z"])),
        ],
    );
    // Red only when all three are present — no single item, no pair is red.
    let mut oracle = AllHitsTripleFails;
    let ev = evaluate_union(&batch, &mut oracle);
    assert_eq!(ev.verdict, UnionVerdict::Red);
    assert_eq!(
        ev.failure,
        Some(FailureLocus::Unlocalised),
        "a ≥3-way interaction is reported explicitly, never silently dropped"
    );
    assert!(
        ev.proceeding.is_empty(),
        "an unlocalised red union holds the whole batch — never a partial land"
    );
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
    // Two distinct facts make up ⑤ post-remediation:
    //
    // (a) An EXCLUDED predecessor is TRANSPARENT — q must still land. A
    //     UnionFail p is settled and never lands, so it must not block an
    //     innocent later green (the wedge property). On `main` this REFUSED q
    //     with PredecessorNotLanded — the bug this oracle catches.
    let batch = Batch::from_entries(
        "batch-5b",
        [
            (landable("p", 0), AffectedSet::new(["p"])),
            (landable("q", 1), AffectedSet::new(["q"])),
        ],
    );
    let mut batch = batch;
    let steps = land_in_order(
        &mut batch,
        &[
            ("p", UnionOutcome::FailingPairMember),
            ("q", UnionOutcome::Green),
        ],
    )
    .expect("an excluded predecessor is transparent; q lands");
    assert_eq!(steps[0].state, EntryState::UnionFail, "p excluded");
    assert_eq!(steps[1].state, EntryState::Landed, "q lands transparently");
    assert_eq!(landed_in_order(&batch), vec!["q"]);

    // (b) A genuinely UNRESOLVED predecessor (still Landable) MUST block: the
    //     state machine has no edge that lands a green while a predecessor is
    //     unsettled. Out-of-order landing remains unrepresentable.
    assert_eq!(
        transition(EntryState::Landable, UnionOutcome::Green, false),
        Err(TransitionError::PredecessorNotLanded),
        "a green cannot land while a predecessor is still unresolved"
    );
}

#[test]
fn item_5_ordered_landing_is_idempotent_under_replay() {
    // Re-driving a fully-landed batch is a no-op: terminal entries are skipped
    // (not re-transitioned, not errored), their state re-reported. This is the
    // idempotency the crash-replay (④) rides on — replaying the SAME batch must
    // not hard-error.
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

    // Replay on the SAME batch: every entry terminal → idempotent no-op, Ok.
    let replay = land_in_order(
        &mut batch,
        &[("a", UnionOutcome::Green), ("b", UnionOutcome::Green)],
    )
    .expect("replay on a terminal batch is an idempotent no-op, not an error");
    assert!(replay.iter().all(|s| s.state == EntryState::Landed));
    assert_eq!(landed_in_order(&batch), first, "replay does not re-land");
}

/// Defect 5 (queue-order integrity): two entries at the SAME `order_index` make
/// queue order ambiguous — a sort would pick an arbitrary winner and silently
/// undermine ⑤. The batch builder must REJECT the duplicate, not tolerate it.
#[test]
fn item_5_duplicate_order_index_is_rejected() {
    let res = Batch::try_from_entries(
        "batch-5d",
        [
            (landable("a", 0), AffectedSet::new(["a"])),
            (landable("b", 1), AffectedSet::new(["b"])),
            // Collides with "b" at position 1 — ambiguous queue order.
            (landable("c", 1), AffectedSet::new(["c"])),
        ],
    );
    assert_eq!(
        res.unwrap_err(),
        BatchError::DuplicateOrderIndex(1),
        "a duplicate order_index is rejected, not silently sorted"
    );
}
