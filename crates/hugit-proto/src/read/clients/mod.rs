//! Client-matrix conformance + jj first-class stacked changes (D2b items ③⑦).
//!
//! The D2a core ([`crate::read::serve`]) serves a real git V2 packfile assembled
//! from CAS objects. This module proves that read path at the *client* edge:
//!
//! - **Client matrix (③)** — git 2.40+, jj, and libgit2 all speak the same
//!   protocol-v2 wire and reconstruct the same byte-identical objects. The
//!   conformance is expressed as a [`ClientKind`] enumeration plus a
//!   [`served_object_ids`] helper that drives the D2a serve path; the acceptance
//!   suite shells out to the real local binaries to prove on-the-wire round-trip.
//! - **jj first-class (⑦)** — jj models a *stacked-changes series* whose entries
//!   carry a **change-id** that is STABLE across forge operations even as the
//!   underlying commit (the git oid) is rewritten. This module owns the
//!   change-id ↔ commit binding and the stack-reconstruction logic so a stack
//!   round-trips identically.
//!
//! jj is the uncontested distribution door (command-catalog: "jj first-class")
//! — it is modeled first-class here, never best-effort. This module never
//! reimplements pack assembly; it consumes [`serve_clone`] / [`serve_fetch`].

use std::collections::BTreeMap;

use gix_hash::ObjectId;

use crate::read::negotiate::RefView;
use crate::read::pack::ObjectSource;
use crate::read::serve::{ServeError, serve_clone};

/// A git-protocol client in the conformance matrix (D2b item ③).
///
/// Every variant negotiates the identical protocol-v2 wire and reconstructs the
/// identical object closure; the served bytes do not depend on which client
/// pulls them — that is the conformance guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ClientKind {
    /// Canonical git, version 2.40 or newer (the protocol-v2 baseline).
    Git,
    /// Jujutsu (`jj`) — first-class, the uncontested distribution door.
    Jujutsu,
    /// A libgit2-class client embedding the library directly.
    Libgit2,
}

impl ClientKind {
    /// Every client in the matrix, in conformance-test order.
    pub fn matrix() -> [ClientKind; 3] {
        [ClientKind::Git, ClientKind::Jujutsu, ClientKind::Libgit2]
    }

    /// The minimum supported version for clients that gate on one (git ≥ 2.40).
    /// `None` for clients with no protocol-v2 floor in the matrix.
    pub fn min_version(self) -> Option<(u32, u32)> {
        match self {
            ClientKind::Git => Some((2, 40)),
            ClientKind::Jujutsu | ClientKind::Libgit2 => None,
        }
    }

    /// The local binary a conformance run shells out to, if the client is an
    /// external process (git, jj). `None` for the in-process libgit2 path.
    pub fn binary(self) -> Option<&'static str> {
        match self {
            ClientKind::Git => Some("git"),
            ClientKind::Jujutsu => Some("jj"),
            ClientKind::Libgit2 => None,
        }
    }
}

/// Decide whether a `git --version` string satisfies the matrix floor (≥ 2.40).
///
/// Parsing is deliberately lenient (git prints `git version 2.51.0`): the first
/// two dotted integers are the major/minor. An unparseable string is treated as
/// not meeting the floor — fail-closed, never a silent pass.
pub fn git_version_meets_floor(version_line: &str) -> bool {
    let floor = match ClientKind::Git.min_version() {
        Some(v) => v,
        None => return true,
    };
    let nums: Vec<u32> = version_line
        .split(|c: char| !c.is_ascii_digit())
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<u32>().ok())
        .collect();
    match (nums.first(), nums.get(1)) {
        (Some(&major), Some(&minor)) => major > floor.0 || (major == floor.0 && minor >= floor.1),
        _ => false,
    }
}

/// The object ids a given client would reconstruct from a full clone of the repo
/// behind `view`/`source`. Identical for every [`ClientKind`] — that *is* the
/// conformance: the served closure is client-independent.
///
/// Drives the D2a [`serve_clone`] path; pack assembly is never reimplemented.
pub fn served_object_ids(
    view: &dyn RefView,
    source: &dyn ObjectSource,
) -> Result<Vec<ObjectId>, ServeError> {
    let (_adv, pack) = serve_clone(view, source)?;
    Ok(pack.object_ids)
}

