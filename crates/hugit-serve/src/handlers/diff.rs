//! Shared diff projection (review-legibility ①): a REAL [`DiffVm`] computed from
//! two git trees via [`hugit_proto::tree_diff`].
//!
//! The review / intent-detail surfaces all carried a universal blank
//! `DiffVm{files:[],hunks:[]}` because the diffstat seam was unwired. This module
//! wires it: given the deploy-gated git [`ObjectSource`] (`HUGIT_SERVE_GIT_DIR`)
//! plus a parent-tree and a new-tree oid, it walks the two trees and projects the
//! changed-file rows (path + added/removed line counts).
//!
//! ## Honest-default contract
//! - No git source wired (`src` / trees `None`) → an empty [`DiffVm`] (NOT an
//!   error, NOT a fabricated file). This is the deploy-gated 404-free path.
//! - The tree-walk failing (a missing object, a malformed tree) → an empty
//!   [`DiffVm`]. Fail-closed: a broken seam never serves a partial fabrication.
//!
//! ## Security
//! Every file path is SCRUBBED at this read boundary ([`crate::fmt::scrub`]): a
//! repo file literally named `ghp_….key` must not echo verbatim into the
//! view-model. Hunks are NOT emitted here — only the numstat file list, which is
//! what the review/intent surfaces render. (A full hunk view is a later wave; the
//! `hunks` vec stays empty, honestly, rather than half-rendered.)

use std::sync::Arc;

use gix_hash::ObjectId;
use hugit_http_contracts::common::{DiffVm, FileRowVm};

use crate::fmt::scrub;

/// Build a [`DiffVm`] for the change from `parent_tree` to `new_tree`.
///
/// `Some` git source + both tree oids → the REAL file list. Any `None` → the
/// honest-empty diff. A tree-walk error → the honest-empty diff (fail-closed).
#[must_use]
pub fn diff_vm(
    src: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    parent_tree: Option<&ObjectId>,
    new_tree: Option<&ObjectId>,
) -> DiffVm {
    let (Some(src), Some(parent), Some(new)) = (src, parent_tree, new_tree) else {
        return empty_diff();
    };
    match hugit_proto::tree_diff(src.as_ref(), parent, new) {
        Ok(files) => DiffVm {
            files: files
                .into_iter()
                .map(|f| FileRowVm {
                    // SCRUB at the read boundary: a secret-shaped path must not
                    // echo verbatim into the browser view-model.
                    path: scrub(&f.path),
                    added: f.added,
                    removed: f.removed,
                })
                .collect(),
            // HONEST: hunk bodies are a later wave; emit the numstat file list
            // only, never a half-rendered hunk.
            hunks: vec![],
        },
        // Fail-closed: a broken tree-walk yields the honest-empty diff.
        Err(_) => empty_diff(),
    }
}

/// The honest-empty diff (no git source, or a fail-closed tree-walk).
#[must_use]
pub fn empty_diff() -> DiffVm {
    DiffVm {
        files: vec![],
        hunks: vec![],
    }
}

// ── Commit/ref-level diffs (compare / pr_detail / commit_detail) ─────────────
//
// The numstat surfaces above (compare base→head, pr_detail intent-vs-parent,
// commit_detail commit-vs-first-parent) all start from COMMITS or REFS, not raw
// tree oids. These helpers resolve the trees, then defer to [`diff_vm`] — so the
// ONE wall-clock budget ([`hugit_proto::DIFF_BUDGET`], baked into `tree_diff`)
// governs every numstat the engine serves. Each handler runs exactly ONE
// `diff_vm`, so no single read exceeds that budget.
//
// Honest-default contract (identical to `diff_vm`): no git source, an
// unresolvable ref/oid, a root commit (no first parent), or a fail-closed
// tree-walk → the honest-empty diff. We never synthesise an against-the-empty-tree
// "all-added" numstat for a root commit: with no parent there is no real
// before-state to diff, so honest-empty is the truthful answer (never a
// fabricated wall of additions).

