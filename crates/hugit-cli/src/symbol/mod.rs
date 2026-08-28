//! `hugit symbol` — local symbol outline (W6 semantic index, CLI surface).
//!
//! Reads a source file either from a LOCAL path (`--file`) or from a git ref
//! (`--ref <ref> --path <path>`), derives its language from the file extension
//! ([`hugit_symbols::lang_for_ext`] — the single source of truth), parses it with
//! [`hugit_symbols::outline_blob`], and emits the symbol outline as stable JSON on
//! stdout under the WB0 one-error/one-exit law ([`crate::porcelain`]).
//!
//! ## Mode selection
//!
//! - `--file <path>` (default / backward-compatible): outline a local working-tree file.
//! - `--ref <ref> --path <path>`: outline a file from a committed git tree. `<ref>`
//!   is any revspec git accepts (`HEAD`, `main`, `abc1234`, `v1.0`, etc.); `<path>`
//!   is the repo-root-relative path to the file (forward-slash separated, no leading `/`).
//!   `--file` and `--ref` are mutually exclusive; omitting both is an error.
//!
//! The `--ref` path uses a lazy [`git_source::GitCatFileSource`] that shells out to
//! `git cat-file` per tree/blob object — correct for CLI depth (1–4 git subprocesses
//! for a typical file) without loading all objects eagerly.
//!
//! Output shape (stable JSON, exit 0):
//! ```json
//! {"file":"<scrubbed path>","lang":"rust"|null,
//!  "outline":[{"kind":"fn","name":"main","line":1}, …]}
//! ```
//! An unsupported/extensionless file is NOT an error — it is an honest empty
//! outline with `lang:null` (the file simply has no recognized language), exactly
//! as the serve blob handler returns `[]`. A missing/unreadable file IS an error:
//! the canonical `{"error":{kind:"file_not_found",…}}` envelope, exit 2.
//!
//! Every surfaced symbol `name` passes through [`crate::redaction::scrub`] at the
//! read boundary — a name derived from source (a `const`/`static` identifier, an
//! `impl … for …` display string) could embed a secret-shaped token, and the
//! outline must not become a redaction bypass.

pub mod git_source;

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::porcelain::PorcelainError;
use crate::redaction::scrub;

/// `hugit symbol` — emit a source file's symbol outline.
///
/// Exactly one of `--file` or (`--ref` + `--path`) must be provided.
#[derive(clap::Args, Debug)]
pub struct SymbolArgs {
    /// Path to a local source file to outline. The language is derived from its
    /// extension (e.g. `.rs` → Rust); an unsupported extension yields an empty
    /// outline, never an error.
    ///
    /// Mutually exclusive with `--ref`.
    #[arg(long, conflicts_with = "git_ref")]
    pub file: Option<PathBuf>,

    /// Git revspec whose committed tree the file is read from (e.g. `HEAD`,
    /// `main`, `abc1234`, `v1.0`). Requires `--path`.
    ///
    /// Mutually exclusive with `--file`.
    #[arg(long = "ref", conflicts_with = "file", requires = "ref_path")]
    pub git_ref: Option<String>,

    /// Repo-root-relative path to the file inside the git tree (forward-slash
    /// separated, no leading `/`). Required when `--ref` is given.
    #[arg(long = "path", requires = "git_ref")]
    pub ref_path: Option<String>,

    /// Path to the git working tree (defaults to the nearest `.git` ancestor of
    /// the current directory). Ignored unless `--ref` is given.
    #[arg(long = "git-dir")]
    pub git_dir: Option<PathBuf>,
}

/// Dispatch `hugit symbol`, emitting stable JSON on stdout and returning the
/// process exit code under the WB0 one-exit-code law.
pub fn run(args: SymbolArgs) -> ExitCode {
    match project(&args) {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", e.to_json());
            e.exit_code()
        }
    }
}

/// Dispatch to the correct source path and build the stable JSON value.
fn project(args: &SymbolArgs) -> Result<Value, PorcelainError> {
    match (&args.file, &args.git_ref) {
        (Some(file), None) => project_file(file),
        (None, Some(git_ref)) => {
            // --ref requires --path, enforced by clap; unwrap is safe.
            let path = args
                .ref_path
                .as_deref()
                .expect("clap requires --path with --ref");
            let git_dir = resolve_git_dir(args.git_dir.as_deref())?;
            project_ref(&git_dir, git_ref, path)
        }
        (None, None) => Err(PorcelainError::new(
            "missing_source",
            "one of --file or --ref must be given",
            "pass --file <path> to outline a local file, or --ref <ref> --path <repo-path> to outline a file from a git ref",
        )),
        (Some(_), Some(_)) => {
            // Clap's `group(multiple = false)` on SymbolArgs catches this before
            // we reach here, but handle it defensively.
            Err(PorcelainError::new(
                "conflicting_args",
                "--file and --ref are mutually exclusive",
                "pass either --file or --ref, not both",
            ))
        }
    }
}

