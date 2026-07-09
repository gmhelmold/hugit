//! `GET /v1/repos/{repo}/home` → [`RepoHomeVm`]. REAL: branches/last_commit/
//! commit_count via refstore `replay`/`project_machine`; the root file TREE +
//! rendered README from the git-from-CAS content seam (wall-clock bounded +
//! scrubbed at the read boundary, mirroring `blob`/`edit`). STUB: about-mirror
//! (GitHub P2) + synergy (no live AC seam).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Instant;

use gix_hash::ObjectId;
use hugit_http_contracts::home::TreeRowVm;
use hugit_http_contracts::{AboutVm, LastCommitVm, RepoHomeVm, SynergyVm};
use hugit_refstore::EventLog;
use hugit_refstore::intent::intents_from_log;
use hugit_refstore::intent::projection::{ProjectionRow, project_machine};
use hugit_refstore::replay::replay;

use crate::budgeted_source::{BudgetedSource, WALK_BUDGET};
use crate::fmt::{CONTRIBUTORS_CAP, humanize_age, scrub};
use crate::home_cache::{CachedHomeContent, HomeRenderCache, RawTreeEntry};

/// Maximum README size (in bytes) that will be buffered + rendered. A README
/// beyond this is skipped (empty `readme_html`) rather than buffering a huge blob
/// into RAM on the single-threaded engine. 1 MB is generous for any real README.
const MAX_README_BYTES: usize = 1024 * 1024;

/// Candidate root README filenames, tried in order (case/extension fallback).
const README_CANDIDATES: &[&str] = &["README.md", "README", "readme.md", "Readme.md"];

/// Extract a display name from a principal-chain entry.
///
/// Strips well-known prefixes (`agent:`, `orchestrator:`) so the raw wire
/// value yields a human-readable contributor name.
fn principal_display_name(entry: &str) -> &str {
    for prefix in &["agent:", "orchestrator:"] {
        if let Some(rest) = entry.strip_prefix(prefix) {
            return rest;
        }
    }
    entry
}

/// Build the repo-home view-model from a verified event log + the git content seam.
///
/// The `log` is ALREADY chain-verified by the caller — do NOT re-load or
/// re-verify. Fields sourced from real engine data are tagged REAL below;
/// fields with no local source are honest defaults (STUB) per master-plan §0/§5.
///
/// `src`/`root_tree` are the same git content seam threaded to `build_blob`/
/// `build_edit`: `Some` → the root file tree + README are read REAL from the git
/// tree at HEAD (see [`build_home_until`]); `None` → `files: []` / `readme_html: ""`
/// (the honest "content seam not live" default, never a fabricated listing).
///
/// DoS: the root-tree listing + README resolve are synchronous CAS `get`s on the
/// single-threaded lazy-CAS engine; they are WALL-CLOCK bounded by [`WALK_BUDGET`]
/// via a [`BudgetedSource`] (mirrors `build_blob`/`build_edit`) — past the deadline
/// every `get` yields `Ok(None)`, so the listing / README degrade to honest-empty
/// rather than wedging the accept loop. A home read touches only the ROOT tree +
/// one README blob, so it is cheap; the deadline is the belt-and-braces guard.
///
/// PERF: the CAS-expensive part (the tree listing + README bytes) is served from the
/// content-addressed [`HomeRenderCache`] keyed by the root tree oid when `cache` is
/// wired — a HIT is a pure in-memory op (ZERO CAS tree-walk), a MISS runs the bounded
/// walk once + populates. A push produces a new tree oid → a MISS → automatic
/// invalidation (a changed tree can never be a stale HIT). The FAST log-derived parts
/// (branch/counts/last-commit/contributors) are rebuilt FRESH per request below — they
/// are NEVER cached. `cache: None` → the walk runs inline every time (the pre-cache
/// behaviour), used by the honest-empty / no-seam paths + the deterministic tests.
pub fn build_home(
    log: &EventLog,
    repo: &str,
    src: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    root_tree: Option<&ObjectId>,
    cache: Option<&HomeRenderCache>,
) -> RepoHomeVm {
    build_home_until(
        log,
        repo,
        src,
        root_tree,
        cache,
        Instant::now() + WALK_BUDGET,
    )
}

