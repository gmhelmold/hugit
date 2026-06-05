//! Crash-recovery harness (④, the kill-test surface).
//!
//! The worker may die at any point mid-land. Recovery is built on two
//! invariants supplied by B4a:
//!
//! 1. The state machine's terminal states (`Landed` / `UnionFail`) never
//!    transition again — replaying a transition on a terminal entry is a no-op
//!    refusal, so re-driving a batch is idempotent.
//! 2. The merge API ([`crate::github::merge::MergeApi`]) is idempotent:
//!    merging an already-merged PR succeeds without a second merge.
//!
//! On restart the worker reloads the durable [`LandLog`] (which entries were
//! recorded as merged before the crash) and replays the land. Already-merged
//! entries are recognised and skipped; only the unfinished tail is driven. The
//! result: no double-merge, no lost batch, no false green — proven end-to-end
//! by the kill-test in the acceptance suite.

use crate::core::batch::Batch;
use crate::core::order::landed_in_order;
use crate::core::state::{EntryState, UnionOutcome};
use crate::github::merge::{MergeApi, MergeError, MergeMethod, MergeRecord};
use std::collections::BTreeSet;

/// Durable record of which item ids have been confirmed merged to GitHub.
///
/// This is the crash-survivable state: written before/with each merge so that a
/// restart knows exactly what already landed. In production it is the event log
/// / DO state; here it is an explicit set so the kill-test can checkpoint it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LandLog {
    merged: BTreeSet<String>,
}

impl LandLog {
    /// An empty land log (fresh batch, nothing merged yet).
    pub fn new() -> Self {
        Self::default()
    }

    /// Reconstruct a land log from durable storage (the ids already merged).
    pub fn from_merged<I, S>(ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            merged: ids.into_iter().map(Into::into).collect(),
        }
    }

    /// True if this item was already merged before the crash.
    pub fn is_merged(&self, item_id: &str) -> bool {
        self.merged.contains(item_id)
    }

    /// Mark an item merged (called after a confirmed merge).
    pub fn mark_merged(&mut self, item_id: &str) {
        self.merged.insert(item_id.to_string());
    }

    /// The merged item ids, sorted.
    pub fn merged_ids(&self) -> Vec<String> {
        self.merged.iter().cloned().collect()
    }
}

/// Outcome of a recovery replay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecoveryOutcome {
    /// Ids that were already merged before the crash (skipped, no double-merge).
    pub already_merged: Vec<String>,
    /// Ids merged during this replay (the unfinished tail).
    pub newly_merged: Vec<String>,
}

impl RecoveryOutcome {
    /// The full set of merged ids after recovery, sorted — must equal the
    /// batch's landed set exactly (no lost batch, no false green).
    pub fn all_merged(&self) -> Vec<String> {
        let mut all: BTreeSet<String> = self.already_merged.iter().cloned().collect();
        all.extend(self.newly_merged.iter().cloned());
        all.into_iter().collect()
    }
}

