//! Union fold + memoised check evaluation + minimal-failing-pair bisection.
//!
//! Whitepaper §6.4: `U = fold(regen-rebase, head, batch)`, then run the
//! affected memoised checks on `U`. Most checks are AC hits — only novelty
//! executes. On a red union, bisect the batch over the (≈ free) memoised
//! checks to the minimal failing pair, exclude it, and let the rest proceed.
//!
//! B4a is engine-pure: the actual rebase and check execution live behind the
//! `MemoCheck` trait so the engine is testable in isolation (no GitHub API,
//! no runner). B4b/B2a supply the real implementations.

use crate::core::batch::Batch;
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

/// Outcome of folding + evaluating a batch's union.
#[derive(Debug, Clone)]
pub struct UnionEvaluation {
    /// Overall verdict of the union.
    pub verdict: UnionVerdict,
    /// The minimal failing pair, present iff the union is red and a pair was
    /// isolated by bisection. Both members are NAMED (contract ①).
    pub minimal_failing_pair: Option<MinimalFailingPair>,
    /// The item-ids that may proceed after excluding the failing pair.
    pub proceeding: Vec<String>,
    /// Total number of check evaluations that actually executed (novelty).
    /// Zero when every check was an AC hit (contract ②).
    pub executed_count: usize,
}

/// Fold the batch into its union and evaluate it; on red, bisect to the
/// minimal failing pair, exclude it, and report who proceeds.
///
/// The fold is `U = fold(regen-rebase, head, batch)` — represented here as the
/// ordered list of item-ids handed to the `MemoCheck` oracle, which owns the
/// rebase/exec. The engine's job is the *control flow*: evaluate, and on red,
/// bisect.
pub fn evaluate_union<M: MemoCheck>(batch: &Batch, oracle: &mut M) -> UnionEvaluation {
    let ids: Vec<&str> = batch.entries().iter().map(|e| e.item_id()).collect();
    let (verdict, sources) = oracle.evaluate(&ids);
    let executed_count = sources
        .iter()
        .filter(|s| **s == CheckSource::Executed)
        .count();

    match verdict {
        UnionVerdict::Green => UnionEvaluation {
            verdict,
            minimal_failing_pair: None,
            proceeding: ids.iter().map(|s| s.to_string()).collect(),
            executed_count,
        },
        UnionVerdict::Red => {
            let (pair, extra_exec) = bisect_failing_pair(&ids, oracle);
            let proceeding: Vec<String> = match &pair {
                Some(p) => ids
                    .iter()
                    .filter(|id| **id != p.item_a && **id != p.item_b)
                    .map(|s| s.to_string())
                    .collect(),
                None => Vec::new(),
            };
            UnionEvaluation {
                verdict,
                minimal_failing_pair: pair,
                proceeding,
                executed_count: executed_count + extra_exec,
            }
        }
    }
}

/// Bisect the batch over the memoised checks to the minimal failing pair.
///
/// The minimal failing pair is the smallest set of two members whose union is
/// red. We probe pairs over the (≈ free, memoised) oracle: the first ordered
/// pair `(a, b)` whose 2-element union is red is the minimal failing pair.
/// Returns the pair (if any) and the count of evaluations that *executed*.
fn bisect_failing_pair<M: MemoCheck>(
    ids: &[&str],
    oracle: &mut M,
) -> (Option<MinimalFailingPair>, usize) {
    let mut executed = 0;
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
                    Some(MinimalFailingPair {
                        item_a: ids[i].to_string(),
                        item_b: ids[j].to_string(),
                    }),
                    executed,
                );
            }
        }
    }
    (None, executed)
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