/// [`build_home`] with an explicit wall-clock `deadline` on the CAS reads —
/// deterministically testable (a deadline already in the past stops before the
/// first fetch → honest-empty files/readme). See [`build_home`] for the DoS
/// rationale; mirrors `handlers::edit::build_edit_until`.
fn build_home_until(
    log: &EventLog,
    repo: &str,
    src: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    root_tree: Option<&ObjectId>,
    cache: Option<&HomeRenderCache>,
    deadline: Instant,
) -> RepoHomeVm {
    // ── REAL: replay → branch / branch_count / tag_count / branches ────────
    let ref_state = replay(log).unwrap_or_default();

    // Primary branch: the first `refs/heads/*` in sorted ref order. replay never
    // inserts a symbolic `refs/HEAD` pointer (a `refs/HEAD` lookup here was dead
    // code — dropped), so this sorted-first heads entry IS the real path. Empty
    // when the log has advanced no branch ref.
    // Ref short-name is attacker-controllable (a pushed branch name is free text), so
    // scrub it at the read boundary — a secret-shaped branch must never echo.
    let branch: String = ref_state
        .iter()
        .find_map(|(name, _)| name.strip_prefix("refs/heads/"))
        .map(scrub)
        .unwrap_or_default();

    let mut branch_count: usize = 0;
    let mut tag_count: u32 = 0;
    let mut branches: Vec<String> = Vec::new();

    for (name, _target) in ref_state.iter() {
        if let Some(short) = name.strip_prefix("refs/heads/") {
            branch_count += 1;
            branches.push(scrub(short)); // attacker-controllable ref name — scrub
        } else if name.starts_with("refs/tags/") {
            tag_count += 1;
        }
    }

    // ── REAL: project_machine → commit_count / last_commit ─────────────────
    let machine = project_machine(log).unwrap_or_default();
    let commit_count = machine.rows().len().to_string();

    let last_commit: LastCommitVm = machine
        .rows()
        .last()
        .map(|row| match row {
            ProjectionRow::Intent(gc) => {
                // The originating `intent.landed` record. By construction the log
                // is gap-free and 0-based, so `seq == index`: ONE indexed lookup
                // (was two O(n) `.iter().find(|r| r.seq == gc.seq)` scans).
                let record = log.records().get(gc.seq as usize);

                // Author: first entry of the originating record's principal_chain,
                // prefix-stripped to a display name. SCRUBBED — a principal entry
                // is free text echoed from the log; a secret-shaped value never
                // reaches the browser (P0 read-path redaction).
                let author = record
                    .and_then(|r| r.principal_chain.first())
                    .map(|p| scrub(principal_display_name(p)))
                    .unwrap_or_default();

                let recorded_at = record.map(|r| r.recorded_at).unwrap_or(0);

                // safe 6-char slice of the target SHA (structural — NOT scrubbed)
                let short_sha = gc.target.get(..6).unwrap_or(gc.target.as_str()).to_string();

                // Strip "Intent-Id: …" trailer (show charter only), then SCRUB —
                // the charter is free text echoed from the log (P0 redaction). The
                // intent_id / short_sha are content-addresses, left structural.
                let message = scrub(
                    gc.message
                        .lines()
                        .take_while(|l| !l.starts_with("Intent-Id:"))
                        .collect::<Vec<_>>()
                        .join("\n")
                        .trim(),
                );

                LastCommitVm {
                    author,
                    intent_id: gc.intent_id.clone(),
                    message,
                    short_sha,
                    age: humanize_age(recorded_at),
                }
            }
            ProjectionRow::ExternalChange { seq, target, .. } => {
                let recorded_at = log
                    .records()
                    .iter()
                    .find(|r| r.seq == *seq)
                    .map(|r| r.recorded_at)
                    .unwrap_or(0);
                let short_sha = target
                    .as_deref()
                    .and_then(|t| t.get(..6))
                    .unwrap_or("")
                    .to_string();
                LastCommitVm {
                    author: String::new(),
                    intent_id: String::new(),
                    message: String::new(),
                    short_sha,
                    age: humanize_age(recorded_at),
                }
            }
        })
        .unwrap_or_default();

    // ── REAL: intents_from_log → contributors (dedup principal-chain names) ─
    // Each principal entry is free text echoed from the log → SCRUBBED (P0). A
    // `BTreeSet<String>` dedups in O(n log n) (was `Vec::contains`, O(n²)) and
    // also gives a stable sorted order. Capped at `CONTRIBUTORS_CAP`.
    let intent_log = intents_from_log(log).unwrap_or_default();
    let mut contributor_set: BTreeSet<String> = BTreeSet::new();
    for intent in intent_log.intents() {
        for entry in &intent.principal_chain {
            contributor_set.insert(scrub(principal_display_name(entry)));
        }
    }
    let contributors: Vec<String> = contributor_set.into_iter().take(CONTRIBUTORS_CAP).collect();

    // ── PRESENTATION: about.updated_ago from last record's recorded_at ──────
    // HONEST: a 0 / absent timestamp emits EMPTY, never a fabricated "há NN anos"
    // (a 0 epoch humanizes to "há 56 anos" — a fake last-update the consumer would
    // render; the read VM must never invent a timestamp it doesn't have).
    let updated_ago = log
        .records()
        .last()
        .filter(|r| r.recorded_at > 0)
        .map(|r| humanize_age(r.recorded_at))
        .unwrap_or_default();

    // ── REAL: the root file tree + rendered README. The CAS-expensive walk output is
    //         served from the content-addressed HOME-RENDER CACHE (keyed by the root
    //         tree oid); a HIT is a pure in-memory op (ZERO CAS), a MISS runs the
    //         WALL-CLOCK-bounded walk once + populates. Stored UNSCRUBBED (raw entry
    //         names + raw README bytes); scrubbed at the READ boundary just below.
    // When the git content seam is not wired (`src`/`root_tree` None) it degrades to
    // the honest-empty default (files: [], readme_html: "") — never fabricated.
    let content: CachedHomeContent = match (src, root_tree) {
        (Some(src), Some(root_tree)) => {
            // Cache HIT → serve from memory, NO CAS tree-walk (no `BudgetedSource`, no
            // `get`). A different tree oid (a push) is a MISS → automatic invalidation.
            if let Some(hit) = cache.and_then(|c| c.get(root_tree)) {
                hit
            } else {
                // MISS → the existing bounded CAS walk (raw). Cache the result keyed by
                // the content-addressed tree oid ONLY when the walk was COMPLETE — a
                // budget-truncated walk is served for THIS request (honest-partial, as
                // before) but never cached (else a later live read would HIT the
                // truncated content and serve honest-empty for a present tree).
                let (built, complete) = walk_home_content(src.as_ref(), root_tree, deadline);
                if complete && let Some(cache) = cache {
                    cache.insert(*root_tree, built.clone());
                }
                built
            }
        }
        _ => CachedHomeContent::empty(),
    };

    // ── READ-BOUNDARY REDACTION: scrub the RAW cached content on the way out. A
    //    secret-shaped filename / a secret in the README redacts HERE (never a
    //    redaction bypass — the cache stores raw exactly like search_index).
    let files = scrub_entries(&content.entries);
    let readme_html = scrub_readme(content.readme.as_deref());

    // ── STUB: all remaining fields with no local engine source ────────────────
    // about.description/stars/forks/license: GitHub-mirror P2 → ""
    // about.topics/languages: GitHub-mirror P2 → []
    // about.releases_count: P2 → 0
    // about.release: P2 → None
    // about.contributors_suffix: "" (honest; N already in contributors vec)
    // synergy.lines: no live AC seam → []
    RepoHomeVm {
        repo: repo.to_string(),
        branch,
        branch_count,
        tag_count,
        commit_count,
        last_commit,
        branches,
        files,       // REAL — root-tree listing (dirs-first, scrubbed) or [] no-seam
        readme_html, // REAL — RAW scrubbed root-README markdown (githugr renders) or ""
        about: AboutVm {
            description: String::new(),         // STUB — GitHub-mirror P2
            topics: vec![],                     // STUB — GitHub-mirror P2
            release: None,                      // STUB — P2 release tag
            contributors,                       // REAL — from principal chains
            contributors_suffix: String::new(), // STUB
            stars: String::new(),               // STUB — GitHub-mirror P2
            forks: String::new(),               // STUB — GitHub-mirror P2
            updated_ago,                        // PRESENTATION
            license: String::new(),             // STUB — GitHub-mirror P2
            releases_count: 0,                  // STUB — P2
            languages: vec![],                  // STUB — GitHub-mirror P2
        },
        synergy: SynergyVm {
            lines: vec![], // STUB — no live AC seam in this wave
        },
    }
}

