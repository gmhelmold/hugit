//! `GET /v1/repos/{repo}/blob/{*path}` → [`BlobVm`].
//!
//! Reverses the PS-18 deferral: this handler actually serves file CONTENT, read
//! from a git tree via [`hugit_proto::resolve_blob_at_path`]. The blob bytes are
//! UTF-8-decoded (lossy), split into lines, and SCRUBBED at the read boundary
//! ([`crate::fmt::scrub`] → `hugit_ledger::redact::apply`) — file content may
//! carry secrets, redacted on read exactly as every sibling handler scrubs free
//! text.
//!
//! ## REAL vs honest-default
//! - **REAL** (from the git object): `path` (scrubbed), `lines` (decoded +
//!   scrubbed), `size` (byte count → human string), `lang` (extension → name),
//!   `outline` (W6 — the symbol outline parsed from the blob bytes via
//!   [`hugit_symbols::outline_blob`], symbol names scrubbed at the read boundary;
//!   empty for an unsupported language, never fabricated), `tree` (the sidebar
//!   file-tree: direct children of the directory containing `path`, symlinks and
//!   gitlinks excluded, fail-closed on any missing object).
//! - **HONEST-DEFAULT** (no local engine seam this wave — NOT faked, documented
//!   here): `blame: []` (the deep why-blame is the intent-graph seam),
//!   `via_intent`/`via_model: None` (no last-writer-intent attribution seam).
//!   `actions` mirrors the sibling pt-BR locale.
//!
//! When the git content seam is not wired (`src`/`root_tree` are `None`), or the
//! path does not resolve to a blob, the handler returns `None` → the caller maps
//! that to a 404. An absent git source is the honest "content seam not live"
//! 404, never a fake blank file.

use std::sync::Arc;
use std::time::Instant;

use gix_hash::ObjectId;
use hugit_http_contracts::blob::{BlobHistoryVm, BlobTreeRowVm, BlobVm};
use hugit_refstore::EventLog;

use crate::blob_history_index::BlobHistoryStore;
use crate::budgeted_source::{BudgetedSource, WALK_BUDGET};
use crate::fmt::{humanize_age, scrub};

/// Hard WALL-CLOCK ceiling on the per-path history walk — re-exported from the proto
/// layer so the budget constant lives next to the bounded primitive it guards.
const BLOB_HISTORY_BUDGET: std::time::Duration = hugit_proto::BLOB_HISTORY_BUDGET;
/// Max revisions surfaced in the "Histórico" drawer (a RESULT cap; the wall-clock
/// budget above is the real latency bound on the single-threaded engine).
const MAX_HISTORY_REVS: usize = hugit_proto::MAX_HISTORY_REVS;

/// Maximum blob size (in bytes) that will be buffered into RAM and served.
/// Blobs larger than this return `None` (→ 404) rather than OOMing the server.
/// 10 MB is generous enough for any source file a human would reasonably view.
const MAX_BLOB_BYTES: usize = 10 * 1024 * 1024;

/// Build the blob view-model for `path` at HEAD. `Some(BlobVm)` when the git
/// content seam is wired AND `path` resolves to a blob; `None` otherwise (→ 404).
///
/// `_log` is unused this wave — the content comes from the git tree, not the
/// event log — but the parameter is kept for signature uniformity with the other
/// `build_*` handlers and for the future blame/attribution seam (which WILL read
/// the log to attribute lines to intents).
///
/// DoS: the `path`→blob resolve does one synchronous CAS `get` per path segment
/// ([`hugit_proto::resolve_blob_at_path`], count-capped at `MAX_PATH_DEPTH`), which
/// on the single-threaded lazy-CAS engine is NOT a latency cap under a cold cache /
/// slow R2 — a deep resolve can block the whole accept loop. So the resolve is ALSO
/// wall-clock bounded by [`WALK_BUDGET`] (mirrors `handlers::edit`): past the
/// deadline the source stops yielding and the resolve returns an honest `None`
/// → 404 (never a fabricated file).
#[must_use]
pub fn build_blob(
    _log: &EventLog,
    repo: &str,
    path: &str,
    src: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    root_tree: Option<&ObjectId>,
    head_commit: Option<&ObjectId>,
) -> Option<BlobVm> {
    build_blob_until(
        _log,
        repo,
        path,
        src,
        root_tree,
        head_commit,
        None,
        Instant::now() + WALK_BUDGET,
    )
}

/// [`build_blob`] plus the engine's precomputed per-path blob-history index
/// ([`BlobHistoryStore`], #70(a)) — the LIVE serve entrypoint. The "Histórico" drawer
/// consults the index FIRST (fast + complete for deep history) and falls back to the
/// live wall-clock-bounded walk on any miss (no index yet / a stale HEAD / an
/// un-indexed path). Every other field is identical to [`build_blob`]. Split from
/// [`build_blob`] so the many hermetic `build_blob` tests keep exercising the pure
/// live-walk path (index `None`) unchanged.
#[must_use]
pub fn build_blob_with_history(
    _log: &EventLog,
    repo: &str,
    path: &str,
    src: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    root_tree: Option<&ObjectId>,
    head_commit: Option<&ObjectId>,
    history: Option<&BlobHistoryStore>,
) -> Option<BlobVm> {
    build_blob_until(
        _log,
        repo,
        path,
        src,
        root_tree,
        head_commit,
        history,
        Instant::now() + WALK_BUDGET,
    )
}

