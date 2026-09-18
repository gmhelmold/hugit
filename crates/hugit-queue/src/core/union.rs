//! Union fold + memoised check evaluation + minimal-failing-pair bisection.
//!
//! Whitepaper §6.4: `U = fold(regen-rebase, head, batch)`, then run the
//! affected memoised checks on `U`. Most checks are AC hits — only novelty
//! executes. On a red union, bisect the batch over the (≈ free) memoised
//! checks to a failing locus, then RE-EVALUATE the exact remainder. Diagnosis
//! never authorizes an untested set. A red remainder is held for a later attempt.
//!
//! B4a is engine-pure: the actual rebase and check execution live behind the
//! `MemoCheck` trait so the engine is testable in isolation (no GitHub API,
//! no runner). B4b/B2a supply the real implementations.

use crate::core::batch::Batch;
use crate::core::state::UnionOutcome;
use hugit_contracts::MinimalFailingPair;

/// The verdict of evaluating a candidate union of changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnionVerdict {
    /// All affected memoised checks passed on the union tree.
    Green,
    /// At least one affected memoised check failed on the union tree.
    Red,
}

/// Whether a memoised-check evaluation was served from the content-addressed
/// cache (a hit) or actually executed (a miss / novelty).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckSource {
    /// Result served from the AC — zero compute (contract ②: "0 re-runs").
    Hit,
    /// Result computed by executing the check (novelty only).
    Executed,
}

/// Pure interface to the memoised check oracle for a *set* of changes.
///
/// The engine asks "does the union of these item-ids pass its affected
/// checks?" and the oracle answers with a verdict plus how each evaluation was
/// served. B4a counts executions to prove the 0-re-run property (②); the real
/// oracle (B2a client + B3 affected-set + CAS) is wired in B4b.
pub trait MemoCheck {
    /// Evaluate the affected memoised checks for the union of `item_ids`.
    /// Returns the verdict and the source (hit/executed) of each underlying
    /// check evaluation, so callers can assert zero re-execution.
    fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>);
}

/// Where a red union's failure was localised by bisection.
///
/// A red union is never silently dropped: bisection always resolves to an
/// explicit locus. This is the discriminant the landing layer keys off to
/// decide *which* entries to exclude (the rest proceed).
///
/// `Eq` is not derived because the frozen contract type `MinimalFailingPair`
/// (owned by `hugit-contracts`) is `PartialEq` only; `String`/this enum are
/// reflexive in practice, so `PartialEq` is sufficient for the engine.
#[derive(Debug, Clone, PartialEq)]
pub enum FailureLocus {
    /// A genuine minimal failing pair: the 2-element union `(a, b)` is red, and
    /// EACH member is individually GREEN (so neither is a single-item failure).
    /// Both members are named and excluded; the remainder needs its own green.
    Pair(MinimalFailingPair),
    /// A single item is individually red — the failure is that one item, not a
    /// pair. Excluding it requires revalidation of the remainder. Naming a second,
    /// innocent item as a "pair member" would be a false accusation, so the
    /// single-item case is reported explicitly and never disguised as a pair.
    SingleItem(String),
    /// The union is red but bisection could not isolate it to any single item
    /// or any 2-element pair (e.g. a ≥3-way interaction). Surfaced explicitly
    /// so the caller can fall back (e.g. exclude nothing and hold the batch)
    /// rather than silently dropping every entry.
    Unlocalised,
}

/// Outcome of folding + evaluating a batch's union.
#[derive(Debug, Clone)]
pub struct UnionEvaluation {
    /// Overall verdict of the union.
    pub verdict: UnionVerdict,
    /// The minimal failing pair, present iff the union is red and a genuine
    /// pair was isolated by bisection (each member individually green). Both
    /// members are NAMED (contract ①). `None` for a single-item or unlocalised
    /// failure — see [`UnionEvaluation::failure`].
    pub minimal_failing_pair: Option<MinimalFailingPair>,
    /// Explicit locus of a red union's failure (`None` on a green union). A red
    /// union ALWAYS carries a locus — never a silently-empty result.
    pub failure: Option<FailureLocus>,
    /// The item-ids that may proceed after excluding the failure locus.
    pub proceeding: Vec<String>,
    /// Total number of check evaluations that actually executed (novelty).
    /// Zero when every check was an AC hit (contract ②).
    pub executed_count: usize,
    /// Members kept in the queue because no green evaluation authorizes them.
    /// A held member is not accused of belonging to the diagnosed locus.
    pub held: Vec<String>,
    /// Result of the exact remainder probe, absent if there was no remainder.
    pub remainder_verdict: Option<UnionVerdict>,
    // Bind the landing bridge to the evaluated input and result, not to a
    // caller-modified public reporting field or a newly supplied ID set.
    evaluated_members: Vec<String>,
    validated_members: Vec<String>,
}

