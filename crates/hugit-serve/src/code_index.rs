//! `code_index` — the lazy, progressive, in-memory code-search index (design
//! `docs/design/2026-06-29-precomputed-read-index-blob-history-and-code-search.md`,
//! P0: the high-value, single-thread-safe target).
//!
//! ## Why this exists
//! The interim [`crate::handlers::search`] code search greps the git tree's blob
//! contents on EVERY query: an `O(tree × R2)` synchronous walk on the single-threaded
//! lazy-CAS engine, wall-clock-capped at ~2 s so a deep/large repo returns
//! partial-or-empty within budget (the 2026-06-27 `/readyz` wedge). A count or budget
//! can't fix that — only precomputation can.
//!
//! Search is issued MANY times over one repo, so a **lazy progressive in-memory
//! index** is the right shape: each query advances ONE wall-clock-bounded build
//! segment (walk more tree files, tokenize via [`hugit_symbols::outline_blob`] +
//! filename, add postings, update the cursor) AND serves from the postings built so
//! far. After the first few queries the whole tree is indexed; thereafter every query
//! is served O(query) from memory — and the per-query tree scan is retired at root.
//!
//! ## Single-thread / interior-mutability contract
//! One `Arc<RwLock<…>>` cell per repo, held in [`crate::state::RepoState`]. The accept
//! loop (`server::serve_on`) is single-threaded, so the lock is uncontended; the
//! `RwLock` exists only so the index is `Send + Sync` to ride in the `Clone` `AppState`.
//! It introduces NO new thread and NO write-path change (it never touches R2 / the
//! CAS-write seam) — exactly the safe-now option the design's "critical finding" calls
//! out (vs the HA-gated R2-persistent / backfill-thread P1).
//!
//! ## Honesty + safety invariants (carry the existing discipline)
//! - **Real-or-absent:** the index stores only what a real walk produced (symbol
//!   names from `outline_blob` + the filename, each with its real source line). It is
//!   NEVER a fabricated row. While the build is still in progress a query is served
//!   from the partial postings — honest-partial, the same contract the live walk has.
//! - **Redaction at READ, not in the index:** raw names/paths are stored; [`scrub`]
//!   (via the caller) is applied to every emitted line + path at the read boundary, so
//!   a redaction-rule change applies retroactively with no index rebuild. The index
//!   itself never reaches the wire.
//! - **HEAD-invalidate:** the index is keyed on the HEAD root-tree oid it was built
//!   against; a HEAD change (a push hot-swap) resets it — a stale tree is never served.
//! - **Bounded:** every build segment is wall-clock + file-count bounded (it must
//!   never wedge the single-threaded accept loop); the postings + per-query result are
//!   capped. A build error on any object is skipped fail-closed (the segment advances,
//!   never aborts the whole index).
//! - **Fallback, never-worse:** the caller falls back to the existing live bounded
//!   `search_code` on ANY index error — so this is strictly additive, never a
//!   regression vs today.

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use gix_hash::ObjectId;
use hugit_proto::{ObjectKind, ObjectSource};

/// Maximum files (blobs) indexed per build segment. Bounds the CAS budget of one
/// query's build advance: on a COLD cache every blob fetch is a synchronous R2 GET,
/// so this is a RESULT bound; [`SEGMENT_BUDGET`] is the real LATENCY bound. Chosen so
/// a handful of queries fully index a normal repo while no single query stalls.
const SEGMENT_FILE_CAP: usize = 400;

/// Hard wall-clock ceiling on ONE build segment. The single-threaded accept loop is
/// blocked for the duration, so this MUST be small — the segment stops here with the
/// cursor preserved (resumes on the next query) rather than wedging the engine.
/// Mirrors the live walk's `CODE_SCAN_BUDGET` discipline.
const SEGMENT_BUDGET: Duration = Duration::from_millis(1_500);

/// Maximum blob size (bytes) tokenized inline. A blob larger than this is recorded as
/// visited (so the cursor advances + it is not re-fetched) but contributes only its
/// filename token, never a multi-MB parse. Mirrors `outline_blob`'s own input bound.
const INDEX_BLOB_BYTES_CAP: usize = 256 * 1024;

