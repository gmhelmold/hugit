//! Ref-op replication: force-push, branch-delete, tag create/delete, with
//! orphan-aware no-false-divergence logic (WP-E1b, item ⑤).
//!
//! Ref ops replicate as first-class operations. A deleted ref is **absent** on
//! the mirror after sync. A force-push leaves orphaned loose objects behind on
//! the mirror; those orphans must **not** register as divergence. The detector
//! is therefore **orphan-aware**: it compares **ref tips**, not loose-object
//! sets. Two repos with identical ref tips are in sync even if their loose-
//! object sets differ.

use std::collections::BTreeMap;

/// A first-class ref operation to replicate to the mirror.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefOp {
    /// Fast-forward or force update a ref to a new tip.
    Update {
        /// Fully-qualified ref name.
        ref_name: String,
        /// New tip the ref points at.
        new_tip: String,
        /// `true` when this is a non-fast-forward (force) update — leaves
        /// orphaned objects on the mirror.
        force: bool,
    },
    /// Delete a ref entirely (branch-delete / tag-delete).
    Delete {
        /// Fully-qualified ref name to remove.
        ref_name: String,
    },
    /// Create a tag pointing at a tip.
    TagCreate {
        /// Fully-qualified tag ref (e.g. `refs/tags/v1`).
        ref_name: String,
        /// Tip the tag points at.
        tip: String,
    },
}

/// A set of ref tips on one side of the mirror — the authoritative comparison
/// surface. Loose objects are deliberately NOT part of this model.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefTips {
    tips: BTreeMap<String, String>,
}

impl RefTips {
    /// Empty ref-tip set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set a ref tip (used by `apply` to replicate updates/creates).
    pub fn set(&mut self, ref_name: impl Into<String>, tip: impl Into<String>) {
        self.tips.insert(ref_name.into(), tip.into());
    }

    /// Remove a ref tip (used by `apply` to replicate deletes).
    pub fn remove(&mut self, ref_name: &str) {
        self.tips.remove(ref_name);
    }

    /// `true` when the ref is present.
    pub fn contains(&self, ref_name: &str) -> bool {
        self.tips.contains_key(ref_name)
    }

    /// Tip for a ref, if present.
    pub fn get(&self, ref_name: &str) -> Option<&String> {
        self.tips.get(ref_name)
    }

    /// All ref names, sorted.
    pub fn names(&self) -> Vec<&String> {
        self.tips.keys().collect()
    }
}

/// Apply a ref op to a tip set, replicating the op as a first-class change.
pub fn apply(tips: &mut RefTips, op: &RefOp) {
    match op {
        RefOp::Update {
            ref_name, new_tip, ..
        } => tips.set(ref_name.clone(), new_tip.clone()),
        RefOp::TagCreate { ref_name, tip } => tips.set(ref_name.clone(), tip.clone()),
        RefOp::Delete { ref_name } => tips.remove(ref_name),
    }
}

/// Apply a batch of ref ops in order.
pub fn apply_all(tips: &mut RefTips, ops: &[RefOp]) {
    for op in ops {
        apply(tips, op);
    }
}

/// Orphan-aware divergence diff: compare **ref tips only** between forge and
/// mirror (item ⑤).
///
/// Loose / orphaned objects left behind by a force-push are intentionally
/// ignored — they are not part of `RefTips` — so they cannot produce a false
/// divergence. The result lists exactly the ref names whose tips disagree.
pub fn diff_ref_tips(forge: &RefTips, mirror: &RefTips) -> Vec<String> {
    let mut diverged = Vec::new();
    // Refs present on forge: must match mirror exactly.
    for (name, forge_tip) in &forge.tips {
        match mirror.get(name) {
            Some(mtip) if mtip == forge_tip => {}
            _ => diverged.push(name.clone()),
        }
    }
    // Refs present on mirror but absent on forge (e.g. failed delete replication).
    for name in mirror.tips.keys() {
        if !forge.contains(name) {
            diverged.push(name.clone());
        }
    }
    diverged.sort();
    diverged.dedup();
    diverged
}

/// `true` when forge and mirror agree on all ref tips (orphans irrelevant).
pub fn tips_in_sync(forge: &RefTips, mirror: &RefTips) -> bool {
    diff_ref_tips(forge, mirror).is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn force_push_replicates() {
        let mut m = RefTips::new();
        m.set("refs/heads/main", "a".repeat(40));
        apply(
            &mut m,
            &RefOp::Update {
                ref_name: "refs/heads/main".into(),
                new_tip: "b".repeat(40),
                force: true,
            },
        );
        assert_eq!(m.get("refs/heads/main"), Some(&"b".repeat(40)));
    }

    #[test]
    fn deleted_ref_absent_after_sync() {
        let mut m = RefTips::new();
        m.set("refs/heads/feature", "c".repeat(40));
        apply(
            &mut m,
            &RefOp::Delete {
                ref_name: "refs/heads/feature".into(),
            },
        );
        assert!(!m.contains("refs/heads/feature"));
    }

    #[test]
    fn tag_create_replicates() {
        let mut m = RefTips::new();
        apply(
            &mut m,
            &RefOp::TagCreate {
                ref_name: "refs/tags/v1".into(),
                tip: "d".repeat(40),
            },
        );
        assert!(m.contains("refs/tags/v1"));
    }

    #[test]
    fn orphans_do_not_cause_false_divergence() {
        // Forge and mirror agree on the only ref tip; the mirror also retains
        // an orphaned object from a force-push — but orphans are not modelled
        // in RefTips, so the diff is empty.
        let mut forge = RefTips::new();
        forge.set("refs/heads/main", "z".repeat(40));
        let mut mirror = RefTips::new();
        mirror.set("refs/heads/main", "z".repeat(40));
        assert!(tips_in_sync(&forge, &mirror));
        assert!(diff_ref_tips(&forge, &mirror).is_empty());
    }
}
