//! Precomputed per-repo in-memory CODE SEARCH index (the `/search` fast path).
//!
//! ## Why this module exists (the /search latency-DoS scar)
//!
//! `/v1/repos/{repo}/search?q=` returns real code matches over a repo's file tree.
//! The naive way to serve that is a LIVE grep — walk the git tree, `src.get` every
//! blob, scan it — at REQUEST time. On the single-threaded lazy-CAS engine every
//! `src.get` on a cold cache is a SYNCHRONOUS R2 fetch that blocks the WHOLE accept
//! loop (incl. `/readyz`) for the duration; a single search that fans out thousands
//! of sequential R2 reads wedged prod once (the 2026-06-27 code-search wedge —
//! `docs/.../single-thread-engine-read-latency-dos.md`). Bounding it by RESULT count
//! is not enough: the cost is per-OBJECT-FETCH, not per-result.
//!
//! This module removes the fetch fan-out from the hot path entirely. The tree is
//! walked ONCE, OFF the accept loop, and each file's (bounded) content is recorded
//! in an in-memory `slug → entries` map. A `/search` query is then a pure in-memory
//! scan of that map — O(indexed bytes), wall-clock-bounded, and NEVER an R2/CAS
//! fetch. A MISS (no index yet, a stale HEAD, or a repo that isn't indexed) returns
//! an HONEST empty/"indexing" — it does NOT fall back to a live CAS scan (that is
//! precisely the DoS we are removing).
//!
//! ## Design (mirrors [`crate::blob_history_index`])
//!
//! - **Built OFF the hot accept loop.** The full-tree walk runs on a DETACHED thread
//!   — at boot ([`bootstrap_search_indexes`]) and after each push/delete
//!   ([`spawn_search_index_build`], so a moved HEAD gets a fresh index). Never
//!   inline. The build is BOUNDED ([`SEARCH_INDEX_BUILD_BUDGET`] wall-clock +
//!   [`SEARCH_INDEX_FILE_CAP`] files + per-file/per-repo/global byte caps) so even a
//!   detached build cannot hang or blow the container's memory, and it never touches
//!   the boot-blocking path.
//! - **Stored in-memory** (rebuilt on boot / push) — the simplest fail-safe v0.
//! - **Visibility-gated at BUILD time.** Only a PUBLIC (anonymous-readable) repo is
//!   indexed ([`crate::authz::authorize_read`] over the anonymous chain — the SAME
//!   predicate the read gate uses, so an erased or private repo is refused). A
//!   private repo therefore has NO index entry → its `/search` code results are
//!   honest-empty, never leaked. (The `/search` route is ALSO already
//!   `authorize_read`-gated per request, so this is defence-in-depth.)
//! - **Stored UNSCRUBBED; scrubbed at the read boundary.** The recorded content is
//!   raw file bytes (first-N-KiB); every emitted match line + path is passed through
//!   [`crate::fmt::scrub`] at QUERY time (mirrors `blob_history_index` /
//!   `handlers::search` — never a redaction bypass, and a secret substring can be
//!   searched for without the raw secret ever reaching the VM).
//! - **Fail-safe MISS.** No index for the slug (building / never built / private), a
//!   HEAD mismatch (a push moved the tip; the stale index is refused), or simply no
//!   match → an honest-empty `(vec![], 0)`. NEVER a live fetch.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};

use gix_hash::ObjectId;
use hugit_http_contracts::search::SearchCodeVm;
use hugit_proto::{ObjectKind, ObjectSource};

use crate::fmt::scrub;
use crate::state::AppState;

// ── Build bounds (OFF-loop, but still hard-bounded — memory + wall-clock) ─────

/// Hard wall-clock ceiling on ONE repo's index build. Off the accept loop, so this
/// is generous, but a build that hits it stops with a PARTIAL index (honest — a
/// later rebuild completes it) rather than running unbounded on a pathological tree.
const SEARCH_INDEX_BUILD_BUDGET: Duration = Duration::from_secs(30);
/// Maximum number of files recorded per repo. Bounds both build time and memory.
const SEARCH_INDEX_FILE_CAP: usize = 5_000;
/// Maximum bytes stored PER FILE (the first N KiB of each blob). A file larger than
/// this is TRUNCATED to this prefix (still searchable, just not past the prefix), so
/// one huge generated file cannot dominate the repo's memory budget.
const SEARCH_INDEX_BLOB_BYTES_CAP: usize = 64 * 1024;
/// Maximum TOTAL content bytes stored for ONE repo. Once a build reaches this it
/// stops adding files (a partial index — honest). Bounds a single huge repo.
const SEARCH_INDEX_REPO_BYTES_CAP: usize = 32 * 1024 * 1024;
/// Maximum TOTAL content bytes stored across ALL repos in this engine. A build stops
/// adding files once the global store would exceed this — the last line of defence
/// against the index itself becoming an OOM vector on a many-repo engine.
const SEARCH_INDEX_GLOBAL_BYTES_CAP: usize = 256 * 1024 * 1024;