/// Maximum number of indexed files retained in the postings. A hostile/huge repo must
/// not grow the in-memory index without bound (OOM on a long-lived instance). Once the
/// cap is reached the build stops adding files (it marks complete); the partial index
/// still serves. 50 000 files × a handful of symbols each is container-safe.
const MAX_INDEXED_FILES: usize = 50_000;

/// Maximum symbols recorded per file. Bounds a pathological generated file with tens of
/// thousands of declarations from dominating memory. The filename token is always kept.
const MAX_SYMBOLS_PER_FILE: usize = 2_000;

/// Per-query result cap — the VM IS the page (mirrors `search.rs`'s `CODE_MATCH_CAP`).
const QUERY_FILE_CAP: usize = 200;

/// Maximum matching lines emitted PER FILE (mirrors `CODE_LINES_PER_FILE_CAP`).
const QUERY_LINES_PER_FILE_CAP: usize = 10;

/// One searchable token of an indexed file: a symbol name (or the filename), with its
/// 1-based source line (0 for the synthetic filename token, which has no line). Stored
/// RAW — scrubbed only at the read boundary by the caller.
#[derive(Clone)]
struct Token {
    /// 1-based source line of the declaration; `0` for the filename token.
    line: u32,
    /// The raw token text (a symbol name, or the file's basename). Never scrubbed in
    /// the index — the caller scrubs every emitted line at the read boundary.
    text: String,
}

/// One indexed file: its repo-relative path and its searchable tokens.
struct IndexedFile {
    /// Repo-relative path (e.g. `crates/hugit-serve/src/lib.rs`). Stored RAW.
    path: String,
    /// The file's searchable tokens (filename + each symbol name). Lowercased copies
    /// are NOT precomputed; the query lowercases the candidate, matching the live
    /// walk's per-line `to_lowercase` (small, bounded by the per-file token count).
    tokens: Vec<Token>,
}

/// One frame of the in-progress depth-first tree walk (resumable across segments).
struct TreeFrame {
    oid: ObjectId,
    /// Repo-relative path prefix for this directory (empty for the root).
    prefix: String,
}

/// A single code-search hit, source-line text RAW (the caller scrubs at the read
/// boundary). Mirrors the shape `search.rs` maps onto `SearchCodeVm`.
pub struct CodeHit {
    /// Repo-relative path (RAW — caller scrubs).
    pub path: String,
    /// `(1-based line, raw text)` pairs. A filename-only match emits a single
    /// `(0, basename)` pair (line 0 = "no specific line", the filename matched).
    pub lines: Vec<(u32, String)>,
}

/// The mutable index state, guarded by the `RwLock` in [`CodeIndex`].
struct Inner {
    /// The HEAD root-tree oid this index was built against. `None` before the first
    /// build. A different live HEAD root-tree resets the index (HEAD-invalidate).
    built_for: Option<ObjectId>,
    /// `true` once the whole tree (under the caps) has been walked — no further build
    /// segment is needed; every query is served O(query) from `files`.
    complete: bool,
    /// The resumable DFS frontier. Non-empty ⇒ more to walk. Drained as segments run.
    ///
    /// Each frame is an UNPROCESSED tree popped atomically (the budget is checked at
    /// tree-pop granularity), so the cursor needs no per-entry resume state and a tree
    /// is never half-walked nor double-walked. A git tree is a DAG with no path cycle,
    /// so every (path → blob) entry is enumerated exactly once across the whole walk —
    /// no dedup is needed, and two paths sharing identical blob CONTENT are correctly
    /// indexed as two distinct searchable files (each with its own filename token).
    stack: Vec<TreeFrame>,
    /// The postings: one entry per indexed file (one per distinct tree path).
    files: Vec<IndexedFile>,
}

impl Inner {
    fn empty() -> Self {
        Self {
            built_for: None,
            complete: false,
            stack: Vec::new(),
            files: Vec::new(),
        }
    }