/// [`build_blob`] with an explicit wall-clock `deadline` on the CAS path→blob
/// resolve — deterministically testable (a deadline already in the past stops
/// before the first fetch → honest `None`). See [`build_blob`] for the DoS
/// rationale; mirrors `handlers::edit::build_edit_until`.
///
/// (The arg count is deliberately flat — each is an independent, orthogonal input to a
/// single render; bundling them would only obscure the read path, so the lint is
/// allowed here rather than papered over with a struct.)
#[must_use]
#[allow(clippy::too_many_arguments)]
fn build_blob_until(
    _log: &EventLog,
    repo: &str,
    path: &str,
    src: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    root_tree: Option<&ObjectId>,
    head_commit: Option<&ObjectId>,
    history: Option<&BlobHistoryStore>,
    deadline: Instant,
) -> Option<BlobVm> {
    // The content seam: both the source and the root tree must be present.
    let (src, root_tree) = (src?, root_tree?);

    // Wall-clock-bound the per-segment CAS walk (single-thread latency-DoS guard):
    // past `deadline` every `get` returns `Ok(None)`, so `resolve_blob_at_path`
    // stops and yields `None` → an honest 404, never a fabricated file. Mirrors
    // `handlers::edit`.
    let budgeted = BudgetedSource::new(src.as_ref(), deadline);

    // Resolve the path to a blob. Ok(None)/Err/absent → 404 (no content oracle):
    // a malformed/corrupt tree, a missing object, a non-blob path, OR a budget
    // cutoff all yield the honest not-found, never a fabricated file.
    let (_oid, bytes) = match hugit_proto::resolve_blob_at_path(&budgeted, root_tree, path) {
        Ok(Some(found)) => found,
        Ok(None) | Err(_) => return None,
    };

    // DoS / OOM guard: refuse to buffer a multi-GB file into RAM.
    // Return None (→ 404) for oversized blobs rather than exhausting memory.
    if bytes.len() > MAX_BLOB_BYTES {
        return None;
    }

    let size = humanize_bytes(bytes.len());
    let lang = lang_for_path(path);
    let lines = decode_lines(&bytes);

    Some(BlobVm {
        repo: scrub(repo),
        // The URL path is echoed back; scrub it at the read boundary (a crafted
        // path could carry a secret-shaped value into the view-model).
        path: scrub(path),
        size,
        lang,
        // HONEST-DEFAULT: no last-writer-intent attribution seam this wave.
        via_intent: None,
        via_model: None,
        lines,
        // HONEST-DEFAULT: the deep why-blame is the intent-graph seam (not wired).
        blame: vec![],
        // REAL (W6): the symbol outline, parsed from the blob bytes via
        // hugit-symbols. Empty for an unsupported language (honest, not faked).
        // Fail-closed: an outline panic must not 503 the whole file read —
        // degrade to an honest-empty outline (the unsupported-language default).
        outline: std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            compute_outline(path, &bytes)
        }))
        .unwrap_or_default(),
        actions: blob_actions(),
        // REAL: the sidebar file tree — entries in the same directory as
        // `path`. Symlinks + gitlinks are excluded (see `list_tree_at_dir`).
        // Fail-closed: any missing object or malformed tree yields an empty
        // sidebar rather than a 404. Walked over the SAME `budgeted` source (the
        // dir-walk is another per-segment CAS walk), so it shares the render's ONE
        // deadline rather than adding an unbounded latency vector.
        tree: build_tree_sidebar(&budgeted, root_tree, path),
        // REAL: the per-path revision timeline (the "Histórico" drawer). Bounded
        // (wall-clock + count) so a deep history never wedges the single-threaded
        // engine; empty when no HEAD commit is threaded (honest "seam not live").
        // Also walked over `budgeted` — the render's shared deadline caps it on top
        // of its own `BLOB_HISTORY_BUDGET` (whichever trips first). Consults the
        // precomputed per-path index FIRST (#70(a) — deep-history serves complete),
        // falling back to that live walk on any miss.
        history: build_history(&budgeted, repo, head_commit, path, history),
    })
}

/// Build the sidebar file-tree: entries in the same directory as `path`.
///
/// Delegates to [`hugit_proto::list_tree_at_dir`] which walks the git tree to
/// the parent directory and returns its direct children (blobs + subtrees;
/// symlinks and gitlinks excluded). The result is mapped to [`BlobTreeRowVm`]:
/// - `depth` is always 0 — the sidebar shows one directory level (no nesting
///   in this wave).
/// - `current` is `true` for the entry whose name matches the leaf of `path`.
/// - Entry names are scrubbed at the read boundary: a file literally named
///   `ghp_….key` would otherwise appear verbatim in the sidebar and leak the
///   credential-shaped name to the browser.
///
/// Fail-closed: errors or a missing source return an empty `Vec` — the handler
/// never returns a 500 because the sidebar could not be populated.
fn build_tree_sidebar(
    src: &dyn hugit_proto::ObjectSource,
    root_tree: &ObjectId,
    path: &str,
) -> Vec<BlobTreeRowVm> {
    // The leaf name of the current file (e.g. "lib.rs" for "src/lib.rs").
    let current_leaf = path.rsplit('/').next().unwrap_or(path);
    // The parent directory whose children are the sidebar entries — "" for a
    // root-level file (e.g. "README.md"). Used to build each entry's
    // repo-relative navigation path.
    let parent_dir = path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("");

    hugit_proto::list_tree_at_dir(src, root_tree, path)
        .into_iter()
        .map(|e| {
            // Repo-relative navigation path (the window builds `/r/{repo}/blob/{path}`
            // from it — REQUIRED on the wire). Scrubbed at the read boundary like
            // `name`; a directory carries a trailing slash per the contract.
            let raw_path = if parent_dir.is_empty() {
                e.name.clone()
            } else {
                format!("{parent_dir}/{}", e.name)
            };
            let mut entry_path = scrub(&raw_path);
            if e.is_dir {
                entry_path.push('/');
            }
            BlobTreeRowVm {
                // `current` is matched against the RAW name before scrubbing so
                // the comparison is not broken by the REDACTED sentinel.
                current: e.name == current_leaf,
                // Scrub the displayed name: a git entry called `ghp_….key` must not
                // appear verbatim in the sidebar view-model.
                name: scrub(&e.name),
                path: entry_path,
                depth: 0,
                is_dir: e.is_dir,
            }
        })
        .collect()
}

