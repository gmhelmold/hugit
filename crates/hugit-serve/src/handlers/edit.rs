//! `GET /v1/repos/{repo}/edit/{*path}` → [`EditVm`].
//!
//! The editor view of a file at HEAD. Like [`super::blob`], the content is REAL
//! — read from the git tree via [`hugit_proto::resolve_blob_at_path`] and SCRUBBED
//! at the read boundary ([`crate::fmt::scrub`]). The editor chrome (workspace
//! note, commit form labels, branch/authorship notes) is an honest-default static
//! projection in the sibling pt-BR locale: this wave serves the file to edit, it
//! does not yet carry a live ephemeral-workspace / proposed-diff seam (that is the
//! write path, `POST .../edit/.../propose`, a separate route).
//!
//! When the git content seam is not wired (`src`/`root_tree` `None`) or the path
//! does not resolve to a blob, returns `None` → the caller maps it to a 404 (the
//! honest "content seam not live" answer, never a fake blank file).

use std::sync::Arc;
use std::time::Instant;

use gix_hash::ObjectId;
use hugit_http_contracts::common::HunkVm;
use hugit_http_contracts::edit::{EditLineVm, EditVm};
use hugit_refstore::EventLog;

use crate::budgeted_source::{BudgetedSource, WALK_BUDGET};
use crate::fmt::scrub;

/// Build the editor view-model for `path` at HEAD. `Some(EditVm)` when the git
/// content seam is wired AND `path` resolves to a blob; `None` otherwise (→ 404).
///
/// `_log` is unused this wave (the content comes from the git tree, not the event
/// log); the parameter is kept for signature uniformity with the sibling handlers.
///
/// DoS: the `path`→blob resolve does one synchronous CAS `get` per path segment
/// ([`hugit_proto::resolve_blob_at_path`], count-capped at `MAX_PATH_DEPTH`). On the
/// single-threaded lazy-CAS engine that count-cap is NOT a latency cap — under a
/// cold cache / slow R2 a deep resolve can block the whole accept loop. So the walk
/// is ALSO wall-clock bounded by [`WALK_BUDGET`]: past the deadline the source stops
/// yielding objects and the resolve returns an honest `None` → 404 (never a
/// fabricated file).
#[must_use]
pub fn build_edit(
    _log: &EventLog,
    repo: &str,
    path: &str,
    src: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    root_tree: Option<&ObjectId>,
) -> Option<EditVm> {
    build_edit_until(
        _log,
        repo,
        path,
        src,
        root_tree,
        Instant::now() + WALK_BUDGET,
    )
}

/// [`build_edit`] with an explicit wall-clock `deadline` on the CAS resolve —
/// deterministically testable (a deadline already in the past stops before the
/// first fetch → honest `None`). See [`build_edit`] for the DoS rationale.
#[must_use]
fn build_edit_until(
    _log: &EventLog,
    repo: &str,
    path: &str,
    src: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    root_tree: Option<&ObjectId>,
    deadline: Instant,
) -> Option<EditVm> {
    let (src, root_tree) = (src?, root_tree?);

    // Wall-clock-bound the per-segment CAS walk (single-thread latency-DoS guard):
    // past `deadline` every `get` returns `Ok(None)`, so `resolve_blob_at_path`
    // stops and yields `None` → an honest 404, never a fabricated file.
    let budgeted = BudgetedSource::new(src.as_ref(), deadline);
    let (_oid, bytes) = match hugit_proto::resolve_blob_at_path(&budgeted, root_tree, path) {
        Ok(Some(found)) => found,
        Ok(None) | Err(_) => return None,
    };

    let scrubbed_path = scrub(path);
    let lines = decode_lines(&bytes);

    Some(EditVm {
        repo: scrub(repo),
        path: scrubbed_path.clone(),
        // HONEST-DEFAULT chrome (sibling pt-BR locale/voice). No live ephemeral
        // workspace seam this wave — the file is served for viewing/editing; the
        // actual edit→propose is the write route, not this read.
        workspace_note: "Workspace efêmero, só seu.".to_string(),
        lines,
        // No pending edits on a fresh read → no change summary.
        changed_note: String::new(),
        changed_count: String::new(),
        // No diff yet (nothing changed) — an empty preview hunk over this file.
        preview: HunkVm {
            file: scrubbed_path,
            header: String::new(),
            lines: vec![],
        },
        honesty_note: "propor vira um commit SEU (mudança externa assinada).".to_string(),
        commit_title: String::new(),
        commit_description: String::new(),
        direct_main_label: "commit direto na main".to_string(),
        direct_main_reason: "main é single-writer.".to_string(),
        branch_label: "criar branch e abrir PR".to_string(),
        branch_note: "branch criada a partir de main · PR rascunho.".to_string(),
        authorship_note: "A autoria da mudança é sua — assinada com sua identidade HuGR."
            .to_string(),
    })
}