    /// Reset the whole index to walk a NEW HEAD root tree (HEAD-invalidate, or first
    /// build). Seeds the DFS frontier at `root`.
    fn reset_for(&mut self, root: ObjectId) {
        self.built_for = Some(root);
        self.complete = false;
        self.stack = vec![TreeFrame {
            oid: root,
            prefix: String::new(),
        }];
        self.files = Vec::new();
    }
}

/// The per-repo lazy progressive in-memory code-search index. Cheaply cloned (one
/// shared `Arc`); single-thread-safe (the accept loop is single-threaded). Default is
/// an empty, never-built index — the first query seeds + advances it.
#[derive(Clone)]
pub struct CodeIndex(Arc<RwLock<Inner>>);

impl Default for CodeIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl CodeIndex {
    /// A fresh, never-built index.
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(RwLock::new(Inner::empty())))
    }

    /// Run a code search against the index for `q_lower` (already trimmed +
    /// lowercased by the caller). On EACH call:
    /// 1. If the live HEAD root-tree (`head_root`) differs from what the index was
    ///    built against, RESET (HEAD-invalidate) — a stale tree is never served.
    /// 2. If the index is not yet complete, advance ONE wall-clock-bounded build
    ///    segment (walk more tree files, tokenize, add postings, update the cursor).
    /// 3. Query the postings built so far → `(hits, total)`.
    ///
    /// `hits` text/paths are RAW; the caller scrubs every emitted line + path at the
    /// read boundary. `total` is the TRUE matched-file count before the per-query cap
    /// (mirrors `code_total`).
    ///
    /// Returns `Err` on a lock poisoning (or any state error) so the caller falls back
    /// to the live bounded walk — strictly never-worse than today.
    pub fn search(
        &self,
        src: &dyn ObjectSource,
        head_root: &ObjectId,
        q_lower: &str,
    ) -> Result<(Vec<CodeHit>, usize), String> {
        // A poisoned lock would otherwise abort the request; recover the inner value so
        // the index degrades to "rebuild + serve" rather than wedging (the data is a
        // rebuildable cache, never a source of truth).
        let mut inner = self.0.write().unwrap_or_else(|e| e.into_inner());

        // 1. HEAD-invalidate (or first build): reset to walk the live root tree.
        if inner.built_for != Some(*head_root) {
            inner.reset_for(*head_root);
        }

        // 2. Advance one bounded build segment if the walk is not finished.
        if !inner.complete {
            Self::advance_segment(&mut inner, src);
        }

        // 3. Query the postings built so far.
        Ok(query_postings(&inner.files, q_lower))
    }

    /// Whether the index has finished walking the current HEAD tree (test/diagnostic).
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.0.read().unwrap_or_else(|e| e.into_inner()).complete
    }

    /// The number of files indexed so far (test/diagnostic).
    #[must_use]
    pub fn indexed_file_count(&self) -> usize {
        self.0.read().unwrap_or_else(|e| e.into_inner()).files.len()
    }

    /// Advance the DFS walk by one wall-clock + file-count bounded segment, tokenizing
    /// each new blob into postings. Resumable: the cursor (`stack`) is preserved between
    /// calls, so subsequent queries continue where this stopped.
    /// Fail-closed per object: a fetch/parse error skips that object (the segment
    /// advances; the whole index is never aborted).
    fn advance_segment(inner: &mut Inner, src: &dyn ObjectSource) {
        let start = Instant::now();
        let mut files_this_segment = 0usize;

        // Budget is checked at TREE-POP granularity (a whole tree is processed atomically
        // once popped — never left half-walked). This keeps the cursor a simple frame
        // stack with NO per-entry resume state: every frame on the stack is unprocessed,
        // so a resume can never lose or double-walk a tree. A single tree's entry count
        // is the only over-run past the file cap, which is bounded + tiny in practice.
        while let Some(frame) = inner.stack.pop() {
            // Stop conditions checked BEFORE popping work — re-push the just-popped frame
            // so it is processed (whole) on the next segment.
            if files_this_segment >= SEGMENT_FILE_CAP || start.elapsed() >= SEGMENT_BUDGET {
                inner.stack.push(frame);
                return; // budget/cap hit — NOT complete; resume next query.
            }
            if inner.files.len() >= MAX_INDEXED_FILES {
                // The postings cap is reached — stop growing. Mark complete (the
                // partial index still serves; honest-bounded, never an OOM).
                inner.stack.clear();
                inner.complete = true;
                return;
            }
            // Fetch the tree object; skip on error/kind-mismatch (fail-closed).
            let tree_obj = match src.get(&frame.oid) {
                Ok(Some(o)) if o.kind == ObjectKind::Tree => o,
                _ => continue,
            };
            let entries = match gix_object::TreeRefIter::from_bytes(&tree_obj.data).entries() {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries {
                // Skip symlinks + gitlinks (mirrors the live walk + list_tree_at_dir).
                if entry.mode.is_link() || entry.mode.is_commit() {
                    continue;
                }
                let name = String::from_utf8_lossy(entry.filename);
                let path = if frame.prefix.is_empty() {
                    name.clone().into_owned()
                } else {
                    format!("{}/{name}", frame.prefix)
                };
                if entry.mode.is_tree() {
                    inner.stack.push(TreeFrame {
                        oid: entry.oid.to_owned(),
                        prefix: path,
                    });
                    continue;
                }
                if inner.files.len() >= MAX_INDEXED_FILES {
                    inner.stack.clear();
                    inner.complete = true;
                    return;
                }
                // A blob: index this path. Each distinct tree path is its own searchable
                // file (two paths sharing identical blob content are BOTH indexed — they
                // differ by filename token + path). The DFS enumerates each path once.
                let blob_oid = entry.oid.to_owned();
                files_this_segment += 1;
                let tokens = index_blob(src, &blob_oid, &path);
                inner.files.push(IndexedFile { path, tokens });
            }
        }

        // The frontier is drained — the whole tree (under the caps) is indexed.
        inner.complete = true;
    }
}