// ─── jj first-class: stacked changes + stable change-ids (item ⑦) ────────────

/// A jj **change-id**: an identity stable across forge operations even as the
/// underlying git commit is rewritten.
///
/// In jj, the change-id is decoupled from the commit hash so a change keeps its
/// identity through rebases/amends/forge rewrites. We model it as an opaque
/// stable token; the binding to the current git commit lives in [`StackEntry`].
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ChangeId(pub String);

impl ChangeId {
    /// Construct a change-id from its stable token.
    pub fn new(id: impl Into<String>) -> Self {
        ChangeId(id.into())
    }

    /// The stable token as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One entry in a jj stacked-changes series: a stable [`ChangeId`] bound to the
/// git commit ([`ObjectId`]) that currently realizes it.
///
/// A forge operation may rewrite the commit (new oid) while the change-id stays
/// put — that is exactly the stability item ⑦ asserts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StackEntry {
    /// The stable change identity.
    pub change_id: ChangeId,
    /// The git commit currently realizing this change.
    pub commit: ObjectId,
}

/// A jj stacked-changes series: an ordered list of [`StackEntry`] from base to
/// tip. The order is the stack order (parent → child); [`Stack::change_ids`] and
/// [`Stack::commits`] project the two halves.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Stack {
    entries: Vec<StackEntry>,
}

impl Stack {
    /// An empty stack.
    pub fn new() -> Self {
        Self::default()
    }

    /// Build a stack from base→tip entries.
    pub fn from_entries(entries: Vec<StackEntry>) -> Self {
        Self { entries }
    }

    /// Push a change onto the tip of the stack.
    pub fn push(&mut self, change_id: ChangeId, commit: ObjectId) {
        self.entries.push(StackEntry { change_id, commit });
    }

    /// The entries, base→tip.
    pub fn entries(&self) -> &[StackEntry] {
        &self.entries
    }

    /// The change-ids in stack order — the identity sequence that must be stable
    /// across forge operations.
    pub fn change_ids(&self) -> Vec<ChangeId> {
        self.entries.iter().map(|e| e.change_id.clone()).collect()
    }

    /// The git commits in stack order (these may be rewritten by a forge op).
    pub fn commits(&self) -> Vec<ObjectId> {
        self.entries.iter().map(|e| e.commit).collect()
    }

    /// The number of changes in the series.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the stack is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Apply a forge operation that rewrites commits (e.g. a rebase that hands
    /// each change a new git oid) while preserving change identities.
    ///
    /// `rewrite` maps an old commit oid to its new one. Change-ids are untouched;
    /// the stack order is preserved. This is the model of "change-ids stable
    /// across forge ops" (item ⑦): identities survive, only oids move.
    pub fn rewrite_commits(&self, rewrite: &BTreeMap<ObjectId, ObjectId>) -> Stack {
        let entries = self
            .entries
            .iter()
            .map(|e| StackEntry {
                change_id: e.change_id.clone(),
                commit: rewrite.get(&e.commit).copied().unwrap_or(e.commit),
            })
            .collect();
        Stack::from_entries(entries)
    }

    /// Reconstruct a stack from a flat set of [`StackEntry`] plus the intended
    /// base→tip change-id order. Used to prove a stack *round-trips identically*:
    /// after a forge op + transport, rebuilding from the entries by change-id
    /// yields the same change sequence.
    ///
    /// Returns `None` if any ordered change-id is missing from `entries` — a
    /// dropped change is a broken round-trip, surfaced explicitly (never silent).
    pub fn reconstruct(entries: &[StackEntry], order: &[ChangeId]) -> Option<Stack> {
        let by_change: BTreeMap<&ChangeId, &StackEntry> =
            entries.iter().map(|e| (&e.change_id, e)).collect();
        let mut rebuilt = Stack::new();
        for change_id in order {
            let entry = by_change.get(change_id)?;
            rebuilt.push(entry.change_id.clone(), entry.commit);
        }
        Some(rebuilt)
    }
}
