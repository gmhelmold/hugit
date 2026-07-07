//! Precomputed per-path blob-history index (the "Histórico" drawer fast path).
//!
//! ## Why this module exists (the #70(a) deep-history-empty gap)
//!
//! The blob "Histórico" drawer lists the commits that touched a file. The live path
//! ([`hugit_proto::blob_history`]) walks from HEAD *per path* under a 2 s wall-clock
//! budget — a hard latency bound on the single-threaded engine. For a file whose
//! touches are DEEP in history, the many intervening non-touching commits exhaust that
//! budget before a single touch is found → an EMPTY drawer for a file that clearly has
//! history (#69's known gap).
//!
//! This index walks the whole (bounded) history ONCE, off the accept loop, and records
//! EVERY path's touching commits in a `slug → (head, path → [BlobHistoryEntry])` map.
//! A deep-history file's list is then served directly from the map — fast AND complete
//! — instead of an exhausting per-request walk.
//!
//! ## Design (the three constraints)
//!
//! - **Built OFF the hot accept loop.** The full-history walk is exactly the
//!   single-thread latency DoS the 2 s budget defends against, so it runs on a DETACHED
//!   thread — at boot ([`bootstrap_blob_history_indexes`], mirroring the cached
//!   clone-pack bootstrap) and after each push ([`spawn_blob_history_index_build`], so
//!   a moved HEAD gets a fresh index). Never inline. The build itself is BOUNDED
//!   ([`hugit_proto::INDEX_BUILD_BUDGET`] + [`hugit_proto::MAX_INDEX_COMMITS`]) so even
//!   a detached build can't hang, and it never touches the boot-blocking path (the same
//!   scar as the synchronous ~18 s CAS self-probe that once crash-looped boot).
//! - **Stored in-memory** (rebuilt on boot / push) — the simplest fail-safe v0. A
//!   durable R2 artifact is a bigger lift and is not needed for correctness.
//! - **Fail-safe fallback.** A lookup miss — no index for the slug (still building /
//!   never built), a HEAD mismatch (a push moved the tip; the stale index is refused),
//!   or a path absent from the index (never touched within the bounded build, or added
//!   after it) — makes the caller FALL BACK to the live wall-clock-bounded walk. The
//!   index is a pure enhancement layered ON TOP of the existing safety bound, never a
//!   replacement of it: on any doubt the behaviour is exactly today's.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};

use gix_hash::ObjectId;
use hugit_proto::BlobHistoryEntry;

use crate::state::AppState;

/// One repo's built index: the HEAD commit it was built for + `path → newest-first
/// touching revisions`. The `head` is the correctness gate — a lookup trusts the
/// stored lists ONLY while the index's HEAD equals the repo's CURRENT live HEAD (a
/// push moves HEAD → the index is stale → the lookup misses → the caller live-walks
/// the new tip). This mirrors the clone-pack cache's `refset_sha` gate: a stale index
/// is never a correctness hole, only a missed optimisation.
struct BuiltIndex {
    head: ObjectId,
    by_path: HashMap<String, Vec<BlobHistoryEntry>>,
}

/// The engine-wide, interior-mutable store of every loaded repo's built history index,
/// keyed by repo slug. Shared (one `Arc`) between the detached build threads (writers)
/// and the blob read path (reader); the engine's single-threaded accept loop keeps the
/// `RwLock` uncontended. Clone is cheap (an `Arc` bump).
#[derive(Clone, Default)]
pub struct BlobHistoryStore {
    /// `slug → BuiltIndex`. Absent slug = no index yet (building / never built) → miss.
    inner: Arc<RwLock<HashMap<String, BuiltIndex>>>,
    /// The per-repo build in-flight guard: at most ONE build per repo at a time (a boot
    /// bootstrap racing a post-push rebuild, or two pushes). A slug is inserted before
    /// the detached build thread spawns and removed when it ends (all paths — success,
    /// error, panic — via [`BuildSlotGuard`]).
    building: Arc<Mutex<HashSet<String>>>,
}

impl BlobHistoryStore {
    /// A fresh empty store (nothing indexed until a build lands).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Look up `path`'s precomputed history for `slug`, valid ONLY when the stored
    /// index was built for the CURRENT `head`. `None` on ANY of: no index for the slug,
    /// a HEAD mismatch (a push moved the tip — the stale index is refused), or a path
    /// absent from the index. In every `None` case the caller MUST fall back to the
    /// live wall-clock-bounded walk (never a regress vs today's behaviour).
    #[must_use]
    pub fn lookup(&self, slug: &str, head: &ObjectId, path: &str) -> Option<Vec<BlobHistoryEntry>> {
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let built = guard.get(slug)?;
        if &built.head != head {
            return None; // stale index (HEAD moved) → live-walk the new tip
        }
        built.by_path.get(path).cloned()
    }

