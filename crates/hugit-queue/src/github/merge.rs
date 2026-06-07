//! Ordered atomic merge via the GitHub merge API (the engine → GitHub driver).
//!
//! When B4a's engine yields a green, ordered batch, this module drives the
//! GitHub merge API to land it. Main stays green by construction: only entries
//! the engine marks `Landed` (green union, predecessor landed) are ever passed
//! to the merge API, and they are passed in strict queue order. The merge
//! method (merge / squash / rebase) configured for the PR is honored (⑥).

use crate::core::batch::Batch;
use crate::core::order::{LandingError, land_in_order};
use crate::core::state::{EntryState, UnionOutcome};

/// The GitHub merge method honored when landing a PR (⑥).
///
/// Mirrors the `merge_method` field of GitHub's
/// `PUT /repos/{owner}/{repo}/pulls/{n}/merge` API. The configured method is
/// transcribed onto the merge call — never overridden — so the repository's
/// merge policy is respected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MergeMethod {
    /// Create a merge commit (`merge`).
    Merge,
    /// Squash all commits into one (`squash`).
    Squash,
    /// Rebase and fast-forward (`rebase`).
    Rebase,
}

impl MergeMethod {
    /// The wire string GitHub's merge API expects for this method.
    pub fn as_api_str(self) -> &'static str {
        match self {
            MergeMethod::Merge => "merge",
            MergeMethod::Squash => "squash",
            MergeMethod::Rebase => "rebase",
        }
    }

    /// Parse a repository-configured merge method string. Unknown values are
    /// refused (returns `None`) rather than silently defaulting — a wrong
    /// merge method is a policy violation, not a fallback.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "merge" => Some(MergeMethod::Merge),
            "squash" => Some(MergeMethod::Squash),
            "rebase" => Some(MergeMethod::Rebase),
            _ => None,
        }
    }
}

/// A single merge instruction sent to the GitHub merge API, in queue order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeRecord {
    /// The landable item id being merged.
    pub item_id: String,
    /// The PR head tree-hash that was union-tested and is being landed. The
    /// merge is rejected by [`MergeApi`] if the live head no longer matches
    /// (a concurrent force-push), closing the stale-union race (③).
    pub expected_head: String,
    /// The merge method honored for this PR (⑥).
    pub method: MergeMethod,
}

/// Result of driving the atomic ordered merge of a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeOutcome {
    /// Item ids merged to GitHub, in the exact queue order they were applied.
    pub merged: Vec<String>,
}

/// Why driving the merge failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MergeError {
    /// The engine refused to land the batch (e.g. out-of-order predecessor).
    /// Carries the underlying B4a landing error.
    Engine(LandingError),
    /// The GitHub merge API rejected a merge. Carries the item id and a
    /// message. The batch stops at the first rejection — main never goes red,
    /// because only already-green entries reach this point and a rejected
    /// merge simply leaves the remaining entries un-landed for the next pass.
    Api(String, String),
    /// The live PR head no longer matches the union-tested head — a force-push
    /// landed under us. The stale union must not merge; recompute first (③).
    StaleHead { item_id: String },
}

/// The impure seam: the actual GitHub merge API call.
///
/// Implementations are responsible for the authenticated
/// `PUT /repos/{owner}/{repo}/pulls/{n}/merge` with the honored `merge_method`,
/// using B1's installation token (write-only secret model). The driver below
/// owns the *ordering and atomicity* policy; the implementation owns transport.
pub trait MergeApi {
    /// Merge one PR with the honored method. Implementations MUST verify the
    /// live head equals `record.expected_head` (the union-tested head) and
    /// return [`MergeError::StaleHead`] otherwise, so a force-pushed stale
    /// union can never land. Idempotent: merging an already-merged PR is a
    /// success no-op (supports crash replay, ④).
    fn merge(&mut self, record: &MergeRecord) -> Result<(), MergeError>;
}