/// Resolve a user-supplied `--git-dir` or discover the nearest `.git` ancestor.
fn resolve_git_dir(explicit: Option<&std::path::Path>) -> Result<PathBuf, PorcelainError> {
    if let Some(dir) = explicit {
        return Ok(dir.to_path_buf());
    }
    let cwd = std::env::current_dir().map_err(|e| {
        PorcelainError::new(
            "cwd_unavailable",
            format!("cannot determine current directory: {e}"),
            "run hugit from inside a git repository",
        )
    })?;
    git_source::open_git_dir(&cwd).map_err(|e| {
        PorcelainError::new(
            "not_a_git_repo",
            e,
            "run hugit symbol --ref from inside a git repository, or pass --git-dir <dir>",
        )
    })
}

// ── --file path ──────────────────────────────────────────────────────────────

/// Read the local file, outline it, and build the stable JSON value.
fn project_file(file: &std::path::Path) -> Result<Value, PorcelainError> {
    let bytes = std::fs::read(file).map_err(|e| {
        PorcelainError::new(
            "file_not_found",
            format!("cannot read source file `{}`: {e}", file.display()),
            "pass --file <path> pointing at an existing, readable source file",
        )
        .with_context("file", json!(file.display().to_string()))
    })?;

    let ext = file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();

    Ok(outline_to_json(
        &scrub(&file.display().to_string()),
        ext,
        &bytes,
    ))
}

// ── --ref path ───────────────────────────────────────────────────────────────

/// Resolve `refspec` → root tree → path → blob bytes, outline, and return JSON.
fn project_ref(
    git_dir: &std::path::Path,
    refspec: &str,
    path: &str,
) -> Result<Value, PorcelainError> {
    // 1. Resolve the ref to the root tree oid.
    let root_tree = git_source::resolve_ref_root_tree(git_dir, refspec).map_err(|e| {
        PorcelainError::new(
            "ref_not_found",
            e,
            "pass a valid git ref (HEAD, a branch, a tag, or a commit hash)",
        )
        .with_context("ref", json!(refspec))
    })?;

    // 2. Walk the tree to the blob.
    let src = git_source::GitCatFileSource::new(git_dir.to_path_buf());
    let blob_bytes = hugit_proto::resolve_blob_at_path(&src, &root_tree, path).map_err(|e| {
        PorcelainError::new(
            "git_object_error",
            format!("reading git objects for `{path}` at `{refspec}`: {e}"),
            "check that the ref and path are correct and the git repository is intact",
        )
        .with_context("ref", json!(refspec))
        .with_context("path", json!(path))
    })?;

    // 3. A missing path (not an error, just absent) → clean not_found.
    let (_oid, bytes) = blob_bytes.ok_or_else(|| {
        PorcelainError::new(
            "path_not_found",
            format!("`{path}` does not exist at ref `{refspec}`"),
            "pass a path that exists in the committed tree at the given ref",
        )
        .with_context("ref", json!(refspec))
        .with_context("path", json!(path))
    })?;

    // 4. Derive the extension from the final path component (same rule as --file).
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();

    // The "file" field echoes the repo-root-relative path, scrubbed.
    Ok(outline_to_json(&scrub(path), ext, &bytes))
}

// ── shared outline builder ────────────────────────────────────────────────────

