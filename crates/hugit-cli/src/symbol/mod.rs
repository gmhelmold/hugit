//! `hugit symbol` — local symbol outline (W6 semantic index, CLI surface).
//!
//! Reads a LOCAL source file, derives its language from the file extension
//! ([`hugit_symbols::lang_for_ext`] — the single source of truth), parses it with
//! [`hugit_symbols::outline_blob`], and emits the symbol outline as stable JSON on
//! stdout under the WB0 one-error/one-exit law ([`crate::porcelain`]).
//!
//! This is the local-file slice of the verb (the buildable-now path): it outlines
//! a file already on disk. The git-tree-backed path (resolve a blob from a ref via
//! `hugit_proto::resolve_blob_at_path`) is the same producer behind a future
//! `--ref`/`--repo` seam; the serve side already serves the outline on
//! `GET /v1/repos/{repo}/blob/{*path}`.
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

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use crate::porcelain::PorcelainError;
use crate::redaction::scrub;

/// `hugit symbol --file <path>` — emit the local file's symbol outline.
#[derive(clap::Args, Debug)]
pub struct SymbolArgs {
    /// Path to the source file to outline. The language is derived from its
    /// extension (e.g. `.rs` → Rust); an unsupported extension yields an empty
    /// outline, never an error.
    #[arg(long)]
    pub file: PathBuf,
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

/// Read the file, outline it, and build the stable JSON value.
fn project(args: &SymbolArgs) -> Result<Value, PorcelainError> {
    let bytes = std::fs::read(&args.file).map_err(|e| {
        PorcelainError::new(
            "file_not_found",
            format!("cannot read source file `{}`: {e}", args.file.display()),
            "pass --file <path> pointing at an existing, readable source file",
        )
        .with_context("file", json!(args.file.display().to_string()))
    })?;

    // Language from the extension (the single source of truth shared with serve).
    let ext = args
        .file
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let lang = hugit_symbols::lang_for_ext(ext);

    // Outline (empty for an unsupported language — honest, never fabricated).
    let outline: Vec<Value> = match lang {
        Some(l) => hugit_symbols::outline_blob(l, &bytes)
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

    Ok(json!({
        // The path is echoed back scrubbed (a crafted path could carry a secret).
        "file": scrub(&args.file.display().to_string()),
        "lang": lang.map(|l| l.as_str()),
        "outline": outline,
    }))
}
