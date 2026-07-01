//! `BudgetedSource` — a wall-clock deadline wrapper over any [`ObjectSource`].
//!
//! ## Why (the single-thread latency-DoS class)
//!
//! The deployed engine is **single-threaded lazy-git-from-CAS**: every
//! [`ObjectSource::get`] is a synchronous R2 fetch that BLOCKS the whole accept
//! loop (including `/readyz`) until it returns. A read whose CAS-fetch count scales
//! with an attacker-influencable input (path depth, tree size, history depth) is a
//! **latency DoS** — it can wedge the engine for minutes.
//!
//! The engine already has WALL-CLOCK guards where the walk API takes a deadline:
//! [`hugit_proto::DIFF_BUDGET`] (tree-diff), `BLOB_HISTORY_BUDGET` (`blob_history`),
//! and `search`'s `CODE_SCAN_BUDGET`. This wrapper extends the SAME convention to a
//! CAS-walk API that does NOT take a deadline of its own — notably
//! [`hugit_proto::resolve_blob_at_path`], which walks one `get` per path segment.
//! Instead of threading a deadline through that (cross-crate) API, we bound it at
//! the SOURCE: wrap the source, and once the shared deadline passes every further
//! `get` returns `Ok(None)`.
//!
//! ## Honest-partial, never fabricated
//!
//! Past the deadline `get` returns **`Ok(None)`** — the "no such object" answer.
//! A downstream walk that consumes this (e.g. `resolve_blob_at_path`) then stops
//! and yields its honest-empty / `None` result (→ a 404 for a blob/edit read). It
//! NEVER fabricates bytes and NEVER loops unbounded: the deadline check runs BEFORE
//! each fetch, so the walk is bounded by wall-clock regardless of the CAS latency
//! or the attacker-chosen depth. [`BudgetedSource::tripped`] lets a caller observe
//! whether the stop was a real miss vs a budget cutoff (both are honest — there is
//! no existence oracle either way).

use std::cell::Cell;
use std::time::{Duration, Instant};

use gix_hash::ObjectId;
use hugit_proto::{GitObject, ObjectSource, PackError};

/// Default wall-clock budget for a bounded CAS walk. Mirrors
/// [`hugit_proto::DIFF_BUDGET`] and `BLOB_HISTORY_BUDGET` (2 s) — the same
/// single-threaded-engine latency ballpark. On the single accept loop this is the
/// worst-case time ONE bounded read may block every other request; keep it in
/// lock-step with the sibling budgets.
pub const WALK_BUDGET: Duration = Duration::from_millis(2_000);

/// A wall-clock-bounded view of an [`ObjectSource`]. Every `get` past `deadline`
/// returns `Ok(None)` (honest "not resolved within budget"), so a walk built on it
/// stops with its honest-empty result rather than wedging the accept loop.
///
/// Borrows the inner source for the duration of ONE request's walk — it is not
/// stored on state. Uses interior mutability ([`Cell`]) for the tripped flag; it is
/// therefore `!Sync` (fine — a walk runs on the single engine thread and the
/// [`ObjectSource`] read API only needs `&dyn ObjectSource`).
pub struct BudgetedSource<'a> {
    inner: &'a dyn ObjectSource,
    deadline: Instant,
    tripped: Cell<bool>,
}

impl<'a> BudgetedSource<'a> {
    /// Wrap `inner`, refusing any `get` at or after `deadline`.
    #[must_use]
    pub fn new(inner: &'a dyn ObjectSource, deadline: Instant) -> Self {
        Self {
            inner,
            deadline,
            tripped: Cell::new(false),
        }
    }

    /// Wrap `inner` with a deadline `budget` from now.
    #[must_use]
    pub fn with_budget(inner: &'a dyn ObjectSource, budget: Duration) -> Self {
        Self::new(inner, Instant::now() + budget)
    }

    /// Whether at least one `get` was refused because the deadline had passed —
    /// i.e. the walk was truncated by the budget, not (only) by a real miss.
    #[must_use]
    pub fn tripped(&self) -> bool {
        self.tripped.get()
    }
}

impl ObjectSource for BudgetedSource<'_> {
    fn get(&self, oid: &ObjectId) -> Result<Option<GitObject>, PackError> {
        // Check BEFORE the fetch: the whole point is to not START another
        // synchronous R2 read once we are over budget.
        if Instant::now() >= self.deadline {
            self.tripped.set(true);
            // Honest "not resolved within budget" — a walk treats this exactly like
            // a clean absence and stops. Never a fabricated object.
            return Ok(None);
        }
        self.inner.get(oid)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::{CasObjectSource, GitObject, ObjectKind};

    /// A source that PANICS if `get` is ever called — proves the deadline gate
    /// short-circuits BEFORE touching the inner (R2) source.
    struct PanicSource;
    impl ObjectSource for PanicSource {
        fn get(&self, _oid: &ObjectId) -> Result<Option<GitObject>, PackError> {
            panic!("inner get must not run past the deadline");
        }
    }

    fn some_oid() -> ObjectId {
        // Any syntactically valid oid; the deadline path never dereferences it.
        ObjectId::from_hex(b"0000000000000000000000000000000000000000").unwrap()
    }

    /// A deadline already in the PAST → `get` returns `Ok(None)` WITHOUT touching
    /// the inner source, and `tripped()` flips. (Bounded stop, honest-empty.)
    #[test]
    fn past_deadline_short_circuits_before_inner_fetch() {
        let past = Instant::now() - Duration::from_secs(1);
        let budgeted = BudgetedSource::new(&PanicSource, past);
        // If the deadline gate did not fire, PanicSource::get would panic.
        assert!(
            budgeted.get(&some_oid()).unwrap().is_none(),
            "past-deadline get is honest-empty"
        );
        assert!(budgeted.tripped(), "the budget cutoff is observable");
    }

    /// A far-FUTURE deadline is a transparent pass-through: the real object comes
    /// back and `tripped()` stays false. (Proves the wrapper is not always-None.)
    #[test]
    fn future_deadline_passes_through_to_inner() {
        let mut cas = CasObjectSource::new();
        let oid = cas.insert(GitObject::new(ObjectKind::Blob, b"hello".to_vec()));
        let future = Instant::now() + Duration::from_secs(60);
        let budgeted = BudgetedSource::new(&cas, future);

        let got = budgeted
            .get(&oid)
            .unwrap()
            .expect("present object resolves");
        assert_eq!(got.data, b"hello");
        assert!(!budgeted.tripped(), "a within-budget get does not trip");
        // A genuine miss under budget is still a clean None (no false trip).
        assert!(budgeted.get(&some_oid()).unwrap().is_none());
        assert!(!budgeted.tripped(), "a real miss is not a budget trip");
    }

    /// `with_budget` composes `now + budget`; a zero budget is immediately expired.
    #[test]
    fn zero_budget_is_immediately_over() {
        let budgeted = BudgetedSource::with_budget(&PanicSource, Duration::from_millis(0));
        assert!(budgeted.get(&some_oid()).unwrap().is_none());
        assert!(budgeted.tripped());
    }
}