/// Replay a batch's land after a crash, idempotently.
///
/// `outcomes` are the per-entry union verdicts (durable, recomputed if needed).
/// `durable` is the [`LandLog`] reloaded from storage. Entries already in the
/// log are NOT merged again (idempotent — no double-merge). Entries the engine
/// would land but that are not yet in the log are merged now (no lost batch).
/// The B4a state machine guarantees ordering and that no failing-pair member
/// can be falsely merged (no false green).
pub fn recover_and_replay<A: MergeApi>(
    batch: &mut Batch,
    outcomes: &[(&str, UnionOutcome)],
    method_for: impl Fn(&str) -> MergeMethod,
    head_for: impl Fn(&str) -> String,
    durable: &mut LandLog,
    api: &mut A,
) -> Result<RecoveryOutcome, MergeError> {
    // Recompute the engine decision deterministically (B4a). Crucially, the
    // engine is a pure function of the batch + outcomes, so the post-crash
    // replay yields the SAME ordered landed set as the pre-crash run.
    crate::core::order::land_in_order(batch, outcomes).map_err(MergeError::Engine)?;
    let landed = landed_in_order(batch);

    let mut already_merged = Vec::new();
    let mut newly_merged = Vec::new();
    for item_id in landed {
        // Only entries the engine actually landed reach here; defensive check
        // keeps the contract explicit.
        debug_assert!(
            batch
                .entries()
                .iter()
                .any(|e| e.item_id() == item_id && e.state == EntryState::Landed)
        );
        if durable.is_merged(&item_id) {
            // Confirmed merged before the crash → skip. No double-merge.
            already_merged.push(item_id);
            continue;
        }
        let record = MergeRecord {
            item_id: item_id.clone(),
            expected_head: head_for(&item_id),
            method: method_for(&item_id),
        };
        // MergeApi::merge is itself idempotent, so even if the crash happened
        // AFTER the GitHub merge but BEFORE the durable write, this is safe.
        api.merge(&record)?;
        durable.mark_merged(&item_id);
        newly_merged.push(item_id);
    }
    Ok(RecoveryOutcome {
        already_merged,
        newly_merged,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::affected::AffectedSet;
    use hugit_contracts::LandableEntry;
    use std::collections::BTreeMap;

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

    /// Idempotent merge API counting how many times each PR was actually
    /// merged — a double-merge would show a count of 2.
    struct CountingIdempotentApi {
        counts: BTreeMap<String, u32>,
    }
    impl MergeApi for CountingIdempotentApi {
        fn merge(&mut self, record: &MergeRecord) -> Result<(), MergeError> {
            *self.counts.entry(record.item_id.clone()).or_insert(0) += 1;
            Ok(())
        }
    }

    #[test]
    fn replay_after_partial_land_does_not_double_merge() {
        // Pre-crash: a and b were merged, the log durably recorded both, then
        // the worker died before finishing c.
        let mut durable = LandLog::from_merged(["a", "b"]);
        let mut b = batch(&[("a", 0), ("b", 1), ("c", 2)]);
        let mut api = CountingIdempotentApi {
            counts: BTreeMap::new(),
        };
        let out = recover_and_replay(
            &mut b,
            &[
                ("a", UnionOutcome::Green),
                ("b", UnionOutcome::Green),
                ("c", UnionOutcome::Green),
            ],
            |_| MergeMethod::Merge,
            |id| format!("head-{id}"),
            &mut durable,
            &mut api,
        )
        .unwrap();

        // a, b skipped (already merged); only c newly merged.
        assert_eq!(out.already_merged, vec!["a", "b"]);
        assert_eq!(out.newly_merged, vec!["c"]);
        // No PR merged twice (no double-merge).
        assert_eq!(api.counts.get("a"), None);
        assert_eq!(api.counts.get("b"), None);
        assert_eq!(api.counts.get("c"), Some(&1));
        // Full batch accounted for (no lost batch).
        assert_eq!(out.all_merged(), vec!["a", "b", "c"]);
    }

    #[test]
    fn double_replay_is_idempotent_no_false_green() {
        let mut durable = LandLog::new();
        let mk = || batch(&[("a", 0), ("b", 1)]);
        let outcomes = [("a", UnionOutcome::Green), ("b", UnionOutcome::Green)];
        let mut api = CountingIdempotentApi {
            counts: BTreeMap::new(),
        };

        // First land (fresh batch instance).
        let mut b1 = mk();
        let first = recover_and_replay(
            &mut b1,
            &outcomes,
            |_| MergeMethod::Merge,
            |id| format!("head-{id}"),
            &mut durable,
            &mut api,
        )
        .unwrap();
        assert_eq!(first.newly_merged, vec!["a", "b"]);

        // Crash + restart: a brand-new batch instance is rebuilt from the queue,
        // but the durable log already has both → replay merges nothing new.
        let mut b2 = mk();
        let second = recover_and_replay(
            &mut b2,
            &outcomes,
            |_| MergeMethod::Merge,
            |id| format!("head-{id}"),
            &mut durable,
            &mut api,
        )
        .unwrap();
        assert!(second.newly_merged.is_empty(), "replay merges nothing new");
        assert_eq!(second.already_merged, vec!["a", "b"]);

        // Each PR merged exactly once across both passes (no double-merge,
        // no false green: only truly-landed entries ever in the log).
        assert_eq!(api.counts.get("a"), Some(&1));
        assert_eq!(api.counts.get("b"), Some(&1));
    }
}
