//! Ordered, idempotent landing over a batch (the ⑤ driver).
//!
//! Given a batch and the per-entry union outcomes, land entries in strict
//! queue order. An entry lands only after every predecessor has reached a
//! terminal (*settled*) state — `Landed` OR `UnionFail`. A `UnionFail`
//! predecessor is **transparent**: it is permanently excluded, so it does not
//! block an innocent later green. This is the wedge property — *exclude the
//! failing pair, the rest proceeds* — realised end-to-end: bisection names the
//! pair, the engine derives per-entry outcomes (failing-pair members vs. the
//! rest), and this driver lands the rest in order even though the excluded
//! members sit between them.
//!
//! Because the only landing edge in the state machine requires
//! `predecessors_settled`, out-of-order landing is *unrepresentable*, not
//! merely rejected. Re-driving an already-driven batch is a no-op (every entry
//! is already terminal, so each is skipped and its state re-reported), which
//! gives idempotency under replay.

use crate::core::batch::Batch;
use crate::core::state::{EntryState, TransitionError, UnionOutcome, transition};
use std::collections::HashMap;

/// Per-entry result of an ordered landing pass.
#[must_use]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandingStep {
    /// Item id this step concerns.
    pub item_id: String,
    /// State after the step.
    pub state: EntryState,
}

/// Error from an ordered landing pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LandingError {
    /// No outcome was supplied for an entry that needed one.
    MissingOutcome(String),
    /// A transition was refused (e.g. predecessor not landed). Carries the
    /// offending item id and the underlying reason.
    Refused(String, TransitionError),
}

/// Land the batch in strict queue order using the supplied per-entry union
/// outcomes (keyed by item id). Returns the ordered sequence of landing steps.
///
/// Structural ordering invariant (⑤): the driver walks entries in queue order
/// and tracks whether every predecessor is *settled* (terminal — `Landed` or
/// `UnionFail`). A green entry is only passed `predecessors_settled = true`
/// once all earlier entries are settled, so the state machine's landing edge
/// fires in order. A `UnionFail` predecessor is settled and therefore
/// transparent: it does NOT close the gate, so innocent later greens still
/// land — this is the pair-exclusion property end-to-end.
///
/// Idempotency under replay (④): an entry already in a terminal state is
/// skipped (its current state is re-reported as the step), never re-transitioned
/// and never erroring — so re-driving a fully- or partially-landed batch is a
/// no-op. An already-`Landed`/`UnionFail` predecessor still counts as settled,
/// so the gate is unaffected by the replay.
///
/// A green entry whose predecessor is genuinely *unresolved* (still `Landable`)
/// surfaces as a refusal — there is no path that lands it ahead of the gap.
#[must_use = "use the returned steps or propagate the error"]
pub fn land_in_order(
    batch: &mut Batch,
    outcomes: &[(&str, UnionOutcome)],
) -> Result<Vec<LandingStep>, LandingError> {
    let mut steps = Vec::with_capacity(batch.len());
    // True while every entry processed so far is SETTLED (terminal: Landed or
    // UnionFail). An UnionFail predecessor is excluded and transparent — it
    // keeps the gate OPEN so innocent later greens still land. The gate only
    // closes if an entry ends unresolved (Landable), which would be a real gap.
    let mut all_predecessors_settled = true;

    // Pre-build a lookup map so each entry is found in O(1) instead of O(n).
    let outcome_map: HashMap<&str, UnionOutcome> = outcomes.iter().copied().collect();

    // Collect outcomes in queue order before mutating, to keep the borrow simple.
    let ordered: Vec<(usize, UnionOutcome)> = {
        let mut v = Vec::with_capacity(batch.len());
        for (idx, entry) in batch.entries().iter().enumerate() {
            let id = entry.item_id();
            let outcome = outcome_map
                .get(id)
                .copied()
                .ok_or_else(|| LandingError::MissingOutcome(id.to_string()))?;
            v.push((idx, outcome));
        }
        v
    };

    for (idx, outcome) in ordered {
        let entry = &mut batch.entries_mut()[idx];
        let id = entry.item_id().to_string();

        // Idempotent replay: an already-terminal entry is skipped, not
        // re-transitioned. A terminal state (Landed or UnionFail) is by
        // definition settled, so the gate stays open for innocent successors
        // (and for the unfinished tail on a crash replay).
        if entry.state.is_terminal() {
            let next = entry.state;
            steps.push(LandingStep {
                item_id: id,
                state: next,
            });
            continue;
        }

        match transition(entry.state, outcome, all_predecessors_settled) {
            Ok(next) => {
                entry.state = next;
                // Landed and UnionFail are both settled (terminal); the gate
                // only closes on a non-terminal end state, which cannot occur
                // here (transition yields a terminal state or an error).
                if !next.is_terminal() {
                    all_predecessors_settled = false;
                }
                steps.push(LandingStep {
                    item_id: id,
                    state: next,
                });
            }
            Err(e) => return Err(LandingError::Refused(id, e)),
        }
    }
    Ok(steps)
}