/// Build the per-path revision timeline (the "Histórico" drawer) via the bounded
/// `hugit_proto::blob_history` walk.
///
/// `head_commit` is the repo's default-branch tip (threaded from `AppState`); `None`
/// → empty history (the honest "no HEAD / seam not live" — the drawer stays disabled).
///
/// ## Index-first, live-walk fallback (#70(a))
///
/// The precomputed per-path index ([`BlobHistoryStore`]) is consulted FIRST: for a
/// DEEP-history file it serves the complete newest-first list instantly, where the live
/// walk below would exhaust its 2 s budget on the intervening non-touching commits and
/// return EMPTY. On ANY index miss — no index built yet, a stale HEAD (a push moved the
/// tip), or a path absent from the index — this FALLS BACK to the live walk, so the
/// behaviour is never worse than today's (the index is a pure enhancement over the
/// existing safety bound, not a replacement).
///
/// DoS: the fallback walk is bounded by BOTH a WALL-CLOCK [`BLOB_HISTORY_BUDGET`]
/// deadline AND the [`MAX_HISTORY_REVS`] count — each commit is a synchronous CAS fetch
/// on the single-threaded engine, so an unbounded deep-history walk would wedge the
/// accept loop (the code-search-wedge class). Fail-closed: a bound trip / garbled object
/// returns the partial newest-first list, never an error.
///
/// REDACTION: `author` and `summary` are FREE TEXT from the commit (a commit message
/// or author identity could embed a secret) → both are [`scrub`]bed at the read
/// boundary, exactly like every other free-text field this handler serves — whether the
/// entries came from the index or the live walk (the index stores UNSCRUBBED proto
/// entries; scrub is always applied HERE, so the index is never a redaction bypass).
/// `rev_ref` is a hex oid (no scrub needed); `when` is a humanized timestamp.
fn build_history(
    src: &dyn hugit_proto::ObjectSource,
    repo: &str,
    head_commit: Option<&ObjectId>,
    path: &str,
    history: Option<&BlobHistoryStore>,
) -> Vec<BlobHistoryVm> {
    let Some(head) = head_commit else {
        return vec![];
    };
    // Index-first (deep-history complete); fall back to the live wall-clock-bounded walk
    // on any miss (no index / stale HEAD / un-indexed path) — never a regress.
    let entries = history
        .and_then(|store| store.lookup(repo, head, path))
        .unwrap_or_else(|| {
            let deadline = std::time::Instant::now() + BLOB_HISTORY_BUDGET;
            hugit_proto::blob_history(src, head, path, deadline, MAX_HISTORY_REVS)
        });
    entries
        .into_iter()
        .map(|e| BlobHistoryVm {
            rev_ref: e.commit_hex,
            when: humanize_age(e.author_time_ms),
            // SCRUB free text at the read boundary — defence-in-depth.
            author: scrub(&e.author),
            summary: scrub(&e.summary),
        })
        .collect()
}

/// Compute the symbol outline for `path`'s content via `hugit-symbols` (W6).
///
/// The file extension selects the language ([`hugit_symbols::lang_for_ext`], the
/// single source of truth); an unsupported/extensionless file yields an empty
/// outline — the honest default, never a fabricated structure. Each symbol's
/// `name` passes through [`scrub`] at the read boundary: a name derived from
/// source (e.g. a `const`/`static` identifier, or an `impl … for …` display
/// string) could embed a secret-shaped token, and the outline must not become a
/// redaction bypass past the line-level scrub. `kind` is the frozen wire token
/// ([`hugit_symbols::SymbolKind::as_wire_str`]); `active` is a UI-cursor flag the
/// parser never sets.
fn compute_outline(path: &str, bytes: &[u8]) -> Vec<hugit_http_contracts::blob::OutlineItemVm> {
    let Some(ext) = path.rsplit('.').next().filter(|e| *e != path) else {
        return vec![];
    };
    let Some(lang) = hugit_symbols::lang_for_ext(ext) else {
        return vec![];
    };
    hugit_symbols::outline_blob(lang, bytes)
        .into_iter()
        .map(|item| hugit_http_contracts::blob::OutlineItemVm {
            kind: item.kind.as_wire_str().to_string(),
            name: scrub(&item.name),
            line: item.line,
            active: false,
        })
        .collect()
}

/// The blob toolbar actions, in the sibling pt-BR locale/voice.
fn blob_actions() -> Vec<String> {
    vec![
        "Raw".to_string(),
        "Copiar".to_string(),
        "✎ Editar".to_string(),
        "Blame".to_string(),
    ]
}

/// Decode raw blob bytes into scrubbed, 1-based-numbered source lines.
///
/// UTF-8 is decoded lossily (a binary/non-UTF-8 blob still renders as mojibake
/// rather than failing the read). Every line's text passes through [`scrub`] so a
/// secret embedded in the file is redacted to `hugit_ledger::redact::REDACTED`
/// before it ever reaches the browser. `hl`/`intent_id` are honest defaults — no
/// syntax highlight this wave (raw mono is intended) and no per-line attribution.
fn decode_lines(bytes: &[u8]) -> Vec<hugit_http_contracts::blob::BlobLineVm> {
    let text = String::from_utf8_lossy(bytes);
    // `split('\n')` keeps a trailing empty line for a file that ends in '\n'
    // (matching an editor's view). An empty file yields a single empty line.
    text.split('\n')
        .enumerate()
        .map(|(i, line)| hugit_http_contracts::blob::BlobLineVm {
            number: (i as u32) + 1,
            // Strip a trailing CR so a CRLF file does not render a stray ^M.
            text: scrub(line.strip_suffix('\r').unwrap_or(line)),
            hl: false,
            intent_id: None,
        })
        .collect()
}

/// Human-readable byte size: `"123 B"`, `"4.2 KB"`, `"1.5 MB"`. Decimal (1000)
/// units to match how file sizes are conventionally shown in the UI.
fn humanize_bytes(n: usize) -> String {
    let n = n as f64;
    if n < 1000.0 {
        format!("{} B", n as u64)
    } else if n < 1_000_000.0 {
        format!("{:.1} KB", n / 1000.0)
    } else if n < 1_000_000_000.0 {
        format!("{:.1} MB", n / 1_000_000.0)
    } else {
        format!("{:.1} GB", n / 1_000_000_000.0)
    }
}