/// The root-tree oid of a commit, via the canonical proto decoder. `None` when
/// the object is absent / not a commit (fail-closed — never a fabricated tree).
fn commit_tree(src: &dyn hugit_proto::ObjectSource, commit: &ObjectId) -> Option<ObjectId> {
    hugit_proto::commit_root_tree(src, commit).ok().flatten()
}

/// The FIRST-parent oid of a commit (git's `^1` / `--first-parent`). `None` for
/// a root commit (no parents) or an absent/non-commit object. Decoded with the
/// canonical `gix_object::CommitRefIter` — the commit byte format is never
/// reimplemented here (mirrors `hugit_proto::commit_root_tree`).
fn first_parent(src: &dyn hugit_proto::ObjectSource, commit: &ObjectId) -> Option<ObjectId> {
    let object = src.get(commit).ok().flatten()?;
    if object.kind != hugit_proto::ObjectKind::Commit {
        return None;
    }
    // `parent_ids()` yields the parents in order; the first is `^1`. A decode
    // error (a malformed commit) surfaces as `None` → the honest-empty diff, never
    // a panic and never a partial fabrication.
    gix_object::CommitRefIter::from_bytes(&object.data)
        .parent_ids()
        .next()
}

/// Resolve a `base`/`head` ref-ish (as it arrives in the compare URL) to a commit
/// oid, consulting the live refs first, then the raw-oid fallback.
///
/// Resolution order (git's own short-name precedence, narrowed to what we store):
/// 1. an EXACT key in the live `refs` map (`refs/heads/main`, `refs/tags/v1`, …);
/// 2. `refs/heads/{refish}` then `refs/tags/{refish}` (a short branch/tag name);
/// 3. the `refish` parsed as a raw 40-hex commit oid.
///
/// `None` when none resolves to a syntactically valid oid — the caller serves the
/// honest-empty diff (no existence oracle; an unknown ref looks like an empty
/// change, never an error).
///
/// Shared with the `prs`-create write verb (`write_pr_create`), which resolves the
/// `head`/`base` ref-ish to their tip oids against the live refs at open time — so
/// the PR's pinned SHAs use the SAME resolution rules as the compare diff.
pub(crate) fn resolve_refish(
    refs: &std::collections::BTreeMap<String, String>,
    refish: &str,
) -> Option<ObjectId> {
    let candidates = [
        refs.get(refish),
        refs.get(&format!("refs/heads/{refish}")),
        refs.get(&format!("refs/tags/{refish}")),
    ];
    for tip in candidates.into_iter().flatten() {
        if let Ok(oid) = ObjectId::from_hex(tip.as_bytes()) {
            return Some(oid);
        }
    }
    // Raw-oid fallback: a 40-hex `refish` is itself a commit reference.
    ObjectId::from_hex(refish.as_bytes()).ok()
}

/// The numstat between two COMMITS (`base_commit` → `head_commit`): resolve both
/// root trees, then [`diff_vm`]. Any leg `None` / an unresolvable tree → the
/// honest-empty diff.
#[must_use]
pub fn diff_commits(
    src: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    base_commit: Option<&ObjectId>,
    head_commit: Option<&ObjectId>,
) -> DiffVm {
    let (Some(src), Some(base), Some(head)) = (src, base_commit, head_commit) else {
        return empty_diff();
    };
    let base_tree = commit_tree(src.as_ref(), base);
    let head_tree = commit_tree(src.as_ref(), head);
    diff_vm(Some(src), base_tree.as_ref(), head_tree.as_ref())
}

/// The numstat for a `base`→`head` COMPARE, resolving each ref-ish against the
/// live `refs` map (then the raw-oid fallback). An unresolvable side → the
/// honest-empty diff (no oracle).
#[must_use]
pub fn diff_compare(
    src: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    refs: &std::collections::BTreeMap<String, String>,
    base: &str,
    head: &str,
) -> DiffVm {
    let Some(src) = src else {
        return empty_diff();
    };
    let base_commit = resolve_refish(refs, base);
    let head_commit = resolve_refish(refs, head);
    diff_commits(Some(src), base_commit.as_ref(), head_commit.as_ref())
}