/// The item-ids that ended in `Landed`, in queue order.
pub fn landed_in_order(batch: &Batch) -> Vec<String> {
    batch
        .entries()
        .iter()
        .filter(|e| e.state == EntryState::Landed)
        .map(|e| e.item_id().to_string())
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

    fn batch(ids: &[(&str, u64)]) -> Batch {
        Batch::from_entries(
            "b",
            ids.iter()
                .map(|(id, o)| (landable(id, *o), AffectedSet::new([*id]))),
        )
    }

    #[test]
    fn all_green_lands_in_queue_order() {
        let mut b = batch(&[("a", 0), ("b", 1), ("c", 2)]);
        let steps = land_in_order(
            &mut b,
            &[
                ("a", UnionOutcome::Green),
                ("b", UnionOutcome::Green),
                ("c", UnionOutcome::Green),
            ],
        )
        .unwrap();
        let landed: Vec<&str> = steps.iter().map(|s| s.item_id.as_str()).collect();
        assert_eq!(landed, vec!["a", "b", "c"]);
        assert_eq!(landed_in_order(&b), vec!["a", "b", "c"]);
    }

    #[test]
    fn excluded_predecessor_is_transparent_innocent_green_still_lands() {
        // `a` is excluded (failing-pair member); `b` is innocent and green.
        // The exclusion of `a` must NOT block `b` — the rest proceeds.
        let mut b = batch(&[("a", 0), ("b", 1)]);
        let steps = land_in_order(
            &mut b,
            &[
                ("a", UnionOutcome::FailingPairMember),
                ("b", UnionOutcome::Green),
            ],
        )
        .expect("excluded predecessor is transparent; innocent green lands");
        assert_eq!(steps[0].state, EntryState::UnionFail);
        assert_eq!(steps[1].state, EntryState::Landed);
        assert_eq!(landed_in_order(&b), vec!["b"]);
    }

    #[test]
    fn unresolved_predecessor_blocks_out_of_order_land() {
        // A predecessor that is already terminal-but-not-settled cannot arise
        // from a single pass, so we model a genuine gap by pre-marking `b`
        // (the successor) Landable and feeding `a` an outcome that leaves it
        // unresolved is impossible (every outcome settles). The structural
        // guard is exercised at the transition level: a green with
        // predecessors_settled=false is refused.
        assert_eq!(
            transition(EntryState::Landable, UnionOutcome::Green, false),
            Err(TransitionError::PredecessorNotLanded),
            "out-of-order landing is structurally prevented"
        );
    }

    #[test]
    fn re_driving_a_landed_batch_is_idempotent() {
        let mut b = batch(&[("a", 0), ("b", 1)]);
        land_in_order(
            &mut b,
            &[("a", UnionOutcome::Green), ("b", UnionOutcome::Green)],
        )
        .unwrap();
        let first = landed_in_order(&b);
        // Replay on the SAME batch: every entry is terminal → skipped as a
        // no-op, current state re-reported. No error, no re-land.
        let replay = land_in_order(
            &mut b,
            &[("a", UnionOutcome::Green), ("b", UnionOutcome::Green)],
        )
        .expect("replay on a terminal batch is an idempotent no-op");
        assert_eq!(replay[0].state, EntryState::Landed);
        assert_eq!(replay[1].state, EntryState::Landed);
        assert_eq!(landed_in_order(&b), first);
        assert_eq!(landed_in_order(&b), vec!["a", "b"]);
    }
}