impl UnionEvaluation {
    /// Exact members authorized by an observed green evaluation.
    /// This is distinct from diagnosing a failing locus and from the public
    /// `proceeding` reporting field. No subset is assumed green by monotonicity.
    pub fn validated_proceeding(&self) -> &[String] {
        &self.validated_members
    }

    /// Map the original ordered batch to authorized landing outcomes.
    /// Held entries deliberately receive NO transition, rather than a fabricated
    /// green or a false failing-pair accusation. A changed input batch receives
    /// no authorization; its exact union must be evaluated first.
    pub fn outcomes_for_landing<'a>(&self, ids: &[&'a str]) -> Vec<(&'a str, UnionOutcome)> {
        if !ids
            .iter()
            .copied()
            .eq(self.evaluated_members.iter().map(String::as_str))
        {
            return Vec::new();
        }
        let excluded: std::collections::BTreeSet<&str> = match &self.failure {
            Some(FailureLocus::Pair(p)) => {
                [p.item_a.as_str(), p.item_b.as_str()].into_iter().collect()
            }
            Some(FailureLocus::SingleItem(id)) => [id.as_str()].into_iter().collect(),
            None | Some(FailureLocus::Unlocalised) => std::collections::BTreeSet::new(),
        };
        let validated: std::collections::BTreeSet<&str> =
            self.validated_members.iter().map(String::as_str).collect();
        ids.iter()
            .filter_map(|id| {
                if validated.contains(id) {
                    Some((*id, UnionOutcome::Green))
                } else if excluded.contains(id) {
                    Some((*id, UnionOutcome::FailingPairMember))
                } else {
                    None
                }
            })
            .collect()
    }
}

/// Fold the batch into its union and evaluate it; on red, diagnose a
/// failing locus, exclude it, and evaluate the entire remaining candidate.
/// A red or unlocalised remainder is held, never automatically authorized.
///
/// The fold is `U = fold(regen-rebase, head, batch)` — represented here as the
/// ordered list of item-ids handed to the `MemoCheck` oracle, which owns the
/// rebase/exec. The engine's job is the *control flow*: evaluate, and on red,
/// bisect.
pub fn evaluate_union<M: MemoCheck>(batch: &Batch, oracle: &mut M) -> UnionEvaluation {
    let ids: Vec<&str> = batch.entries().iter().map(|e| e.item_id()).collect();
    let evaluated_members: Vec<String> = ids.iter().map(|s| (*s).to_string()).collect();
    let (verdict, sources) = oracle.evaluate(&ids);
    let executed_count = sources
        .iter()
        .filter(|s| **s == CheckSource::Executed)
        .count();

    match verdict {
        UnionVerdict::Green => UnionEvaluation {
            verdict,
            minimal_failing_pair: None,
            failure: None,
            proceeding: evaluated_members.clone(),
            validated_members: evaluated_members.clone(),
            evaluated_members,
            held: Vec::new(),
            remainder_verdict: None,
            executed_count,
        },
        UnionVerdict::Red => {
            let (locus, extra_exec) = bisect_failure(&ids, oracle);
            let excluded: Vec<&str> = match &locus {
                FailureLocus::Pair(p) => vec![p.item_a.as_str(), p.item_b.as_str()],
                FailureLocus::SingleItem(id) => vec![id.as_str()],
                FailureLocus::Unlocalised => Vec::new(),
            };
            let remainder: Vec<&str> = ids
                .iter()
                .copied()
                .filter(|id| !excluded.contains(id))
                .collect();
            let mut proceeding = Vec::new();
            let mut held = Vec::new();
            let mut remainder_verdict = None;
            let mut remainder_exec = 0;
            if matches!(locus, FailureLocus::Unlocalised) {
                // No diagnosed locus: hold the batch without blaming each member.
                held = evaluated_members.clone();
            } else if !remainder.is_empty() {
                // Load-bearing F02 rule: excluded != proved remainder.
                // In particular A+B and C+D may conflict independently, and a
                // non-monotone oracle can turn red only after a member is removed.
                let (result, sources) = oracle.evaluate(&remainder);
                remainder_exec = sources
                    .iter()
                    .filter(|s| **s == CheckSource::Executed)
                    .count();
                remainder_verdict = Some(result);
                let members = remainder.iter().map(|s| (*s).to_string()).collect();
                match result {
                    UnionVerdict::Green => proceeding = members,
                    UnionVerdict::Red => held = members,
                }
            }
            let minimal_failing_pair = match &locus {
                FailureLocus::Pair(p) => Some(p.clone()),
                _ => None,
            };
            UnionEvaluation {
                verdict,
                minimal_failing_pair,
                failure: Some(locus),
                validated_members: proceeding.clone(),
                proceeding,
                held,
                remainder_verdict,
                evaluated_members,
                executed_count: executed_count + extra_exec + remainder_exec,
            }
        }
    }
}

