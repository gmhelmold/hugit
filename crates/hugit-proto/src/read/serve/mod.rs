//! Clone + delta-only fetch entrypoints.
//!
//! These tie protocol-v2 negotiation ([`crate::read::negotiate`]) to pack
//! assembly from CAS ([`crate::read::pack`]):
//!
//! - [`serve_clone`] — want every advertised tip, have nothing → a pack of the
//!   full reachable object closure. Byte-identical to the mirror (D2 item ①).
//! - [`serve_fetch`] — want the new tips, have the client's frontier → a pack of
//!   **only** the objects reachable from the wants but NOT already reachable
//!   from the haves. Delta-only (D2 item ②).
//!
//! Reachability is walked with the git library's streaming object decoders
//! (`gix-object`); the object graph traversal is standard git, not reinvented.

use std::collections::BTreeSet;
use std::time::Instant;

use gix_hash::ObjectId;
use gix_object::{CommitRefIter, TagRefIter, TreeRefIter};

use crate::read::negotiate::{NegotiationError, RefAdvertisement, RefView, WantHave};
use crate::read::pack::{ObjectKind, ObjectSource, PackAssembly, PackError, assemble_pack_until};

/// Generous wall-clock ceiling on a single clone/fetch ([`serve_fetch`] /
/// [`serve_clone`]).
///
/// The reachability walk + pack assembly touch every reachable object. Against a
/// per-object CAS that was up to `2 × #objects` synchronous (R2) GETs; the walk now
/// [`ObjectSource::prefetch`]es each frontier and the closure is prefetched before
/// assembly, so a lazy CAS source resolves them in O(objects / chunk) BATCH reads —
/// a large clone finishes in seconds, not minutes.
///
/// **This budget bounds the WORKER, not the accept loop.** The `hugit-serve`
/// upload-pack handler runs the whole [`serve_fetch`] OFF the single accept thread
/// on a DETACHED worker that owns the client connection and responds itself; the
/// accept loop hands off and returns to accept IMMEDIATELY (it keeps serving
/// `/readyz` + other requests) and NEVER waits on this budget. So this deadline is
/// the worker's SOLE bound, and it plays exactly one role:
///
/// * It caps a truly runaway / cold-CAS walk so the detached worker eventually
///   gives up (fail-clean → the client gets no pack) instead of walking forever and
///   leaking a thread. Because the loop no longer imposes any shorter outer cutoff,
///   this is the ONLY thing that lets a real, legitimately slow clone COMPLETE — so
///   it is chosen GENEROUS (300 s). With the batch-prefetch walk a real hugit clone
///   (~6862 objects) transfers in a few SECONDS — two orders of magnitude under this
///   budget — so it can never trip a legitimate clone (the batching is what closed
///   the earlier per-object regression that 404'd a large clone at a too-tight 45 s).
///
/// FAIL-CLEAN, NOT truncate — a clone/fetch pack MUST be complete. A packfile
/// missing an object reachable from a `want` is a CORRUPT clone (git aborts
/// mid-checkout, or the client believes it holds a history it does not). So
/// exceeding this deadline ABORTS the whole fetch with
/// [`ServeError::DeadlineExceeded`] (the handler maps it to no-pack, never a
/// half-written one). A legitimate clone that finishes under the budget is
/// byte-identical and unaffected; only a pathologically large or cold-CAS-slow walk
/// is bounded.
pub const SERVE_FETCH_BUDGET: std::time::Duration = std::time::Duration::from_secs(300);

