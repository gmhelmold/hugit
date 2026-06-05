//! The pure landing-queue state machine (whitepaper §4.1).
//!
//! `LANDABLE → (union-test) → LANDED | UNION_FAIL`, modelled as a
//! deterministic transition function with NO transition that lands an entry
//! out of order — the ordering invariant (⑤) is enforced structurally by the
//! absence of an edge, not by a runtime guard that could be bypassed.
//!
//! Crash-idempotency end-to-end (item ④, kill-test) is proven in B4b; B4a
//! supplies only the pure, deterministic transitions, which are idempotent by
//! construction (applying a terminal transition again is a no-op).

/// The state of a single queue entry.
///
/// Reachable transitions:
/// * `Landable → Landed`     (union test passed, predecessor already landed)
/// * `Landable → UnionFail`  (entry is a member of the minimal failing pair)
///
/// `Landed` and `UnionFail` are terminal. There is deliberately NO transition
/// that moves an entry to `Landed` while a queue predecessor is still
/// `Landable` — out-of-order landing is unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryState {
    /// Ready to land, not yet union-tested to completion.
    Landable,
    /// Union-tested green and applied in queue order. Terminal.
    Landed,
    /// Excluded because it is a member of the minimal failing pair. Terminal.
    UnionFail,
}

impl EntryState {
    /// True when no further transition is possible.
    pub fn is_terminal(self) -> bool {
        matches!(self, EntryState::Landed | EntryState::UnionFail)
    }
}

/// The outcome of a union test for a single entry, fed into the transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnionOutcome {
    /// The entry's checks passed on the union tree.
    Green,
    /// The entry is a member of the minimal failing pair.
    FailingPairMember,
}

/// Why a transition was refused. A refusal is never a panic — the caller
/// decides what to do, and the entry stays in its current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionError {
    /// The entry is already terminal; re-applying is a no-op refusal, which
    /// keeps the machine idempotent under replay.
    AlreadyTerminal,
    /// A queue predecessor of this entry has not landed yet. Landing here
    /// would be out of order, so the transition does not exist.
    PredecessorNotLanded,
}

/// Apply the pure transition for one entry given its union outcome and
/// whether every queue predecessor has already landed.
///
/// This is the ONLY function that can produce `Landed`, and it produces it
/// only when `predecessors_landed` is true — that is the structural ordering
/// invariant (⑤). No out-of-order landing edge exists.
pub fn transition(
    current: EntryState,
    outcome: UnionOutcome,
    predecessors_landed: bool,
) -> Result<EntryState, TransitionError> {
    if current.is_terminal() {
        // Idempotent replay: terminal states never move again.
        return Err(TransitionError::AlreadyTerminal);
    }
    match outcome {
        // A failing-pair member is excluded regardless of order — it never
        // lands, so ordering does not gate it.
        UnionOutcome::FailingPairMember => Ok(EntryState::UnionFail),
        UnionOutcome::Green => {
            if predecessors_landed {
                Ok(EntryState::Landed)
            } else {
                // The landing edge simply does not exist while a predecessor
                // is unlanded. Caller must land the predecessor first.
                Err(TransitionError::PredecessorNotLanded)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_with_landed_predecessors_lands() {
        assert_eq!(
            transition(EntryState::Landable, UnionOutcome::Green, true),
            Ok(EntryState::Landed)
        );
    }

    #[test]
    fn green_without_landed_predecessors_is_refused() {
        assert_eq!(
            transition(EntryState::Landable, UnionOutcome::Green, false),
            Err(TransitionError::PredecessorNotLanded)
        );
    }

    #[test]
    fn failing_pair_member_excluded_regardless_of_order() {
        assert_eq!(
            transition(EntryState::Landable, UnionOutcome::FailingPairMember, false),
            Ok(EntryState::UnionFail)
        );
    }

    #[test]
    fn terminal_states_are_idempotent_under_replay() {
        for s in [EntryState::Landed, EntryState::UnionFail] {
            assert_eq!(
                transition(s, UnionOutcome::Green, true),
                Err(TransitionError::AlreadyTerminal)
            );
            assert!(s.is_terminal());
        }
    }
}