/// Map a file extension to a coarse language name for the `lang` field. `None`
/// for an unknown/extensionless file — there is NO syntax highlight this wave, so
/// `lang` is purely a label; an honest `None` is better than a wrong guess.
fn lang_for_path(path: &str) -> Option<String> {
    let ext = path.rsplit('.').next().filter(|e| *e != path)?;
    let name = match ext.to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "ts" | "tsx" => "typescript",
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "py" => "python",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" => "cpp",
        "java" => "java",
        "rb" => "ruby",
        "sh" | "bash" => "shell",
        "toml" => "toml",
        "yaml" | "yml" => "yaml",
        "json" => "json",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" => "css",
        "sql" => "sql",
        _ => return None,
    };
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::{CasObjectSource, ObjectKind};

    // ── Tree-wire-format helpers (mirror hugit-proto's own resolve_tests) ────
    const MODE_BLOB: &str = "100644";
    const MODE_TREE: &str = "40000";

    struct TreeEntry<'a> {
        mode: &'a str,
        name: &'a str,
        oid: ObjectId,
    }

    /// Build the raw bytes of a git tree object: a concatenation of
    /// `<ascii-octal-mode> <name>\0<20-byte-binary-oid>`, name-sorted (git
    /// canonical) so the bytes round-trip through `gix_object::TreeRefIter`.
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

    /// An empty event log (the handler does not read it this wave).
    fn log() -> EventLog {
        EventLog::new()
    }

    #[test]
    fn resolves_real_file_content_size_and_lang() {
        let mut src = CasObjectSource::new();
        let content = "pub fn main() {}\nlet x = 1;\n";
        let blob = src.insert_raw(ObjectKind::Blob, content.as_bytes().to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "main.rs",
                oid: blob,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_blob(&log(), "hugit", "main.rs", Some(&src), Some(&root), None)
            .expect("a present blob resolves to Some");

        assert_eq!(vm.path, "main.rs");
        assert_eq!(vm.lang.as_deref(), Some("rust"));
        // "pub fn main() {}" (16) + "\n" + "let x = 1;" (10) + "\n" = 28 bytes.
        assert_eq!(vm.size, "28 B");
        // The line text equals the file content (trailing '\n' → a final empty line).
        assert_eq!(vm.lines[0].text, "pub fn main() {}");
        assert_eq!(vm.lines[0].number, 1);
        assert_eq!(vm.lines[1].text, "let x = 1;");
        assert_eq!(vm.lines[2].text, ""); // trailing newline
        // REAL (W6): the outline carries the one top-level fn from the content.
        assert_eq!(vm.outline.len(), 1, "outline: {:?}", vm.outline);
        assert_eq!(vm.outline[0].kind, "fn");
        assert_eq!(vm.outline[0].name, "main");
        assert_eq!(vm.outline[0].line, 1);
        // Honest defaults (still absent this wave).
        assert!(vm.blame.is_empty());
        assert!(vm.via_intent.is_none());
        // REAL: the sidebar tree lists the one file in the root dir.
        assert_eq!(vm.tree.len(), 1, "tree: {:?}", vm.tree);
        assert_eq!(vm.tree[0].name, "main.rs");
        assert!(
            vm.tree[0].current,
            "the requested file must be marked current"
        );
        assert!(!vm.tree[0].is_dir);
    }

    #[test]
    fn outline_panic_guard_is_transparent_on_the_happy_path() {
        // The catch_unwind guard added to fail-close an outline panic (so a
        // tree-sitter panic returns an empty outline instead of 503-ing the
        // whole file read) must be TRANSPARENT on a normal blob: a healthy Rust
        // file still yields its NON-empty outline through build_blob.
        let mut src = CasObjectSource::new();
        let content = "pub fn alpha() {}\npub struct Beta;\npub fn gamma() -> u8 { 0 }\n";
        let blob = src.insert_raw(ObjectKind::Blob, content.as_bytes().to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "k.rs",
                oid: blob,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm =
            build_blob(&log(), "r", "k.rs", Some(&src), Some(&root), None).expect("rs resolves");
        // The guard did NOT swallow the real outline — the file's symbols survive.
        assert!(
            !vm.outline.is_empty(),
            "the panic guard must be transparent on a healthy blob: {:?}",
            vm.outline
        );
        let names: Vec<&str> = vm.outline.iter().map(|o| o.name.as_str()).collect();
        assert!(names.contains(&"alpha"), "outline: {names:?}");
        assert!(names.contains(&"Beta"), "outline: {names:?}");
        assert!(names.contains(&"gamma"), "outline: {names:?}");
    }

    #[test]
    fn outline_captures_multiple_kinds_and_unsupported_lang_is_empty() {
        let mut src = CasObjectSource::new();
        let content = "pub struct Point;\npub fn area() -> u32 { 0 }\n";
        let blob = src.insert_raw(ObjectKind::Blob, content.as_bytes().to_vec());
        let txt = src.insert_raw(ObjectKind::Blob, b"plain text, no lang".to_vec());
        let root = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "m.rs",
                    oid: blob,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "notes.txt",
                    oid: txt,
                },
            ],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm =
            build_blob(&log(), "r", "m.rs", Some(&src), Some(&root), None).expect("rs resolves");
        let kinds: Vec<(&str, &str)> = vm
            .outline
            .iter()
            .map(|o| (o.kind.as_str(), o.name.as_str()))
            .collect();
        assert!(kinds.contains(&("struct", "Point")), "outline: {kinds:?}");
        assert!(kinds.contains(&("fn", "area")), "outline: {kinds:?}");

        // An unsupported extension (.txt) yields an empty outline — honest, not faked.
        let vm_txt = build_blob(&log(), "r", "notes.txt", Some(&src), Some(&root), None)
            .expect("txt resolves");
        assert!(
            vm_txt.outline.is_empty(),
            "unsupported lang → empty outline"
        );
    }

    #[test]
    fn outline_symbol_name_is_scrubbed_at_the_read_boundary() {
        // A secret-shaped identifier name must be redacted in the outline, not
        // just in the line text — the outline must not be a redaction bypass.
        let mut src = CasObjectSource::new();
        let content = "const gho_16C7e42F292c6912E7710c838347Ae178B4a: u32 = 1;\n";
        let blob = src.insert_raw(ObjectKind::Blob, content.as_bytes().to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "s.rs",
                oid: blob,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_blob(&log(), "r", "s.rs", Some(&src), Some(&root), None).expect("resolves");
        let names: String = vm.outline.iter().map(|o| o.name.clone()).collect();
        assert!(
            !names.contains("gho_16C7e42F292c6912E7710c838347Ae178B4a"),
            "a secret-shaped symbol name must be scrubbed in the outline: {names}"
        );
    }

    #[test]
    fn resolves_nested_file() {
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"deep".to_vec());
        let sub = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "c.txt",
                oid: blob,
            }],
        );
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_TREE,
                name: "a",
                oid: sub,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm =
            build_blob(&log(), "r", "a/c.txt", Some(&src), Some(&root), None).expect("nested file");
        assert_eq!(vm.lines[0].text, "deep");
        assert_eq!(vm.lang, None); // .txt has no mapping
    }

    #[test]
    fn tree_rows_carry_repo_relative_path() {
        // REGRESSION (prod blob-503): the sidebar tree entries MUST carry a
        // repo-relative `path` — the window deserializes it as a REQUIRED field
        // and builds `/r/{repo}/blob/{path}` links from it. A missing `path` is
        // a hard decode error window-side (was the live 503 after the outline
        // panic was guarded). Cover root-level, nested, and a directory's
        // trailing slash.
        let mut src = CasObjectSource::new();
        let cblob = src.insert_raw(ObjectKind::Blob, b"c".to_vec());
        let dblob = src.insert_raw(ObjectKind::Blob, b"d".to_vec());
        let deep = src.insert_raw(ObjectKind::Blob, b"deep".to_vec());
        let nested = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "deep.txt",
                oid: deep,
            }],
        );
        // dir "a" holds: c.txt, d.txt, and a subdir "nested/".
        let a = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "c.txt",
                    oid: cblob,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "d.txt",
                    oid: dblob,
                },
                TreeEntry {
                    mode: MODE_TREE,
                    name: "nested",
                    oid: nested,
                },
            ],
        );
        let topblob = src.insert_raw(ObjectKind::Blob, b"top".to_vec());
        let root = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_TREE,
                    name: "a",
                    oid: a,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "top.txt",
                    oid: topblob,
                },
            ],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        // Viewing a NESTED file: the sidebar lists "a/"'s children, each with a
        // path PREFIXED by the parent dir; the open file is `current`.
        let vm =
            build_blob(&log(), "r", "a/c.txt", Some(&src), Some(&root), None).expect("nested file");
        let by_name = |n: &str| vm.tree.iter().find(|r| r.name == n).expect("row present");
        assert_eq!(by_name("c.txt").path, "a/c.txt");
        assert!(by_name("c.txt").current, "open file marked current");
        assert_eq!(by_name("d.txt").path, "a/d.txt");
        // A directory entry carries a TRAILING SLASH per the wire contract.
        assert_eq!(by_name("nested").path, "a/nested/");
        assert!(by_name("nested").is_dir);

        // Viewing a ROOT-level file: entry path has no parent prefix.
        let vm_root =
            build_blob(&log(), "r", "top.txt", Some(&src), Some(&root), None).expect("root file");
        let top = vm_root
            .tree
            .iter()
            .find(|r| r.name == "top.txt")
            .expect("top row");
        assert_eq!(top.path, "top.txt");
        assert_eq!(
            vm_root
                .tree
                .iter()
                .find(|r| r.name == "a")
                .expect("dir row")
                .path,
            "a/"
        );
    }

    #[test]
    fn missing_path_is_none() {
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"x".to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "present.txt",
                oid: blob,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);
        assert!(build_blob(&log(), "r", "absent.txt", Some(&src), Some(&root), None).is_none());
    }

    #[test]
    fn absent_git_source_is_none_not_fake_content() {
        // The honest "content seam not live" 404 — NOT a fake blank file.
        assert!(build_blob(&log(), "r", "any.rs", None, None, None).is_none());
        // root_tree present but src absent → still None (lock-step).
        let root = ObjectId::from_hex(b"0000000000000000000000000000000000000001").unwrap();
        assert!(build_blob(&log(), "r", "any.rs", None, Some(&root), None).is_none());
    }

    #[test]
    fn secret_in_content_is_redacted() {
        // A secret embedded in the file content MUST be scrubbed at the read
        // boundary — the raw secret absent, the REDACTED sentinel present.
        let secret = "gho_16C7e42F292c6912E7710c838347Ae178B4a";
        let mut src = CasObjectSource::new();
        let content = format!("const KEY = \"{secret}\";\n");
        let blob = src.insert_raw(ObjectKind::Blob, content.into_bytes());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "cfg.rs",
                oid: blob,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm =
            build_blob(&log(), "r", "cfg.rs", Some(&src), Some(&root), None).expect("resolves");
        let joined: String = vm.lines.iter().map(|l| l.text.clone()).collect();
        assert!(
            !joined.contains(secret),
            "the raw secret must not survive to the view-model: {joined}"
        );
        assert!(
            joined.contains(hugit_ledger::redact::REDACTED),
            "the REDACTED sentinel must be present: {joined}"
        );
    }

    #[test]
    fn humanize_bytes_units() {
        assert_eq!(humanize_bytes(0), "0 B");
        assert_eq!(humanize_bytes(999), "999 B");
        assert_eq!(humanize_bytes(1500), "1.5 KB");
        assert_eq!(humanize_bytes(2_500_000), "2.5 MB");
    }

    #[test]
    fn blob_over_size_cap_returns_none_not_oom() {
        // A blob exceeding MAX_BLOB_BYTES must return None (→ 404) rather than
        // buffering the whole thing into RAM and OOMing the server.
        let mut src = CasObjectSource::new();
        // Allocate one byte over the cap.
        let big = vec![b'A'; MAX_BLOB_BYTES + 1];
        let blob = src.insert_raw(ObjectKind::Blob, big);
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "huge.bin",
                oid: blob,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        // Must be None — not OOM, not a partial result.
        assert!(
            build_blob(&log(), "r", "huge.bin", Some(&src), Some(&root), None).is_none(),
            "a blob over the cap must not be served"
        );

        // A blob AT (not over) the cap is still served.
        let mut src2 = CasObjectSource::new();
        let exact = vec![b'B'; MAX_BLOB_BYTES];
        let blob2 = src2.insert_raw(ObjectKind::Blob, exact);
        let root2 = insert_tree(
            &mut src2,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "exact.bin",
                oid: blob2,
            }],
        );
        let src2: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src2);
        assert!(
            build_blob(&log(), "r", "exact.bin", Some(&src2), Some(&root2), None).is_some(),
            "a blob exactly at the cap must still be served"
        );
    }

    // ── blob.tree (sidebar file-tree) tests ──────────────────────────────────

    /// A root-level file: `tree` lists all siblings in the root directory.
    /// The requested file must have `current: true`; all others `current: false`.
    #[test]
    fn tree_root_level_lists_siblings() {
        let mut src = CasObjectSource::new();
        let a = src.insert_raw(ObjectKind::Blob, b"a".to_vec());
        let b = src.insert_raw(ObjectKind::Blob, b"b".to_vec());
        let sub = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "inner.rs",
                oid: a,
            }],
        );
        let root = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "lib.rs",
                    oid: a,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "main.rs",
                    oid: b,
                },
                TreeEntry {
                    mode: MODE_TREE,
                    name: "src",
                    oid: sub,
                },
            ],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_blob(&log(), "r", "lib.rs", Some(&src), Some(&root), None)
            .expect("root-level file resolves");

        // Three entries: lib.rs, main.rs, src (dir) — all in the root tree.
        assert_eq!(vm.tree.len(), 3, "tree entries: {:?}", vm.tree);

        // lib.rs is the requested file → current: true.
        let lib = vm
            .tree
            .iter()
            .find(|e| e.name == "lib.rs")
            .expect("lib.rs in tree");
        assert!(lib.current, "requested file must be marked current");
        assert!(!lib.is_dir);

        // main.rs is a sibling → current: false.
        let main = vm
            .tree
            .iter()
            .find(|e| e.name == "main.rs")
            .expect("main.rs in tree");
        assert!(!main.current);
        assert!(!main.is_dir);

        // src is a subdirectory.
        let src_row = vm
            .tree
            .iter()
            .find(|e| e.name == "src")
            .expect("src in tree");
        assert!(!src_row.current);
        assert!(src_row.is_dir);

        // depth is always 0 (one-level sidebar).
        assert!(vm.tree.iter().all(|e| e.depth == 0));
    }

    /// A nested file: `tree` lists siblings in the *same* directory, not the root.
    #[test]
    fn tree_nested_file_lists_parent_dir_siblings() {
        let mut src = CasObjectSource::new();
        let a = src.insert_raw(ObjectKind::Blob, b"a".to_vec());
        let b_blob = src.insert_raw(ObjectKind::Blob, b"b".to_vec());
        let inner_sub = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "handlers.rs",
                oid: a,
            }],
        );
        let sub = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "lib.rs",
                    oid: a,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "util.rs",
                    oid: b_blob,
                },
                TreeEntry {
                    mode: MODE_TREE,
                    name: "handlers",
                    oid: inner_sub,
                },
            ],
        );
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_TREE,
                name: "src",
                oid: sub,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        // Requesting "src/util.rs" → tree = entries of the "src/" directory.
        let vm = build_blob(&log(), "r", "src/util.rs", Some(&src), Some(&root), None)
            .expect("nested file resolves");

        assert_eq!(vm.tree.len(), 3, "tree entries: {:?}", vm.tree);

        let util = vm
            .tree
            .iter()
            .find(|e| e.name == "util.rs")
            .expect("util.rs in tree");
        assert!(util.current, "requested file must be marked current");
        assert!(!util.is_dir);

        let lib_row = vm
            .tree
            .iter()
            .find(|e| e.name == "lib.rs")
            .expect("lib.rs in tree");
        assert!(!lib_row.current);

        let handlers_row = vm
            .tree
            .iter()
            .find(|e| e.name == "handlers")
            .expect("handlers in tree");
        assert!(handlers_row.is_dir);
        assert!(!handlers_row.current);
    }

    /// A tree entry whose filename is secret-shaped must be scrubbed in `tree`.
    #[test]
    fn tree_entry_name_with_secret_is_scrubbed() {
        use hugit_ledger::redact::REDACTED;

        let mut src = CasObjectSource::new();
        let secret_named = src.insert_raw(ObjectKind::Blob, b"content".to_vec());
        let normal = src.insert_raw(ObjectKind::Blob, b"normal".to_vec());
        // A file literally named with a secret-shaped prefix (e.g. `ghp_….key`).
        let secret_filename = "ghp_16C7e42F292c6912E7710c838347Ae178B4a.key";
        let root = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "real.rs",
                    oid: normal,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: secret_filename,
                    oid: secret_named,
                },
            ],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_blob(&log(), "r", "real.rs", Some(&src), Some(&root), None)
            .expect("real.rs resolves");

        // The secret-shaped filename must not appear verbatim in the sidebar.
        let names: Vec<&str> = vm.tree.iter().map(|e| e.name.as_str()).collect();
        assert!(
            !names.iter().any(|n| n.contains("ghp_")),
            "secret-shaped tree entry name must be scrubbed: {names:?}"
        );
        // The REDACTED sentinel must be present for the scrubbed entry.
        assert!(
            names.contains(&REDACTED),
            "REDACTED sentinel must appear for the secret-named entry: {names:?}"
        );
        // real.rs is still present and marked current.
        let real = vm.tree.iter().find(|e| e.name == "real.rs");
        assert!(real.is_some(), "real.rs must still appear in the sidebar");
        assert!(real.unwrap().current);
    }

    /// Symlinks in the parent directory must NOT appear in `tree`.
    #[test]
    fn tree_excludes_symlinks() {
        let mut src = CasObjectSource::new();
        let real = src.insert_raw(ObjectKind::Blob, b"real".to_vec());
        let link_target = src.insert_raw(ObjectKind::Blob, b"other/real.rs".to_vec());
        let root = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "real.rs",
                    oid: real,
                },
                TreeEntry {
                    mode: "120000", // symlink
                    name: "link.rs",
                    oid: link_target,
                },
            ],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_blob(&log(), "r", "real.rs", Some(&src), Some(&root), None)
            .expect("real file resolves");

        // Only real.rs should appear; link.rs is excluded.
        assert_eq!(
            vm.tree.len(),
            1,
            "tree should exclude symlinks: {:?}",
            vm.tree
        );
        assert_eq!(vm.tree[0].name, "real.rs");
    }

    // ── blob.history ("Histórico" drawer) tests ──────────────────────────────

    /// Build a commit object referencing `tree`, with one optional `parent`.
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

    /// REAL history flows END-TO-END through build_blob: a HEAD commit threaded in
    /// populates `history` with the touching revisions (newest-first); a `None` HEAD
    /// keeps it empty (the honest "seam not live" → the drawer stays disabled).
    #[test]
    fn history_flows_through_build_blob_and_is_empty_without_head() {
        let mut src = CasObjectSource::new();
        // C1 (root): introduces foo.rs.
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
        // C2 (head): edits foo.rs.
        let foo_v2 = src.insert_raw(ObjectKind::Blob, b"v2".to_vec());
        let t2 = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "foo.rs",
                oid: foo_v2,
            }],
        );
        let c2 = insert_commit(
            &mut src,
            t2,
            Some(c1),
            "Bob <b@x>",
            2000,
            "edit foo\n\nbody",
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        // With the HEAD commit threaded: real revisions, newest-first.
        let vm = build_blob(&log(), "r", "foo.rs", Some(&src), Some(&t2), Some(&c2))
            .expect("foo.rs resolves");
        assert_eq!(vm.history.len(), 2, "history: {:?}", vm.history);
        assert_eq!(vm.history[0].rev_ref, c2.to_hex().to_string());
        assert_eq!(vm.history[0].summary, "edit foo"); // FIRST LINE only
        assert_eq!(vm.history[0].author, "Bob <b@x>");
        assert!(!vm.history[0].when.is_empty(), "when is humanized");
        assert_eq!(vm.history[1].rev_ref, c1.to_hex().to_string());

        // Without a HEAD commit: empty history (drawer stays disabled-honest).
        let vm_none = build_blob(&log(), "r", "foo.rs", Some(&src), Some(&t2), None)
            .expect("foo.rs resolves");
        assert!(
            vm_none.history.is_empty(),
            "no HEAD → empty history (honest seam-not-live)"
        );
    }

    /// SECRET-MATRIX: a commit author identity AND a commit summary containing a
    /// secret-shaped string MUST be `[REDACTED]` in the history VM — the free-text
    /// commit fields are scrubbed at the read boundary, no redaction bypass.
    #[test]
    fn history_author_and_summary_are_scrubbed_at_the_read_boundary() {
        let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
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
        // Both the author identity and the message carry a secret-shaped token.
        let author = format!("leak {secret} <l@x>");
        let message = format!("token {secret} added");
        let c1 = insert_commit(&mut src, t1, None, &author, 1000, &message);
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_blob(&log(), "r", "foo.rs", Some(&src), Some(&t1), Some(&c1))
            .expect("foo.rs resolves");
        assert_eq!(vm.history.len(), 1, "history: {:?}", vm.history);
        let row = &vm.history[0];
        assert!(
            !row.author.contains(secret),
            "a secret in the author must be scrubbed: {}",
            row.author
        );
        assert!(
            !row.summary.contains(secret),
            "a secret in the summary must be scrubbed: {}",
            row.summary
        );
        assert!(
            row.author.contains(hugit_ledger::redact::REDACTED)
                && row.summary.contains(hugit_ledger::redact::REDACTED),
            "the REDACTED sentinel must be present in both fields"
        );
    }

    /// The history walk is WALL-CLOCK bounded inside build_blob: a deadline-in-the-
    /// past path returns the partial newest list, never exhausting the walk (covered
    /// at the proto layer; here we assert build_blob never errors / 503s on a deep
    /// history — it returns Some with a bounded list ≤ MAX_HISTORY_REVS).
    #[test]
    fn history_is_count_bounded_in_build_blob() {
        let mut src = CasObjectSource::new();
        let mut parent: Option<ObjectId> = None;
        let mut head_tree = None;
        let mut head = None;
        // Build more touching commits than the cap.
        for i in 0..(MAX_HISTORY_REVS as u32 + 10) {
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
            head_tree = Some(tree);
        }
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);
        let vm = build_blob(
            &log(),
            "r",
            "foo.rs",
            Some(&src),
            head_tree.as_ref(),
            head.as_ref(),
        )
        .expect("foo.rs resolves");
        assert_eq!(
            vm.history.len(),
            MAX_HISTORY_REVS,
            "history must be count-capped at MAX_HISTORY_REVS"
        );
    }

    // ── blob.history precomputed-index path (#70(a)) ─────────────────────────

    use hugit_proto::BlobHistoryEntry;
    use std::collections::HashMap;

    /// #70(a) CORE: a DEEP-history file whose revisions the LIVE walk cannot produce
    /// (here the HEAD commit object is unreachable from the source → the walk yields
    /// EMPTY, the exact bug shape) is served COMPLETE from the precomputed index.
    #[test]
    fn deep_history_served_from_index_where_live_walk_is_empty() {
        let mut src = CasObjectSource::new();
        let foo = src.insert_raw(ObjectKind::Blob, b"v1".to_vec());
        let t = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "foo.rs",
                oid: foo,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);
        // A HEAD commit oid whose object is NOT in the source → the live history walk
        // finds NOTHING (load_commit fails) → empty, standing in for a deep-history file
        // whose touch the 2 s per-request walk can't reach.
        let head = ObjectId::from_hex(b"1234123412341234123412341234123412341234").unwrap();

        // Baseline (no index): the live walk cannot produce the history → EMPTY (the bug).
        let vm_live = build_blob_with_history(
            &log(),
            "r",
            "foo.rs",
            Some(&src),
            Some(&t),
            Some(&head),
            None,
        )
        .expect("foo.rs resolves");
        assert!(
            vm_live.history.is_empty(),
            "the live walk cannot produce the deep history → empty (the #70(a) bug)"
        );

        // The off-loop index build recorded foo.rs's revisions → served directly.
        let store = BlobHistoryStore::new();
        let mut by_path = HashMap::new();
        by_path.insert(
            "foo.rs".to_string(),
            vec![BlobHistoryEntry {
                commit_hex: "a".repeat(40),
                author_time_ms: 3_000,
                author: "Deep <d@x>".into(),
                summary: "introduce foo".into(),
            }],
        );
        store.install("r", head, by_path);

        let vm_idx = build_blob_with_history(
            &log(),
            "r",
            "foo.rs",
            Some(&src),
            Some(&t),
            Some(&head),
            Some(&store),
        )
        .expect("foo.rs resolves");
        assert_eq!(
            vm_idx.history.len(),
            1,
            "the index serves the deep-history revision the live walk missed"
        );
        assert_eq!(vm_idx.history[0].summary, "introduce foo");
        assert_eq!(vm_idx.history[0].rev_ref, "a".repeat(40));
        assert_eq!(vm_idx.history[0].author, "Deep <d@x>");
    }

    /// FALLBACK: a path ABSENT from the index (the index was built for other paths, or
    /// this path was added after it) falls back to the live wall-clock-bounded walk —
    /// never a regress. Here the live walk over a real 2-commit chain still serves.
    #[test]
    fn history_falls_back_to_live_walk_on_index_miss() {
        let mut src = CasObjectSource::new();
        // C1 (root) adds foo.rs; C2 (head) edits it → the live walk finds 2 touches.
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
        let foo_v2 = src.insert_raw(ObjectKind::Blob, b"v2".to_vec());
        let t2 = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "foo.rs",
                oid: foo_v2,
            }],
        );
        let c2 = insert_commit(&mut src, t2, Some(c1), "Bob <b@x>", 2000, "edit foo");
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        // An index built for the SAME head but a DIFFERENT path → foo.rs is a MISS.
        let store = BlobHistoryStore::new();
        let mut by_path = HashMap::new();
        by_path.insert(
            "other.rs".to_string(),
            vec![BlobHistoryEntry {
                commit_hex: "b".repeat(40),
                author_time_ms: 9_000,
                author: "X <x@x>".into(),
                summary: "unrelated".into(),
            }],
        );
        store.install("r", c2, by_path);

        let vm = build_blob_with_history(
            &log(),
            "r",
            "foo.rs",
            Some(&src),
            Some(&t2),
            Some(&c2),
            Some(&store),
        )
        .expect("foo.rs resolves");
        // foo.rs absent from the index → fell back to the live walk → the real revisions.
        assert_eq!(
            vm.history.len(),
            2,
            "an un-indexed path falls back to the live walk: {:?}",
            vm.history
        );
        assert_eq!(vm.history[0].rev_ref, c2.to_hex().to_string());
        assert_eq!(vm.history[1].rev_ref, c1.to_hex().to_string());
    }

    /// FALLBACK: a STALE index (built for a different HEAD — a push moved the tip) is
    /// refused; the read falls back to the live walk of the CURRENT tip.
    #[test]
    fn stale_head_index_falls_back_to_live_walk() {
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
        let c1 = insert_commit(&mut src, t1, None, "Alice <a@x>", 1000, "add foo");
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        // The index was built for an OLD head; the current HEAD is c1.
        let old_head = ObjectId::from_hex(b"9999999999999999999999999999999999999999").unwrap();
        let store = BlobHistoryStore::new();
        let mut by_path = HashMap::new();
        by_path.insert(
            "foo.rs".to_string(),
            vec![BlobHistoryEntry {
                commit_hex: "c".repeat(40),
                author_time_ms: 1,
                author: "Stale <s@x>".into(),
                summary: "stale".into(),
            }],
        );
        store.install("r", old_head, by_path);

        let vm = build_blob_with_history(
            &log(),
            "r",
            "foo.rs",
            Some(&src),
            Some(&t1),
            Some(&c1),
            Some(&store),
        )
        .expect("foo.rs resolves");
        // The stale index is refused (HEAD mismatch) → live walk of c1 → the real intro.
        assert_eq!(vm.history.len(), 1, "stale index refused → live walk");
        assert_eq!(vm.history[0].rev_ref, c1.to_hex().to_string());
        assert_ne!(
            vm.history[0].summary, "stale",
            "the stale entry was NOT served"
        );
    }

    /// REDACTION (no index bypass): an index entry whose author/summary carries a
    /// secret-shaped token is [`scrub`]bed at the read boundary exactly like the live
    /// walk — the index stores UNSCRUBBED proto entries; scrub is always applied HERE.
    #[test]
    fn index_served_history_is_scrubbed_at_the_read_boundary() {
        let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let mut src = CasObjectSource::new();
        let foo = src.insert_raw(ObjectKind::Blob, b"v1".to_vec());
        let t = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "foo.rs",
                oid: foo,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);
        let head = ObjectId::from_hex(b"2222222222222222222222222222222222222222").unwrap();

        let store = BlobHistoryStore::new();
        let mut by_path = HashMap::new();
        by_path.insert(
            "foo.rs".to_string(),
            vec![BlobHistoryEntry {
                commit_hex: "d".repeat(40),
                author_time_ms: 1,
                author: format!("leak {secret} <l@x>"),
                summary: format!("token {secret} added"),
            }],
        );
        store.install("r", head, by_path);

        let vm = build_blob_with_history(
            &log(),
            "r",
            "foo.rs",
            Some(&src),
            Some(&t),
            Some(&head),
            Some(&store),
        )
        .expect("foo.rs resolves");
        let row = &vm.history[0];
        assert!(
            !row.author.contains(secret) && !row.summary.contains(secret),
            "a secret in an INDEX entry must be scrubbed at the read boundary"
        );
        assert!(
            row.author.contains(hugit_ledger::redact::REDACTED)
                && row.summary.contains(hugit_ledger::redact::REDACTED),
            "the REDACTED sentinel must be present in both index-served fields"
        );
    }

    // ── DoS bound: the path→blob CAS resolve is wall-clock bounded ────────────
    /// A deadline already in the PAST stops the CAS path→blob resolve before the
    /// first fetch → an honest `None` (404) EVEN THOUGH the file is present. Proves
    /// the resolve is `BudgetedSource`-wrapped (wall-clock bounded), not merely
    /// count-capped, and the truncation is honest-empty (never a fabricated file) —
    /// the exact mirror of `handlers::edit::past_deadline_bounds_resolve_to_honest_none`.
    /// The live-budget happy path is covered by every other test (default budget).
    #[test]
    fn past_deadline_bounds_blob_resolve_to_honest_none() {
        use std::time::{Duration, Instant};

        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"present\n".to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "here.txt",
                oid: blob,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        // Sanity: with a live budget the present file resolves.
        let live = Instant::now() + Duration::from_secs(60);
        assert!(
            build_blob_until(
                &log(),
                "r",
                "here.txt",
                Some(&src),
                Some(&root),
                None,
                None,
                live
            )
            .is_some(),
            "a present file resolves under a live budget"
        );

        // Past deadline: the resolve is refused at the first `get` → honest 404.
        let past = Instant::now() - Duration::from_secs(1);
        assert!(
            build_blob_until(
                &log(),
                "r",
                "here.txt",
                Some(&src),
                Some(&root),
                None,
                None,
                past
            )
            .is_none(),
            "a present file is honest-404'd once the walk budget is spent"
        );
    }
}