/// Run the bounded CAS walk for the home content — the RAW (unscrubbed) root-tree
/// listing + README bytes — over a [`BudgetedSource`], returning
/// `(content, complete)`. `complete` is `false` when the walk was budget-TRUNCATED
/// (at least one `get` refused past the deadline): the partial content is honest to
/// SERVE for this request, but the caller must NOT cache it (a truncated result is
/// not the tree's full content). Shared by the read-path MISS
/// ([`build_home_until`]) and the boot pre-warm
/// ([`crate::home_cache::spawn_home_cache_prewarm`]) — one walk, one source of truth.
pub(crate) fn walk_home_content(
    src: &dyn hugit_proto::ObjectSource,
    root_tree: &ObjectId,
    deadline: Instant,
) -> (CachedHomeContent, bool) {
    let budgeted = BudgetedSource::new(src, deadline);
    let content = CachedHomeContent {
        entries: list_root_files_raw(&budgeted, root_tree),
        readme: render_root_readme_raw(&budgeted, root_tree),
    };
    // A within-budget walk (`!tripped`) is COMPLETE and safe to cache; a truncated
    // one is served for this request but never cached.
    (content, !budgeted.tripped())
}

/// List the DIRECT entries of the root tree at HEAD as RAW (unscrubbed)
/// [`RawTreeEntry`]s — the display scrub + dirs-first sort are applied at the READ
/// boundary ([`scrub_entries`]) so the cache stores raw content (mirrors
/// `search_index`).
///
/// Delegates to [`hugit_proto::list_tree_at_dir`] (the same primitive the blob
/// sidebar uses). Passing `""` lists the ROOT tree — the parent-of-path for a
/// root-level file IS the root — so this is a ONE-LEVEL listing, not recursive.
/// Symlinks + gitlinks are excluded by the primitive.
///
/// Fail-closed: a missing object / malformed tree / budget cutoff yields an empty
/// `Vec` (the primitive is itself `unwrap_or_default`), never an error.
fn list_root_files_raw(
    src: &dyn hugit_proto::ObjectSource,
    root_tree: &ObjectId,
) -> Vec<RawTreeEntry> {
    hugit_proto::list_tree_at_dir(src, root_tree, "")
        .into_iter()
        .map(|e| RawTreeEntry {
            name: e.name,
            is_dir: e.is_dir,
        })
        .collect()
}