// ── Query bounds (in-memory, but still bounded) ───────────────────────────────

/// Wall-clock ceiling on ONE `/search` query's in-memory scan. The scan touches NO
/// I/O (pure memory), so this is small; it exists so even a pathological
/// index+query can never block the single-threaded accept loop past this.
const SEARCH_QUERY_BUDGET: Duration = Duration::from_millis(500);
/// Maximum number of match lines emitted across the whole query (mirrors the
/// handler's `RESULT_CAP`). `code_total` keeps counting past it (the caller can page).
const SEARCH_MATCH_CAP: usize = 200;
/// Maximum match lines emitted PER FILE — keeps one file from consuming the whole
/// result budget at the expense of others (mirrors the handler's per-file cap).
const SEARCH_LINES_PER_FILE_CAP: usize = 10;

/// One indexed file: its repo-relative path + a bounded prefix of its raw content.
/// Content is UNSCRUBBED (scrubbed at query time) and at most
/// [`SEARCH_INDEX_BLOB_BYTES_CAP`] bytes.
struct FileEntry {
    path: String,
    /// Raw (unscrubbed) UTF-8-lossy content prefix. Lossy decode means a binary blob
    /// renders as mojibake but never panics; a query still scans it harmlessly.
    content: String,
}

/// One repo's built search index: the HEAD it was built for + its indexed files. The
/// `head` is the correctness gate — a query trusts the stored files ONLY while the
/// index's HEAD equals the repo's CURRENT live HEAD (a push moves HEAD → the stale
/// index is refused → honest-empty until the rebuild lands).
struct BuiltIndex {
    head: ObjectId,
    files: Vec<FileEntry>,
}

impl BuiltIndex {
    fn content_bytes(&self) -> usize {
        self.files.iter().map(|f| f.content.len()).sum()
    }
}

/// The engine-wide, interior-mutable store of every PUBLIC repo's built code-search
/// index, keyed by repo slug. Shared (one `Arc`) between the detached build threads
/// (writers) and the `/search` read path (reader); the engine's single-threaded
/// accept loop keeps the `RwLock` uncontended. Clone is cheap (an `Arc` bump).
#[derive(Clone, Default)]
pub struct SearchStore {
    /// `slug → BuiltIndex`. An absent slug = no index (building / never built /
    /// private) → a query MISS (honest-empty, never a live fetch).
    inner: Arc<RwLock<HashMap<String, BuiltIndex>>>,
    /// The per-repo build in-flight guard: at most ONE build per repo at a time.
    building: Arc<Mutex<HashSet<String>>>,
    /// The GLOBAL stored-content byte counter (sum of every repo's `content` bytes).
    /// A build stops adding files once this would exceed [`SEARCH_INDEX_GLOBAL_BYTES_CAP`]
    /// — so the store cannot become an unbounded OOM vector on a many-repo engine.
    /// Adjusted on every `install` (old repo bytes subtracted, new added).
    total_bytes: Arc<AtomicUsize>,
}

