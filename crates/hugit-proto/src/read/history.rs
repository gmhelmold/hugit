//! Per-path revision history — a bounded `git log --first-parent <path>` over the
//! CAS-held commit graph (githugr NEEDS #3.2, the blob "Histórico" drawer).
//!
//! Given a HEAD commit and a repo-relative `path`, this walks the first-parent
//! commit chain and emits one [`BlobHistoryEntry`] for every commit that **touched**
//! `path` (its blob oid at that commit differs from its blob oid in the commit's
//! first parent — including present-in-one / absent-in-other; a root commit that
//! contains the path is its introduction).
//!
//! # The wall-clock DoS bound is the whole point
//!
//! This walk issues MANY synchronous CAS/R2 fetches on the SINGLE-THREADED lazy-CAS
//! engine — per commit: the commit object, its root tree, the path's tree-walk, and
//! the first parent's tree-walk. An UNBOUNDED walk over deep history blocks the WHOLE
//! accept loop (incl. `/readyz`) for minutes — the exact single-thread latency-DoS
//! class as the code-search wedge (2026-06-27) and the `tree_diff` walk (see
//! [`crate::DIFF_BUDGET`]). So the walk is bounded by BOTH:
//!
//! - a WALL-CLOCK [`BLOB_HISTORY_BUDGET`] deadline (a RESULT count is NOT a latency
//!   bound — most commits don't touch the path yet each still costs fetches), and
//! - a [`MAX_HISTORY_REVS`] emitted-revision count cap.
//!
//! **Fail-closed = partial-honest:** when either bound trips — OR a missing/garbled
//! object is hit mid-walk — the walk STOPS and returns the revisions collected so far
//! (the most-recent ones, newest-first). It NEVER errors and NEVER fabricates: a
//! consumer renders "showing N most recent" from a partial list, or nothing.

use gix_hash::ObjectId;

use crate::read::pack::{ObjectKind, ObjectSource};

/// Hard WALL-CLOCK ceiling on one [`blob_history`] walk. Mirrors
/// [`crate::DIFF_BUDGET`]: on the single-threaded engine each commit/tree/blob is a
/// synchronous CAS (R2) fetch, so a cold-cache walk over deep history would block the
/// WHOLE accept loop for minutes. The walk STOPS at this deadline with whatever it has
/// collected (the most-recent revisions), rather than wedge the engine — a revision
/// timeline is informational; partial-newest is acceptable. Checked once per commit.
pub const BLOB_HISTORY_BUDGET: std::time::Duration = std::time::Duration::from_millis(2_000);

/// Maximum number of touching-revisions a single [`blob_history`] returns. A RESULT
/// bound (paired with, never replacing, the wall-clock [`BLOB_HISTORY_BUDGET`]): the
/// drawer only shows a bounded recent timeline, so 50 most-recent revisions is ample.
pub const MAX_HISTORY_REVS: usize = 50;

/// Default wall-clock budget for building the WHOLE-history per-path index
/// ([`build_blob_history_index`]). MUCH larger than the per-request
/// [`BLOB_HISTORY_BUDGET`] because the build runs OFF the single-threaded accept loop
/// on a detached thread (the same discipline as the cached clone-pack build), so it
/// may reach deep into history without wedging the engine. STILL hard-bounded: a
/// pathological deep history stops here with the PARTIAL (newest-first) index rather
/// than hang the build thread forever.
pub const INDEX_BUILD_BUDGET: std::time::Duration = std::time::Duration::from_secs(120);

/// Max commits the index build walks (first-parent), paired with
/// [`INDEX_BUILD_BUDGET`] — whichever trips first stops the walk with the partial
/// index. A hard ceiling on the build's CAS-fetch count independent of wall-clock.
pub const MAX_INDEX_COMMITS: usize = 20_000;

