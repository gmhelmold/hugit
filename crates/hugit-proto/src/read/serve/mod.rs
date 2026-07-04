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

use std::collections::{BTreeMap, BTreeSet};
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

/// Serve a **shallow** (depth-bounded) clone — a client's `git clone --depth N`.
///
/// Like [`serve_clone`] but the commit ancestry reachable from each tip is
/// truncated at `depth` commits. Returns the advertisement, the depth-bounded
/// pack, AND the set of SHALLOW boundary commits (an included commit that has a
/// parent cut from the pack). The serve layer emits one `shallow <oid>` pkt-line
/// per boundary commit before the pack (the v1 shallow-info section) so the client
/// records its shallow frontier.
///
/// Depth semantics (git `--depth N`, N ≥ 1): the tips are depth 1; commits are
/// included through depth N. A commit is shallow iff at least one of its parents
/// is NOT in the included set — git-correct across merges, and a root commit (no
/// parents) is never shallow. `depth == 0` is treated as `1` (git rejects it).
///
/// The tree/blob closure of EVERY included commit is packed in full (a shallow
/// clone still checks out a complete working tree); only the *ancestry* is bounded.
pub fn serve_clone_shallow(
    view: &dyn RefView,
    source: &dyn ObjectSource,
    depth: u32,
) -> Result<(RefAdvertisement, PackAssembly, Vec<ObjectId>), ServeError> {
    let adv = RefAdvertisement::from_view(view);
    let wants = adv.tip_oids()?;
    let (pack, boundary) = serve_shallow(source, &wants, depth)?;
    Ok((adv, pack, boundary))
}

/// The depth-bounded pack for an EXPLICIT want-set (the serve layer's path).
///
/// `git clone --depth N` implies `--single-branch`, so the client sends `want` for
/// only the branch(es) it tracks — this assembles the shallow pack from exactly
/// those validated wants (NOT every advertised tip). Returns the pack and the
/// shallow boundary commits (see [`serve_clone_shallow`]).
pub fn serve_shallow(
    source: &dyn ObjectSource,
    wants: &[ObjectId],
    depth: u32,
) -> Result<(PackAssembly, Vec<ObjectId>), ServeError> {
    let deadline = Instant::now() + SERVE_FETCH_BUDGET;
    let (oids, boundary) = collect_shallow(source, wants, depth, deadline)?;
    source.prefetch(&oids);
    let pack = assemble_pack_until(source, &oids, Some(deadline))?;
    Ok((pack, boundary))
}

/// The depth-bounded pack for a request that carries an EXPLICIT shallow boundary
/// (the client's `shallow <oid>` lines) instead of a `deepen` — round 2 of a stateless
/// HTTP shallow clone. The commit walk from `wants` is cut at any commit in `boundary`
/// (that commit + its tree are packed; its parents are NOT). Returns the pack and the
/// EFFECTIVE boundary (the boundary commits actually reached), which the serve layer
/// echoes back as the `shallow` section. See [`parse_client_shallow`](crate::parse_client_shallow).
pub fn serve_shallow_at(
    source: &dyn ObjectSource,
    wants: &[ObjectId],
    boundary: &[ObjectId],
) -> Result<(PackAssembly, Vec<ObjectId>), ServeError> {
    let deadline = Instant::now() + SERVE_FETCH_BUDGET;
    let bset: BTreeSet<ObjectId> = boundary.iter().copied().collect();
    let (oids, effective) = collect_shallow_at(source, wants, &bset, deadline)?;
    source.prefetch(&oids);
    let pack = assemble_pack_until(source, &oids, Some(deadline))?;
    Ok((pack, effective))
}