/// Decode raw blob bytes into scrubbed, 1-based-numbered editor lines. UTF-8 is
/// decoded lossily; every line passes through [`scrub`] (a secret in the file is
/// redacted before the browser). `edited: false` — a fresh read has no pending
/// edits (the ● gutter marker is set by the client as the user types).
fn decode_lines(bytes: &[u8]) -> Vec<EditLineVm> {
    let text = String::from_utf8_lossy(bytes);
    text.split('\n')
        .enumerate()
        .map(|(i, line)| EditLineVm {
            number: (i as u32) + 1,
            text: scrub(line.strip_suffix('\r').unwrap_or(line)),
            edited: false,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::{CasObjectSource, ObjectKind};

    const MODE_BLOB: &str = "100644";

    struct TreeEntry<'a> {
        mode: &'a str,
        name: &'a str,
        oid: ObjectId,
    }

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

    fn log() -> EventLog {
        EventLog::new()
    }

    #[test]
    fn resolves_real_file_into_editor_lines() {
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"line one\nline two\n".to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "f.txt",
                oid: blob,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_edit(&log(), "r", "f.txt", Some(&src), Some(&root)).expect("present file");
        assert_eq!(vm.path, "f.txt");
        assert_eq!(vm.lines[0].text, "line one");
        assert_eq!(vm.lines[0].number, 1);
        assert!(!vm.lines[0].edited);
        assert_eq!(vm.lines[1].text, "line two");
        // No pending edits on a fresh read.
        assert!(vm.changed_note.is_empty());
        assert!(vm.preview.lines.is_empty());
        assert_eq!(vm.preview.file, "f.txt");
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
        assert!(build_edit(&log(), "r", "nope.txt", Some(&src), Some(&root)).is_none());
    }

    #[test]
    fn absent_git_source_is_none() {
        assert!(build_edit(&log(), "r", "any.rs", None, None).is_none());
    }

    /// DoS bound: a deadline already in the PAST stops the CAS resolve before the
    /// first fetch → an honest `None` (404) EVEN THOUGH the file is present. Proves
    /// the walk is wall-clock-bounded, not merely count-capped, and the truncation
    /// is honest-empty (never a fabricated file). The mirror far-future case is
    /// covered by `resolves_real_file_into_editor_lines` (default budget).
    #[test]
    fn past_deadline_bounds_resolve_to_honest_none() {
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
            build_edit_until(&log(), "r", "here.txt", Some(&src), Some(&root), live).is_some(),
            "a present file resolves under a live budget"
        );

        // Past deadline: the resolve is refused at the first `get` → honest 404.
        let past = Instant::now() - Duration::from_secs(1);
        assert!(
            build_edit_until(&log(), "r", "here.txt", Some(&src), Some(&root), past).is_none(),
            "a present file is honest-404'd once the walk budget is spent"
        );
    }

    #[test]
    fn secret_in_content_is_redacted() {
        let secret = "gho_16C7e42F292c6912E7710c838347Ae178B4a";
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, format!("token = {secret}\n").into_bytes());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "s.txt",
                oid: blob,
            }],
        );
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(src);

        let vm = build_edit(&log(), "r", "s.txt", Some(&src), Some(&root)).expect("resolves");
        let joined: String = vm.lines.iter().map(|l| l.text.clone()).collect();
        assert!(
            !joined.contains(secret),
            "raw secret must not survive: {joined}"
        );
        assert!(
            joined.contains(hugit_ledger::redact::REDACTED),
            "REDACTED sentinel must be present: {joined}"
        );
    }
}