/// Read the root README (trying [`README_CANDIDATES`] in order) as RAW (unscrubbed)
/// markdown bytes → `Some(text)`; `None` when no README resolves or it is oversized.
/// Scrubbing happens at the READ boundary ([`scrub_readme`]).
///
/// The README is resolved via [`hugit_proto::resolve_blob_at_path`] over the SAME
/// budgeted source (path-traversal-guarded, wall-clock-bounded). An oversized README
/// (> [`MAX_README_BYTES`]) is skipped (returns `None`) rather than buffered into RAM.
fn render_root_readme_raw(
    src: &dyn hugit_proto::ObjectSource,
    root_tree: &ObjectId,
) -> Option<String> {
    for candidate in README_CANDIDATES {
        let bytes = match hugit_proto::resolve_blob_at_path(src, root_tree, candidate) {
            Ok(Some((_oid, bytes))) => bytes,
            // A miss / decode error / budget cutoff on this candidate → try the next.
            Ok(None) | Err(_) => continue,
        };
        // OOM guard: skip an oversized README rather than buffering it into RAM.
        if bytes.len() > MAX_README_BYTES {
            return None;
        }
        // RAW (unscrubbed) markdown — lossy UTF-8 (binary → harmless mojibake, never a
        // panic). Scrubbed line-by-line at the read boundary in `scrub_readme`.
        return Some(String::from_utf8_lossy(&bytes).into_owned());
    }
    None
}

/// Compose the repo-home file table from the RAW cached tree entries — SCRUB each
/// displayed name at the read boundary (a git entry literally named `ghp_….key` must
/// not echo verbatim), then apply the GitHub/githugr sort: directories first, then
/// files, each alphabetical by the displayed (already-scrubbed) name so the on-wire
/// order matches what the browser shows.
///
/// `intent_id`/`message`/`age` are honest-empty: attributing a last-touch commit per
/// entry needs a per-file history walk (one CAS walk EACH → a latency-DoS on the
/// single-threaded engine), a separate seam — this wave serves the listing, not
/// per-row attribution, and never fabricates it.
fn scrub_entries(entries: &[RawTreeEntry]) -> Vec<TreeRowVm> {
    let mut rows: Vec<TreeRowVm> = entries
        .iter()
        .map(|e| TreeRowVm {
            // SCRUB the displayed name at the read boundary — a secret-shaped
            // filename must redact before it reaches the browser.
            name: scrub(&e.name),
            is_dir: e.is_dir,
            // HONEST-EMPTY: no per-entry last-touch attribution this wave (would be
            // a per-file history walk → single-thread latency-DoS). Never fabricated.
            intent_id: None,
            message: String::new(),
            age: String::new(),
        })
        .collect();
    // Dirs first, then files; alphabetical within each group (on the displayed,
    // already-scrubbed name so the on-wire order matches what the browser shows).
    rows.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
    rows
}