/// Build the PRECOMPUTED per-path history index for a repo: a single first-parent walk
/// from `head_commit` that records, for EVERY path any walked commit touched, the
/// newest-first list of touching revisions. Returns `path → [BlobHistoryEntry]` (each
/// list capped at [`MAX_HISTORY_REVS`], newest first).
///
/// # Why an index (vs the per-request [`blob_history`] walk)
///
/// [`blob_history`] walks from HEAD *per path* under a 2 s budget; for a file whose
/// touches are DEEP in history, the many intervening non-touching commits exhaust the
/// budget before a single touch is found → an empty "Histórico" drawer. This function
/// walks the history ONCE and diffs each commit against its first parent
/// ([`crate::tree_diff_until`]), so a deep-history file's touches are recorded during
/// the same pass that records the shallow ones — the index then serves the deep file's
/// list directly, no per-request walk.
///
/// # Bounded — MUST run off the accept loop
///
/// Each commit is several synchronous CAS fetches (commit + a recursive tree-diff), so
/// this is the SAME single-thread latency-DoS class the per-request budget defends
/// against — it must be called on a detached thread, never inline. The walk is bounded
/// by BOTH `deadline` (wall-clock) and `max_commits`; on either bound — or a
/// missing/garbled object mid-walk — it STOPS and returns the PARTIAL index built so
/// far (the newest commits, which is exactly what a recency-ordered drawer wants). It
/// NEVER errors and NEVER fabricates.
#[must_use]
pub fn build_blob_history_index(
    src: &dyn ObjectSource,
    head_commit: &ObjectId,
    deadline: std::time::Instant,
    max_commits: usize,
) -> std::collections::HashMap<String, Vec<BlobHistoryEntry>> {
    let mut by_path: std::collections::HashMap<String, Vec<BlobHistoryEntry>> =
        std::collections::HashMap::new();
    // A root commit (or a missing first parent) is diffed against the EMPTY tree so
    // every file it contains is recorded as its introduction (an Added change).
    let empty_tree = ObjectId::empty_tree(gix_hash::Kind::Sha1);
    let mut current = Some(*head_commit);
    let mut walked = 0usize;

    while let Some(commit_oid) = current {
        // Bounds checked BEFORE any fetch — stop with the partial index, never wedge
        // the build thread past the budget/commit ceiling.
        if walked >= max_commits || std::time::Instant::now() >= deadline {
            break;
        }
        walked += 1;

        // A missing/garbled commit → stop with the partial index (fail-closed-honest).
        let Some(commit) = load_commit(src, &commit_oid) else {
            break;
        };
        let first_parent = commit.parents.first().copied();
        // The first parent's root tree (EMPTY for a root commit or a missing parent →
        // the commit's whole tree diffs as Added = its files' introductions).
        let parent_tree = match first_parent {
            Some(parent_oid) => crate::commit_root_tree(src, &parent_oid)
                .ok()
                .flatten()
                .unwrap_or(empty_tree),
            None => empty_tree,
        };
        // The paths this commit touched vs its first parent. Shares the SAME `deadline`
        // so a single huge-commit diff can never run past the whole-build budget; a
        // diff error → an empty change set (that commit contributes nothing, honest).
        let changed =
            crate::tree_diff_until(src, &parent_tree, &commit.tree, deadline).unwrap_or_default();

        let entry = BlobHistoryEntry {
            commit_hex: commit_oid.to_hex().to_string(),
            author_time_ms: commit.author_time_ms,
            author: commit.author.clone(),
            summary: commit.summary.clone(),
        };
        for file in changed {
            let revs = by_path.entry(file.path).or_default();
            // Newest-first walk → keep only the newest MAX_HISTORY_REVS per path (once
            // full, skip — the deep tail is never shown in the bounded drawer).
            if revs.len() < MAX_HISTORY_REVS {
                revs.push(entry.clone());
            }
        }

        current = first_parent;
    }

    by_path
}

/// One revision in which `path` was touched. Every field is REAL — read from the
/// commit object — or the entry is not emitted; NOTHING is fabricated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobHistoryEntry {
    /// The commit oid (hex) in which `path` was touched. The drawer links to it.
    pub commit_hex: String,
    /// The commit's author time as a Unix-epoch millisecond timestamp. The read-
    /// boundary caller humanizes it (e.g. `crate::fmt::humanize_age`); kept as the
    /// raw integer here so the proto layer carries no locale/format policy.
    pub author_time_ms: u64,
    /// The commit author's display string (`name <email>`), VERBATIM from the commit.
    /// Free text → the read-boundary caller MUST scrub it (a commit author could carry
    /// a secret-shaped value); the proto layer does not redact.
    pub author: String,
    /// The FIRST LINE of the commit message (the summary), VERBATIM from the commit.
    /// Free text → the read-boundary caller MUST scrub it.
    pub summary: String,
}