/// Depth-bounded object closure for a shallow clone. Two phases:
///
/// 1. **Commit BFS bounded by `depth`** — from each want (a tip; a tag is peeled
///    to its target at the SAME depth), walk parents breadth-first, enqueuing a
///    commit's parents only while its depth `< depth`. Yields the INCLUDED commit
///    set (depths `1..=depth`) and each included commit's parent list.
/// 2. **Boundary + closure** — a commit is SHALLOW iff a parent is not included
///    (git-correct across merges; a root commit is never shallow). Every included
///    commit's full tree/blob closure (+ any peeled tag objects) is collected.
///
/// Returns `(all_object_oids_sorted, shallow_boundary_commits_sorted)`. Bounded by
/// `deadline`, fail-CLEAN (never a partial closure — same invariant as the full walk).
fn collect_shallow(
    source: &dyn ObjectSource,
    wants: &[ObjectId],
    depth: u32,
    deadline: Instant,
) -> Result<(Vec<ObjectId>, Vec<ObjectId>), ServeError> {
    let depth = depth.max(1);

    // Phase 1: depth-bounded commit BFS. `commit_parents` records ancestry so
    // phase 2 decides shallow-ness with no extra fetch.
    let mut commit_parents: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
    let mut commit_trees: Vec<ObjectId> = Vec::new();
    let mut tag_objects: Vec<ObjectId> = Vec::new();
    let mut extra_roots: Vec<ObjectId> = Vec::new();
    // frontier of (oid, depth-from-tip). A tag peels to its target at the same depth.
    let mut frontier: Vec<(ObjectId, u32)> = wants.iter().map(|w| (*w, 1)).collect();
    while !frontier.is_empty() {
        if Instant::now() >= deadline {
            return Err(ServeError::DeadlineExceeded);
        }
        let batch: Vec<ObjectId> = frontier.iter().map(|(o, _)| *o).collect();
        source.prefetch(&batch);
        let mut next: Vec<(ObjectId, u32)> = Vec::new();
        for (oid, d) in std::mem::take(&mut frontier) {
            if Instant::now() >= deadline {
                return Err(ServeError::DeadlineExceeded);
            }
            let object = match source.get(&oid)? {
                Some(o) => o,
                None => return Err(ServeError::IncompleteClosure(oid)),
            };
            match object.kind {
                ObjectKind::Commit => {
                    if commit_parents.contains_key(&oid) {
                        continue; // already walked (merge / shared ancestor)
                    }
                    let tree = CommitRefIter::from_bytes(&object.data)
                        .tree_id()
                        .map_err(|source| ServeError::Decode { oid, source })?;
                    commit_trees.push(tree);
                    let parents: Vec<ObjectId> = CommitRefIter::from_bytes(&object.data)
                        .parent_ids()
                        .collect();
                    if d < depth {
                        for p in &parents {
                            next.push((*p, d + 1));
                        }
                    }
                    commit_parents.insert(oid, parents);
                }
                ObjectKind::Tag => {
                    // A tag is not a commit level: pack the tag object, peel to its
                    // target, and continue at the SAME depth.
                    tag_objects.push(oid);
                    let target = TagRefIter::from_bytes(&object.data)
                        .target_id()
                        .map_err(|source| ServeError::Decode { oid, source })?;
                    next.push((target, d));
                }
                // A want pointing straight at a tree/blob (unusual for a clone) — its
                // closure is collected wholesale in phase 2.
                ObjectKind::Tree | ObjectKind::Blob => extra_roots.push(oid),
            }
        }
        frontier = next;
    }

    // Phase 2: object closure.
    let mut included: BTreeSet<ObjectId> = BTreeSet::new();
    for c in commit_parents.keys() {
        included.insert(*c);
    }
    for t in &tag_objects {
        included.insert(*t);
    }
    for tree in &commit_trees {
        collect_reachable(source, tree, &mut included, /*strict=*/ true, deadline)?;
    }
    for root in &extra_roots {
        collect_reachable(source, root, &mut included, /*strict=*/ true, deadline)?;
    }

    // Shallow boundary: an included commit with a parent NOT in the included set.
    let mut boundary: BTreeSet<ObjectId> = BTreeSet::new();
    for (commit, parents) in &commit_parents {
        if parents.iter().any(|p| !commit_parents.contains_key(p)) {
            boundary.insert(*commit);
        }
    }

    Ok((
        included.into_iter().collect(),
        boundary.into_iter().collect(),
    ))
}

