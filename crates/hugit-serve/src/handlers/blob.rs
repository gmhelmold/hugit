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
//!   empty for an unsupported language, never fabricated).
//! - **HONEST-DEFAULT** (no local engine seam this wave — NOT faked, documented
//!   here): `blame: []` (the deep why-blame is the intent-graph seam),
//!   `tree: []` (no sidebar file-tree projection), `via_intent`/`via_model:
//!   None` (no last-writer-intent attribution seam). `actions` mirrors the
//!   sibling pt-BR locale.
//!
//! When the git content seam is not wired (`src`/`root_tree` are `None`), or the
//! path does not resolve to a blob, the handler returns `None` → the caller maps
//! that to a 404. An absent git source is the honest "content seam not live"
//! 404, never a fake blank file.

use std::sync::Arc;

use gix_hash::ObjectId;
use hugit_http_contracts::blob::BlobVm;
use hugit_refstore::EventLog;

use crate::fmt::scrub;

/// Build the blob view-model for `path` at HEAD. `Some(BlobVm)` when the git
/// content seam is wired AND `path` resolves to a blob; `None` otherwise (→ 404).
///
/// `_log` is unused this wave — the content comes from the git tree, not the
/// event log — but the parameter is kept for signature uniformity with the other
/// `build_*` handlers and for the future blame/attribution seam (which WILL read
/// the log to attribute lines to intents).
#[must_use]
pub fn build_blob(
    _log: &EventLog,
    repo: &str,
    path: &str,
    src: Option<&Arc<hugit_proto::CasObjectSource>>,
    root_tree: Option<&ObjectId>,
) -> Option<BlobVm> {
    // The content seam: both the source and the root tree must be present.
    let (src, root_tree) = (src?, root_tree?);

    // Resolve the path to a blob. Ok(None)/Err/absent → 404 (no content oracle):
    // a malformed/corrupt tree, a missing object, or a non-blob path all yield
    // the honest not-found, never a fabricated file.
    let (_oid, bytes) = match hugit_proto::resolve_blob_at_path(src.as_ref(), root_tree, path) {
        Ok(Some(found)) => found,
        Ok(None) | Err(_) => return None,
    };

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
        outline: compute_outline(path, &bytes),
        actions: blob_actions(),
        // HONEST-DEFAULT: no sidebar file-tree projection this wave.
        tree: vec![],
    })
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
        let src = Arc::new(src);

        let vm = build_blob(&log(), "hugit", "main.rs", Some(&src), Some(&root))
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
        assert!(vm.tree.is_empty());
        assert!(vm.via_intent.is_none());
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
        let src = Arc::new(src);

        let vm = build_blob(&log(), "r", "m.rs", Some(&src), Some(&root)).expect("rs resolves");
        let kinds: Vec<(&str, &str)> = vm
            .outline
            .iter()
            .map(|o| (o.kind.as_str(), o.name.as_str()))
            .collect();
        assert!(kinds.contains(&("struct", "Point")), "outline: {kinds:?}");
        assert!(kinds.contains(&("fn", "area")), "outline: {kinds:?}");

        // An unsupported extension (.txt) yields an empty outline — honest, not faked.
        let vm_txt =
            build_blob(&log(), "r", "notes.txt", Some(&src), Some(&root)).expect("txt resolves");
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
        let src = Arc::new(src);

        let vm = build_blob(&log(), "r", "s.rs", Some(&src), Some(&root)).expect("resolves");
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
        let src = Arc::new(src);

        let vm = build_blob(&log(), "r", "a/c.txt", Some(&src), Some(&root)).expect("nested file");
        assert_eq!(vm.lines[0].text, "deep");
        assert_eq!(vm.lang, None); // .txt has no mapping
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
        let src = Arc::new(src);
        assert!(build_blob(&log(), "r", "absent.txt", Some(&src), Some(&root)).is_none());
    }

    #[test]
    fn absent_git_source_is_none_not_fake_content() {
        // The honest "content seam not live" 404 — NOT a fake blank file.
        assert!(build_blob(&log(), "r", "any.rs", None, None).is_none());
        // root_tree present but src absent → still None (lock-step).
        let root = ObjectId::from_hex(b"0000000000000000000000000000000000000001").unwrap();
        assert!(build_blob(&log(), "r", "any.rs", None, Some(&root)).is_none());
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
        let src = Arc::new(src);

        let vm = build_blob(&log(), "r", "cfg.rs", Some(&src), Some(&root)).expect("resolves");
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
}