impl SearchStore {
    /// A fresh empty store (nothing indexed until a build lands).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Run a `/search` code query against `slug`'s precomputed index — an in-memory
    /// scan ONLY, NEVER a CAS/R2 fetch. Returns `(hits, code_total)` where
    /// `code_total` is the TRUE match-line count (may exceed [`SEARCH_MATCH_CAP`];
    /// `hits` is the capped, SCRUBBED subset). Honest-empty `(vec![], 0)` on ANY of:
    /// no index for the slug (building / never built / private repo), a HEAD mismatch
    /// (a push moved the tip → the stale index is refused), an empty query, or no
    /// match. Bounded by [`SEARCH_QUERY_BUDGET`] wall-clock (defence-in-depth; the
    /// scan does no I/O).
    #[must_use]
    pub fn search(
        &self,
        slug: &str,
        head: Option<&ObjectId>,
        q_lower: &str,
        budget: Duration,
    ) -> (Vec<SearchCodeVm>, usize) {
        if q_lower.is_empty() {
            return (Vec::new(), 0);
        }
        let guard = self.inner.read().unwrap_or_else(|e| e.into_inner());
        let Some(built) = guard.get(slug) else {
            return (Vec::new(), 0); // no index → honest-empty (NEVER a live fetch)
        };
        // HEAD gate: a stale index (a push moved the tip) is refused — honest
        // "indexing" empty until the rebuild lands, never a stale-but-plausible dump.
        match head {
            Some(h) if &built.head != h => return (Vec::new(), 0),
            _ => {}
        }

        let start = Instant::now();
        let mut hits: Vec<SearchCodeVm> = Vec::new();
        let mut code_total: usize = 0;
        for file in &built.files {
            if start.elapsed() >= budget {
                break; // wall-clock guard (in-memory, but bounded regardless)
            }
            let mut file_hits: Vec<(u32, String)> = Vec::new();
            for (i, line) in file.content.lines().enumerate() {
                if line.to_lowercase().contains(q_lower) {
                    code_total += 1;
                    if hits.len() + file_hits.len() < SEARCH_MATCH_CAP
                        && file_hits.len() < SEARCH_LINES_PER_FILE_CAP
                    {
                        // 1-based line number; SCRUB the raw line at the read boundary.
                        file_hits.push(((i as u32) + 1, scrub(line)));
                    }
                }
            }
            if !file_hits.is_empty() {
                hits.push(SearchCodeVm {
                    path: scrub(&file.path), // scrub the path at the read boundary too
                    lines: file_hits,
                    lines_tokens: vec![],
                });
            }
        }
        (hits, code_total)
    }

    /// Install a freshly built index for `slug` (replacing any prior one) and adjust
    /// the global byte counter. Private — called by the detached build thread and by
    /// this module's tests (same module, so they reach it) to seed a known index.
    fn install(&self, slug: &str, built: BuiltIndex) {
        let new_bytes = built.content_bytes();
        let mut guard = self.inner.write().unwrap_or_else(|e| e.into_inner());
        let old_bytes = guard
            .insert(slug.to_string(), built)
            .map_or(0, |b| b.content_bytes());
        // total = total - old + new, each step saturating (never underflows/wraps).
        if old_bytes > 0 {
            self.total_bytes.fetch_sub(old_bytes, Ordering::Relaxed);
        }
        if new_bytes > 0 {
            self.total_bytes.fetch_add(new_bytes, Ordering::Relaxed);
        }
    }

    /// Current global stored-content byte total (the build's global-cap check reads this).
    fn total_bytes(&self) -> usize {
        self.total_bytes.load(Ordering::Relaxed)
    }

    /// Reserve the per-repo build slot, returning a drop-guard that releases it in
    /// EVERY exit path — or `None` if a build for `slug` is already running.
    fn try_reserve_build(&self, slug: &str) -> Option<BuildSlotGuard> {
        let mut building = self.building.lock().unwrap_or_else(|e| e.into_inner());
        if !building.insert(slug.to_string()) {
            return None;
        }
        Some(BuildSlotGuard {
            set: Arc::clone(&self.building),
            slug: slug.to_string(),
        })
    }
}

/// Releases a repo's search-index BUILD slot on drop — so the per-repo `building`
/// guard is cleared in every exit path (success, error, AND a panic in the build).
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

/// The `/search` code-index query, bundled so the dispatcher passes ONE param. The
/// query consults the in-memory [`SearchStore`] ONLY (never a CAS fetch); a MISS is
/// honest-empty.
pub struct CodeIndexQuery<'a> {
    /// The engine-wide store to consult.
    pub store: &'a SearchStore,
    /// The repo slug whose index to look up.
    pub slug: &'a str,
    /// The repo's LIVE HEAD (the correctness gate: a stale index is refused). `None`
    /// for a refless repo (→ a MISS, since a refless repo is never indexed anyway).
    pub head: Option<ObjectId>,
}