/// Bisect a red batch over the memoised checks to the explicit failure locus.
///
/// The bisection is *minimal* and *honest*:
/// 1. First probe each item individually. If any single item's 1-element union
///    is red, the failure is that one item — a [`FailureLocus::SingleItem`].
///    Pairing it with an innocent neighbour (the old "first red 2-element
///    probe" bug) would falsely accuse the neighbour, so single items win.
/// 2. Otherwise probe every ordered pair. The first 2-element red union whose
///    BOTH members are individually green is a genuine
///    [`FailureLocus::Pair`] — neither member is itself broken, so it is truly
///    a *pair* interaction (contract ①, minimality verified).
/// 3. If neither localises (≥3-way interaction), return
///    [`FailureLocus::Unlocalised`] — explicit, never a silent empty drop.
///
/// Returns the locus and the count of evaluations that actually *executed*
/// (AC hits are free; only novelty counts toward ②).
fn bisect_failure<M: MemoCheck>(ids: &[&str], oracle: &mut M) -> (FailureLocus, usize) {
    let mut executed = 0;

    // Phase 1: individual innocence. Record which singletons are individually
    // red; a red singleton is a single-item failure, not a pair member.
    let mut individually_red = vec![false; ids.len()];
    for (i, id) in ids.iter().enumerate() {
        let probe = [*id];
        let (verdict, sources) = oracle.evaluate(&probe);
        executed += sources
            .iter()
            .filter(|s| **s == CheckSource::Executed)
            .count();
        if verdict == UnionVerdict::Red {
            individually_red[i] = true;
        }
    }
    if let Some(i) = individually_red.iter().position(|&r| r) {
        // A single item is the failure locus. Exclude it alone; never drag an
        // innocent neighbour in as a fake pair member.
        return (FailureLocus::SingleItem(ids[i].to_string()), executed);
    }

    // Phase 2: genuine pairs. Every item is individually green here, so any red
    // 2-element union is a true pair interaction — both members verified
    // innocent in isolation (defect-3 minimality).
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            let probe = [ids[i], ids[j]];
            let (verdict, sources) = oracle.evaluate(&probe);
            executed += sources
                .iter()
                .filter(|s| **s == CheckSource::Executed)
                .count();
            if verdict == UnionVerdict::Red {
                return (
                    FailureLocus::Pair(MinimalFailingPair {
                        item_a: ids[i].to_string(),
                        item_b: ids[j].to_string(),
                    }),
                    executed,
                );
            }
        }
    }

    // Phase 3: the whole union is red but no single item and no pair is — a
    // ≥3-way interaction. Surface it EXPLICITLY (defect-4): the caller must not
    // silently drop the batch.
    (FailureLocus::Unlocalised, executed)
}