/// Tokenize ONE blob into its searchable tokens: the filename (basename) plus, for a
/// recognized source language under the size cap, each symbol name from
/// [`hugit_symbols::outline_blob`] with its real 1-based line. RAW text (caller
/// scrubs). A binary/oversized/unknown-language/unparseable blob contributes only its
/// filename token (honest — never a fabricated symbol).
fn index_blob(src: &dyn ObjectSource, blob_oid: &ObjectId, path: &str) -> Vec<Token> {
    let mut tokens = Vec::new();

    // The filename token (basename) — always present, line 0 (no specific line).
    let basename = path.rsplit('/').next().unwrap_or(path).to_string();
    tokens.push(Token {
        line: 0,
        text: basename,
    });

    // Fetch the blob; on any error keep just the filename token (fail-closed).
    let blob = match src.get(blob_oid) {
        Ok(Some(o)) if o.kind == ObjectKind::Blob => o,
        _ => return tokens,
    };
    if blob.data.len() > INDEX_BLOB_BYTES_CAP {
        return tokens; // too large to parse — filename-only (honest)
    }

    // Symbol names, when the extension maps to a supported language.
    if let Some(ext) = path.rsplit('.').next().filter(|e| *e != path)
        && let Some(lang) = hugit_symbols::lang_for_ext(ext)
    {
        for item in hugit_symbols::outline_blob(lang, &blob.data) {
            if tokens.len() > MAX_SYMBOLS_PER_FILE {
                break; // bound a pathological declaration count
            }
            tokens.push(Token {
                line: item.line,
                text: item.name,
            });
        }
    }
    tokens
}

