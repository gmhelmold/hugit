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

use gix_hash::ObjectId;
use gix_object::{CommitRefIter, TagRefIter, TreeRefIter};

use crate::read::negotiate::{NegotiationError, RefAdvertisement, RefView, WantHave};
use crate::read::pack::{ObjectKind, ObjectSource, PackAssembly, PackError, assemble_pack};

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
    let pack = serve_request(source, &request)?;
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
    serve_request(source, request)
}

/// The shared core: compute the want-minus-have object closure and assemble it.
fn serve_request(
    source: &dyn ObjectSource,
    request: &WantHave,
) -> Result<PackAssembly, ServeError> {
    // 1. Everything the client already has (its negotiation frontier closure) is
    //    excluded — these objects are never re-sent.
    let mut excluded = BTreeSet::new();
    for have in &request.haves {
        // A `have` the server doesn't hold contributes nothing to exclusion.
        collect_reachable(source, have, &mut excluded, /*strict=*/ false)?;
    }

    // 2. Walk the wants, collecting reachable objects not already excluded.
    let mut included = BTreeSet::new();
    for want in &request.wants {
        collect_reachable_excluding(source, want, &excluded, &mut included)?;
    }

    // 3. Deterministic pack order: object id ascending (BTreeSet is sorted).
    let oids: Vec<ObjectId> = included.into_iter().collect();
    let pack = assemble_pack(source, &oids)?;
    Ok(pack)
}

/// Collect every object reachable from `root` into `out`. With `strict`, a
/// missing object is an error (the closure must be complete); without it, a
/// missing root is silently skipped (a `have` we don't recognize).
fn collect_reachable(
    source: &dyn ObjectSource,
    root: &ObjectId,
    out: &mut BTreeSet<ObjectId>,
    strict: bool,
) -> Result<(), ServeError> {
    let empty = BTreeSet::new();
    collect_inner(source, root, &empty, out, strict)
}

/// Collect every object reachable from `root` that is NOT in `excluded`.
fn collect_reachable_excluding(
    source: &dyn ObjectSource,
    root: &ObjectId,
    excluded: &BTreeSet<ObjectId>,
    out: &mut BTreeSet<ObjectId>,
) -> Result<(), ServeError> {
    collect_inner(source, root, excluded, out, /*strict=*/ true)
}

/// Iterative DFS over the git object graph from `root`.
///
/// - commit → its tree + parents
/// - tree → its (non-gitlink) entries
/// - tag → its target
/// - blob → leaf
///
/// Objects in `excluded` (and their subgraphs) are pruned. Visiting stops at
/// objects already in `out` so shared subgraphs are walked once.
fn collect_inner(
    source: &dyn ObjectSource,
    root: &ObjectId,
    excluded: &BTreeSet<ObjectId>,
    out: &mut BTreeSet<ObjectId>,
    strict: bool,
) -> Result<(), ServeError> {
    let mut stack = vec![*root];
    while let Some(oid) = stack.pop() {
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
                push_unseen(tree, excluded, out, &mut stack);
                // `parent_ids` consumes the iterator; re-create for the walk.
                let parents = CommitRefIter::from_bytes(&object.data).parent_ids();
                for parent in parents {
                    push_unseen(parent, excluded, out, &mut stack);
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
                    push_unseen(entry.oid.to_owned(), excluded, out, &mut stack);
                }
            }
            ObjectKind::Tag => {
                let target = TagRefIter::from_bytes(&object.data)
                    .target_id()
                    .map_err(|source| ServeError::Decode { oid, source })?;
                push_unseen(target, excluded, out, &mut stack);
            }
        }
    }
    Ok(())
}

/// Push `oid` onto the DFS stack unless it's excluded or already collected.
fn push_unseen(
    oid: ObjectId,
    excluded: &BTreeSet<ObjectId>,
    out: &BTreeSet<ObjectId>,
    stack: &mut Vec<ObjectId>,
) {
    if !excluded.contains(&oid) && !out.contains(&oid) {
        stack.push(oid);
    }
}