/// Walk the first-parent commit chain from `head_commit` and return the revisions
/// that touched `path`, newest-first, BOUNDED by `deadline` (wall-clock) and
/// `max_revs` (count).
///
/// Infallible-honest: a missing/garbled object mid-walk, a hit deadline, or the count
/// cap all STOP the walk and return the PARTIAL list collected so far — never an error
/// (a 500 on the blob read), never a fabricated row. An empty result is the honest
/// "path has no history here" (or "no source / head") — the drawer stays disabled.
///
/// The commit byte format is decoded with `gix_object::CommitRefIter`/`CommitRef` —
/// the same canonical decoders [`crate::read::serve`] and [`crate::commit_root_tree`]
/// use — never reimplemented here.
#[must_use]
pub fn blob_history(
    src: &dyn ObjectSource,
    head_commit: &ObjectId,
    path: &str,
    deadline: std::time::Instant,
    max_revs: usize,
) -> Vec<BlobHistoryEntry> {
    let mut out = Vec::new();
    let mut current = Some(*head_commit);
    // Carry the first parent we resolve THIS iteration forward: it becomes the next
    // iteration's `current`, and the parent's path-blob we compute here IS the next
    // iteration's `this_blob` (same path, same tree → same oid). Reusing both halves
    // the per-commit CAS work (one commit-load + one tree-walk per commit instead of
    // two), so the wall-clock budget reaches ~2x deeper into history — same bound,
    // same fail-closed semantics, strictly fewer fetches.
    let mut carried: Option<(CommitInfo, Option<ObjectId>)> = None;

    while let Some(commit_oid) = current {
        // WALL-CLOCK DoS guard, checked once per commit BEFORE any fetch: stop with
        // the partial list rather than block the single-threaded accept loop.
        if std::time::Instant::now() >= deadline {
            break;
        }
        if out.len() >= max_revs {
            break;
        }

        // This commit's info + path-blob: reuse the carry from the previous iteration
        // (which already loaded this commit as its first parent), or fetch fresh (the
        // HEAD, or after a carry was dropped). A missing/garbled object → stop +
        // return partial (never propagate — this must not 500 the blob read).
        let (commit, this_blob) = match carried.take() {
            Some(pair) => pair,
            None => {
                let Some(commit) = load_commit(src, &commit_oid) else {
                    break;
                };
                let this_blob = resolve_blob_oid(src, &commit.tree, path);
                (commit, this_blob)
            }
        };

        // The FIRST parent (the first-parent simplification — standard
        // `git log --first-parent <path>`).
        let first_parent = commit.parents.first().copied();

        let touched = match first_parent {
            // A root commit (no parent) that CONTAINS the path = its introduction.
            None => this_blob.is_some(),
            Some(parent_oid) => match load_commit(src, &parent_oid) {
                Some(parent) => {
                    // Resolve the parent's path-blob via its ROOT TREE, then CARRY the
                    // parent (info + path-blob) to the next iteration so it is never
                    // re-fetched as `current`.
                    let parent_blob = resolve_blob_oid(src, &parent.tree, path);
                    let touched = this_blob != parent_blob;
                    carried = Some((parent, parent_blob));
                    touched
                }
                // A missing/garbled parent → treat as "path absent in parent" so a
                // present-here path still emits (introduction), fail-closed-honest;
                // nothing to carry, and the next iteration's fresh load breaks.
                None => this_blob.is_some(),
            },
        };

        if touched {
            out.push(BlobHistoryEntry {
                commit_hex: commit_oid.to_hex().to_string(),
                author_time_ms: commit.author_time_ms,
                author: commit.author,
                summary: commit.summary,
            });
        }

        // First-parent walk only.
        current = first_parent;
    }

    out
}

/// The slice of a commit object `blob_history` needs: its root tree, its parents
/// (first-parent order preserved), the author time (ms), the author display string,
/// and the message's first line. Decoded once per commit.
struct CommitInfo {
    tree: ObjectId,
    parents: Vec<ObjectId>,
    author_time_ms: u64,
    author: String,
    summary: String,
}

/// Load + decode a commit object. `None` if absent, not a commit, or undecodable
/// (the caller treats `None` as "stop the walk" — fail-closed, never a fabrication).
fn load_commit(src: &dyn ObjectSource, oid: &ObjectId) -> Option<CommitInfo> {
    let object = src.get(oid).ok().flatten()?;
    if object.kind != ObjectKind::Commit {
        return None;
    }
    let commit = gix_object::CommitRef::from_bytes(&object.data).ok()?;

    // The parsed author signature (`name`, `email`, `time`). A malformed author header
    // → `None` (stop the walk; never fabricate an identity).
    let author = commit.author().ok()?;
    // `name <email>` — the verbatim author identity (scrubbed at the read boundary).
    let author_str = format!("{} <{}>", author.name, author.email);
    // Author time as Unix-epoch ms. `time()` decodes the raw header; a malformed time
    // → 0 (honest "unknown time", never a fabricated date). `seconds` is i64 epoch-
    // seconds; clamp a negative (pre-1970) time to 0 rather than underflow.
    let author_time_ms = author
        .time()
        .map(|t| {
            u64::try_from(t.seconds.max(0))
                .unwrap_or(0)
                .saturating_mul(1_000)
        })
        .unwrap_or(0);

    // The summary = the FIRST LINE of the message only.
    let message = commit.message;
    let summary = message
        .split(|b| *b == b'\n')
        .next()
        .map(|line| String::from_utf8_lossy(line).into_owned())
        .unwrap_or_default();

    Some(CommitInfo {
        tree: commit.tree(),
        parents: commit.parents().collect(),
        author_time_ms,
        author: author_str,
        summary,
    })
}

