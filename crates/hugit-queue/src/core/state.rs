//! The pure landing-queue state machine (whitepaper §4.1).
//!
//! `LANDABLE → (union-test) → LANDED | UNION_FAIL`, modelled as a
//! deterministic transition function with NO transition that lands an entry
//! out of order — the ordering invariant (⑤) is enforced structurally by the
//! absence of an edge, not by a runtime guard that could be bypassed.
//!
//! Ordering is gated on predecessors being *settled* — every earlier entry has
//! reached a terminal state (`Landed` OR `UnionFail`). A `UnionFail`
//! predecessor is **transparent**: it is permanently excluded and never lands,
//! so it does not — and must not — block an innocent later green. The product's
//! reason to exist is exactly this: exclude the failing pair, the rest proceeds.
//! Blocking is reserved for a predecessor that is genuinely *unresolved* (still
//! `Landable`), where landing the successor first would be a true out-of-order
//! land.
//!
//! Crash-idempotency end-to-end (item ④, kill-test) is proven in B4b; B4a
//! supplies only the pure, deterministic transitions, which are idempotent by
//! construction (applying a terminal transition again is a no-op).

/// The state of a single queue entry.
///
/// Reachable transitions:
/// * `Landable → Landed`     (union test passed, every predecessor settled)
/// * `Landable → UnionFail`  (entry is a member of the minimal failing pair)
///
/// `Landed` and `UnionFail` are terminal. There is deliberately NO transition
/// that moves an entry to `Landed` while a queue predecessor is still
/// `Landable` — out-of-order landing is unrepresentable. A `UnionFail`
/// predecessor is settled (terminal), so it is transparent and does not block.
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
    /// A queue predecessor of this entry is still unresolved (`Landable`) —
    /// not yet settled. Landing here would be out of order, so the transition
    /// does not exist. NOTE: an *excluded* (`UnionFail`) predecessor is settled
    /// and therefore transparent — it does NOT raise this error.
    PredecessorNotLanded,
}

/// Apply the pure transition for one entry given its union outcome and
/// whether every queue predecessor is *settled* (terminal — `Landed` or
/// `UnionFail`).
///
/// This is the ONLY function that can produce `Landed`, and it produces it
/// only when `predecessors_settled` is true — that is the structural ordering
/// invariant (⑤). No out-of-order landing edge exists. A `UnionFail`
/// predecessor counts as settled (it is excluded and will never land), so the
/// failing pair is transparent to innocent later greens.
pub fn transition(
    current: EntryState,
    outcome: UnionOutcome,
    predecessors_settled: bool,
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
            if predecessors_settled {
                Ok(EntryState::Landed)
            } else {
                // The landing edge simply does not exist while a predecessor
                // is still unresolved. Caller must settle the predecessor first
                // (land it, or exclude it as a failing-pair member).
                Err(TransitionError::PredecessorNotLanded)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_with_settled_predecessors_lands() {
        assert_eq!(
            transition(EntryState::Landable, UnionOutcome::Green, true),
            Ok(EntryState::Landed)
        );
    }

    #[test]
    fn green_with_unresolved_predecessor_is_refused() {
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