/// Drive the ordered atomic merge of a batch.
///
/// 1. Run B4a's pure `land_in_order` to compute which entries land and in what
///    order (the engine is the single source of ordering truth — ⑤).
/// 2. For each entry the engine moved to `Landed`, in queue order, first ENFORCE
///    stale-head at the engine — compare the union-tested `expected_head`
///    (`head_for`) against the live head (`live_head_for`); a mismatch is a
///    concurrent force-push under us, rejected with [`MergeError::StaleHead`]
///    BEFORE any merge call (③) — then call the GitHub merge API with the
///    honored merge method (⑥).
///
/// The stale-head check is enforced HERE, not merely delegated to the
/// `MergeApi` implementation: even an implementation that ignores
/// `expected_head` cannot land a stale union, because the engine refuses before
/// `merge` is ever reached. (The trait contract still asks implementations to
/// re-verify against the *truly* live head at merge time, closing the residual
/// TOCTOU; the engine check is the load-bearing, testable guard.)
///
/// Only engine-`Landed` entries are merged, so main stays green by
/// construction. The whole driver is replay-safe (④): re-driving a batch whose
/// entries are already terminal merges nothing new, and an idempotent
/// `MergeApi::merge` makes a mid-land crash safe to resume.
pub fn drive_atomic_merge<A: MergeApi>(
    batch: &mut Batch,
    outcomes: &[(&str, UnionOutcome)],
    method_for: impl Fn(&str) -> MergeMethod,
    head_for: impl Fn(&str) -> String,
    live_head_for: impl Fn(&str) -> String,
    api: &mut A,
) -> Result<MergeOutcome, MergeError> {
    // Engine decides the ordered set of landed entries (B4a). A refusal here is
    // an ordering/idempotency signal, surfaced verbatim — never bypassed.
    let steps = land_in_order(batch, outcomes).map_err(MergeError::Engine)?;

    let mut merged = Vec::new();
    for step in steps {
        if step.state != EntryState::Landed {
            // Not landed by the engine (failing-pair member etc.) → never
            // touched on GitHub. No force path exists.
            continue;
        }
        let expected_head = head_for(&step.item_id);
        // ENGINE-LEVEL stale-head enforcement (③): if the live head no longer
        // matches the union-tested head, a force-push landed under us. Refuse
        // before merging — independent of what the MergeApi impl checks.
        let live_head = live_head_for(&step.item_id);
        if live_head != expected_head {
            return Err(MergeError::StaleHead {
                item_id: step.item_id.clone(),
            });
        }
        let record = MergeRecord {
            item_id: step.item_id.clone(),
            expected_head,
            method: method_for(&step.item_id),
        };
        api.merge(&record)?;
        merged.push(step.item_id);
    }
    Ok(MergeOutcome { merged })
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

    /// In-process merge API double recording the calls it received.
    struct RecordingApi {
        calls: Vec<MergeRecord>,
        stale: Option<String>,
    }
    impl MergeApi for RecordingApi {
        fn merge(&mut self, record: &MergeRecord) -> Result<(), MergeError> {
            if self.stale.as_deref() == Some(record.item_id.as_str()) {
                return Err(MergeError::StaleHead {
                    item_id: record.item_id.clone(),
                });
            }
            self.calls.push(record.clone());
            Ok(())
        }
    }

    /// Live head matches the union-tested head for every id (the steady state).
    fn matching_live_head(id: &str) -> String {
        format!("head-{id}")
    }

    #[test]
    fn merge_method_round_trips() {
        for (s, m) in [
            ("merge", MergeMethod::Merge),
            ("squash", MergeMethod::Squash),
            ("rebase", MergeMethod::Rebase),
        ] {
            assert_eq!(MergeMethod::parse(s), Some(m));
            assert_eq!(m.as_api_str(), s);
        }
        assert_eq!(MergeMethod::parse("ff-only"), None);
    }

    #[test]
    fn drives_only_landed_entries_in_queue_order_with_honored_method() {
        let mut b = batch(&[("a", 0), ("b", 1), ("c", 2)]);
        let mut api = RecordingApi {
            calls: Vec::new(),
            stale: None,
        };
        let out = drive_atomic_merge(
            &mut b,
            &[
                ("a", UnionOutcome::Green),
                ("b", UnionOutcome::Green),
                ("c", UnionOutcome::Green),
            ],
            |id| match id {
                "b" => MergeMethod::Squash,
                _ => MergeMethod::Merge,
            },
            |id| format!("head-{id}"),
            matching_live_head,
            &mut api,
        )
        .unwrap();
        assert_eq!(out.merged, vec!["a", "b", "c"]);
        let methods: Vec<MergeMethod> = api.calls.iter().map(|c| c.method).collect();
        assert_eq!(
            methods,
            vec![MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Merge]
        );
    }

    #[test]
    fn failing_pair_member_is_excluded_innocent_successor_still_merges() {
        // a is an excluded failing-pair member; b is innocent and green. With
        // pair-exclusion semantics the engine lands b transparently (a is never
        // merged). The driver merges only b.
        let mut b = batch(&[("a", 0), ("b", 1)]);
        let mut api = RecordingApi {
            calls: Vec::new(),
            stale: None,
        };
        let out = drive_atomic_merge(
            &mut b,
            &[
                ("a", UnionOutcome::FailingPairMember),
                ("b", UnionOutcome::Green),
            ],
            |_| MergeMethod::Merge,
            |id| format!("head-{id}"),
            matching_live_head,
            &mut api,
        )
        .unwrap();
        assert_eq!(out.merged, vec!["b"], "innocent successor lands");
        let merged_ids: Vec<&str> = api.calls.iter().map(|c| c.item_id.as_str()).collect();
        assert_eq!(merged_ids, vec!["b"]);
        assert!(
            !merged_ids.contains(&"a"),
            "the excluded failing-pair member is never merged"
        );
    }

    #[test]
    fn stale_head_blocks_merge_when_api_checks() {
        let mut b = batch(&[("a", 0)]);
        let mut api = RecordingApi {
            calls: Vec::new(),
            stale: Some("a".to_string()),
        };
        let err = drive_atomic_merge(
            &mut b,
            &[("a", UnionOutcome::Green)],
            |_| MergeMethod::Merge,
            |id| format!("head-{id}"),
            matching_live_head,
            &mut api,
        )
        .unwrap_err();
        assert_eq!(
            err,
            MergeError::StaleHead {
                item_id: "a".to_string()
            }
        );
    }
}