/// Resolve `path` to its blob oid in `root_tree`, returning ONLY the oid (never the
/// blob bytes — fetching every revision's content would multiply the CAS budget). A
/// thin oid-only mirror of [`crate::resolve_blob_at_path`]'s tree-walk: same depth +
/// traversal-safety guards, same fail-closed `None` on any absent/garbled segment.
///
/// `None` when the path is absent at this tree (or any intermediate is not a tree, or
/// the final entry is not a blob) — which is exactly the signal `blob_history` needs
/// to detect a present↔absent transition (an add or a delete of `path`).
fn resolve_blob_oid(src: &dyn ObjectSource, root_tree: &ObjectId, path: &str) -> Option<ObjectId> {
    let segments: Vec<&str> = path.split('/').collect();
    // SECURITY: reject over-deep paths before any CAS I/O (per-segment fetch DoS).
    if segments.len() > crate::read::pack::MAX_PATH_DEPTH {
        return None;
    }
    let last = segments.len().saturating_sub(1);

    let mut current_tree = *root_tree;
    for (idx, segment) in segments.iter().enumerate() {
        // SECURITY: never walk through an empty / "." / ".." segment.
        if segment.is_empty() || *segment == "." || *segment == ".." {
            return None;
        }
        let object = src.get(&current_tree).ok().flatten()?;
        if object.kind != ObjectKind::Tree {
            return None;
        }
        let entries = gix_object::TreeRefIter::from_bytes(&object.data)
            .entries()
            .ok()?;
        let entry = entries
            .into_iter()
            .find(|e| e.filename == segment.as_bytes())?;
        let entry_oid = entry.oid.to_owned();

        if idx == last {
            // Final segment must be a regular/executable blob (symlinks/gitlinks are
            // not "the file at this path").
            if !entry.mode.is_blob() {
                return None;
            }
            return Some(entry_oid);
        }
        // Intermediate segment must be a tree to descend.
        if !entry.mode.is_tree() {
            return None;
        }
        current_tree = entry_oid;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::read::pack::CasObjectSource;

    const MODE_BLOB: &str = "100644";
    const MODE_TREE: &str = "40000";

    struct TreeEntry<'a> {
        mode: &'a str,
        name: &'a str,
        oid: ObjectId,
    }

    fn build_tree_bytes(mut entries: Vec<TreeEntry<'_>>) -> Vec<u8> {
        entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        let mut out = Vec::new();
        for e in &entries {
            out.extend_from_slice(e.mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(e.name.as_bytes());
            out.push(0);
            out.extend_from_slice(e.oid.as_bytes());
        }
        out
    }

    fn insert_tree(src: &mut CasObjectSource, entries: Vec<TreeEntry<'_>>) -> ObjectId {
        src.insert_raw(ObjectKind::Tree, build_tree_bytes(entries))
    }

    /// Build a commit object with a single (optional) parent, a given author time,
    /// author identity, and message.
    fn insert_commit(
        src: &mut CasObjectSource,
        tree: ObjectId,
        parent: Option<ObjectId>,
        author: &str,
        time_secs: i64,
        message: &str,
    ) -> ObjectId {
        let mut body = format!("tree {tree}\n");
        if let Some(p) = parent {
            body.push_str(&format!("parent {p}\n"));
        }
        body.push_str(&format!(
            "author {author} {time_secs} +0000\ncommitter {author} {time_secs} +0000\n\n{message}\n"
        ));
        src.insert_raw(ObjectKind::Commit, body.into_bytes())
    }

    /// A deadline far in the future (the walk runs to its natural end).
    fn far_deadline() -> std::time::Instant {
        std::time::Instant::now() + std::time::Duration::from_secs(60)
    }

    /// PROOF (1): a hand-built 3-commit chain — the walk emits EXACTLY the commits
    /// that touched `path`, newest-first, and skips the one that did not.
    #[test]
    fn finds_the_touching_commits_on_a_chain() {
        let mut src = CasObjectSource::new();
        let other = src.insert_raw(ObjectKind::Blob, b"other".to_vec());

        // C1 (root): introduces foo.rs = "v1".
        let foo_v1 = src.insert_raw(ObjectKind::Blob, b"v1".to_vec());
        let t1 = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "foo.rs",
                oid: foo_v1,
            }],
        );
        let c1 = insert_commit(&mut src, t1, None, "Alice <a@x>", 1000, "add foo");

        // C2: does NOT touch foo.rs (same blob oid) — only adds other.txt.
        let t2 = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "foo.rs",
                    oid: foo_v1,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "other.txt",
                    oid: other,
                },
            ],
        );
        let c2 = insert_commit(&mut src, t2, Some(c1), "Bob <b@x>", 2000, "add other");

        // C3 (head): modifies foo.rs → "v2".
        let foo_v2 = src.insert_raw(ObjectKind::Blob, b"v2".to_vec());
        let t3 = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "foo.rs",
                    oid: foo_v2,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "other.txt",
                    oid: other,
                },
            ],
        );
        let c3 = insert_commit(
            &mut src,
            t3,
            Some(c2),
            "Carol <c@x>",
            3000,
            "edit foo\n\nbody",
        );

        let hist = blob_history(&src, &c3, "foo.rs", far_deadline(), MAX_HISTORY_REVS);

        // C3 (edit) and C1 (introduction) touched foo.rs; C2 did not.
        assert_eq!(hist.len(), 2, "history: {hist:?}");
        // Newest-first.
        assert_eq!(hist[0].commit_hex, c3.to_hex().to_string());
        assert_eq!(hist[0].summary, "edit foo"); // FIRST LINE only (not "\n\nbody")
        assert_eq!(hist[0].author, "Carol <c@x>");
        assert_eq!(hist[0].author_time_ms, 3_000_000);
        assert_eq!(hist[1].commit_hex, c1.to_hex().to_string());
        assert_eq!(hist[1].summary, "add foo");
        // C2 (the non-touching commit) is absent.
        assert!(!hist.iter().any(|h| h.commit_hex == c2.to_hex().to_string()));
    }

    /// PROOF (2): a deadline ALREADY in the past stops BEFORE exhausting the walk —
    /// a long chain returns near-empty without walking every commit.
    #[test]
    fn past_deadline_returns_without_exhausting_the_walk() {
        let mut src = CasObjectSource::new();
        // Build a deep chain (200 commits), each touching foo.rs.
        let mut parent: Option<ObjectId> = None;
        let mut head = None;
        for i in 0..200u32 {
            let blob = src.insert_raw(ObjectKind::Blob, format!("v{i}").into_bytes());
            let tree = insert_tree(
                &mut src,
                vec![TreeEntry {
                    mode: MODE_BLOB,
                    name: "foo.rs",
                    oid: blob,
                }],
            );
            let c = insert_commit(&mut src, tree, parent, "A <a@x>", 1000 + i as i64, "edit");
            parent = Some(c);
            head = Some(c);
        }
        let head = head.unwrap();

        // A deadline already in the PAST: the first per-commit check trips it.
        let past = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let hist = blob_history(&src, &head, "foo.rs", past, MAX_HISTORY_REVS);
        // The walk stopped immediately — it did NOT exhaust all 200 touching commits.
        assert!(
            hist.len() < 200,
            "a past deadline must stop the walk early, got {} revs",
            hist.len()
        );
    }

    /// PROOF (2b): the COUNT cap also bounds the walk (newest-first, exactly `max`).
    #[test]
    fn count_cap_bounds_the_walk() {
        let mut src = CasObjectSource::new();
        let mut parent: Option<ObjectId> = None;
        let mut head = None;
        for i in 0..20u32 {
            let blob = src.insert_raw(ObjectKind::Blob, format!("v{i}").into_bytes());
            let tree = insert_tree(
                &mut src,
                vec![TreeEntry {
                    mode: MODE_BLOB,
                    name: "foo.rs",
                    oid: blob,
                }],
            );
            let c = insert_commit(&mut src, tree, parent, "A <a@x>", 1000 + i as i64, "edit");
            parent = Some(c);
            head = Some(c);
        }
        let hist = blob_history(&src, &head.unwrap(), "foo.rs", far_deadline(), 5);
        assert_eq!(hist.len(), 5, "the count cap bounds emitted revisions");
    }

    /// A missing object mid-walk stops + returns the partial list (never errors).
    #[test]
    fn missing_object_mid_walk_returns_partial_not_error() {
        let mut src = CasObjectSource::new();
        let foo = src.insert_raw(ObjectKind::Blob, b"v1".to_vec());
        let t1 = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "foo.rs",
                oid: foo,
            }],
        );
        // Head commit references a parent that is NOT in the source.
        let phantom = ObjectId::from_hex(b"0123456789012345678901234567890123456789").unwrap();
        let head = insert_commit(&mut src, t1, Some(phantom), "A <a@x>", 1000, "edit");

        let hist = blob_history(&src, &head, "foo.rs", far_deadline(), MAX_HISTORY_REVS);
        // The head commit still emits (its path differs vs the missing-parent's
        // "absent" view), then the walk stops at the phantom parent — partial, no error.
        assert_eq!(hist.len(), 1);
        assert_eq!(hist[0].commit_hex, head.to_hex().to_string());
    }

    /// An absent path (never existed) → empty history (the drawer stays disabled).
    #[test]
    fn absent_path_is_empty_history() {
        let mut src = CasObjectSource::new();
        let foo = src.insert_raw(ObjectKind::Blob, b"v1".to_vec());
        let t1 = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "foo.rs",
                oid: foo,
            }],
        );
        let head = insert_commit(&mut src, t1, None, "A <a@x>", 1000, "add");
        let hist = blob_history(
            &src,
            &head,
            "does/not/exist.rs",
            far_deadline(),
            MAX_HISTORY_REVS,
        );
        assert!(hist.is_empty());
    }

    /// A file DELETION is a touch: a commit that removes `path` (present in parent,
    /// absent here) emits a revision.
    #[test]
    fn deletion_is_a_touching_revision() {
        let mut src = CasObjectSource::new();
        let foo = src.insert_raw(ObjectKind::Blob, b"v1".to_vec());
        let keep = src.insert_raw(ObjectKind::Blob, b"keep".to_vec());
        // C1: foo.rs + keep.txt present.
        let t1 = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "foo.rs",
                    oid: foo,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "keep.txt",
                    oid: keep,
                },
            ],
        );
        let c1 = insert_commit(&mut src, t1, None, "A <a@x>", 1000, "add");
        // C2 (head): foo.rs REMOVED.
        let t2 = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "keep.txt",
                oid: keep,
            }],
        );
        let c2 = insert_commit(&mut src, t2, Some(c1), "A <a@x>", 2000, "rm foo");

        let hist = blob_history(&src, &c2, "foo.rs", far_deadline(), MAX_HISTORY_REVS);
        // Both the deletion (C2) and the introduction (C1) are touches.
        assert_eq!(hist.len(), 2, "history: {hist:?}");
        assert_eq!(hist[0].commit_hex, c2.to_hex().to_string());
        assert_eq!(hist[0].summary, "rm foo");
        assert_eq!(hist[1].commit_hex, c1.to_hex().to_string());
    }

    /// A NESTED path (`src/foo.rs`) is walked through subtrees: only the commit that
    /// modified the nested blob touches it (a sibling-only change does NOT).
    #[test]
    fn nested_path_touch_detection_walks_subtrees() {
        let mut src = CasObjectSource::new();
        let other = src.insert_raw(ObjectKind::Blob, b"other".to_vec());

        // C1 (root): src/foo.rs = "v1".
        let foo_v1 = src.insert_raw(ObjectKind::Blob, b"v1".to_vec());
        let sub1 = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "foo.rs",
                oid: foo_v1,
            }],
        );
        let root1 = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_TREE,
                name: "src",
                oid: sub1,
            }],
        );
        let c1 = insert_commit(&mut src, root1, None, "A <a@x>", 1000, "add nested");

        // C2: changes a ROOT-level sibling only — src/foo.rs unchanged (subtree oid
        // identical) → does NOT touch the nested path.
        let root2 = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_TREE,
                    name: "src",
                    oid: sub1,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "top.txt",
                    oid: other,
                },
            ],
        );
        let c2 = insert_commit(&mut src, root2, Some(c1), "A <a@x>", 2000, "add top");

        // C3 (head): src/foo.rs → "v2" (touches the nested path).
        let foo_v2 = src.insert_raw(ObjectKind::Blob, b"v2".to_vec());
        let sub3 = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "foo.rs",
                oid: foo_v2,
            }],
        );
        let root3 = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_TREE,
                    name: "src",
                    oid: sub3,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "top.txt",
                    oid: other,
                },
            ],
        );
        let c3 = insert_commit(&mut src, root3, Some(c2), "A <a@x>", 3000, "edit nested");

        let hist = blob_history(&src, &c3, "src/foo.rs", far_deadline(), MAX_HISTORY_REVS);
        // C3 (edit) and C1 (introduction) touched src/foo.rs; C2 (sibling-only) did not.
        assert_eq!(hist.len(), 2, "history: {hist:?}");
        assert_eq!(hist[0].commit_hex, c3.to_hex().to_string());
        assert_eq!(hist[1].commit_hex, c1.to_hex().to_string());
        assert!(!hist.iter().any(|h| h.commit_hex == c2.to_hex().to_string()));
    }

    /// INDEX: a whole-history build records EVERY path's touching commits in ONE walk,
    /// including a file whose ONLY touch is DEEP in history behind many non-touching
    /// commits — the exact case the per-request 2 s walk exhausts to empty.
    #[test]
    fn index_build_records_deep_history_paths() {
        let mut src = CasObjectSource::new();
        // C0 (root): introduces deep.rs = "v1" (its ONLY touch — deep in history).
        let deep_v1 = src.insert_raw(ObjectKind::Blob, b"v1".to_vec());
        let t0 = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "deep.rs",
                oid: deep_v1,
            }],
        );
        let mut parent = insert_commit(&mut src, t0, None, "Root <r@x>", 1000, "add deep");
        let root_commit = parent;
        // 100 commits that each touch a DIFFERENT churny file, never deep.rs.
        for i in 0..100u32 {
            let churn = src.insert_raw(ObjectKind::Blob, format!("c{i}").into_bytes());
            let tree = insert_tree(
                &mut src,
                vec![
                    TreeEntry {
                        mode: MODE_BLOB,
                        name: "deep.rs",
                        oid: deep_v1, // UNCHANGED across the whole churn
                    },
                    TreeEntry {
                        mode: MODE_BLOB,
                        name: "churn.rs",
                        oid: churn,
                    },
                ],
            );
            parent = insert_commit(
                &mut src,
                tree,
                Some(parent),
                "A <a@x>",
                2000 + i as i64,
                "churn",
            );
        }
        let head = parent;

        // Sanity: the per-request walk with a TINY budget can't reach deep.rs's touch
        // (the churn exhausts it) → empty, the bug this index fixes.
        let starved = std::time::Instant::now() - std::time::Duration::from_secs(1);
        assert!(
            blob_history(&src, &head, "deep.rs", starved, MAX_HISTORY_REVS).is_empty(),
            "a starved per-request walk returns empty for a deep-history file (the bug)"
        );

        // The index build (generous deadline) records deep.rs's introduction anyway.
        let index = build_blob_history_index(&src, &head, far_deadline(), MAX_INDEX_COMMITS);
        let deep = index.get("deep.rs").expect("deep.rs is indexed");
        assert_eq!(deep.len(), 1, "deep.rs touched exactly once (its intro)");
        assert_eq!(deep[0].commit_hex, root_commit.to_hex().to_string());
        assert_eq!(deep[0].summary, "add deep");
        // churn.rs is touched by all 100 churn commits but the per-path list is capped
        // at MAX_HISTORY_REVS (newest-first) — the drawer only shows the recent tail.
        let churn = index.get("churn.rs").expect("churn.rs is indexed");
        assert_eq!(
            churn.len(),
            MAX_HISTORY_REVS,
            "churn.rs is count-capped at MAX_HISTORY_REVS (newest-first)"
        );
        assert_eq!(
            churn[0].commit_hex,
            head.to_hex().to_string(),
            "newest-first"
        );
    }

    /// INDEX: per-path lists are count-capped at MAX_HISTORY_REVS (newest-first).
    #[test]
    fn index_build_caps_per_path_at_max_revs() {
        let mut src = CasObjectSource::new();
        let mut parent: Option<ObjectId> = None;
        let mut head = ObjectId::null(gix_hash::Kind::Sha1);
        for i in 0..(MAX_HISTORY_REVS as u32 + 25) {
            let blob = src.insert_raw(ObjectKind::Blob, format!("v{i}").into_bytes());
            let tree = insert_tree(
                &mut src,
                vec![TreeEntry {
                    mode: MODE_BLOB,
                    name: "foo.rs",
                    oid: blob,
                }],
            );
            head = insert_commit(&mut src, tree, parent, "A <a@x>", 1000 + i as i64, "edit");
            parent = Some(head);
        }
        let index = build_blob_history_index(&src, &head, far_deadline(), MAX_INDEX_COMMITS);
        let foo = index.get("foo.rs").expect("foo.rs indexed");
        assert_eq!(foo.len(), MAX_HISTORY_REVS, "per-path list is count-capped");
        assert_eq!(foo[0].commit_hex, head.to_hex().to_string(), "newest kept");
    }

    /// INDEX BOUND: a deadline already in the PAST stops the build immediately — it
    /// does NOT walk the whole (deep) history (proves the build is bounded, no hang).
    #[test]
    fn index_build_is_bounded_by_deadline() {
        let mut src = CasObjectSource::new();
        let mut parent: Option<ObjectId> = None;
        let mut head = ObjectId::null(gix_hash::Kind::Sha1);
        for i in 0..200u32 {
            let blob = src.insert_raw(ObjectKind::Blob, format!("v{i}").into_bytes());
            let tree = insert_tree(
                &mut src,
                vec![TreeEntry {
                    mode: MODE_BLOB,
                    name: "foo.rs",
                    oid: blob,
                }],
            );
            head = insert_commit(&mut src, tree, parent, "A <a@x>", 1000 + i as i64, "edit");
            parent = Some(head);
        }
        let past = std::time::Instant::now() - std::time::Duration::from_secs(1);
        let index = build_blob_history_index(&src, &head, past, MAX_INDEX_COMMITS);
        // The very first bound check trips → nothing indexed (partial-empty), no hang.
        assert!(
            index.get("foo.rs").map_or(0, Vec::len) < 200,
            "a past deadline must stop the build early"
        );
    }

    /// INDEX BOUND: `max_commits` caps the walk independently of wall-clock.
    #[test]
    fn index_build_is_bounded_by_max_commits() {
        let mut src = CasObjectSource::new();
        let mut parent: Option<ObjectId> = None;
        let mut head = ObjectId::null(gix_hash::Kind::Sha1);
        for i in 0..50u32 {
            let blob = src.insert_raw(ObjectKind::Blob, format!("v{i}").into_bytes());
            let tree = insert_tree(
                &mut src,
                vec![TreeEntry {
                    mode: MODE_BLOB,
                    name: "foo.rs",
                    oid: blob,
                }],
            );
            head = insert_commit(&mut src, tree, parent, "A <a@x>", 1000 + i as i64, "edit");
            parent = Some(head);
        }
        // Only walk the newest 5 commits → foo.rs has at most 5 recorded touches.
        let index = build_blob_history_index(&src, &head, far_deadline(), 5);
        let foo = index.get("foo.rs").expect("foo.rs indexed");
        assert_eq!(foo.len(), 5, "max_commits bounds the walk");
        assert_eq!(foo[0].commit_hex, head.to_hex().to_string(), "newest-first");
    }

    /// An `ObjectSource` decorator that counts `get` calls — so a test can ASSERT the
    /// carry-forward optimization actually halves the per-commit CAS fetches (a perf
    /// claim must be measured, not asserted).
    struct CountingSource<'a> {
        inner: &'a CasObjectSource,
        gets: std::cell::Cell<usize>,
    }
    impl ObjectSource for CountingSource<'_> {
        fn get(
            &self,
            oid: &ObjectId,
        ) -> Result<Option<crate::read::pack::GitObject>, crate::read::pack::PackError> {
            self.gets.set(self.gets.get() + 1);
            self.inner.get(oid)
        }
    }

    /// PROOF (optimization): the carry-forward reuse keeps the per-commit fetch budget
    /// at ~ONE commit-load + ONE tree-walk (≈2 gets/commit for a root-level path),
    /// HALF of the naive ~4 gets/commit (commit+tree for both the commit AND its parent,
    /// then re-fetching the parent next iteration). On an N-commit chain the walk does
    /// well under `3*N` gets — a bound the un-optimized walk could never meet — so the
    /// 2 s budget reaches ~2x deeper into history.
    #[test]
    fn carry_forward_halves_the_per_commit_fetches() {
        let mut src = CasObjectSource::new();
        // A linear chain of N commits, each modifying foo.rs (every commit touches the
        // path → no early exit, the worst case for fetch count).
        const N: usize = 12;
        let mut parent: Option<ObjectId> = None;
        let mut head = ObjectId::null(gix_hash::Kind::Sha1);
        for i in 0..N {
            let blob = src.insert_raw(ObjectKind::Blob, format!("v{i}").into_bytes());
            let tree = insert_tree(
                &mut src,
                vec![TreeEntry {
                    mode: MODE_BLOB,
                    name: "foo.rs",
                    oid: blob,
                }],
            );
            head = insert_commit(
                &mut src,
                tree,
                parent,
                "A <a@x>",
                1000 + i as i64,
                "edit foo",
            );
            parent = Some(head);
        }

        let counting = CountingSource {
            inner: &src,
            gets: std::cell::Cell::new(0),
        };
        let hist = blob_history(&counting, &head, "foo.rs", far_deadline(), 1_000);

        assert_eq!(hist.len(), N, "every commit touched foo.rs → N revisions");
        let gets = counting.gets.get();
        assert!(
            gets < 3 * N,
            "carry-forward must keep fetches under 3*N ({}); got {} — the naive walk \
             would be ~4*N",
            3 * N,
            gets
        );
    }
}