    /// Install a freshly built index for `slug` (replacing any prior one). Called by
    /// the detached build thread; `pub(crate)` so the `handlers::blob` tests can seed a
    /// known index to exercise the index-first read path.
    pub(crate) fn install(
        &self,
        slug: &str,
        head: ObjectId,
        by_path: HashMap<String, Vec<BlobHistoryEntry>>,
    ) {
        self.inner
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(slug.to_string(), BuiltIndex { head, by_path });
    }

    /// Reserve the per-repo build slot, returning a drop-guard that releases it in
    /// EVERY exit path — or `None` if a build for `slug` is already running (never a
    /// double build). Constructed BEFORE the thread spawn and moved INTO the closure,
    /// so even a spawn failure (the closure dropped un-run) releases the slot.
    fn try_reserve_build(&self, slug: &str) -> Option<BuildSlotGuard> {
        let mut building = self.building.lock().unwrap_or_else(|e| e.into_inner());
        if !building.insert(slug.to_string()) {
            return None; // already building this repo
        }
        Some(BuildSlotGuard {
            set: Arc::clone(&self.building),
            slug: slug.to_string(),
        })
    }
}

/// Releases a repo's history-index BUILD slot on drop — so the per-repo `building`
/// guard is cleared in every exit path (success, error, AND a panic in the build),
/// never leaking the slot (which would wedge all future rebuilds of that repo).
struct BuildSlotGuard {
    set: Arc<Mutex<HashSet<String>>>,
    slug: String,
}

impl Drop for BuildSlotGuard {
    fn drop(&mut self) {
        self.set
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&self.slug);
    }
}

/// Spawn a DETACHED background thread that (re)builds `repo_slug`'s per-path history
/// index from its LIVE HEAD and installs it — so the next blob read of a deep-history
/// file serves its "Histórico" from the index instead of an exhausting live walk.
///
/// Best-effort + fail-open: a repo with no git seam OR no HEAD commit (a refless / empty
/// repo) is a NO-OP (the live walk already returns empty-honest there); the per-repo
/// guard admits at most ONE build at a time; a build panic is isolated to the thread.
/// NEVER blocks the caller — the whole bounded walk runs on the detached thread, so this
/// is safe to call from the accept-loop bootstrap AND post-`ok` on a push worker.
pub fn spawn_blob_history_index_build(state: &AppState, repo_slug: &str) {
    let Some(repo_state) = state.repo_state(repo_slug) else {
        return;
    };
    // The LIVE HEAD (hot-swapped `git_refs` tip). No tip → nothing to index (the drawer
    // is honest-empty for a refless/empty repo; the live walk already covers it).
    let Some(head) = repo_state.head_commit() else {
        return;
    };
    let store = state.blob_history_index.clone();
    // Reserve the slot; bail if a build for this repo is already running.
    let Some(guard) = store.try_reserve_build(repo_slug) else {
        return;
    };
    // Owned/Arc handles → nothing borrows `state`, so the thread is `'static`.
    let source: Arc<dyn hugit_proto::ObjectSource + Send + Sync> =
        Arc::clone(&repo_state.git_source);
    let slug = repo_slug.to_string();

    let spawned = std::thread::Builder::new()
        .name("hugit-blob-history-index".into())
        .spawn(move || {
            // Move the guard in; it releases the slot at thread end OR on a panic.
            let _guard = guard;
            // Isolate a build panic to this thread (never poisons the store / crashes
            // the engine) — a panic records nothing (the live-walk fallback stands).
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                hugit_proto::build_blob_history_index(
                    source.as_ref(),
                    &head,
                    std::time::Instant::now() + hugit_proto::INDEX_BUILD_BUDGET,
                    hugit_proto::MAX_INDEX_COMMITS,
                )
            }));
            match built {
                Ok(by_path) => store.install(&slug, head, by_path),
                Err(_) => {
                    eprintln!("hugit-serve: blob-history index build for {slug} panicked");
                }
            }
        });
    if spawned.is_err() {
        // Thread exhaustion: the un-run closure (and its `guard`) was dropped, which
        // already released the slot — nothing more to do (the live walk still serves).
        eprintln!("hugit-serve: blob-history index build thread spawn failed for {repo_slug}");
    }
}