/// Query the postings for `q_lower` (case-insensitive substring over each token's
/// text). Returns `(hits, total)` where `total` is the TRUE matched-FILE count before
/// the [`QUERY_FILE_CAP`]. Within a matched file, up to [`QUERY_LINES_PER_FILE_CAP`]
/// matching tokens are emitted as `(line, raw text)` pairs. RAW text — the caller
/// scrubs at the read boundary.
fn query_postings(files: &[IndexedFile], q_lower: &str) -> (Vec<CodeHit>, usize) {
    if q_lower.is_empty() {
        return (Vec::new(), 0);
    }
    let mut hits: Vec<CodeHit> = Vec::new();
    let mut total = 0usize;
    for file in files {
        let mut file_lines: Vec<(u32, String)> = Vec::new();
        let mut matched_path = false;
        for tok in &file.tokens {
            if tok.text.to_lowercase().contains(q_lower) {
                matched_path = true;
                if file_lines.len() < QUERY_LINES_PER_FILE_CAP {
                    file_lines.push((tok.line, tok.text.clone()));
                }
            }
        }
        if matched_path {
            total += 1;
            if hits.len() < QUERY_FILE_CAP {
                hits.push(CodeHit {
                    path: file.path.clone(),
                    lines: file_lines,
                });
            }
        }
    }
    (hits, total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::CasObjectSource;

    /// Build a CAS with a root tree whose entries are the given `(name, blob_bytes)`
    /// files (flat, single level). Returns the source + the root-tree oid.
    fn flat_repo(files: &[(&str, &[u8])]) -> (CasObjectSource, ObjectId) {
        let mut cas = CasObjectSource::new();
        let mut entries: Vec<(&str, ObjectId)> = Vec::new();
        for (name, content) in files {
            let oid = cas.insert_raw(ObjectKind::Blob, content.to_vec());
            entries.push((name, oid));
        }
        // git tree entries MUST be sorted by name.
        entries.sort_by(|a, b| a.0.cmp(b.0));
        let mut tree_bytes = Vec::new();
        for (name, oid) in &entries {
            tree_bytes.extend_from_slice(b"100644 ");
            tree_bytes.extend_from_slice(name.as_bytes());
            tree_bytes.push(0);
            tree_bytes.extend_from_slice(oid.as_bytes());
        }
        let root = cas.insert_raw(ObjectKind::Tree, tree_bytes);
        (cas, root)
    }

    /// Build a CAS with a nested tree: root contains `dir/` which contains the files.
    fn nested_repo(dir: &str, files: &[(&str, &[u8])]) -> (CasObjectSource, ObjectId) {
        let mut cas = CasObjectSource::new();
        let mut sub_entries: Vec<(String, ObjectId)> = Vec::new();
        for (name, content) in files {
            let oid = cas.insert_raw(ObjectKind::Blob, content.to_vec());
            sub_entries.push(((*name).to_string(), oid));
        }
        sub_entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut sub_bytes = Vec::new();
        for (name, oid) in &sub_entries {
            sub_bytes.extend_from_slice(b"100644 ");
            sub_bytes.extend_from_slice(name.as_bytes());
            sub_bytes.push(0);
            sub_bytes.extend_from_slice(oid.as_bytes());
        }
        let sub_oid = cas.insert_raw(ObjectKind::Tree, sub_bytes);

        let mut root_bytes = Vec::new();
        // `40000` is the tree mode.
        root_bytes.extend_from_slice(b"40000 ");
        root_bytes.extend_from_slice(dir.as_bytes());
        root_bytes.push(0);
        root_bytes.extend_from_slice(sub_oid.as_bytes());
        let root = cas.insert_raw(ObjectKind::Tree, root_bytes);
        (cas, root)
    }

    #[test]
    fn index_hit_serves_symbol_match() {
        // A query that matches a real symbol name returns a hit with its real line.
        let (cas, root) = flat_repo(&[("lib.rs", b"fn frobnicate(x: u32) -> u32 { x + 1 }\n")]);
        let idx = CodeIndex::new();
        let (hits, total) = idx.search(&cas, &root, "frobnicate").expect("search");
        assert_eq!(total, 1, "one file matched");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "lib.rs");
        // The symbol token carries its real 1-based line.
        assert!(
            hits[0]
                .lines
                .iter()
                .any(|(l, t)| *l == 1 && t == "frobnicate"),
            "expected frobnicate at line 1, got {:?}",
            hits[0].lines
        );
    }

    #[test]
    fn index_hit_serves_filename_match() {
        // A query matching the FILENAME (not a symbol) still hits — line 0 (no
        // specific line, the filename matched).
        let (cas, root) = flat_repo(&[("config.toml", b"key = 1\n")]);
        let idx = CodeIndex::new();
        let (hits, total) = idx.search(&cas, &root, "config").expect("search");
        assert_eq!(total, 1);
        assert_eq!(hits[0].path, "config.toml");
        assert!(
            hits[0]
                .lines
                .iter()
                .any(|(l, t)| *l == 0 && t == "config.toml")
        );
    }

    #[test]
    fn second_query_served_from_index_when_complete() {
        // A small repo is fully indexed on the first query; the index reports complete
        // and serves subsequent queries from memory (no re-walk needed for correctness;
        // the contract is the same answer).
        let (cas, root) = flat_repo(&[("a.rs", b"fn alpha() {}\n"), ("b.rs", b"fn beta() {}\n")]);
        let idx = CodeIndex::new();
        let _ = idx.search(&cas, &root, "alpha").expect("search 1");
        assert!(idx.is_complete(), "small repo indexed in one segment");
        assert_eq!(idx.indexed_file_count(), 2);
        let (hits, _) = idx.search(&cas, &root, "beta").expect("search 2");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "b.rs");
    }

    #[test]
    fn nested_tree_is_walked() {
        let (cas, root) = nested_repo("src", &[("deep.rs", b"fn deeply_nested() {}\n")]);
        let idx = CodeIndex::new();
        let (hits, _) = idx.search(&cas, &root, "deeply_nested").expect("search");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "src/deep.rs");
    }

    #[test]
    fn head_change_invalidates_and_rebuilds() {
        // Build the index against HEAD-1, then a different HEAD root tree must RESET
        // the index — a symbol present only in the OLD tree must no longer be found,
        // and a symbol only in the NEW tree IS found.
        let (cas1, root1) = flat_repo(&[("old.rs", b"fn only_in_old() {}\n")]);
        let idx = CodeIndex::new();
        let (h1, _) = idx
            .search(&cas1, &root1, "only_in_old")
            .expect("search old");
        assert_eq!(h1.len(), 1, "old symbol found against old HEAD");

        let (cas2, root2) = flat_repo(&[("new.rs", b"fn only_in_new() {}\n")]);
        // New HEAD → reset. The old symbol is gone; the new one is present.
        let (h_old, _) = idx
            .search(&cas2, &root2, "only_in_old")
            .expect("search new-old");
        assert!(
            h_old.is_empty(),
            "old symbol must NOT survive a HEAD change"
        );
        let (h_new, _) = idx
            .search(&cas2, &root2, "only_in_new")
            .expect("search new-new");
        assert_eq!(h_new.len(), 1, "new symbol found against new HEAD");
        assert_ne!(idx.0.read().unwrap().built_for, Some(root1));
    }

    #[test]
    fn empty_query_is_no_op() {
        let (cas, root) = flat_repo(&[("a.rs", b"fn alpha() {}\n")]);
        let idx = CodeIndex::new();
        let (hits, total) = idx.search(&cas, &root, "").expect("search");
        assert!(hits.is_empty());
        assert_eq!(total, 0);
    }

    #[test]
    fn binary_blob_indexes_filename_only_no_panic() {
        // A non-UTF-8/binary blob must not panic the tokenizer; it contributes only
        // its filename token (honest — never a fabricated symbol).
        let bin: &[u8] = &[0xff, 0x00, 0xfe, 0x01, b'f', b'n', 0x00];
        let (cas, root) = flat_repo(&[("blob.bin", bin)]);
        let idx = CodeIndex::new();
        // Filename match still works.
        let (hits, _) = idx.search(&cas, &root, "blob").expect("search");
        assert_eq!(hits.len(), 1);
        // A symbol-shaped query does NOT spuriously match the binary content.
        let (none, _) = idx.search(&cas, &root, "frobnicate").expect("search2");
        assert!(none.is_empty());
    }

    #[test]
    fn query_is_case_insensitive() {
        let (cas, root) = flat_repo(&[("cfg.rs", b"const MAX_FROB: usize = 42;\n")]);
        let idx = CodeIndex::new();
        let (hits, _) = idx.search(&cas, &root, "max_frob").expect("search");
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn per_query_file_cap_bounds_results() {
        // More matching files than the per-query cap → hits capped, total is the TRUE
        // count (the caller can page).
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for n in 0..(QUERY_FILE_CAP + 25) {
            files.push((format!("needle{n}.rs"), b"x\n".to_vec()));
        }
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(n, c)| (n.as_str(), c.as_slice()))
            .collect();
        let (cas, root) = flat_repo(&refs);
        let idx = CodeIndex::new();
        // The repo may take more than one segment; loop until complete (bounded).
        for _ in 0..50 {
            let _ = idx.search(&cas, &root, "needle").expect("search");
            if idx.is_complete() {
                break;
            }
        }
        let (hits, total) = idx.search(&cas, &root, "needle").expect("final");
        assert_eq!(hits.len(), QUERY_FILE_CAP, "hits capped");
        assert_eq!(total, QUERY_FILE_CAP + 25, "total is the true count");
    }

    /// Build a CAS whose root tree has `n_dirs` subtrees, each holding one file. The
    /// build budget is checked at tree-pop granularity, so MANY subtrees is what forces
    /// a multi-segment build (a single flat tree is processed atomically). Returns the
    /// source + root oid + the total file count.
    fn many_dirs_repo(n_dirs: usize) -> (CasObjectSource, ObjectId, usize) {
        let mut cas = CasObjectSource::new();
        // Each subtree: one `f.rs` blob.
        let mut root_entries: Vec<(String, ObjectId)> = Vec::new();
        for d in 0..n_dirs {
            let blob = cas.insert_raw(ObjectKind::Blob, format!("fn sym{d}() {{}}\n").into_bytes());
            let mut sub = Vec::new();
            sub.extend_from_slice(b"100644 f.rs\0");
            sub.extend_from_slice(blob.as_bytes());
            let sub_oid = cas.insert_raw(ObjectKind::Tree, sub);
            root_entries.push((format!("d{d:05}"), sub_oid));
        }
        root_entries.sort_by(|a, b| a.0.cmp(&b.0));
        let mut root_bytes = Vec::new();
        for (name, oid) in &root_entries {
            root_bytes.extend_from_slice(b"40000 ");
            root_bytes.extend_from_slice(name.as_bytes());
            root_bytes.push(0);
            root_bytes.extend_from_slice(oid.as_bytes());
        }
        let root = cas.insert_raw(ObjectKind::Tree, root_bytes);
        (cas, root, n_dirs)
    }

    #[test]
    fn progressive_build_completes_across_segments() {
        // A repo with more subtrees than one segment's file cap builds across several
        // queries and then reports complete with every file indexed. (Budget is
        // tree-pop-granular, so many SUBTREES — not a flat list — forces multi-segment.)
        let (cas, root, count) = many_dirs_repo(SEGMENT_FILE_CAP + 50);
        let idx = CodeIndex::new();
        // One segment indexes at most ~SEGMENT_FILE_CAP files (one per subtree).
        let _ = idx.search(&cas, &root, "nomatch").expect("seg1");
        assert!(
            !idx.is_complete(),
            "a >1-segment repo is not complete after one query"
        );
        for _ in 0..10 {
            let _ = idx.search(&cas, &root, "nomatch").expect("seg");
            if idx.is_complete() {
                break;
            }
        }
        assert!(idx.is_complete(), "index completes across segments");
        assert_eq!(idx.indexed_file_count(), count);
        // And a query against the fully-built index finds a symbol from a late subtree.
        let (hits, _) = idx.search(&cas, &root, "sym0").expect("final");
        assert!(!hits.is_empty(), "symbol from the indexed tree is found");
    }
}