/// Build the stable JSON outline value from (display_path, extension, bytes).
/// Shared by both the `--file` and `--ref` paths so the output shape is identical.
fn outline_to_json(display_path: &str, ext: &str, bytes: &[u8]) -> Value {
    let lang = hugit_symbols::lang_for_ext(ext);

    // Outline (empty for an unsupported language — honest, never fabricated).
    let outline: Vec<Value> = match lang {
        Some(l) => hugit_symbols::outline_blob(l, bytes)
            .into_iter()
            .map(|item| {
                json!({
                    "kind": item.kind.as_wire_str(),
                    // Scrub at the read boundary — a symbol name could embed a secret.
                    "name": scrub(&item.name),
                    "line": item.line,
                })
            })
            .collect(),
        None => Vec::new(),
    };

    json!({
        // The path is echoed back scrubbed (a crafted path could carry a secret).
        "file": display_path,
        "lang": lang.map(|l| l.as_str()),
        "outline": outline,
    })
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    // ── --file tests (backward compatibility) ────────────────────────────────

    /// `--file` on a Rust source produces a non-empty outline.
    #[test]
    fn file_rust_outline_nonempty() {
        let rs_path = std::env::temp_dir().join("hugit_symbol_test_hello.rs");
        std::fs::write(&rs_path, "pub fn hello() {}\npub struct Foo;\n").unwrap();
        let args = SymbolArgs {
            file: Some(rs_path.clone()),
            git_ref: None,
            ref_path: None,
            git_dir: None,
        };
        let v = project(&args).expect("project_file should succeed");
        assert_eq!(v["lang"], "rust");
        assert!(!v["outline"].as_array().unwrap().is_empty());
        std::fs::remove_file(rs_path).ok();
    }

    /// `--file` on an unsupported extension produces an empty outline, not an error.
    #[test]
    fn file_unknown_ext_empty_outline() {
        let unk_path = std::env::temp_dir().join("hugit_symbol_test_unk.unk_ext_xyz");
        std::fs::write(&unk_path, "hello").unwrap();
        let args = SymbolArgs {
            file: Some(unk_path.clone()),
            git_ref: None,
            ref_path: None,
            git_dir: None,
        };
        let v = project(&args).expect("project_file should succeed");
        assert_eq!(v["lang"], serde_json::Value::Null);
        assert_eq!(v["outline"].as_array().unwrap().len(), 0);
        std::fs::remove_file(unk_path).ok();
    }

    /// `--file` on a missing path returns a structured error, not a panic.
    #[test]
    fn file_missing_returns_error() {
        let args = SymbolArgs {
            file: Some(PathBuf::from("/tmp/hugit_test_nonexistent_file_xyzzy.rs")),
            git_ref: None,
            ref_path: None,
            git_dir: None,
        };
        let err = project(&args).expect_err("expected error for missing file");
        assert_eq!(err.kind(), "file_not_found");
    }

    // ── --ref tests ──────────────────────────────────────────────────────────

    /// Helper: discover the repo root from cwd (tests run inside the workspace).
    fn repo_root() -> PathBuf {
        let cwd = env::current_dir().expect("cwd");
        git_source::open_git_dir(&cwd).expect("find repo root")
    }

    /// `--ref HEAD --path <stable_rs_file>` must produce a non-empty Rust outline.
    /// Uses led/mod.rs which is committed at HEAD and unmodified in this wave.
    #[test]
    fn ref_head_outlines_a_committed_rs_file() {
        let root = repo_root();
        let args = SymbolArgs {
            file: None,
            git_ref: Some("HEAD".to_string()),
            ref_path: Some("crates/hugit-cli/src/ident.rs".to_string()),
            git_dir: Some(root),
        };
        let v = project(&args).expect("project_ref should succeed at HEAD");
        assert_eq!(v["lang"], "rust", "led/mod.rs is Rust");
        let outline = v["outline"].as_array().expect("outline array");
        assert!(
            !outline.is_empty(),
            "HEAD outline of led/mod.rs must not be empty"
        );
    }

    /// Outline the same committed file via `--ref HEAD` and `--file`, confirming
    /// the same producer produces the same outline for the same bytes.
    ///
    /// Uses the led/mod.rs file because it is stable (no edits in this wave)
    /// and committed at HEAD in this worktree.
    #[test]
    fn ref_head_matches_file_outline() {
        let root = repo_root();
        // Pick a stable committed file.
        let stable_path = "crates/hugit-cli/src/ident.rs";

        // --ref HEAD path
        let ref_args = SymbolArgs {
            file: None,
            git_ref: Some("HEAD".to_string()),
            ref_path: Some(stable_path.to_string()),
            git_dir: Some(root.clone()),
        };
        let ref_v = project(&ref_args).expect("project_ref");

        // --file path (the same file on disk — unmodified in this wave)
        let file_path = root.join(stable_path);
        let file_args = SymbolArgs {
            file: Some(file_path),
            git_ref: None,
            ref_path: None,
            git_dir: None,
        };
        let file_v = project(&file_args).expect("project_file");

        // The outlines must agree: same lang, same symbol names in the same order.
        assert_eq!(ref_v["lang"], file_v["lang"], "lang must match");
        let ref_names: Vec<_> = ref_v["outline"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap_or(""))
            .collect();
        let file_names: Vec<_> = file_v["outline"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(
            ref_names, file_names,
            "symbol names from --ref and --file must match"
        );
    }

    /// `--ref HEAD --path path/that/does/not/exist.rs` returns `path_not_found`.
    #[test]
    fn ref_missing_path_returns_path_not_found() {
        let root = repo_root();
        let args = SymbolArgs {
            file: None,
            git_ref: Some("HEAD".to_string()),
            ref_path: Some("no/such/path/ever_exists.rs".to_string()),
            git_dir: Some(root),
        };
        let err = project(&args).expect_err("expected error for missing path");
        assert_eq!(err.kind(), "path_not_found");
    }

    /// `--ref refs/heads/this-branch-does-not-exist` returns `ref_not_found`.
    #[test]
    fn ref_bad_ref_returns_ref_not_found() {
        let root = repo_root();
        let args = SymbolArgs {
            file: None,
            git_ref: Some("refs/heads/this-branch-does-not-exist-ever-xyzzy".to_string()),
            ref_path: Some("crates/hugit-cli/src/symbol/mod.rs".to_string()),
            git_dir: Some(root),
        };
        let err = project(&args).expect_err("expected error for bad ref");
        assert_eq!(err.kind(), "ref_not_found");
    }

    /// Omitting both `--file` and `--ref` returns `missing_source`.
    #[test]
    fn neither_file_nor_ref_returns_error() {
        let args = SymbolArgs {
            file: None,
            git_ref: None,
            ref_path: None,
            git_dir: None,
        };
        let err = project(&args).expect_err("expected error with no source");
        assert_eq!(err.kind(), "missing_source");
    }
}
