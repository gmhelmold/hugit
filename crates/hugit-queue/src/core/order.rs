//! Ordered, idempotent landing over a batch (the ⑤ driver).
//!
//! Given a batch and the per-entry union outcomes, land entries in strict
//! queue order. An entry lands only after every predecessor has reached a
//! terminal state — and because the only landing edge in the state machine
//! requires `predecessors_landed`, out-of-order landing is *unrepresentable*,
//! not merely rejected. Re-driving an already-driven batch is a no-op (every
//! entry is already terminal), which gives idempotency under replay.

use crate::core::batch::Batch;
use crate::core::state::{EntryState, TransitionError, UnionOutcome, transition};

/// Per-entry result of an ordered landing pass.
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
/// and tracks whether every predecessor is terminal. A green entry is only
/// passed `predecessors_landed = true` once all earlier entries are terminal,
/// so the state machine's landing edge fires in order. A green entry whose
/// predecessor failed to land surfaces as a refusal — there is no path that
/// lands it ahead of the gap.
pub fn land_in_order(
    batch: &mut Batch,
    outcomes: &[(&str, UnionOutcome)],
) -> Result<Vec<LandingStep>, LandingError> {
    let mut steps = Vec::with_capacity(batch.len());
    // True while every entry processed so far has LANDED. A single non-landed
    // predecessor (UnionFail) closes the gate for all successors' green edge.
    let mut all_predecessors_landed = true;

    // Collect outcomes in queue order before mutating, to keep the borrow simple.
    let ordered: Vec<(usize, UnionOutcome)> = {
        let mut v = Vec::with_capacity(batch.len());
        for (idx, entry) in batch.entries().iter().enumerate() {
            let id = entry.item_id();
            let outcome = outcomes
                .iter()
                .find(|(oid, _)| *oid == id)
                .map(|(_, o)| *o)
                .ok_or_else(|| LandingError::MissingOutcome(id.to_string()))?;
            v.push((idx, outcome));
        }
        v
    };

    for (idx, outcome) in ordered {
        let entry = &mut batch.entries_mut()[idx];
        let id = entry.item_id().to_string();
        match transition(entry.state, outcome, all_predecessors_landed) {
            Ok(next) => {
                entry.state = next;
                if next != EntryState::Landed {
                    all_predecessors_landed = false;
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
    fn green_after_failed_predecessor_is_refused_not_landed_out_of_order() {
        let mut b = batch(&[("a", 0), ("b", 1)]);
        let err = land_in_order(
            &mut b,
            &[
                ("a", UnionOutcome::FailingPairMember),
                ("b", UnionOutcome::Green),
            ],
        )
        .unwrap_err();
        assert_eq!(
            err,
            LandingError::Refused("b".to_string(), TransitionError::PredecessorNotLanded)
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
        // Replay: every entry is terminal → every transition refuses, no change.
        let replay = land_in_order(
            &mut b,
            &[("a", UnionOutcome::Green), ("b", UnionOutcome::Green)],
        );
        assert_eq!(
            replay,
            Err(LandingError::Refused(
                "a".to_string(),
                TransitionError::AlreadyTerminal
            ))
        );
        assert_eq!(landed_in_order(&b), vec!["a", "b"]);
    }
}