impl CodeIndexQuery<'_> {
    /// Run the query with the standard [`SEARCH_QUERY_BUDGET`].
    #[must_use]
    pub fn run(&self, q_lower: &str) -> (Vec<SearchCodeVm>, usize) {
        self.store
            .search(self.slug, self.head.as_ref(), q_lower, SEARCH_QUERY_BUDGET)
    }
}

/// Whether a repo with `meta` is PUBLICLY searchable — reuses the anonymous-read
/// authorization ([`crate::authz::authorize_read`] over the empty principal chain),
/// so a PRIVATE or ERASED repo returns `false` and is NEVER indexed (its `/search`
/// code results stay honest-empty). This is the SAME predicate the per-request read
/// gate uses, so the index can never leak content the read gate would deny anonymously.
#[must_use]
fn is_publicly_searchable(meta: &crate::authz::RepoMeta) -> bool {
    crate::authz::authorize_read(&[], meta)
}

/// Walk the git tree rooted at `root_tree` ONCE (bounded), recording each file's path
/// plus a bounded content prefix into a `Vec<FileEntry>`. Depth-first iterative (no
/// recursion overflow on deep trees). Bounded by `budget` (wall-clock), the file cap,
/// the per-file byte cap, the per-repo byte cap, and the GLOBAL byte cap — where
/// `already_stored` is the store's current global total (excluding this build). A cap
/// hit stops the walk with a PARTIAL index (honest). Content is stored UNSCRUBBED and
/// scrubbed at query time.
fn build_index_files(
    src: &dyn ObjectSource,
    root_tree: &ObjectId,
    budget: Duration,
    already_stored: usize,
) -> Vec<FileEntry> {
    struct Frame {
        oid: ObjectId,
        prefix: String,
    }
    let start = Instant::now();
    let mut files: Vec<FileEntry> = Vec::new();
    let mut repo_bytes: usize = 0;
    let mut stack: Vec<Frame> = vec![Frame {
        oid: *root_tree,
        prefix: String::new(),
    }];

    while let Some(frame) = stack.pop() {
        if files.len() >= SEARCH_INDEX_FILE_CAP
            || start.elapsed() >= budget
            || repo_bytes >= SEARCH_INDEX_REPO_BYTES_CAP
            || already_stored + repo_bytes >= SEARCH_INDEX_GLOBAL_BYTES_CAP
        {
            break;
        }
        let tree_obj = match src.get(&frame.oid) {
            Ok(Some(o)) if o.kind == ObjectKind::Tree => o,
            _ => continue, // fail-closed on a missing/typed-wrong tree (skip, not abort)
        };
        let entries = match gix_object::TreeRefIter::from_bytes(&tree_obj.data).entries() {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries {
            if files.len() >= SEARCH_INDEX_FILE_CAP
                || start.elapsed() >= budget
                || repo_bytes >= SEARCH_INDEX_REPO_BYTES_CAP
                || already_stored + repo_bytes >= SEARCH_INDEX_GLOBAL_BYTES_CAP
            {
                break;
            }
            // Skip symlinks + gitlinks (mirrors the tree read path).
            if entry.mode.is_link() || entry.mode.is_commit() {
                continue;
            }
            let name = String::from_utf8_lossy(entry.filename);
            let path = if frame.prefix.is_empty() {
                name.into_owned()
            } else {
                format!("{}/{name}", frame.prefix)
            };
            if entry.mode.is_tree() {
                stack.push(Frame {
                    oid: entry.oid.to_owned(),
                    prefix: path,
                });
                continue;
            }
            // A blob: fetch + record a bounded content prefix.
            let blob_obj = match src.get(&entry.oid.to_owned()) {
                Ok(Some(o)) if o.kind == ObjectKind::Blob => o,
                _ => continue,
            };
            // Store at most the first N KiB of the blob (a huge file is truncated,
            // still searchable up to the prefix). Lossy UTF-8 (binary → harmless
            // mojibake, never a panic).
            let take = blob_obj.data.len().min(SEARCH_INDEX_BLOB_BYTES_CAP);
            let content = String::from_utf8_lossy(&blob_obj.data[..take]).into_owned();
            repo_bytes += content.len();
            files.push(FileEntry { path, content });
        }
    }
    files
}

/// Spawn a DETACHED background thread that (re)builds `repo_slug`'s code-search index
/// from its LIVE HEAD and installs it — so the next `/search` serves from the index
/// instead of a per-request CAS grep (the latency-DoS this module removes).
///
/// Visibility-gated + best-effort + fail-open:
/// - a repo with no git seam, no HEAD (refless/empty), or a slug whose log won't
///   load is a NO-OP (its `/search` code results are honest-empty);
/// - a repo that is NOT publicly searchable (private / erased) is a NO-OP — it is
///   NEVER indexed, so its content can never leak via `/search`;
/// - the per-repo guard admits at most ONE build at a time;
/// - a build panic is isolated to the thread.
///
/// NEVER blocks the caller — the whole bounded walk runs on the detached thread, so
/// it is safe to call from the accept-loop bootstrap AND post-`ok` on a push worker.
pub fn spawn_search_index_build(state: &AppState, repo_slug: &str) {
    let Some(repo_state) = state.repo_state(repo_slug) else {
        return;
    };
    // VISIBILITY GATE (build time): only index a PUBLIC (anonymous-readable) repo.
    // Load + project the repo's meta off-loop (this runs on a detached thread, so the
    // load cost never touches the accept loop). An un-loadable log or a non-public /
    // erased repo → NO-OP (no index entry → honest-empty `/search`, no leak).
    let Ok(log) = state.load_verified(repo_slug) else {
        return;
    };
    if !is_publicly_searchable(&state.repo_meta_cached(repo_slug, &log)) {
        return;
    }
    // The LIVE HEAD (hot-swapped `git_refs` tip). No tip → nothing to index.
    let Some(head) = repo_state.head_commit() else {
        return;
    };
    // Resolve the root tree from the LIVE head (a push hot-swaps the ref but NOT the
    // boot `git_root_tree` snapshot); fall back to the boot snapshot if unresolvable.
    let root_tree = hugit_proto::commit_root_tree(repo_state.git_source.as_ref(), &head)
        .ok()
        .flatten()
        .unwrap_or(repo_state.git_root_tree);

    let store = state.search_index.clone();
    let Some(guard) = store.try_reserve_build(repo_slug) else {
        return; // a build for this repo is already running
    };
    let source: Arc<dyn ObjectSource + Send + Sync> = Arc::clone(&repo_state.git_source);
    let slug = repo_slug.to_string();
    let already_stored = store.total_bytes();

    let spawned = std::thread::Builder::new()
        .name("hugit-search-index".into())
        .spawn(move || {
            let _guard = guard; // releases the slot at thread end OR on a panic
            let built = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                build_index_files(
                    source.as_ref(),
                    &root_tree,
                    SEARCH_INDEX_BUILD_BUDGET,
                    already_stored,
                )
            }));
            match built {
                Ok(files) => store.install(&slug, BuiltIndex { head, files }),
                Err(_) => eprintln!("hugit-serve: search index build for {slug} panicked"),
            }
        });
    if spawned.is_err() {
        // Thread exhaustion: the un-run closure (and its `guard`) was dropped, which
        // already released the slot — nothing more to do (a MISS stays honest-empty).
        eprintln!("hugit-serve: search index build thread spawn failed for {repo_slug}");
    }
}

