//! Content-addressed repo-home render cache — the CAS-EXPENSIVE part of a `/home`
//! read (the root-tree file listing + the README bytes), keyed by the **root TREE
//! OID**.
//!
//! ## Why this module exists (the repeated-`/home` tree-walk latency pain)
//!
//! `GET /v1/repos/{repo}/home` ([`crate::handlers::build_home`]) lists the root tree
//! and resolves the README on EVERY request. On the single-threaded lazy-CAS engine
//! each `src.get` is a synchronous R2 fetch that blocks the whole accept loop, so a
//! home read costs ~0.5–1 s of tree-walk work — and it does NOT warm: the walk is
//! re-run per request even for an unchanged tree.
//!
//! This module caches the two CAS-expensive outputs (the tree listing + the README)
//! keyed by the **content-addressed root tree oid**. That key is the whole trick:
//!
//! - **Self-invalidating.** A push produces a NEW root tree → a NEW oid → a cache
//!   MISS → a fresh walk. A changed tree is NEVER a stale HIT (a different tree can
//!   only ever have a different oid), so no explicit push/delete rebuild hook is
//!   needed for correctness — the key IS the invalidation.
//! - **Warms with traffic.** The first read of a given tree walks + populates; every
//!   later read of the SAME tree is a pure in-memory HIT (ZERO CAS). An optional boot
//!   pre-warm ([`bootstrap_home_caches`]) populates each served repo's current-head
//!   home OFF the accept loop so the first real read after a boot isn't cold.
//!
//! The FAST, log-derived parts of the home VM (branch/counts/last-commit/contributors)
//! are NOT cached — [`crate::handlers::build_home`] rebuilds them per request from the
//! (already chain-verified) event log, so they stay FRESH; only the CAS-expensive tree
//! content rides this cache.
//!
//! ## Stored UNSCRUBBED, scrubbed at the read boundary
//!
//! Mirroring [`crate::search_index`] / [`crate::blob_history_index`], the cached
//! content is the RAW (unscrubbed) tree-entry names + README bytes; every emitted
//! path/README line is passed through [`crate::fmt::scrub`] at COMPOSE time in
//! `build_home` (never a redaction bypass — a secret-shaped filename or a secret in
//! the README redacts before it reaches the VM).
//!
//! ## Bounded (byte-capped, FIFO eviction)
//!
//! The store is byte-bounded ([`DEFAULT_HOME_CACHE_BYTES`], 64 MiB of content bytes)
//! with FIFO eviction (mirrors [`crate::cas`]'s `BoundedObjectCache`) so a long-lived
//! many-repo engine can never grow the cache without bound; a miss-after-evict simply
//! re-walks (content-addressing keeps eviction correctness-free).

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, RwLock};

use gix_hash::ObjectId;

/// Default byte budget for the home-render cache: 64 MiB of CONTENT bytes (summed
/// tree-entry names + README bytes across every cached tree). Generous enough to
/// retain every served repo's current home many times over, yet small relative to
/// the engine container's memory so the cache can never be an OOM vector on a
/// long-lived instance. A miss-after-evict re-walks (content-addressing → an evicted
/// entry re-populates byte-identically).
pub const DEFAULT_HOME_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// One raw (unscrubbed) root-tree entry: its display name + whether it is a
/// directory. Stored raw; the name is [`crate::fmt::scrub`]-ed at the read boundary
/// when `build_home` composes the VM.
#[derive(Clone)]
pub struct RawTreeEntry {
    /// The raw (unscrubbed) entry name (git-canonical order; the dirs-first display
    /// sort is applied AFTER scrub at the read boundary).
    pub name: String,
    /// From the git mode — a directory sorts first in the rendered file table.
    pub is_dir: bool,
}

/// The CAS-EXPENSIVE outputs of one home read, keyed by the root tree oid. Stored
/// UNSCRUBBED (scrubbed at the read boundary in `build_home`). `Clone` is a deep copy
/// — a HIT returns an owned clone so the lock is never held across the compose.
#[derive(Clone)]
pub struct CachedHomeContent {
    /// The raw root-tree listing (one entry per direct child).
    pub entries: Vec<RawTreeEntry>,
    /// The raw README markdown (first resolved candidate), or `None` when no README
    /// resolved / it was oversized. Scrubbed line-by-line at the read boundary.
    pub readme: Option<String>,
}

