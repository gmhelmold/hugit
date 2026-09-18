//! Bounded diagnosis of a union and explicit validation of its exact remainder.
//!
//! Diagnosis is not authorization. An interrupted or inconclusive evaluation
//! holds the whole batch, even if some earlier probes were green. This engine
//! does not build Git trees or move refs; an oracle owns the evaluated content.

use crate::core::batch::Batch;
use crate::core::state::UnionOutcome;
use hugit_contracts::MinimalFailingPair;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

/// A check result, distinct from whether the diagnostic operation completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnionVerdict {
    Green,
    Red,
    /// No trustworthy check result is available. Never a failing author.
    Unknown,
    /// The evaluation infrastructure failed, not the changes under evaluation.
    InfrastructureFailure,
}

impl UnionVerdict {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Green => "green",
            Self::Red => "red",
            Self::Unknown => "unknown",
            Self::InfrastructureFailure => "infrastructure_failure",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckSource {
    Hit,
    Executed,
}

/// Cooperative in-process oracle. Remote/process adapters must implement their
/// own bounded I/O and cleanup; this trait cannot preempt a blocking callback.
pub trait MemoCheck {
    fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>);

    /// The SAME absolute deadline is passed to every probe. Override to propagate
    /// it inside a multi-step oracle. The engine also checks before/after return:
    /// a late green cannot authorize anything. Legacy callbacks are not killed.
    fn evaluate_before(
        &mut self,
        item_ids: &[&str],
        _deadline: Instant,
    ) -> (UnionVerdict, Vec<CheckSource>) {
        self.evaluate(item_ids)
    }
}

/// Why diagnosis stopped without granting transitions. An earlier red remains
/// a diagnostic observation; it does not turn infra/unknown into a failing pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationStop {
    ProbeBudgetExhausted,
    DeadlineExceeded,
    Unknown,
    InfrastructureFailure,
    InvalidBatch,
    InvalidLimits,
    Invalidated,
}
impl EvaluationStop {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProbeBudgetExhausted => "probe_budget_exhausted",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Unknown => "unknown",
            Self::InfrastructureFailure => "infrastructure_failure",
            Self::InvalidBatch => "invalid_batch",
            Self::InvalidLimits => "invalid_limits",
            Self::Invalidated => "invalidated",
        }
    }
}

pub const DEFAULT_MAX_PROBES: usize = 64;
pub const DEFAULT_EVALUATION_TIMEOUT: Duration = Duration::from_secs(30);

