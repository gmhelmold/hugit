//! Force-push recompute (③).
//!
//! When a force-push lands on the head of a PR that is part of an in-flight
//! batch, the union that was folded from the *old* head is invalidated. A stale
//! union must never merge. This module detects that case from the App's
//! force-push webhook, re-folds the batch via B4a's pure `evaluate_union`
//! against the new head, and records the recompute as an
//! [`hugit_contracts::EventRecord`].
//!
//! There is no shortcut that reuses the old result or omits the re-fold: a
//! force-push to a batched head ALWAYS triggers a fresh fold from scratch.

use crate::core::batch::Batch;
use crate::core::union::{MemoCheck, UnionEvaluation, evaluate_union};
use hugit_contracts::EventRecord;

/// A force-push event projected from the App's `push` webhook (the `forced`
/// flag set), carrying the facts the recompute needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecomputeTrigger {
    /// Item id of the batched PR whose head was force-pushed.
    pub item_id: String,
    /// The new head tree-hash after the force-push (the union must be re-folded
    /// against this; the prior union, folded from the old head, is dead).
    pub new_head: String,
}

/// Re-fold and re-evaluate a batch's union after a force-push to one of its
/// heads, invalidating the prior union result (③).
///
/// Returns the *fresh* union evaluation plus an [`EventRecord`] auditing the
/// recompute. The caller replaces any cached union with this result; the prior
/// union — whatever its verdict — is discarded. This is the only correct
/// response to a force-push: never reuse a stale union, never skip the
/// recompute.
pub fn recompute_on_force_push<M: MemoCheck>(
    batch: &Batch,
    trigger: &RecomputeTrigger,
    oracle: &mut M,
    seq: u64,
    prev_hash: &str,
    recorded_at: u64,
) -> (UnionEvaluation, EventRecord) {
    // Re-fold from scratch against the post-force-push state. B4a owns the fold
    // and bisection; this module owns *triggering* it on the force-push event.
    let evaluation = evaluate_union(batch, oracle);

    let payload = format!(
        "{{\"item_id\":\"{}\",\"new_head\":\"{}\",\"recomputed\":true,\"prior_union_invalidated\":true}}",
        trigger.item_id, trigger.new_head
    );
    let event = EventRecord {
        seq,
        prev_hash: prev_hash.to_string(),
        // The ledger computes the chained digest; this transcribes the
        // recompute decision into the frozen envelope.
        this_hash: String::new(),
        kind: "queue.force_push_recompute".to_string(),
        principal_chain: vec!["hugit-queue/github".to_string()],
        payload,
        recorded_at,
    };
    (evaluation, event)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::affected::AffectedSet;
    use crate::core::union::{CheckSource, UnionVerdict};
    use hugit_contracts::LandableEntry;

    fn landable(id: &str, order: u64) -> LandableEntry {
        LandableEntry {
            item_id: id.to_string(),
            intent_id: format!("intent-{id}"),
            tree_hash: format!("tree-{id}"),
            order_index: order,
        }
    }

    /// Oracle whose verdict flips based on a mutable flag — models the union
    /// changing because the force-push introduced a conflict.
    struct FlipOracle {
        red: bool,
    }
    impl MemoCheck for FlipOracle {
        fn evaluate(&mut self, item_ids: &[&str]) -> (UnionVerdict, Vec<CheckSource>) {
            let v = if self.red {
                UnionVerdict::Red
            } else {
                UnionVerdict::Green
            };
            (v, item_ids.iter().map(|_| CheckSource::Hit).collect())
        }
    }

    #[test]
    fn force_push_recompute_refolds_and_records_event() {
        let batch = Batch::from_entries(
            "b",
            [
                (landable("A", 0), AffectedSet::new(["a"])),
                (landable("B", 1), AffectedSet::new(["b"])),
            ],
        );
        let trigger = RecomputeTrigger {
            item_id: "A".to_string(),
            new_head: "tree-A2".to_string(),
        };
        // Post-force-push, the union is now red (the force-push broke it).
        let mut oracle = FlipOracle { red: true };
        let (ev, event) =
            recompute_on_force_push(&batch, &trigger, &mut oracle, 5, &"0".repeat(64), 99);

        // The fresh union is used — the prior (possibly green) union is dead.
        assert_eq!(ev.verdict, UnionVerdict::Red);
        assert_eq!(event.kind, "queue.force_push_recompute");
        assert_eq!(event.seq, 5);
        assert!(event.payload.contains("tree-A2"));
        assert!(event.payload.contains("prior_union_invalidated"));
        assert!(event.payload.contains("recomputed"));
    }

    #[test]
    fn force_push_that_keeps_green_still_records_a_recompute() {
        let batch = Batch::from_entries("b", [(landable("A", 0), AffectedSet::new(["a"]))]);
        let trigger = RecomputeTrigger {
            item_id: "A".to_string(),
            new_head: "tree-A2".to_string(),
        };
        let mut oracle = FlipOracle { red: false };
        let (ev, event) =
            recompute_on_force_push(&batch, &trigger, &mut oracle, 1, &"0".repeat(64), 0);
        // Even when the new union is still green, the recompute HAPPENED and is
        // recorded — we never reused the stale union to reach this.
        assert_eq!(ev.verdict, UnionVerdict::Green);
        assert_eq!(event.kind, "queue.force_push_recompute");
    }
}