/// Like [`collect_shallow`] but the commit walk is cut at an EXPLICIT `boundary`
/// (the client's `shallow <oid>` set from round 2) instead of a depth: a commit in
/// `boundary` is packed WITH its tree closure but its parents are NOT walked. Returns
/// `(all_object_oids_sorted, effective_boundary_sorted)` where the effective boundary
/// is the boundary commits actually reached from `wants` (echoed back as `shallow`).
fn collect_shallow_at(
    source: &dyn ObjectSource,
    wants: &[ObjectId],
    boundary: &BTreeSet<ObjectId>,
    deadline: Instant,
) -> Result<(Vec<ObjectId>, Vec<ObjectId>), ServeError> {
    let mut commit_parents: BTreeMap<ObjectId, Vec<ObjectId>> = BTreeMap::new();
    let mut commit_trees: Vec<ObjectId> = Vec::new();
    let mut tag_objects: Vec<ObjectId> = Vec::new();
    let mut extra_roots: Vec<ObjectId> = Vec::new();
    let mut effective_boundary: BTreeSet<ObjectId> = BTreeSet::new();
    let mut frontier: Vec<ObjectId> = wants.to_vec();
    while !frontier.is_empty() {
        if Instant::now() >= deadline {
            return Err(ServeError::DeadlineExceeded);
        }
        source.prefetch(&frontier);
        let mut next: Vec<ObjectId> = Vec::new();
        for oid in std::mem::take(&mut frontier) {
            if Instant::now() >= deadline {
                return Err(ServeError::DeadlineExceeded);
            }
            let object = match source.get(&oid)? {
                Some(o) => o,
                None => return Err(ServeError::IncompleteClosure(oid)),
            };
            match object.kind {
                ObjectKind::Commit => {
                    if commit_parents.contains_key(&oid) {
                        continue;
                    }
                    let tree = CommitRefIter::from_bytes(&object.data)
                        .tree_id()
                        .map_err(|source| ServeError::Decode { oid, source })?;
                    commit_trees.push(tree);
                    let parents: Vec<ObjectId> = CommitRefIter::from_bytes(&object.data)
                        .parent_ids()
                        .collect();
                    if boundary.contains(&oid) {
                        // A client-declared shallow cut: pack this commit, stop at its parents.
                        effective_boundary.insert(oid);
                    } else {
                        for p in &parents {
                            next.push(*p);
                        }
                    }
                    commit_parents.insert(oid, parents);
                }
                ObjectKind::Tag => {
                    tag_objects.push(oid);
                    let target = TagRefIter::from_bytes(&object.data)
                        .target_id()
                        .map_err(|source| ServeError::Decode { oid, source })?;
                    next.push(target);
                }
                ObjectKind::Tree | ObjectKind::Blob => extra_roots.push(oid),
            }
        }
        frontier = next;
    }

    let mut included: BTreeSet<ObjectId> = BTreeSet::new();
    for c in commit_parents.keys() {
        included.insert(*c);
    }
    for t in &tag_objects {
        included.insert(*t);
    }
    for tree in &commit_trees {
        collect_reachable(source, tree, &mut included, /*strict=*/ true, deadline)?;
    }
    for root in &extra_roots {
        collect_reachable(source, root, &mut included, /*strict=*/ true, deadline)?;
    }

    Ok((
        included.into_iter().collect(),
        effective_boundary.into_iter().collect(),
    ))
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

    // ── shallow (depth-bounded) clone ────────────────────────────────────────

    fn commit_p(src: &mut CasObjectSource, tree_oid: ObjectId, parents: &[ObjectId]) -> ObjectId {
        let mut body = format!("tree {tree_oid}\n");
        for p in parents {
            body.push_str(&format!("parent {p}\n"));
        }
        body.push_str("author a <a@a> 0 +0000\ncommitter a <a@a> 0 +0000\n\nmsg\n");
        src.insert(GitObject::new(ObjectKind::Commit, body.into_bytes()))
    }

    /// A linear 3-commit chain `c0`(root) ← `c1` ← `c2`(tip), each carrying its own
    /// distinct tree + blob (so a depth-N pack has exactly `3 × N` objects).
    fn chain3() -> (CasObjectSource, ObjectId, ObjectId, ObjectId) {
        let mut src = CasObjectSource::new();
        let b0 = blob(&mut src, "v0\n");
        let t0 = tree(&mut src, &[("100644", "f.txt", b0)]);
        let c0 = commit_p(&mut src, t0, &[]);
        let b1 = blob(&mut src, "v1\n");
        let t1 = tree(&mut src, &[("100644", "f.txt", b1)]);
        let c1 = commit_p(&mut src, t1, &[c0]);
        let b2 = blob(&mut src, "v2\n");
        let t2 = tree(&mut src, &[("100644", "f.txt", b2)]);
        let c2 = commit_p(&mut src, t2, &[c1]);
        (src, c0, c1, c2)
    }

    fn refs_for(tip: ObjectId) -> std::collections::BTreeMap<String, String> {
        let mut r = std::collections::BTreeMap::new();
        r.insert("refs/heads/main".to_string(), tip.to_string());
        r
    }

    /// `--depth 1`: pack ONLY the tip's snapshot (commit + its tree + blob), and
    /// mark the tip as the shallow boundary (its parent is cut).
    #[test]
    fn shallow_depth_1_packs_only_tip_snapshot_and_marks_boundary() {
        let (src, _c0, _c1, c2) = chain3();
        let (_adv, pack, boundary) =
            serve_clone_shallow(&refs_for(c2), &src, 1).expect("a depth-1 clone succeeds");
        assert_eq!(
            pack.object_count(),
            3,
            "depth 1 packs only the tip commit + its tree + blob (no ancestry)"
        );
        assert_eq!(
            boundary,
            vec![c2],
            "the tip is shallow — its parent is cut from the pack"
        );
    }

    /// `--depth 2`: two commits deep; the boundary moves back to `c1` (whose parent
    /// `c0` is cut), and `c2` is NOT a boundary (its parent `c1` is included).
    #[test]
    fn shallow_depth_2_includes_two_commits_boundary_moves_back() {
        let (src, _c0, c1, c2) = chain3();
        let (_adv, pack, boundary) =
            serve_clone_shallow(&refs_for(c2), &src, 2).expect("a depth-2 clone succeeds");
        assert_eq!(
            pack.object_count(),
            6,
            "two commits, each with its tree + blob"
        );
        assert_eq!(
            boundary,
            vec![c1],
            "c1's parent (c0) is cut → c1 shallow; c2's parent (c1) is included → not shallow"
        );
    }

    /// A depth covering the whole history has NO shallow boundary, and its object
    /// set is byte-identical to a full clone (the root commit is never shallow).
    #[test]
    fn shallow_depth_covering_full_history_equals_full_clone_no_boundary() {
        let (src, _c0, _c1, c2) = chain3();
        let (_adv, pack, boundary) =
            serve_clone_shallow(&refs_for(c2), &src, 5).expect("a deep clone succeeds");
        assert!(
            boundary.is_empty(),
            "the whole history is reached → nothing is shallow: {boundary:?}"
        );
        let (_a, full) = serve_clone(&refs_for(c2), &src).expect("a full clone succeeds");
        assert_eq!(
            pack.object_count(),
            full.object_count(),
            "a depth ≥ history is the full closure (9 objects: 3 commits × tree+blob)"
        );
        assert_eq!(pack.object_count(), 9);
    }

    /// `--depth 0` is rejected by git; the server treats it as depth 1 (never an
    /// empty or unbounded pack).
    #[test]
    fn shallow_depth_0_is_treated_as_depth_1() {
        let (src, _c0, _c1, c2) = chain3();
        let (_adv, pack, boundary) =
            serve_clone_shallow(&refs_for(c2), &src, 0).expect("depth 0 → 1");
        assert_eq!(pack.object_count(), 3);
        assert_eq!(boundary, vec![c2]);
    }

    /// `serve_shallow_at` (the round-2 path) cuts the walk at the CLIENT-declared
    /// boundary instead of a depth: a client shallow at the tip → the pack is only the
    /// tip snapshot, and the tip is echoed back as the effective boundary.
    #[test]
    fn serve_shallow_at_cuts_at_the_client_declared_boundary() {
        let (src, _c0, _c1, c2) = chain3();
        let (pack, eff) = serve_shallow_at(&src, &[c2], &[c2]).expect("a boundary serve succeeds");
        assert_eq!(
            pack.object_count(),
            3,
            "cut at the tip → only the tip commit + its tree + blob"
        );
        assert_eq!(
            eff,
            vec![c2],
            "the reached client-shallow commit is the boundary"
        );
    }
}