/// At engine start, kick a background code-search-index build for every loaded
/// (boot-set) repo. Runs on the serve thread BEFORE the accept loop but only SPAWNS
/// the detached builds — the boot path is never blocked on the walk (the same
/// off-boot discipline as the clone-pack + blob-history bootstraps). Until a build
/// lands, `/search` code results are honest-empty (never a live fetch).
pub fn bootstrap_search_indexes(state: &AppState) {
    let slugs: Vec<String> = state.repos.keys().cloned().collect();
    for slug in slugs {
        spawn_search_index_build(state, &slug);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::{CasObjectSource, GitObject, PackError};

    const PAT: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";

    fn oid(seed: u8) -> ObjectId {
        ObjectId::from_hex(format!("{seed:02x}").repeat(20).as_bytes()).unwrap()
    }

    /// Build a minimal CAS: a root tree with the given `(name, content)` blobs.
    fn cas_with_files(files: &[(&str, &[u8])]) -> (CasObjectSource, ObjectId) {
        let mut cas = CasObjectSource::new();
        let mut tree_bytes = Vec::new();
        for (name, content) in files {
            let blob_oid = cas.insert_raw(ObjectKind::Blob, content.to_vec());
            tree_bytes.extend_from_slice(b"100644 ");
            tree_bytes.extend_from_slice(name.as_bytes());
            tree_bytes.push(0);
            tree_bytes.extend_from_slice(blob_oid.as_bytes());
        }
        let root = cas.insert_raw(ObjectKind::Tree, tree_bytes);
        (cas, root)
    }

    /// The build records a repo's file PATHS + content (bounded), so a query finds
    /// real matches.
    #[test]
    fn build_records_paths_and_content() {
        let (cas, root) = cas_with_files(&[
            ("lib.rs", b"fn frobnicate(x: u32) -> u32 { x + 1 }\n"),
            ("README.md", b"# hello world\n"),
        ]);
        let files = build_index_files(&cas, &root, SEARCH_INDEX_BUILD_BUDGET, 0);
        assert_eq!(files.len(), 2, "both files recorded");
        assert!(files.iter().any(|f| f.path == "lib.rs"));
        assert!(files.iter().any(|f| f.path == "README.md"));
        assert!(
            files
                .iter()
                .find(|f| f.path == "lib.rs")
                .unwrap()
                .content
                .contains("frobnicate")
        );
    }

    /// A query returns real matches FROM THE INDEX (repo + path + snippet), capped.
    #[test]
    fn query_returns_real_matches_from_index() {
        let store = SearchStore::new();
        let (cas, root) =
            cas_with_files(&[("lib.rs", b"fn frobnicate() {}\nlet frobnicate = 1;\n")]);
        let files = build_index_files(&cas, &root, SEARCH_INDEX_BUILD_BUDGET, 0);
        let head = oid(0xaa);
        store.install("r", BuiltIndex { head, files });

        let (hits, total) = store.search("r", Some(&head), "frobnicate", SEARCH_QUERY_BUDGET);
        assert_eq!(hits.len(), 1, "one file matched");
        assert_eq!(hits[0].path, "lib.rs");
        assert_eq!(hits[0].lines.len(), 2, "both lines matched");
        assert_eq!(hits[0].lines[0].0, 1, "1-based line number");
        assert_eq!(total, 2, "code_total is the true match count");
    }

    /// A query for a repo with NO index entry (a private repo is never indexed →
    /// absent) returns nothing — the visibility guarantee at the store level.
    #[test]
    fn absent_slug_private_repo_returns_nothing() {
        let store = SearchStore::new();
        // Only a PUBLIC repo was indexed; the private one has no entry.
        let (cas, root) = cas_with_files(&[("lib.rs", b"fn frobnicate() {}\n")]);
        let files = build_index_files(&cas, &root, SEARCH_INDEX_BUILD_BUDGET, 0);
        store.install(
            "public-repo",
            BuiltIndex {
                head: oid(1),
                files,
            },
        );

        let (hits, total) = store.search("private-repo", None, "frobnicate", SEARCH_QUERY_BUDGET);
        assert!(hits.is_empty(), "a repo with no index entry yields no hits");
        assert_eq!(total, 0);
    }

    /// The visibility PREDICATE the build gates on: PUBLIC → indexable; PRIVATE and
    /// ERASED → NEVER indexed (so `/search` can never leak private content).
    #[test]
    fn visibility_predicate_public_only() {
        use crate::authz::{RepoMeta, Visibility};
        let public = RepoMeta {
            visibility: Visibility::Public,
            owner_tenant: Some("org-a".into()),
            erased: false,
        };
        let private = RepoMeta {
            visibility: Visibility::Private,
            owner_tenant: Some("org-a".into()),
            erased: false,
        };
        let erased = RepoMeta {
            visibility: Visibility::Public,
            owner_tenant: Some("org-a".into()),
            erased: true,
        };
        assert!(is_publicly_searchable(&public), "public → indexable");
        assert!(!is_publicly_searchable(&private), "private → NEVER indexed");
        assert!(!is_publicly_searchable(&erased), "erased → NEVER indexed");
    }

    /// A panicking source proves the QUERY path never touches the source — a query is
    /// a pure in-memory scan, NEVER a CAS/R2 fetch (the whole point: no fetch fan-out
    /// on the single-threaded accept loop).
    struct PanicSource;
    impl ObjectSource for PanicSource {
        fn get(&self, _oid: &ObjectId) -> Result<Option<GitObject>, PackError> {
            panic!("the /search query must NOT touch the source (in-memory only)");
        }
    }

    /// An UNBUILT index → honest-empty, and NO source fetch (the source would panic).
    #[test]
    fn unbuilt_index_is_honest_empty_no_fetch() {
        let store = SearchStore::new();
        let (hits, total) = store.search("never-built", None, "anything", SEARCH_QUERY_BUDGET);
        assert!(hits.is_empty());
        assert_eq!(total, 0);
        // Document the guard the honest-empty path relies on: a query does no `get`.
        let src = PanicSource;
        assert!(std::panic::catch_unwind(|| src.get(&oid(0))).is_err());
    }

    /// A stale HEAD (a push moved the tip) → honest-empty (the stale index is refused,
    /// NOT served as plausible-but-old). It is a MISS, never a live fetch.
    #[test]
    fn stale_head_is_refused_honest_empty() {
        let store = SearchStore::new();
        let (cas, root) = cas_with_files(&[("lib.rs", b"fn frobnicate() {}\n")]);
        let files = build_index_files(&cas, &root, SEARCH_INDEX_BUILD_BUDGET, 0);
        store.install(
            "r",
            BuiltIndex {
                head: oid(0xaa),
                files,
            },
        );
        // A HIT under the matching head…
        assert!(
            !store
                .search("r", Some(&oid(0xaa)), "frobnicate", SEARCH_QUERY_BUDGET)
                .0
                .is_empty()
        );
        // …but the repo's live HEAD moved → the stale index is refused.
        let (hits, total) = store.search("r", Some(&oid(0xbb)), "frobnicate", SEARCH_QUERY_BUDGET);
        assert!(hits.is_empty(), "stale head → honest-empty");
        assert_eq!(total, 0);
    }

    /// The wall-clock query budget bounds the scan: a ZERO budget stops before any
    /// file is scanned (defence-in-depth on the single-threaded engine).
    #[test]
    fn zero_query_budget_stops_immediately() {
        let store = SearchStore::new();
        let (cas, root) = cas_with_files(&[("lib.rs", b"fn frobnicate() {}\n")]);
        let files = build_index_files(&cas, &root, SEARCH_INDEX_BUILD_BUDGET, 0);
        let head = oid(0xaa);
        store.install("r", BuiltIndex { head, files });
        let (hits, total) = store.search("r", Some(&head), "frobnicate", Duration::ZERO);
        assert!(hits.is_empty(), "zero budget scans nothing");
        assert_eq!(total, 0);
    }

    /// The per-file byte cap bounds memory: a blob larger than the cap is TRUNCATED to
    /// the prefix (a match past the prefix is not recorded).
    #[test]
    fn per_file_byte_cap_truncates() {
        // A blob padded past the cap with the needle only AFTER the cap boundary.
        let mut content = vec![b'a'; SEARCH_INDEX_BLOB_BYTES_CAP + 100];
        content.extend_from_slice(b"\nNEEDLE_PAST_CAP\n");
        let (cas, root) = cas_with_files(&[("big.txt", &content)]);
        let files = build_index_files(&cas, &root, SEARCH_INDEX_BUILD_BUDGET, 0);
        assert_eq!(files.len(), 1);
        assert!(
            files[0].content.len() <= SEARCH_INDEX_BLOB_BYTES_CAP,
            "content is truncated to the per-file cap"
        );
        assert!(
            !files[0].content.contains("NEEDLE_PAST_CAP"),
            "a match past the byte cap is not recorded"
        );
    }

    /// The global byte cap stops a build from adding files once the store is full —
    /// the index itself can never become an unbounded OOM vector.
    #[test]
    fn global_byte_cap_stops_build() {
        let (cas, root) = cas_with_files(&[("a.txt", b"hello\n"), ("b.txt", b"world\n")]);
        // Pretend the store is already AT the global cap → this build records nothing.
        let files = build_index_files(
            &cas,
            &root,
            SEARCH_INDEX_BUILD_BUDGET,
            SEARCH_INDEX_GLOBAL_BYTES_CAP,
        );
        assert!(
            files.is_empty(),
            "at the global cap the build adds no files"
        );
    }

    /// Content is stored UNSCRUBBED but every emitted match line is SCRUBBED at the
    /// read boundary — a secret in a file never reaches the VM.
    #[test]
    fn secret_scrubbed_at_read_boundary() {
        let store = SearchStore::new();
        let content = format!("const KEY: &str = \"{PAT}\";\n");
        let (cas, root) = cas_with_files(&[("secret.rs", content.as_bytes())]);
        let files = build_index_files(&cas, &root, SEARCH_INDEX_BUILD_BUDGET, 0);
        // Stored raw (unscrubbed) — the secret IS in the recorded content…
        assert!(
            files.iter().any(|f| f.content.contains(PAT)),
            "content is stored unscrubbed (scrubbed at the read boundary)"
        );
        let head = oid(0xaa);
        store.install("r", BuiltIndex { head, files });
        // …but a query that surfaces the line SCRUBS it — the raw secret never escapes.
        let (hits, _) = store.search("r", Some(&head), "key", SEARCH_QUERY_BUDGET);
        assert_eq!(hits.len(), 1);
        let j = serde_json::to_string(&hits).unwrap();
        assert!(
            !j.contains(PAT),
            "raw secret must not appear in the VM: {j}"
        );
        assert!(
            j.contains("[REDACTED]"),
            "REDACTED sentinel in place of the secret"
        );
    }

    /// End-to-end: `handlers::build_search` served through a seeded index returns
    /// real code matches (repo + path + snippet) — proving the `/search` code path is
    /// wired to the in-memory index, not the removed live CAS grep.
    #[test]
    fn build_search_serves_code_from_the_index() {
        use hugit_refstore::EventLog;
        let store = SearchStore::new();
        let (cas, root) = cas_with_files(&[("src/lib.rs", b"fn frobnicate() {}\n")]);
        let files = build_index_files(&cas, &root, SEARCH_INDEX_BUILD_BUDGET, 0);
        let head = oid(0xaa);
        store.install("humangr/hugit", BuiltIndex { head, files });

        let ci = CodeIndexQuery {
            store: &store,
            slug: "humangr/hugit",
            head: Some(head),
        };
        let vm = crate::handlers::build_search(
            &EventLog::new(),
            "humangr/hugit",
            "frobnicate",
            Some(&ci),
        );
        assert_eq!(vm.code.len(), 1, "a real code match from the index");
        assert_eq!(vm.code[0].path, "src/lib.rs");
        assert_eq!(vm.code[0].lines[0].0, 1);
        assert!(vm.code_total >= 1);
        // A MISS (wrong slug) → honest-empty, never a live scan.
        let ci_miss = CodeIndexQuery {
            store: &store,
            slug: "someone/private",
            head: Some(head),
        };
        let vm_miss = crate::handlers::build_search(
            &EventLog::new(),
            "someone/private",
            "frobnicate",
            Some(&ci_miss),
        );
        assert!(
            vm_miss.code.is_empty(),
            "unindexed repo → honest-empty code"
        );
        assert_eq!(vm_miss.code_total, 0);
    }

    /// The build-slot guard is single-flight + releases on drop (mirrors blob-history).
    #[test]
    fn build_slot_single_flight_and_releases() {
        let store = SearchStore::new();
        let g1 = store.try_reserve_build("r");
        assert!(g1.is_some());
        assert!(
            store.try_reserve_build("r").is_none(),
            "second reservation refused"
        );
        drop(g1);
        assert!(
            store.try_reserve_build("r").is_some(),
            "slot released on drop"
        );
    }

    /// `install` keeps the global byte counter accurate across a REPLACE (a rebuild
    /// swaps the old index): total = new bytes, not old+new.
    #[test]
    fn install_adjusts_global_byte_counter() {
        let store = SearchStore::new();
        let (cas1, root1) = cas_with_files(&[("a.txt", b"aaaaaaaaaa\n")]);
        let f1 = build_index_files(&cas1, &root1, SEARCH_INDEX_BUILD_BUDGET, 0);
        let b1: usize = f1.iter().map(|f| f.content.len()).sum();
        store.install(
            "r",
            BuiltIndex {
                head: oid(1),
                files: f1,
            },
        );
        assert_eq!(store.total_bytes(), b1);
        // Rebuild with a SMALLER index → the counter reflects the replacement, not the sum.
        let (cas2, root2) = cas_with_files(&[("b.txt", b"bb\n")]);
        let f2 = build_index_files(&cas2, &root2, SEARCH_INDEX_BUILD_BUDGET, 0);
        let b2: usize = f2.iter().map(|f| f.content.len()).sum();
        store.install(
            "r",
            BuiltIndex {
                head: oid(2),
                files: f2,
            },
        );
        assert_eq!(
            store.total_bytes(),
            b2,
            "global counter = replacement bytes, not old+new"
        );
    }
}