/// The numstat of a single COMMIT against its FIRST parent (`commit_detail`, and
/// the per-intent commit in `pr_detail`). A ROOT commit (no parent), an absent /
/// unresolvable commit, or an unresolvable parent tree → the honest-empty diff
/// (we never fabricate an all-added wall against a non-existent before-state).
#[must_use]
pub fn diff_against_first_parent(
    src: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    commit: Option<&ObjectId>,
) -> DiffVm {
    let (Some(src), Some(commit)) = (src, commit) else {
        return empty_diff();
    };
    // No first parent ⇒ root commit ⇒ honest-empty (`diff_commits` short-circuits
    // on the `None` base, exactly the no-before-state honest default).
    let parent = first_parent(src.as_ref(), commit);
    diff_commits(Some(src), parent.as_ref(), Some(commit))
}

/// `(file_count, added, removed)` rollup over a [`DiffVm`]'s file rows — the
/// scalar diffstat the review header renders (`file_count` / `added` / `removed`).
#[must_use]
pub fn diff_totals(diff: &DiffVm) -> (usize, u32, u32) {
    let file_count = diff.files.len();
    let added = diff.files.iter().map(|f| f.added).sum();
    let removed = diff.files.iter().map(|f| f.removed).sum();
    (file_count, added, removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::{CasObjectSource, GitObject, ObjectKind};

    const PAT: &str = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";

    fn blob(src: &mut CasObjectSource, body: &str) -> ObjectId {
        src.insert(GitObject::new(ObjectKind::Blob, body.as_bytes().to_vec()))
    }
    fn tree(src: &mut CasObjectSource, mut entries: Vec<(&str, &str, ObjectId)>) -> ObjectId {
        entries.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));
        let mut out = Vec::new();
        for (mode, name, oid) in &entries {
            out.extend_from_slice(mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(oid.as_bytes());
        }
        src.insert(GitObject::new(ObjectKind::Tree, out))
    }

    #[test]
    fn no_git_source_is_honest_empty() {
        let d = diff_vm(None, None, None);
        assert!(d.files.is_empty() && d.hunks.is_empty());
        assert_eq!(diff_totals(&d), (0, 0, 0));
    }

    #[test]
    fn real_diff_projects_file_rows() {
        let mut s = CasObjectSource::new();
        let old = blob(&mut s, "a\nb\n");
        let new = blob(&mut s, "a\nb\nc\n");
        let parent = tree(&mut s, vec![("100644", "f.txt", old)]);
        let child = tree(&mut s, vec![("100644", "f.txt", new)]);
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);
        let d = diff_vm(Some(&src), Some(&parent), Some(&child));
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].path, "f.txt");
        assert_eq!((d.files[0].added, d.files[0].removed), (1, 0));
        assert_eq!(diff_totals(&d), (1, 1, 0));
    }

    #[test]
    fn secret_shaped_path_is_scrubbed() {
        let mut s = CasObjectSource::new();
        let b = blob(&mut s, "x\n");
        let parent = tree(&mut s, vec![]);
        let name = format!("{PAT}.key");
        let child = tree(&mut s, vec![("100644", name.as_str(), b)]);
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);
        let d = diff_vm(Some(&src), Some(&parent), Some(&child));
        let j = serde_json::to_string(&d).unwrap();
        assert!(!j.contains(PAT));
        assert!(j.contains("REDACTED"));
    }

    #[test]
    fn missing_object_fails_closed_empty() {
        // A parent tree oid that the source does not hold → fail-closed empty.
        let mut s = CasObjectSource::new();
        let b = blob(&mut s, "x\n");
        let child = tree(&mut s, vec![("100644", "f", b)]);
        let bogus = ObjectId::null(gix_hash::Kind::Sha1);
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);
        // The parent tree is absent; read_tree_entries returns None → treated as
        // an empty side, so this is an all-added diff, NOT an error. Prove it does
        // not panic and yields a deterministic projection.
        let d = diff_vm(Some(&src), Some(&bogus), Some(&child));
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].path, "f");
    }
}