/// Errors raised while serving a clone or fetch.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// Negotiation (ref advertisement / want-have parsing) failed.
    #[error(transparent)]
    Negotiation(#[from] NegotiationError),
    /// Reading an object from CAS or assembling the pack failed.
    #[error(transparent)]
    Pack(#[from] PackError),
    /// A reachable object referenced from the graph was absent from CAS —
    /// fail-closed: an incomplete closure can never be served as a clone.
    #[error("reachable object {0} missing from CAS")]
    IncompleteClosure(ObjectId),
    /// A git object failed to decode during the reachability walk.
    #[error("object decode failed for {oid}: {source}")]
    Decode {
        /// The object that failed to decode.
        oid: ObjectId,
        /// The underlying decode error.
        source: gix_object::decode::Error,
    },
    /// The clone/fetch exceeded its wall-clock budget ([`SERVE_FETCH_BUDGET`]).
    /// Fail-CLEAN: the fetch is ABORTED (never served as a truncated/corrupt pack)
    /// so a runaway walk cannot wedge the single-threaded engine's accept loop.
    #[error("clone/fetch exceeded its wall-clock budget")]
    DeadlineExceeded,
}

/// Serve a full clone against a [`RefView`] (the D1 derived view) and a
/// CAS-backed object [`ObjectSource`].
///
/// Returns the [`RefAdvertisement`] the client negotiated against and the
/// assembled [`PackAssembly`] carrying every object reachable from the tips.
pub fn serve_clone(
    view: &dyn RefView,
    source: &dyn ObjectSource,
) -> Result<(RefAdvertisement, PackAssembly), ServeError> {
    let adv = RefAdvertisement::from_view(view);
    let request = WantHave::clone_all(&adv)?;
    let pack = serve_request(source, &request, Instant::now() + SERVE_FETCH_BUDGET)?;
    Ok((adv, pack))
}

/// Serve a delta-only fetch: assemble a pack of exactly the objects reachable
/// from `request.wants` minus those reachable from `request.haves`.
///
/// When the client already holds everything the wants reach, the pack is empty
/// (zero objects) — no full re-send (D2 item ②).
pub fn serve_fetch(
    source: &dyn ObjectSource,
    request: &WantHave,
) -> Result<PackAssembly, ServeError> {
    serve_fetch_until(source, request, Instant::now() + SERVE_FETCH_BUDGET)
}

/// Like [`serve_fetch`] but with an explicit wall-clock `deadline` shared across
/// the reachability walk AND the pack assembly. Exceeding it aborts the WHOLE
/// fetch with [`ServeError::DeadlineExceeded`] — fail-CLEAN, never a truncated
/// pack (see [`SERVE_FETCH_BUDGET`]). Deterministically testable: a deadline
/// already in the past aborts before any object is fetched.
pub fn serve_fetch_until(
    source: &dyn ObjectSource,
    request: &WantHave,
    deadline: Instant,
) -> Result<PackAssembly, ServeError> {
    serve_request(source, request, deadline)
}

/// The shared core: compute the want-minus-have object closure and assemble it,
/// bounded by a SINGLE wall-clock `deadline` shared across the walk + assembly so
/// the whole request is bounded (not each phase independently). Fail-CLEAN on the
/// deadline (abort, never truncate).
fn serve_request(
    source: &dyn ObjectSource,
    request: &WantHave,
    deadline: Instant,
) -> Result<PackAssembly, ServeError> {
    // 1. Everything the client already has (its negotiation frontier closure) is
    //    excluded — these objects are never re-sent.
    let mut excluded = BTreeSet::new();
    for have in &request.haves {
        // A `have` the server doesn't hold contributes nothing to exclusion.
        collect_reachable(
            source,
            have,
            &mut excluded,
            /*strict=*/ false,
            deadline,
        )?;
    }

    // 2. Walk the wants, collecting reachable objects not already excluded.
    let mut included = BTreeSet::new();
    for want in &request.wants {
        collect_reachable_excluding(source, want, &excluded, &mut included, deadline)?;
    }

    // 3. Deterministic pack order: object id ascending (BTreeSet is sorted). The
    //    SAME deadline bounds assembly — a fetch that spent its budget on the walk
    //    fails clean here rather than start emitting a pack it can't finish.
    let oids: Vec<ObjectId> = included.into_iter().collect();
    // Batch-warm the WHOLE closure before assembly. The walk already cached most of
    // it, but a byte-bounded lazy cache may have evicted early frontiers on a large
    // closure — one final batch prefetch keeps assembly's re-reads O(objects/chunk)
    // rather than O(objects) cold GETs. Best-effort: correctness is in `assemble`'s
    // per-object `get` (a not-warmed object simply takes the cold, fail-closed path).
    source.prefetch(&oids);
    let pack = assemble_pack_until(source, &oids, Some(deadline))?;
    Ok(pack)
}

/// Collect every object reachable from `root` into `out`. With `strict`, a
/// missing object is an error (the closure must be complete); without it, a
/// missing root is silently skipped (a `have` we don't recognize). `deadline`
/// bounds the walk (fail-clean; see [`SERVE_FETCH_BUDGET`]).
fn collect_reachable(
    source: &dyn ObjectSource,
    root: &ObjectId,
    out: &mut BTreeSet<ObjectId>,
    strict: bool,
    deadline: Instant,
) -> Result<(), ServeError> {
    let empty = BTreeSet::new();
    collect_inner(source, root, &empty, out, strict, deadline)
}

/// Collect every object reachable from `root` that is NOT in `excluded`.
fn collect_reachable_excluding(
    source: &dyn ObjectSource,
    root: &ObjectId,
    excluded: &BTreeSet<ObjectId>,
    out: &mut BTreeSet<ObjectId>,
    deadline: Instant,
) -> Result<(), ServeError> {
    collect_inner(source, root, excluded, out, /*strict=*/ true, deadline)
}

/// Level-batched BFS over the git object graph from `root`.
///
/// - commit → its tree + parents
/// - tree → its (non-gitlink) entries
/// - tag → its target
/// - blob → leaf
///
/// Objects in `excluded` (and their subgraphs) are pruned. Visiting stops at
/// objects already in `out` so shared subgraphs are walked once.
///
/// The walk proceeds by **frontier** (breadth-first), not depth-first: every
/// object in the current frontier is [`ObjectSource::prefetch`]ed in ONE batch
/// before any is read, so a lazy CAS source resolves a whole level in a handful of
/// bulk reads instead of one blocking HTTP GET per object. The reachable SET this
/// produces is IDENTICAL to a depth-first walk (the caller sorts `out` by oid before
/// packing), so the assembled pack stays byte-for-byte unchanged — only the fetch
/// *batching* differs.
fn collect_inner(
    source: &dyn ObjectSource,
    root: &ObjectId,
    excluded: &BTreeSet<ObjectId>,
    out: &mut BTreeSet<ObjectId>,
    strict: bool,
    deadline: Instant,
) -> Result<(), ServeError> {
    let mut frontier = vec![*root];
    while !frontier.is_empty() {
        // WALL-CLOCK guard BEFORE the batch prefetch/reads: abort CLEAN (Err, never a
        // partial closure that would assemble into a truncated/corrupt pack) if the
        // shared budget is spent. Checked once per frontier AND once per object below,
        // so a deadline already in the past aborts before ANY fetch (unchanged
        // invariant).
        if Instant::now() >= deadline {
            return Err(ServeError::DeadlineExceeded);
        }
        // Dedupe this frontier against what is already excluded/collected (and against
        // itself) so the prefetch batch — and the reads — touch each oid at most once.
        let mut wave_seen = BTreeSet::new();
        let batch: Vec<ObjectId> = frontier
            .iter()
            .copied()
            .filter(|oid| !excluded.contains(oid) && !out.contains(oid) && wave_seen.insert(*oid))
            .collect();

        // Batch-warm the whole frontier in one (internally chunked) round-trip; a
        // lazy CAS source turns this into O(frontier/chunk) bulk reads. No-op for an
        // in-memory source. Purely a warm-up — every `get` below is still authoritative.
        source.prefetch(&batch);

        let mut next: Vec<ObjectId> = Vec::new();
        for oid in batch {
            // Per-object wall-clock guard (parity with the original per-`get` check).
            if Instant::now() >= deadline {
                return Err(ServeError::DeadlineExceeded);
            }
            // A sibling earlier in THIS frontier may already have collected `oid`.
            if excluded.contains(&oid) || out.contains(&oid) {
                continue;
            }
            let object = match source.get(&oid)? {
                Some(o) => o,
                None => {
                    if strict {
                        return Err(ServeError::IncompleteClosure(oid));
                    }
                    continue;
                }
            };
            out.insert(oid);
            match object.kind {
                ObjectKind::Blob => {}
                ObjectKind::Commit => {
                    let mut iter = CommitRefIter::from_bytes(&object.data);
                    let tree = iter
                        .tree_id()
                        .map_err(|source| ServeError::Decode { oid, source })?;
                    push_unseen(tree, excluded, out, &mut next);
                    // `parent_ids` consumes the iterator; re-create for the walk.
                    let parents = CommitRefIter::from_bytes(&object.data).parent_ids();
                    for parent in parents {
                        push_unseen(parent, excluded, out, &mut next);
                    }
                }
                ObjectKind::Tree => {
                    let entries = TreeRefIter::from_bytes(&object.data)
                        .entries()
                        .map_err(|source| ServeError::Decode { oid, source })?;
                    for entry in entries {
                        // Gitlinks (submodule commits) live in another repo's CAS —
                        // never part of this repo's closure.
                        if entry.mode.is_commit() {
                            continue;
                        }
                        push_unseen(entry.oid.to_owned(), excluded, out, &mut next);
                    }
                }
                ObjectKind::Tag => {
                    let target = TagRefIter::from_bytes(&object.data)
                        .target_id()
                        .map_err(|source| ServeError::Decode { oid, source })?;
                    push_unseen(target, excluded, out, &mut next);
                }
            }
        }
        frontier = next;
    }
    Ok(())
}

/// Push `oid` onto the next BFS frontier unless it's excluded or already collected.
/// Duplicates within a frontier are re-deduped when that frontier is processed.
fn push_unseen(
    oid: ObjectId,
    excluded: &BTreeSet<ObjectId>,
    out: &BTreeSet<ObjectId>,
    next: &mut Vec<ObjectId>,
) {
    if !excluded.contains(&oid) && !out.contains(&oid) {
        next.push(oid);
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;
    use crate::read::pack::{CasObjectSource, GitObject};
    use std::time::Duration;

    fn blob(src: &mut CasObjectSource, body: &str) -> ObjectId {
        src.insert(GitObject::new(ObjectKind::Blob, body.as_bytes().to_vec()))
    }

    fn tree(src: &mut CasObjectSource, entries: &[(&str, &str, ObjectId)]) -> ObjectId {
        let mut es = entries.to_vec();
        es.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));
        let mut out = Vec::new();
        for (mode, name, oid) in &es {
            out.extend_from_slice(mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(oid.as_bytes());
        }
        src.insert(GitObject::new(ObjectKind::Tree, out))
    }

    fn commit(src: &mut CasObjectSource, tree_oid: ObjectId) -> ObjectId {
        let body =
            format!("tree {tree_oid}\nauthor a <a@a> 0 +0000\ncommitter a <a@a> 0 +0000\n\nmsg\n");
        src.insert(GitObject::new(ObjectKind::Commit, body.into_bytes()))
    }

    /// A trivial commit→tree→blob graph and its tip.
    fn sample() -> (CasObjectSource, ObjectId) {
        let mut src = CasObjectSource::new();
        let b = blob(&mut src, "hello\n");
        let t = tree(&mut src, &[("100644", "f.txt", b)]);
        let c = commit(&mut src, t);
        (src, c)
    }

    /// A normal fetch that finishes under the budget succeeds UNCHANGED — the
    /// full closure (commit + tree + blob) is packed. The deadline bounds latency;
    /// it never breaks a legit clone.
    #[test]
    fn fetch_under_budget_completes_unchanged() {
        let (src, c) = sample();
        let request = WantHave {
            wants: vec![c],
            haves: Vec::new(),
            done: true,
        };
        let pack = serve_fetch(&src, &request).expect("a small fetch under budget succeeds");
        assert_eq!(
            pack.object_count(),
            3,
            "commit + tree + blob must all be packed (no truncation)"
        );
        // A future (generous) explicit deadline behaves identically.
        let pack2 = serve_fetch_until(&src, &request, Instant::now() + Duration::from_secs(30))
            .expect("a generous deadline still completes");
        assert_eq!(pack2.object_count(), 3);
    }

    /// A fetch past its deadline FAILS CLEAN — `Err(DeadlineExceeded)`, NEVER a
    /// truncated/partial pack. A deadline already in the past aborts before any
    /// object is fetched (deterministic, no timing flake). This is the wedge guard:
    /// a runaway walk errors instead of blocking the single-threaded accept loop.
    #[test]
    fn past_deadline_fetch_fails_clean_never_a_partial_pack() {
        let (src, c) = sample();
        let request = WantHave {
            wants: vec![c],
            haves: Vec::new(),
            done: true,
        };
        let past = Instant::now() - Duration::from_secs(1);
        let err = serve_fetch_until(&src, &request, past)
            .expect_err("a past-deadline fetch must fail CLEAN, never yield a pack");
        assert!(
            matches!(err, ServeError::DeadlineExceeded),
            "the abort must be the deadline error (fail-clean, not a truncated pack): {err:?}"
        );
    }

    /// serve_clone is bounded too: a clone with a graph in the store completes.
    #[test]
    fn clone_under_budget_completes() {
        let (src, c) = sample();
        let mut refs = std::collections::BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), c.to_string());
        let (_adv, pack) = serve_clone(&refs, &src).expect("a small clone under budget succeeds");
        assert_eq!(pack.object_count(), 3);
    }
}