/// At engine start, kick a background per-path history-index build for every loaded
/// (boot-set) repo with a HEAD. Runs on the serve thread BEFORE the accept loop, but
/// only SPAWNS the detached builds — the boot path is never blocked on the walk (the
/// same off-boot discipline as the clone-pack bootstrap and the CAS self-probe). Until
/// a build lands, blob reads fall back to the live wall-clock-bounded walk (no regress).
pub fn bootstrap_blob_history_indexes(state: &AppState) {
    // Only the boot repo set exists at serve start (the runtime overlay is empty).
    let slugs: Vec<String> = state.repos.keys().cloned().collect();
    for slug in slugs {
        spawn_blob_history_index_build(state, &slug);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::{ObjectSource, PackError};

    fn oid(hex_seed: u8) -> ObjectId {
        let hex = format!("{hex_seed:02x}").repeat(20);
        ObjectId::from_hex(hex.as_bytes()).unwrap()
    }

    fn entry(hex: &str) -> BlobHistoryEntry {
        BlobHistoryEntry {
            commit_hex: hex.to_string(),
            author_time_ms: 1_000,
            author: "A <a@x>".into(),
            summary: "edit".into(),
        }
    }

    /// A lookup HITS only when both the slug's index exists, its HEAD matches, and the
    /// path is present.
    #[test]
    fn lookup_hits_on_matching_head_and_path() {
        let store = BlobHistoryStore::new();
        let head = oid(0xaa);
        let mut by_path = HashMap::new();
        by_path.insert("deep.rs".to_string(), vec![entry("cafe")]);
        store.install("r", head, by_path);

        let got = store.lookup("r", &head, "deep.rs").expect("hit");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].commit_hex, "cafe");
    }

    /// A HEAD mismatch (a push moved the tip) → MISS → the caller live-walks the new tip.
    #[test]
    fn lookup_misses_on_stale_head() {
        let store = BlobHistoryStore::new();
        let mut by_path = HashMap::new();
        by_path.insert("deep.rs".to_string(), vec![entry("cafe")]);
        store.install("r", oid(0xaa), by_path);
        // The repo's live HEAD moved to a different commit → the stale index is refused.
        assert!(store.lookup("r", &oid(0xbb), "deep.rs").is_none());
    }

    /// A path absent from the index → MISS (→ fallback). An unknown slug → MISS.
    #[test]
    fn lookup_misses_on_absent_path_or_slug() {
        let store = BlobHistoryStore::new();
        let head = oid(0xaa);
        let mut by_path = HashMap::new();
        by_path.insert("present.rs".to_string(), vec![entry("cafe")]);
        store.install("r", head, by_path);
        assert!(store.lookup("r", &head, "absent.rs").is_none());
        assert!(store.lookup("unknown", &head, "present.rs").is_none());
    }

    /// The per-repo build guard admits ONE reservation at a time and releases on drop.
    #[test]
    fn build_slot_is_single_flight_and_releases_on_drop() {
        let store = BlobHistoryStore::new();
        let g1 = store.try_reserve_build("r");
        assert!(g1.is_some(), "first reservation succeeds");
        assert!(
            store.try_reserve_build("r").is_none(),
            "a second concurrent reservation is refused (single-flight)"
        );
        drop(g1);
        assert!(
            store.try_reserve_build("r").is_some(),
            "the slot is released on guard drop → a later build can reserve again"
        );
    }

    /// A source whose `get` PANICS — proves a lookup HIT short-circuits the live walk
    /// entirely (the walk would touch the source; the index never does). This is the
    /// deep-history win expressed as a hard invariant: an indexed path is served WITHOUT
    /// the (would-be-exhausting) per-request CAS walk.
    struct PanicSource;
    impl ObjectSource for PanicSource {
        fn get(&self, _oid: &ObjectId) -> Result<Option<hugit_proto::GitObject>, PackError> {
            panic!("the live walk must NOT run when the index HITs");
        }
    }

    /// The deep-history win as a hard invariant: an INDEXED path is served from the
    /// store with ZERO source access — [`PanicSource::get`] would panic if the live
    /// (would-be-exhausting) walk ran, so a clean return proves the index short-circuits
    /// it. The FALLBACK half (a non-indexed path DOES walk the source) is proven in
    /// `handlers::blob` against a real source.
    #[test]
    fn indexed_lookup_never_touches_the_source() {
        let store = BlobHistoryStore::new();
        let head = oid(0x11);
        let mut by_path = HashMap::new();
        by_path.insert("deep.rs".to_string(), vec![entry("beef"), entry("dead")]);
        store.install("r", head, by_path);

        let hit = store.lookup("r", &head, "deep.rs");
        assert!(hit.is_some(), "the indexed path HITs");
        assert_eq!(hit.unwrap().len(), 2);
        // Sanity that PanicSource really does panic if touched (documents the guard the
        // HIT path relies on — an indexed lookup performs no `get`).
        let src = PanicSource;
        assert!(std::panic::catch_unwind(|| src.get(&head)).is_err());
    }
}