impl CachedHomeContent {
    /// An empty content (no seam / honest-empty) — never cached, only served.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            entries: Vec::new(),
            readme: None,
        }
    }

    /// The content-byte weight this entry adds to the cache (what the byte budget
    /// bounds): the sum of entry-name bytes + the README bytes.
    fn content_bytes(&self) -> usize {
        self.entries.iter().map(|e| e.name.len()).sum::<usize>()
            + self.readme.as_ref().map_or(0, String::len)
    }
}

/// The byte-bounded, FIFO-eviction inner store (guarded by the [`HomeRenderCache`]
/// `RwLock`).
#[derive(Default)]
struct HomeCacheInner {
    /// `tree_oid → cached content`. A different tree oid is a distinct key, so a
    /// changed tree can never be served from a stale entry.
    map: HashMap<ObjectId, CachedHomeContent>,
    /// Insertion order, oldest at the front — the FIFO eviction queue.
    order: VecDeque<ObjectId>,
    /// Running sum of cached content bytes (what the budget bounds).
    bytes: usize,
}

/// The engine-wide, interior-mutable home-render cache, keyed by the content-addressed
/// root tree oid. Shared (one `Arc`) between the read path (HIT/insert) and the boot
/// pre-warm; the engine's single-threaded accept loop keeps the `RwLock` uncontended.
/// Clone is cheap (an `Arc` bump).
#[derive(Clone)]
pub struct HomeRenderCache {
    inner: Arc<RwLock<HomeCacheInner>>,
    /// The content-byte ceiling; an insertion past it evicts oldest entries first.
    max_bytes: usize,
    /// The per-repo pre-warm in-flight guard: at most ONE pre-warm build per repo at
    /// a time (a boot bootstrap racing a post-push warm, or two pushes).
    building: Arc<Mutex<std::collections::HashSet<String>>>,
}

impl Default for HomeRenderCache {
    fn default() -> Self {
        Self::new()
    }
}

impl HomeRenderCache {
    /// A fresh empty cache at the default byte budget ([`DEFAULT_HOME_CACHE_BYTES`]).
    #[must_use]
    pub fn new() -> Self {
        Self::with_max_bytes(DEFAULT_HOME_CACHE_BYTES)
    }

    /// A fresh empty cache bounded to `max_bytes` of content bytes. Public for the
    /// eviction test (to exercise the bound without allocating 64 MiB of fixtures).
    #[must_use]
    pub fn with_max_bytes(max_bytes: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HomeCacheInner::default())),
            max_bytes,
            building: Arc::new(Mutex::new(std::collections::HashSet::new())),
        }
    }

    /// A clone of the cached content for `tree_oid`, if present — a pure in-memory
    /// lookup (a HIT NEVER touches CAS). `None` = a MISS (never cached / evicted /
    /// a changed tree) → the caller runs the bounded walk.
    #[must_use]
    pub fn get(&self, tree_oid: &ObjectId) -> Option<CachedHomeContent> {
        self.inner
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .map
            .get(tree_oid)
            .cloned()
    }

    /// Insert `content` under `tree_oid` (a COMPLETE walk's output — a budget-truncated
    /// walk must never be cached), then evict oldest entries until within the byte
    /// budget. A re-insert of the same oid is a no-op on the accounting (content-
    /// addressing: the same tree yields byte-identical content). Never evicts the
    /// just-inserted entry (`order.len() > 1`), so a single oversized tree is retained
    /// transiently rather than dropped.
    pub fn insert(&self, tree_oid: ObjectId, content: CachedHomeContent) {
        let sz = content.content_bytes();
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if guard.map.insert(tree_oid, content).is_none() {
            guard.order.push_back(tree_oid);
            guard.bytes = guard.bytes.saturating_add(sz);
        }
        while guard.bytes > self.max_bytes && guard.order.len() > 1 {
            if let Some(old) = guard.order.pop_front()
                && let Some(removed) = guard.map.remove(&old)
            {
                let rb = removed.content_bytes();
                guard.bytes = guard.bytes.saturating_sub(rb);
            }
        }
    }

    /// Current summed content-byte total (test-visibility, for the eviction bound).
    #[cfg(test)]
    fn total_bytes(&self) -> usize {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).bytes
    }

    /// Reserve the per-repo pre-warm slot, returning a drop-guard that releases it in
    /// EVERY exit path — or `None` if a pre-warm for `slug` is already running.
    fn try_reserve_prewarm(&self, slug: &str) -> Option<PrewarmSlotGuard> {
        let mut building = self.building.lock().unwrap_or_else(|e| e.into_inner());
        if !building.insert(slug.to_string()) {
            return None;
        }
        Some(PrewarmSlotGuard {
            set: Arc::clone(&self.building),
            slug: slug.to_string(),
        })
    }
}