/// Compose the `readme_html` field from the RAW cached README markdown, SCRUBBING at
/// the read boundary. `String::new()` when no README resolved (`None`).
///
/// RENDERING IS THE WINDOW'S JOB (headless-engine doctrine): the engine returns the
/// RAW (scrubbed, size-bounded) markdown; githugr renders + SANITIZES it through its
/// single audited `render_markdown` (comrak → ammonia strict-allowlist) sink. No
/// markdown renderer AND no HTML sanitizer engine-side — one sink, not two (avoids a
/// second mXSS surface + version skew, and keeps 1/1 mock fidelity: only githugr owns
/// the `.readme` prose markup). The wire field is still named `readme_html` (a
/// non-breaking migration — a coordinated rename to `readme_raw` is a tracked
/// follow-up), but it now carries RAW markdown, which githugr treats as UNTRUSTED.
///
/// REDACTION IS LINE-SCOPED: [`scrub`] ([`hugit_ledger::redact::apply`]) replaces its
/// WHOLE input with the `[REDACTED]` sentinel when ANY detector fires, so scrubbing the
/// entire README at once nuked a perfectly normal README to a bare `[REDACTED]` the
/// moment ONE span looked secret-shaped (a real UX bug — fail-safe, but hides the whole
/// file). We instead scrub LINE-BY-LINE: only the offending line(s) redact and every
/// other line survives verbatim as prose/markdown. The read-boundary guarantee is
/// preserved (a real secret in the README STILL redacts — just its line, not the file).
fn scrub_readme(raw: Option<&str>) -> String {
    match raw {
        // Split on '\n' + re-join with '\n' preserves the exact line structure incl. a
        // trailing newline; each line is scrubbed independently (line-scoped redaction).
        Some(text) => text.split('\n').map(scrub).collect::<Vec<_>>().join("\n"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::{CasObjectSource, ObjectKind};

    const MODE_BLOB: &str = "100644";
    const MODE_TREE: &str = "40000";

    struct TreeEntry<'a> {
        mode: &'a str,
        name: &'a str,
        oid: ObjectId,
    }

    /// Build the raw bytes of a git tree object from its entries (name-sorted, as
    /// git canonicalises them). Mirrors the `edit.rs` test double.
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

    /// HONESTY (epoch-0): a record whose `recorded_at` is 0 (an ingested/synthetic
    /// record with no real timestamp) must yield an EMPTY `updated_ago` — never the
    /// fabricated "há 56 anos" that a 0-epoch humanizes to (the consumer renders any
    /// non-empty string). The read VM never invents a timestamp it does not have.
    #[test]
    fn epoch_zero_timestamp_yields_empty_updated_ago_never_fabricated() {
        let mut log = EventLog::new();
        log.append_for_test(
            "ref.update",
            vec!["test".to_string()],
            r#"{"ref":"refs/heads/main","target":"aabbcc112233"}"#.to_string(),
            0, // recorded_at = 0 → must NOT humanize to "há 56 anos"
        );
        let vm = build_home(&log, "githugr", None, None, None);
        assert!(
            vm.about.updated_ago.is_empty(),
            "a 0/absent timestamp must emit empty updated_ago, got: {:?}",
            vm.about.updated_ago
        );
    }

    /// A real (non-zero) timestamp still humanizes — the guard only drops epoch-0.
    #[test]
    fn real_timestamp_still_humanizes_updated_ago() {
        let mut log = EventLog::new();
        log.append_for_test(
            "ref.update",
            vec!["test".to_string()],
            r#"{"ref":"refs/heads/main","target":"aabbcc112233"}"#.to_string(),
            1_700_000_000, // a real epoch
        );
        let vm = build_home(&log, "githugr", None, None, None);
        assert!(
            !vm.about.updated_ago.is_empty(),
            "a real timestamp must humanize to a non-empty updated_ago"
        );
    }

    /// No git content seam (`src`/`root_tree` None) → the honest-empty default:
    /// an empty file listing AND an empty README (never a fabricated tree).
    #[test]
    fn no_git_seam_yields_empty_files_and_readme() {
        let log = EventLog::new();
        let vm = build_home(&log, "hugit", None, None, None);
        assert!(vm.files.is_empty(), "no seam → empty file listing");
        assert!(vm.readme_html.is_empty(), "no seam → empty readme_html");
    }

    /// REAL: with the git seam wired, the root tree is listed (dirs FIRST, then
    /// files, each alphabetical), the README is rendered, and per-row attribution
    /// is honest-empty (no fabricated last-touch commit).
    #[test]
    fn root_tree_is_listed_dirs_first_and_readme_rendered() {
        let mut src = CasObjectSource::new();
        let readme = src.insert_raw(ObjectKind::Blob, b"# hugit\nhello".to_vec());
        let a_blob = src.insert_raw(ObjectKind::Blob, b"x".to_vec());
        // A subtree object for the "src" directory entry (its content is irrelevant
        // to a one-level root listing — only the mode/name matter).
        let sub = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "lib.rs",
                oid: a_blob,
            }],
        );
        let root = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "README.md",
                    oid: readme,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "Cargo.toml",
                    oid: a_blob,
                },
                TreeEntry {
                    mode: MODE_TREE,
                    name: "src",
                    oid: sub,
                },
            ],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_home(&EventLog::new(), "hugit", Some(&src), Some(&root), None);

        // Dirs first (src), then files alphabetical (Cargo.toml, README.md).
        let names: Vec<&str> = vm.files.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["src", "Cargo.toml", "README.md"]);
        assert!(vm.files[0].is_dir, "the directory sorts first");
        assert!(!vm.files[1].is_dir);
        // Per-row attribution is honest-empty (no fabricated last-touch commit).
        assert!(vm.files[0].intent_id.is_none());
        assert!(vm.files[0].message.is_empty());
        assert!(vm.files[0].age.is_empty());
        // README returned RAW (scrubbed markdown) — githugr renders + sanitizes it,
        // the engine does NOT escape/wrap it.
        assert!(
            !vm.readme_html.contains("<pre>"),
            "raw markdown, not an escaped-<pre> blob: {}",
            vm.readme_html
        );
        assert!(
            vm.readme_html.contains("# hugit"),
            "README body preserved verbatim: {}",
            vm.readme_html
        );
    }

    /// REDACTION: a secret-shaped filename must redact at the read boundary, and a
    /// secret inside the README must redact before it is escaped into `readme_html`.
    #[test]
    fn secret_filename_and_readme_content_redact() {
        let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let secret_name = format!("{secret}.key");
        let mut src = CasObjectSource::new();
        let readme = src.insert_raw(ObjectKind::Blob, format!("token = {secret}\n").into_bytes());
        let leaked = src.insert_raw(ObjectKind::Blob, b"x".to_vec());
        let root = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "README.md",
                    oid: readme,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: &secret_name,
                    oid: leaked,
                },
            ],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_home(&EventLog::new(), "hugit", Some(&src), Some(&root), None);

        let all_names: String = vm.files.iter().map(|r| r.name.clone()).collect();
        assert!(
            !all_names.contains(secret),
            "raw secret filename must not survive: {all_names}"
        );
        assert!(
            !vm.readme_html.contains(secret),
            "raw secret in README must not survive: {}",
            vm.readme_html
        );
        assert!(
            all_names.contains(hugit_ledger::redact::REDACTED)
                || vm.readme_html.contains(hugit_ledger::redact::REDACTED),
            "the REDACTED sentinel must be present after scrub"
        );
    }

    /// DoS bound: a deadline already in the PAST stops the root-tree read + README
    /// resolve before the first CAS fetch → honest-empty files/readme EVEN THOUGH
    /// the tree is present. Proves the home read is wall-clock-bounded, not just
    /// count-capped, and the truncation is honest-empty (never fabricated).
    #[test]
    fn past_deadline_bounds_home_reads_to_honest_empty() {
        use std::time::Duration;

        let mut src = CasObjectSource::new();
        let readme = src.insert_raw(ObjectKind::Blob, b"# r".to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "README.md",
                oid: readme,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        // Sanity: a live budget lists + renders.
        let live = Instant::now() + Duration::from_secs(60);
        let vm_live = build_home_until(&EventLog::new(), "r", Some(&src), Some(&root), None, live);
        assert!(!vm_live.files.is_empty(), "live budget lists the tree");
        assert!(
            !vm_live.readme_html.is_empty(),
            "live budget renders README"
        );

        // Past deadline: every get is refused → honest-empty files + readme.
        let past = Instant::now() - Duration::from_secs(1);
        let vm_past = build_home_until(&EventLog::new(), "r", Some(&src), Some(&root), None, past);
        assert!(
            vm_past.files.is_empty(),
            "past-deadline home read is honest-empty (files)"
        );
        assert!(
            vm_past.readme_html.is_empty(),
            "past-deadline home read is honest-empty (readme)"
        );
    }

    /// A README that itself contains HTML/script is returned RAW (NOT escaped) — the
    /// engine never renders/sanitizes; githugr's audited `render_markdown` (comrak →
    /// ammonia) is the single trusted sink. Proves the engine does no HTML handling.
    #[test]
    fn readme_html_field_carries_raw_markdown_not_escaped() {
        let mut src = CasObjectSource::new();
        let readme_oid = src.insert_raw(
            ObjectKind::Blob,
            b"# Title\n<script>alert(1)</script>\n".to_vec(),
        );
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "README.md",
                oid: readme_oid,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_home(&EventLog::new(), "hugit", Some(&src), Some(&root), None);
        // Raw markdown verbatim — no &lt;/&amp; escaping, no <pre> wrap. githugr sanitizes.
        assert_eq!(vm.readme_html, "# Title\n<script>alert(1)</script>\n");
    }

    /// LINE-SCOPED REDACTION: a README with ONE secret-shaped line + several normal
    /// markdown lines must redact ONLY the offending line — every other line survives
    /// verbatim. Regression for the whole-file `[REDACTED]` UX bug (`scrub` is a
    /// whole-INPUT redactor; scrubbing the entire README nuked normal prose the moment
    /// any span looked secret-shaped). The raw secret must still not appear.
    #[test]
    fn secret_line_redacts_only_its_line_normal_lines_survive() {
        use hugit_ledger::redact::REDACTED;

        let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let mut src = CasObjectSource::new();
        // A realistic README: heading + prose + a config snippet whose ONE line embeds
        // a secret + more prose after it.
        let body = format!(
            "# hugit\n\nThe git-native forge.\n\n## Setup\n\ntoken = {secret}\n\nRun it locally.\n"
        );
        let readme = src.insert_raw(ObjectKind::Blob, body.into_bytes());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "README.md",
                oid: readme,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_home(&EventLog::new(), "hugit", Some(&src), Some(&root), None);

        // The raw secret never survives.
        assert!(
            !vm.readme_html.contains(secret),
            "raw secret must not survive: {}",
            vm.readme_html
        );
        // Only the offending line redacted (exactly one REDACTED sentinel line).
        let lines: Vec<&str> = vm.readme_html.split('\n').collect();
        assert!(
            lines.contains(&REDACTED),
            "the secret line must redact to the sentinel: {}",
            vm.readme_html
        );
        // Every NORMAL line survives verbatim — the whole file was NOT redacted.
        for expected in [
            "# hugit",
            "The git-native forge.",
            "## Setup",
            "Run it locally.",
        ] {
            assert!(
                lines.contains(&expected),
                "normal line {expected:?} must survive verbatim, got: {}",
                vm.readme_html
            );
        }
        assert_ne!(
            vm.readme_html, REDACTED,
            "the WHOLE README must NOT collapse to a bare sentinel"
        );
    }

    // ── Home-render cache (content-addressed) ─────────────────────────────────

    /// A source whose `get` PANICS — proves a cache HIT serves the home content with
    /// ZERO CAS access (the walk would touch the source; a HIT never does). Mirrors
    /// `search_index`'s `PanicSource`.
    struct PanicSource;
    impl hugit_proto::ObjectSource for PanicSource {
        fn get(
            &self,
            _oid: &ObjectId,
        ) -> Result<Option<hugit_proto::GitObject>, hugit_proto::PackError> {
            panic!("a home-cache HIT must NOT touch the source (in-memory only)");
        }
    }

    /// A pre-populated cache serves the listing + README from memory with ZERO CAS —
    /// the `src` is a `PanicSource` that would panic if the walk ran. Proves a HIT is a
    /// pure in-memory op (the whole perf point).
    #[test]
    fn cache_hit_serves_home_without_any_cas() {
        let cache = HomeRenderCache::new();
        let root = ObjectId::from_hex(b"aa".repeat(20).as_slice()).unwrap();
        // Seed the cache with RAW content (dirs-first sort + scrub happen on emit).
        cache.insert(
            root,
            CachedHomeContent {
                entries: vec![
                    RawTreeEntry {
                        name: "Cargo.toml".into(),
                        is_dir: false,
                    },
                    RawTreeEntry {
                        name: "src".into(),
                        is_dir: true,
                    },
                ],
                readme: Some("# hugit\nhello".into()),
            },
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(PanicSource);

        // If the HIT touched CAS, PanicSource::get would panic and fail the test.
        let vm = build_home(
            &EventLog::new(),
            "hugit",
            Some(&src),
            Some(&root),
            Some(&cache),
        );

        // Composed at the read boundary: dirs first, then files alphabetical.
        let names: Vec<&str> = vm.files.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["src", "Cargo.toml"],
            "served from the cache HIT"
        );
        assert!(vm.files[0].is_dir);
        assert!(vm.readme_html.contains("# hugit"));
    }

    /// A MISS walks the CAS once + POPULATES the cache; the NEXT read of the SAME tree
    /// oid is a HIT (proven by swapping to a `PanicSource` for the second read — it
    /// must not touch CAS).
    #[test]
    fn miss_walks_then_populates_then_next_read_hits() {
        let cache = HomeRenderCache::new();
        let mut cas = CasObjectSource::new();
        let readme = cas.insert_raw(ObjectKind::Blob, b"# r\nbody".to_vec());
        let root = insert_tree(
            &mut cas,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "README.md",
                oid: readme,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(cas);

        // MISS → walks + populates.
        assert!(cache.get(&root).is_none(), "cold: no entry yet");
        let vm1 = build_home(
            &EventLog::new(),
            "hugit",
            Some(&src),
            Some(&root),
            Some(&cache),
        );
        assert_eq!(vm1.files.len(), 1, "the walk listed the tree");
        assert!(cache.get(&root).is_some(), "the MISS populated the cache");

        // HIT → the SAME tree oid served with ZERO CAS (PanicSource would panic).
        let panic_src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(PanicSource);
        let vm2 = build_home(
            &EventLog::new(),
            "hugit",
            Some(&panic_src),
            Some(&root),
            Some(&cache),
        );
        assert_eq!(
            vm2.files.len(),
            vm1.files.len(),
            "same content served from HIT"
        );
        assert_eq!(vm2.readme_html, vm1.readme_html);
    }

    /// The content-addressed key makes a stale serve IMPOSSIBLE: a DIFFERENT tree oid
    /// (a push produced a new tree) is a MISS that walks the NEW tree — it never serves
    /// the previously-cached tree's content. Proven by caching tree A's content, then
    /// reading tree B (a real, different CAS tree) and asserting B's listing, not A's.
    #[test]
    fn different_tree_oid_is_a_miss_never_a_stale_serve() {
        let cache = HomeRenderCache::new();
        // Cache tree A's content under a fabricated oid the read will NOT ask for.
        let tree_a = ObjectId::from_hex(b"aa".repeat(20).as_slice()).unwrap();
        cache.insert(
            tree_a,
            CachedHomeContent {
                entries: vec![RawTreeEntry {
                    name: "OLD_A_ONLY.txt".into(),
                    is_dir: false,
                }],
                readme: Some("stale A readme".into()),
            },
        );
        // Build a REAL, different tree B in CAS (the "push" result).
        let mut cas = CasObjectSource::new();
        let b_blob = cas.insert_raw(ObjectKind::Blob, b"x".to_vec());
        let tree_b = insert_tree(
            &mut cas,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "NEW_B_ONLY.rs",
                oid: b_blob,
            }],
        );
        assert_ne!(tree_a, tree_b, "a changed tree has a different oid");
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(cas);

        let vm = build_home(
            &EventLog::new(),
            "hugit",
            Some(&src),
            Some(&tree_b),
            Some(&cache),
        );
        let names: Vec<&str> = vm.files.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["NEW_B_ONLY.rs"],
            "served the NEW tree, not stale A"
        );
        assert!(
            !vm.readme_html.contains("stale A"),
            "the stale tree-A README must never be served for tree B"
        );
    }

    /// REDACTION at the read boundary through the cache: content is stored UNSCRUBBED,
    /// but a secret-shaped filename / a secret in the README redacts before it reaches
    /// the VM (never a redaction bypass, mirroring `search_index`).
    #[test]
    fn cache_stores_raw_but_scrubs_at_read_boundary() {
        use hugit_ledger::redact::REDACTED;
        let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let cache = HomeRenderCache::new();
        let root = ObjectId::from_hex(b"cc".repeat(20).as_slice()).unwrap();
        cache.insert(
            root,
            CachedHomeContent {
                entries: vec![RawTreeEntry {
                    name: format!("{secret}.key"),
                    is_dir: false,
                }],
                readme: Some(format!("intro\ntoken = {secret}\noutro")),
            },
        );
        // The cache holds the RAW secret (stored unscrubbed, like search_index).
        let raw = cache.get(&root).expect("hit");
        assert!(
            raw.entries[0].name.contains(secret) && raw.readme.as_deref().unwrap().contains(secret),
            "content is stored UNSCRUBBED"
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(PanicSource);

        let vm = build_home(
            &EventLog::new(),
            "hugit",
            Some(&src),
            Some(&root),
            Some(&cache),
        );
        let names: String = vm.files.iter().map(|r| r.name.clone()).collect();
        assert!(
            !names.contains(secret),
            "raw secret filename must not survive"
        );
        assert!(
            !vm.readme_html.contains(secret),
            "raw secret in README must not survive"
        );
        assert!(
            names.contains(REDACTED) || vm.readme_html.contains(REDACTED),
            "the REDACTED sentinel must be present after the read-boundary scrub"
        );
        // Line-scoped README redaction: normal lines survive, only the secret line redacts.
        assert!(vm.readme_html.contains("intro") && vm.readme_html.contains("outro"));
    }

    /// The FAST log-derived parts stay FRESH (not cached): with the SAME tree oid (a
    /// cache HIT for files/README) but DIFFERENT logs, the branch/commit_count reflect
    /// the CURRENT log each time — only the CAS-expensive tree content rides the cache.
    #[test]
    fn log_derived_parts_stay_fresh_not_cached() {
        let cache = HomeRenderCache::new();
        let root = ObjectId::from_hex(b"dd".repeat(20).as_slice()).unwrap();
        cache.insert(
            root,
            CachedHomeContent {
                entries: vec![RawTreeEntry {
                    name: "lib.rs".into(),
                    is_dir: false,
                }],
                readme: None,
            },
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(PanicSource);

        // Log 1: branch `main`.
        let mut log1 = EventLog::new();
        log1.append_for_test(
            "ref.update",
            vec!["a:one".to_string()],
            r#"{"ref":"refs/heads/main","target":"aabbcc112233"}"#.to_string(),
            1_700_000_000,
        );
        let vm1 = build_home(&log1, "hugit", Some(&src), Some(&root), Some(&cache));

        // Log 2: branch `dev` + a second ref → a DIFFERENT branch/count.
        let mut log2 = EventLog::new();
        log2.append_for_test(
            "ref.update",
            vec!["a:two".to_string()],
            r#"{"ref":"refs/heads/dev","target":"ddeeff445566"}"#.to_string(),
            1_700_000_100,
        );
        let vm2 = build_home(&log2, "hugit", Some(&src), Some(&root), Some(&cache));

        // The CAS-expensive content is identical (both served from the same cached tree).
        assert_eq!(vm1.files.len(), 1);
        assert_eq!(vm2.files.len(), 1);
        // …but the log-derived parts differ per log (fresh, never cached).
        assert_eq!(vm1.branch, "main");
        assert_eq!(vm2.branch, "dev");
        assert_ne!(
            vm1.branch, vm2.branch,
            "log-derived branch is rebuilt fresh per request, not cached"
        );
    }
}