/// Limits for one diagnostic operation, not one member or one cache miss.
/// Zero probes/time intentionally admits no probe. Explicit callers may lower
/// or select their own limits; the normal entry point always uses these defaults.
#[derive(Debug, Clone, Copy)]
pub struct EvaluationLimits {
    pub max_probes: usize,
    pub timeout: Duration,
}
impl Default for EvaluationLimits {
    fn default() -> Self {
        Self {
            max_probes: DEFAULT_MAX_PROBES,
            timeout: DEFAULT_EVALUATION_TIMEOUT,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum FailureLocus {
    Pair(MinimalFailingPair),
    SingleItem(String),
    Unlocalised,
}

/// Public fields report observations; private snapshots grant transitions.
/// Identity here is ordered member IDs, NOT a qualified Git/base/config token.
#[derive(Debug, Clone)]
pub struct UnionEvaluation {
    pub verdict: UnionVerdict,
    pub minimal_failing_pair: Option<MinimalFailingPair>,
    pub failure: Option<FailureLocus>,
    pub proceeding: Vec<String>,
    /// Known completed underlying executions, not the diagnostic probe count.
    pub executed_count: usize,
    /// False when an inconclusive/late callback may have unreported work.
    pub execution_count_complete: bool,
    pub held: Vec<String>,
    pub remainder_verdict: Option<UnionVerdict>,
    pub stop_reason: Option<EvaluationStop>,
    /// ALL oracle invocations, including hits and the final remainder probe.
    pub probe_count: usize,
    pub max_probes: usize,
    pub elapsed: Duration,
    pub timeout: Duration,
    evaluated_members: Vec<String>,
    validated_members: Vec<String>,
    excluded_members: Vec<String>,
    valid_until: Option<Instant>,
    revoked: Arc<AtomicBool>,
}

impl UnionEvaluation {
    fn authorization_live(&self) -> bool {
        !self.revoked.load(Ordering::Acquire)
            && self.valid_until.is_some_and(|until| Instant::now() < until)
    }

    pub fn deadline_exceeded(&self) -> bool {
        self.valid_until.is_none_or(|until| Instant::now() >= until)
    }

    pub fn validated_proceeding(&self) -> &[String] {
        if self.authorization_live() {
            &self.validated_members
        } else {
            &[]
        }
    }

    /// Only the diagnosed exclusion snapshot, never a caller-edited `failure`.
    pub fn validated_excluded(&self) -> &[String] {
        if self.authorization_live() {
            &self.excluded_members
        } else {
            &[]
        }
    }

    /// Local revocation shared by clones of this result. No queue/ref is mutated.
    /// A new attempt must evaluate again; distributed operation recovery is not
    /// implemented by this in-memory capability.
    pub fn invalidate(&mut self) {
        self.revoked.store(true, Ordering::Release);
        self.stop_reason = Some(EvaluationStop::Invalidated);
        self.proceeding.clear();
        self.held = self.evaluated_members.clone();
    }

    /// Reject altered input sets/order and results past their evaluation deadline.
    pub fn outcomes_for_landing<'a>(&self, ids: &[&'a str]) -> Vec<(&'a str, UnionOutcome)> {
        if !self.authorization_live()
            || !ids
                .iter()
                .copied()
                .eq(self.evaluated_members.iter().map(String::as_str))
        {
            return Vec::new();
        }
        let excluded: std::collections::BTreeSet<&str> =
            self.excluded_members.iter().map(String::as_str).collect();
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

struct ProbeBudget<F> {
    limits: EvaluationLimits,
    start: Instant,
    deadline: Instant,
    now: F,
    probes: usize,
    executed: usize,
    execution_count_complete: bool,
}
impl<F: Fn() -> Instant> ProbeBudget<F> {
    fn probe<M: MemoCheck>(
        &mut self,
        oracle: &mut M,
        ids: &[&str],
    ) -> Result<UnionVerdict, EvaluationStop> {
        if (self.now)() >= self.deadline {
            return Err(EvaluationStop::DeadlineExceeded);
        }
        if self.probes >= self.limits.max_probes {
            return Err(EvaluationStop::ProbeBudgetExhausted);
        }
        self.probes += 1;
        let (verdict, sources) = oracle.evaluate_before(ids, self.deadline);
        self.executed = self.executed.saturating_add(
            sources
                .iter()
                .filter(|s| **s == CheckSource::Executed)
                .count(),
        );
        if (self.now)() >= self.deadline {
            self.execution_count_complete = false;
            return Err(EvaluationStop::DeadlineExceeded);
        }
        match verdict {
            UnionVerdict::Green | UnionVerdict::Red => Ok(verdict),
            UnionVerdict::Unknown => {
                self.execution_count_complete = false;
                Err(EvaluationStop::Unknown)
            }
            UnionVerdict::InfrastructureFailure => {
                self.execution_count_complete = false;
                Err(EvaluationStop::InfrastructureFailure)
            }
        }
    }
}

/// Normal path: bounded even when every check is a cache hit.
pub fn evaluate_union<M: MemoCheck>(batch: &Batch, oracle: &mut M) -> UnionEvaluation {
    evaluate_union_with_limits(batch, oracle, EvaluationLimits::default())
}

/// Evaluate with a single cooperative deadline. This is NOT an OS supervisor:
/// elapsed wall time can exceed the limit inside a noncooperative callback, but
/// its result is discarded and no further probe or transition is authorized.
pub fn evaluate_union_with_limits<M: MemoCheck>(
    batch: &Batch,
    oracle: &mut M,
    limits: EvaluationLimits,
) -> UnionEvaluation {
    evaluate_union_with_clock(batch, oracle, limits, Instant::now)
}

fn evaluate_union_with_clock<M: MemoCheck, F: Fn() -> Instant>(
    batch: &Batch,
    oracle: &mut M,
    limits: EvaluationLimits,
    now: F,
) -> UnionEvaluation {
    let start = now();
    let deadline = start.checked_add(limits.timeout);
    let ids: Vec<&str> = batch.entries().iter().map(|e| e.item_id()).collect();
    let members: Vec<String> = ids.iter().map(|s| (*s).to_string()).collect();
    let mut ev = UnionEvaluation {
        verdict: UnionVerdict::Unknown,
        minimal_failing_pair: None,
        failure: None,
        proceeding: Vec::new(),
        executed_count: 0,
        execution_count_complete: true,
        held: members.clone(),
        remainder_verdict: None,
        stop_reason: None,
        probe_count: 0,
        max_probes: limits.max_probes,
        elapsed: Duration::ZERO,
        timeout: limits.timeout,
        evaluated_members: members,
        validated_members: Vec::new(),
        excluded_members: Vec::new(),
        valid_until: deadline,
        revoked: Arc::new(AtomicBool::new(false)),
    };
    let Some(deadline) = deadline else {
        ev.stop_reason = Some(EvaluationStop::InvalidLimits);
        ev.revoked.store(true, Ordering::Release);
        return ev;
    };
    let mut budget = ProbeBudget {
        limits,
        start,
        deadline,
        now,
        probes: 0,
        executed: 0,
        execution_count_complete: true,
    };
    let result = (|| -> Result<(), EvaluationStop> {
        let unique: std::collections::BTreeSet<&str> = ids.iter().copied().collect();
        if unique.len() != ids.len() || ids.iter().any(|s| s.is_empty()) {
            return Err(EvaluationStop::InvalidBatch);
        }
        if ids.is_empty() {
            // No changes or authority to grant: do not call the oracle.
            ev.verdict = UnionVerdict::Green;
            ev.held.clear();
            return Ok(());
        }
        ev.verdict = budget.probe(oracle, &ids)?;
        if ev.verdict == UnionVerdict::Green {
            ev.proceeding = ev.evaluated_members.clone();
            ev.held.clear();
            return Ok(());
        }
        let locus = bisect_failure(&ids, oracle, &mut budget)?;
        ev.minimal_failing_pair = match &locus {
            FailureLocus::Pair(pair) => Some(pair.clone()),
            _ => None,
        };
        ev.failure = Some(locus.clone());
        let excluded: Vec<&str> = match &locus {
            FailureLocus::Pair(pair) => vec![pair.item_a.as_str(), pair.item_b.as_str()],
            FailureLocus::SingleItem(id) => vec![id.as_str()],
            FailureLocus::Unlocalised => return Ok(()),
        };
        let remainder: Vec<&str> = ids
            .iter()
            .copied()
            .filter(|id| !excluded.contains(id))
            .collect();
        if !remainder.is_empty() {
            let verdict = budget.probe(oracle, &remainder)?;
            ev.remainder_verdict = Some(verdict);
            if verdict == UnionVerdict::Green {
                ev.proceeding = remainder.iter().map(|s| (*s).to_string()).collect();
                ev.held.clear();
            } else {
                ev.held = remainder.iter().map(|s| (*s).to_string()).collect();
            }
        } else {
            ev.held.clear();
        }
        ev.excluded_members = excluded.iter().map(|s| (*s).to_string()).collect();
        Ok(())
    })();
    if let Err(reason) = result {
        if ev.verdict == UnionVerdict::Unknown && reason == EvaluationStop::InfrastructureFailure {
            ev.verdict = UnionVerdict::InfrastructureFailure;
        }
        ev.stop_reason = Some(reason);
        ev.proceeding.clear();
        ev.excluded_members.clear();
        ev.held = ev.evaluated_members.clone();
        ev.revoked.store(true, Ordering::Release);
    }
    ev.validated_members = ev.proceeding.clone();
    ev.probe_count = budget.probes;
    ev.executed_count = budget.executed;
    ev.execution_count_complete = budget.execution_count_complete;
    ev.elapsed = (budget.now)().saturating_duration_since(budget.start);
    // Include time spent constructing the result, not just oracle callbacks.
    if ev.stop_reason.is_none() && !ids.is_empty() && (budget.now)() >= deadline {
        ev.stop_reason = Some(EvaluationStop::DeadlineExceeded);
        ev.proceeding.clear();
        ev.validated_members.clear();
        ev.excluded_members.clear();
        ev.held = ev.evaluated_members.clone();
        ev.revoked.store(true, Ordering::Release);
    }
    ev
}

/// At most O(n²) possible pairs, bounded by the SAME operation probe budget.
/// A verified red singleton ends diagnosis; otherwise only pairs of known-green
/// singletons are called pairs. Unknown/infra stops, rather than blaming a member.
fn bisect_failure<M: MemoCheck, F: Fn() -> Instant>(
    ids: &[&str],
    oracle: &mut M,
    budget: &mut ProbeBudget<F>,
) -> Result<FailureLocus, EvaluationStop> {
    for id in ids {
        if budget.probe(oracle, &[*id])? == UnionVerdict::Red {
            return Ok(FailureLocus::SingleItem((*id).to_string()));
        }
    }
    for i in 0..ids.len() {
        for j in i + 1..ids.len() {
            if budget.probe(oracle, &[ids[i], ids[j]])? == UnionVerdict::Red {
                return Ok(FailureLocus::Pair(MinimalFailingPair {
                    item_a: ids[i].to_string(),
                    item_b: ids[j].to_string(),
                }));
            }
        }
    }
    Ok(FailureLocus::Unlocalised)
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

#[cfg(test)]
mod budget_clock_tests {
    use super::*;
    use crate::core::AffectedSet;
    use hugit_contracts::LandableEntry;
    use std::cell::Cell;

    fn sample() -> Batch {
        Batch::from_entries(
            "clock",
            ["A", "B", "C"].iter().enumerate().map(|(i, id)| {
                (
                    LandableEntry {
                        item_id: (*id).into(),
                        intent_id: (*id).into(),
                        tree_hash: format!("fixture-{id}"),
                        order_index: i as u64,
                    },
                    AffectedSet::new(["fixture"]),
                )
            }),
        )
    }
    struct Timed<'a> {
        clock: &'a Cell<Instant>,
        calls: usize,
        late_at: usize,
    }
    impl MemoCheck for Timed<'_> {
        fn evaluate(&mut self, _: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
            panic!("deadline must be forwarded")
        }
        fn evaluate_before(
            &mut self,
            ids: &[&str],
            deadline: Instant,
        ) -> (UnionVerdict, Vec<CheckSource>) {
            self.calls += 1;
            if self.calls == self.late_at {
                self.clock.set(deadline);
            }
            (
                if ids.contains(&"A") && ids.contains(&"B") {
                    UnionVerdict::Red
                } else {
                    UnionVerdict::Green
                },
                vec![CheckSource::Executed],
            )
        }
    }

    #[test]
    fn deadline_at_each_phase_discards_late_results_without_sleep_or_threads() {
        for late_at in [1, 2, 5, 6] {
            let now = Cell::new(Instant::now());
            let mut oracle = Timed {
                clock: &now,
                calls: 0,
                late_at,
            };
            let ev = evaluate_union_with_clock(
                &sample(),
                &mut oracle,
                EvaluationLimits::default(),
                || now.get(),
            );
            assert_eq!(ev.stop_reason, Some(EvaluationStop::DeadlineExceeded));
            assert_eq!(ev.probe_count, late_at);
            assert_eq!(oracle.calls, late_at);
            assert_eq!(ev.held, ["A", "B", "C"]);
            assert!(ev.outcomes_for_landing(&["A", "B", "C"]).is_empty());
            assert_eq!(ev.executed_count, late_at);
            assert!(!ev.execution_count_complete);
        }
    }

    #[test]
    fn authorization_checks_its_deadline_after_evaluation() {
        let now = Cell::new(Instant::now());
        let mut oracle = Timed {
            clock: &now,
            calls: 0,
            late_at: usize::MAX,
        };
        let mut ev =
            evaluate_union_with_clock(&sample(), &mut oracle, EvaluationLimits::default(), || {
                now.get()
            });
        assert_eq!(ev.validated_proceeding(), ["C"]);
        ev.valid_until = Some(Instant::now());
        assert!(ev.validated_proceeding().is_empty());
        assert!(ev.validated_excluded().is_empty());
        assert!(ev.outcomes_for_landing(&["A", "B", "C"]).is_empty());
    }
}