/// Releases a repo's home-cache PRE-WARM slot on drop — cleared in every exit path
/// (success, error, AND a panic in the build), never leaking the slot.
struct PrewarmSlotGuard {
    set: Arc<Mutex<std::collections::HashSet<String>>>,
    slug: String,
}

impl Drop for PrewarmSlotGuard {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.slug);
    }
}

/// Spawn a DETACHED background thread that pre-walks `repo_slug`'s CURRENT-head home
/// content (root-tree listing + README) and populates the cache — so the FIRST real
/// `/home` read after a boot (or a push) is a HIT instead of a cold tree-walk on the
/// accept loop.
///
/// Best-effort + fail-open + OFF the accept loop:
/// - a repo with no git seam / no HEAD / an unresolvable root tree is a NO-OP (the
///   read path just walks + populates on the first real request);
/// - the per-repo guard admits at most ONE pre-warm at a time;
/// - the whole bounded walk is `catch_unwind`-isolated to the thread — a panic
///   records nothing (the read-path MISS still serves);
/// - NEVER blocks the caller — safe from the boot bootstrap AND post-`ok` on a push.
///
/// A budget-truncated walk is NOT installed (an incomplete tree must never be served
/// as the tree's content); the read path re-walks on the first request.
pub fn spawn_home_cache_prewarm(state: &crate::state::AppState, repo_slug: &str) {
    let Some(repo_state) = state.repo_state(repo_slug) else {
        return;
    };
    // The LIVE HEAD (hot-swapped `git_refs` tip). No tip → nothing to pre-warm (an
    // empty/refless repo's home is honest-empty; the read path covers it).
    let Some(head) = repo_state.head_commit() else {
        return;
    };
    // Resolve the root tree from the LIVE head (a push hot-swaps the ref but NOT the
    // boot `git_root_tree` snapshot); fall back to the boot snapshot if unresolvable.
    let root_tree = hugit_proto::commit_root_tree(repo_state.git_source.as_ref(), &head)
        .ok()
        .flatten()
        .unwrap_or(repo_state.git_root_tree);

    let cache = state.home_cache.clone();
    let Some(guard) = cache.try_reserve_prewarm(repo_slug) else {
        return; // a pre-warm for this repo is already running
    };
    let source: Arc<dyn hugit_proto::ObjectSource + Send + Sync> =
        Arc::clone(&repo_state.git_source);
    let slug = repo_slug.to_string();

    let spawned = std::thread::Builder::new()
        .name("hugit-home-prewarm".into())
        .spawn(move || {
            let _guard = guard; // releases the slot at thread end OR on a panic
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::handlers::home::walk_home_content(
                    source.as_ref(),
                    &root_tree,
                    std::time::Instant::now() + crate::budgeted_source::WALK_BUDGET,
                )
            }));
            match built {
                // Only a COMPLETE walk is installed (a truncated one is not the tree's
                // full content — the read path re-walks on the first real request).
                Ok((content, true)) => cache.insert(root_tree, content),
                Ok((_, false)) => {}
                Err(_) => eprintln!("hugit-serve: home-cache pre-warm for {slug} panicked"),
            }
        });
    if spawned.is_err() {
        // Thread exhaustion: the un-run closure (and its `guard`) was dropped, which
        // already released the slot — nothing more to do (the read path still serves).
        eprintln!("hugit-serve: home-cache pre-warm thread spawn failed for {repo_slug}");
    }
}