/// Partition the batch entries into maximal groups of pairwise-disjoint
/// changes (non-overlapping affected-sets). Each group can land in its own
/// parallel lane (contract ②). Greedy by queue order: an entry joins the
/// first lane it is disjoint with, else opens a new lane. Lane order and
/// within-lane order both follow queue order, preserving ⑤.
pub fn disjoint_lanes(batch: &Batch) -> Vec<Vec<String>> {
    let mut lanes: Vec<Vec<usize>> = Vec::new();
    let entries = batch.entries();
    for (idx, entry) in entries.iter().enumerate() {
        let mut placed = false;
        for lane in lanes.iter_mut() {
            let disjoint = lane
                .iter()
                .all(|&k| entries[k].affected.is_disjoint(&entry.affected));
            if disjoint {
                lane.push(idx);
                placed = true;
                break;
            }
        }
        if !placed {
            lanes.push(vec![idx]);
        }
    }
    lanes
        .into_iter()
        .map(|lane| {
            lane.into_iter()
                .map(|k| entries[k].item_id().to_string())
                .collect()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::affected::AffectedSet;
    use hugit_contracts::LandableEntry;

    fn landable(id: &str, order: u64) -> LandableEntry {
        LandableEntry {
            item_id: id.to_string(),
            intent_id: format!("intent-{id}"),
            tree_hash: format!("tree-{id}"),
            order_index: order,
        }
    }

    /// Oracle whose union is red exactly when both `bad_a` and `bad_b` are
    /// present together. Every evaluation is a cache hit (0 executions).
    struct PairFails {
        bad_a: &'static str,
        bad_b: &'static str,
    }
    impl MemoCheck for PairFails {
        fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
            let has_a = item_ids.contains(&self.bad_a);
            let has_b = item_ids.contains(&self.bad_b);
            let verdict = if has_a && has_b {
                UnionVerdict::Red
            } else {
                UnionVerdict::Green
            };
            let sources = item_ids.iter().map(|_| CheckSource::Hit).collect();
            (verdict, sources)
        }
    }

    #[test]
    fn green_union_proceeds_with_all() {
        let batch = Batch::from_entries(
            "b",
            [
                (landable("a", 0), AffectedSet::new(["x"])),
                (landable("b", 1), AffectedSet::new(["y"])),
            ],
        );
        let mut oracle = PairFails {
            bad_a: "none1",
            bad_b: "none2",
        };
        let ev = evaluate_union(&batch, &mut oracle);
        assert_eq!(ev.verdict, UnionVerdict::Green);
        assert_eq!(ev.executed_count, 0);
        assert!(ev.minimal_failing_pair.is_none());
    }

    #[test]
    fn red_union_isolates_and_excludes_pair() {
        let batch = Batch::from_entries(
            "b",
            [
                (landable("A", 0), AffectedSet::new(["x"])),
                (landable("B", 1), AffectedSet::new(["y"])),
                (landable("C", 2), AffectedSet::new(["z"])),
            ],
        );
        let mut oracle = PairFails {
            bad_a: "A",
            bad_b: "B",
        };
        let ev = evaluate_union(&batch, &mut oracle);
        assert_eq!(ev.verdict, UnionVerdict::Red);
        let pair = ev.minimal_failing_pair.expect("pair named");
        assert_eq!(pair.item_a, "A");
        assert_eq!(pair.item_b, "B");
        assert_eq!(ev.proceeding, vec!["C".to_string()]);
        assert_eq!(ev.executed_count, 0);
        assert_eq!(
            ev.failure,
            Some(FailureLocus::Pair(MinimalFailingPair {
                item_a: "A".to_string(),
                item_b: "B".to_string(),
            }))
        );
    }

    /// Oracle where a named single item is individually red (and so is any
    /// union containing it). No pair interaction — the failure is one item.
    struct ItemFails {
        bad: &'static str,
    }
    impl MemoCheck for ItemFails {
        fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
            let verdict = if item_ids.contains(&self.bad) {
                UnionVerdict::Red
            } else {
                UnionVerdict::Green
            };
            (verdict, item_ids.iter().map(|_| CheckSource::Hit).collect())
        }
    }

    #[test]
    fn single_item_red_is_not_named_as_a_pair() {
        // item0 ("A") is individually red. The bisection must NOT name an
        // innocent neighbour ("B") as a pair member (defect 3).
        let batch = Batch::from_entries(
            "b",
            [
                (landable("A", 0), AffectedSet::new(["x"])),
                (landable("B", 1), AffectedSet::new(["y"])),
                (landable("C", 2), AffectedSet::new(["z"])),
            ],
        );
        let mut oracle = ItemFails { bad: "A" };
        let ev = evaluate_union(&batch, &mut oracle);
        assert_eq!(ev.verdict, UnionVerdict::Red);
        assert_eq!(ev.failure, Some(FailureLocus::SingleItem("A".to_string())));
        assert!(
            ev.minimal_failing_pair.is_none(),
            "a single-item failure is never disguised as a pair"
        );
        assert_eq!(ev.proceeding, vec!["B".to_string(), "C".to_string()]);
    }

    #[test]
    fn single_item_red_batch_signals_explicitly_not_silent_empty() {
        // A one-item red batch (defect 4): the locus is the single item, the
        // proceeding set is empty BY AN EXPLICIT SingleItem signal — not a
        // silent Vec::new() drop.
        let batch = Batch::from_entries("b", [(landable("A", 0), AffectedSet::new(["x"]))]);
        let mut oracle = ItemFails { bad: "A" };
        let ev = evaluate_union(&batch, &mut oracle);
        assert_eq!(ev.verdict, UnionVerdict::Red);
        assert_eq!(ev.failure, Some(FailureLocus::SingleItem("A".to_string())));
        assert!(ev.proceeding.is_empty());
    }

    /// Oracle red only when ALL three of a,b,c are present (a ≥3-way
    /// interaction): no single item and no pair is red.
    struct TripleFails;
    impl MemoCheck for TripleFails {
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

    #[test]
    fn unlocalisable_red_union_signals_explicitly_not_silent_drop() {
        // The union is red but no single item and no pair is — defect 4's
        // silent-empty path. The result must be an EXPLICIT Unlocalised, and
        // (because nothing can be safely excluded) NOTHING proceeds.
        let batch = Batch::from_entries(
            "b",
            [
                (landable("a", 0), AffectedSet::new(["x"])),
                (landable("b", 1), AffectedSet::new(["y"])),
                (landable("c", 2), AffectedSet::new(["z"])),
            ],
        );
        let mut oracle = TripleFails;
        let ev = evaluate_union(&batch, &mut oracle);
        assert_eq!(ev.verdict, UnionVerdict::Red);
        assert_eq!(ev.failure, Some(FailureLocus::Unlocalised));
        assert!(ev.minimal_failing_pair.is_none());
        assert!(
            ev.proceeding.is_empty(),
            "an unlocalised red union holds the whole batch — never a partial land"
        );
    }

    #[test]
    fn outcomes_for_landing_excludes_pair_proceeds_rest() {
        let batch = Batch::from_entries(
            "b",
            [
                (landable("A", 0), AffectedSet::new(["x"])),
                (landable("B", 1), AffectedSet::new(["y"])),
                (landable("C", 2), AffectedSet::new(["z"])),
            ],
        );
        let mut oracle = PairFails {
            bad_a: "A",
            bad_b: "B",
        };
        let ev = evaluate_union(&batch, &mut oracle);
        let ids: Vec<&str> = batch.entries().iter().map(|e| e.item_id()).collect();
        let outcomes = ev.outcomes_for_landing(&ids);
        assert_eq!(
            outcomes,
            vec![
                ("A", UnionOutcome::FailingPairMember),
                ("B", UnionOutcome::FailingPairMember),
                ("C", UnionOutcome::Green),
            ]
        );
    }

    #[test]
    fn disjoint_lanes_groups_non_overlapping() {
        let batch = Batch::from_entries(
            "b",
            [
                (landable("a", 0), AffectedSet::new(["x"])),
                (landable("b", 1), AffectedSet::new(["y"])),
                (landable("c", 2), AffectedSet::new(["x"])),
            ],
        );
        let lanes = disjoint_lanes(&batch);
        // a and c overlap on "x" → different lanes; b disjoint → joins lane 0.
        assert_eq!(lanes.len(), 2);
        assert_eq!(lanes[0], vec!["a".to_string(), "b".to_string()]);
        assert_eq!(lanes[1], vec!["c".to_string()]);
    }
}