/// At engine start, kick a background home-cache pre-warm for every loaded (boot-set)
/// repo. Runs on the serve thread BEFORE the accept loop but only SPAWNS the detached
/// walks — the boot path is never blocked (the same off-boot discipline as the
/// clone-pack / blob-history / search-index bootstraps). Until a pre-warm lands the
/// first `/home` read walks + populates inline (no regress vs today).
pub fn bootstrap_home_caches(state: &crate::state::AppState) {
    let slugs: Vec<String> = state.repos.keys().cloned().collect();
    for slug in slugs {
        spawn_home_cache_prewarm(state, &slug);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(seed: u8) -> ObjectId {
        ObjectId::from_hex(format!("{seed:02x}").repeat(20).as_bytes()).unwrap()
    }

    fn content(name: &str, readme: Option<&str>) -> CachedHomeContent {
        CachedHomeContent {
            entries: vec![RawTreeEntry {
                name: name.to_string(),
                is_dir: false,
            }],
            readme: readme.map(str::to_string),
        }
    }

    /// A MISS (never inserted) → `None`; an inserted tree oid → a HIT with the exact
    /// content (content-addressed lookup).
    #[test]
    fn insert_then_hit_miss_on_absent() {
        let cache = HomeRenderCache::new();
        assert!(cache.get(&oid(1)).is_none(), "absent tree → MISS");
        cache.insert(oid(1), content("lib.rs", Some("# hi")));
        let hit = cache.get(&oid(1)).expect("inserted tree → HIT");
        assert_eq!(hit.entries.len(), 1);
        assert_eq!(hit.entries[0].name, "lib.rs");
        assert_eq!(hit.readme.as_deref(), Some("# hi"));
        // A DIFFERENT tree oid (a "push" produced a new tree) is still a MISS — the
        // content-addressed key makes a stale serve impossible.
        assert!(cache.get(&oid(2)).is_none(), "different tree oid → MISS");
    }

    /// The byte cap evicts oldest-first (FIFO) and keeps the summed bytes within
    /// budget — the cache can never grow without bound.
    #[test]
    fn byte_cap_evicts_fifo_within_budget() {
        // Each entry name is 10 bytes → cap of 25 holds ~2 entries.
        let cache = HomeRenderCache::with_max_bytes(25);
        cache.insert(oid(1), content("aaaaaaaaaa", None)); // 10 bytes
        cache.insert(oid(2), content("bbbbbbbbbb", None)); // 20 bytes total
        cache.insert(oid(3), content("cccccccccc", None)); // 30 > 25 → evict oldest
        assert!(
            cache.total_bytes() <= 25,
            "summed content bytes within budget after eviction"
        );
        assert!(cache.get(&oid(1)).is_none(), "oldest (oid 1) evicted FIFO");
        assert!(
            cache.get(&oid(3)).is_some(),
            "the just-inserted entry stays"
        );
    }

    /// A re-insert of the SAME tree oid is a no-op on the byte accounting (content-
    /// addressing: the same tree is byte-identical), so the total never double-counts.
    #[test]
    fn reinsert_same_oid_does_not_double_count() {
        let cache = HomeRenderCache::new();
        cache.insert(oid(1), content("aaaaaaaaaa", None));
        let after_first = cache.total_bytes();
        cache.insert(oid(1), content("aaaaaaaaaa", None));
        assert_eq!(
            cache.total_bytes(),
            after_first,
            "re-inserting the same oid does not double-count bytes"
        );
    }

    /// The per-repo pre-warm guard is single-flight + releases on drop.
    #[test]
    fn prewarm_slot_single_flight_and_releases() {
        let cache = HomeRenderCache::new();
        let g1 = cache.try_reserve_prewarm("r");
        assert!(g1.is_some());
        assert!(
            cache.try_reserve_prewarm("r").is_none(),
            "second reservation refused (single-flight)"
        );
        drop(g1);
        assert!(
            cache.try_reserve_prewarm("r").is_some(),
            "slot released on drop"
        );
    }
}
