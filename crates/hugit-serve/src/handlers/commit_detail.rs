//! `GET /v1/repos/{repo}/commit/{sha}` → `CommitDetailVm` (None = 404).
//!
//! The `log` is ALREADY chain-verified by the caller. REAL: sha / external /
//! intent_id / author / age / title / checks_ok AND the `diff` numstat — the
//! commit-vs-first-parent tree-diff computed from the per-repo git source
//! ([`crate::handlers::diff`]), wall-clock bounded by `hugit_proto::DIFF_BUDGET`.
//! Honest defaults for parent_sha / description / checks_summary / checks_detail
//! (no extended-description or rollup seam — never faked); the diff is honest-empty
//! when there is no git source, a root commit, or an unresolvable commit. 404
//! (None) leaks nothing.

use std::sync::Arc;
use std::time::Instant;

use gix_hash::ObjectId;
use hugit_http_contracts::CommitDetailVm;
use hugit_refstore::EventLog;
use hugit_refstore::intent::projection::{ProjectionRow, project_machine};

use crate::budgeted_source::{BudgetedSource, WALK_BUDGET};
use crate::fmt::{COMMITS_CAP, humanize_age, scrub, sha_prefix};
use crate::handlers::commit_meta::commit_meta_from_cas;
use crate::handlers::diff::{diff_against_first_parent, empty_diff};

/// Build the commit-detail view-model for the commit whose full or 6-char `sha`
/// prefix matches a projection row. Returns `None` (→ 404) when none matches.
///
/// `git_source` is the per-repo object source; when wired the `diff` field carries
/// the REAL commit-vs-first-parent numstat (honest-empty otherwise — never faked).
///
/// `head_commit` is the LIVE default-branch tip: when the sha matches NO
/// `project_machine` row (a git-pushed-but-never-landed commit — the flagship
/// `githugr`, whose event log is empty), the commit is resolved DIRECTLY from the CAS
/// (WP3) by a bounded first-parent walk from `head_commit`, then decoded via
/// [`commit_meta_from_cas`]. That fallback path renders REAL git data (title / author
/// / age / diff) with `intent_id: None` and honest-empty checks — it writes NOTHING
/// to the log and fabricates no landing / cost / check.
pub fn build_commit_detail(
    log: &EventLog,
    repo: &str,
    sha: &str,
    git_source: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    head_commit: Option<ObjectId>,
) -> Option<CommitDetailVm> {
    let machine = project_machine(log).ok()?;
    let checks_ok_shas = build_checks_ok_set(log);
    let sha_lower = sha.to_ascii_lowercase();

    let Some(row) = machine.rows().iter().find(|row| {
        let target = row_target(row);
        let target_lower = target.to_ascii_lowercase();
        target_lower == sha_lower || sha_prefix(&target_lower, 6) == sha_lower
    }) else {
        // WP3: no log row owns this sha → resolve the commit DIRECTLY from CAS.
        return commit_detail_from_cas(repo, &sha_lower, git_source, head_commit, &checks_ok_shas);
    };

    let seq = row_seq(row);
    let recorded_at_ms = log
        .records()
        .get(seq as usize)
        .map(|r| r.recorded_at)
        .unwrap_or(0);
    let author_raw = extract_author(log, seq);
    let author = scrub(&author_raw);
    let age = humanize_age(recorded_at_ms);

    match row {
        ProjectionRow::Intent(c) => {
            let charter_first_line = c.message.lines().next().unwrap_or("").to_string();
            let title = scrub(&charter_first_line);
            let matched_sha = c.target.clone();
            let checks_ok = checks_ok_shas.contains(matched_sha.as_str())
                || checks_ok_shas.contains(sha_prefix(&matched_sha, 6).as_str());
            let provenance_note = scrub(&format!("intent {} via {}", c.intent_id, author_raw));
            // REAL: the commit-vs-first-parent numstat (honest-empty w/o git seam
            // or for a root commit / an unresolvable oid).
            let diff = commit_vs_parent_diff(git_source, &matched_sha);
            Some(CommitDetailVm {
                repo: repo.to_string(),
                sha: matched_sha,
                parent_sha: String::new(), // HONEST-DEFAULT — no parent-sha string seam
                title,
                description: String::new(), // HONEST-DEFAULT — no extended description seam
                author,
                age,
                external: false,
                provenance_note,
                intent_id: Some(c.intent_id.clone()),
                checks_ok,
                checks_summary: String::new(), // HONEST-DEFAULT — no rollup string seam
                checks_detail: String::new(),  // HONEST-DEFAULT
                diff,                          // REAL — commit vs first parent
            })
        }
        ProjectionRow::ExternalChange { kind, .. } => {
            let matched_sha = row_target(row);
            let checks_ok = !matched_sha.is_empty()
                && (checks_ok_shas.contains(matched_sha.as_str())
                    || checks_ok_shas.contains(sha_prefix(&matched_sha, 6).as_str()));
            let title = scrub(kind);
            let provenance_note = scrub(&format!("external {} via {}", kind, author_raw));
            let diff = commit_vs_parent_diff(git_source, &matched_sha);
            Some(CommitDetailVm {
                repo: repo.to_string(),
                sha: matched_sha,
                parent_sha: String::new(),
                title,
                description: String::new(),
                author,
                age,
                external: true,
                provenance_note,
                intent_id: None,
                checks_ok,
                checks_summary: String::new(),
                checks_detail: String::new(),
                diff, // REAL — commit vs first parent
            })
        }
    }
}

/// WP3 — resolve the commit-detail for a sha that matches NO `project_machine` row,
/// DIRECTLY from the CAS (a git-pushed-but-never-landed commit).
///
/// Resolves `sha_lower` (a full 40-hex oid or a short prefix) to a full oid via
/// [`resolve_sha_in_history`], decodes it via [`commit_meta_from_cas`] (WP0), and
/// renders REAL git data: the title (message subject), author, age (REAL commit
/// time), and the commit-vs-first-parent numstat. `intent_id: None` + honest-empty
/// checks/cost/description — a git-only commit carries no landing provenance and we
/// never fabricate one. `None` (→ 404, no oracle) when there is no git seam / head,
/// or the sha resolves to nothing decodable.
fn commit_detail_from_cas(
    repo: &str,
    sha_lower: &str,
    git_source: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    head_commit: Option<ObjectId>,
    checks_ok_shas: &std::collections::HashSet<String>,
) -> Option<CommitDetailVm> {
    let src = git_source?;
    let head = head_commit?;
    // A bounded walk (single-thread latency guard) over a wall-clock-budgeted source.
    let deadline = Instant::now() + WALK_BUDGET;
    let budgeted = BudgetedSource::new(src.as_ref(), deadline);
    let oid = resolve_sha_in_history(&budgeted, head, sha_lower, deadline)?;
    let meta = commit_meta_from_cas(&budgeted, &oid)?;

    let full = oid.to_hex().to_string();
    // Title = the message SUBJECT (first line); free text → scrub at the read boundary.
    let title = scrub(meta.message.lines().next().unwrap_or(""));
    let author = scrub(&meta.author);
    let age = humanize_age(meta.commit_time_ms);
    let checks_ok = checks_ok_shas.contains(full.as_str())
        || checks_ok_shas.contains(sha_prefix(&full, 6).as_str());
    // REAL numstat vs the first parent (honest-empty for a root commit) — the diff
    // seam uses the FULL oid we just resolved (not the URL prefix).
    let diff = diff_against_first_parent(git_source, Some(&oid));

    Some(CommitDetailVm {
        repo: repo.to_string(),
        sha: full,
        parent_sha: String::new(), // HONEST-DEFAULT — no parent-sha string seam
        title,
        description: String::new(), // HONEST-DEFAULT
        author,
        age,
        // A git-pushed commit with no landing is an EXTERNAL change (provenance-free),
        // exactly as `project_machine` classifies a raw push (never a fabricated intent).
        external: true,
        provenance_note: String::new(), // HONEST — no intent/log provenance
        intent_id: None,
        checks_ok,
        checks_summary: String::new(), // HONEST-DEFAULT
        checks_detail: String::new(),  // HONEST-DEFAULT
        diff,                          // REAL — commit vs first parent
    })
}

/// Resolve the CANONICAL content identity of the commit a `GET /commit/{sha}` addresses —
/// the ETag for the conditional-GET path (#ETAG). Returns the full canonical sha the detail
/// view WOULD render for `sha` (a full oid, a 6-char prefix, or a CAS-resolved short prefix),
/// or `None` when the commit does not resolve (→ the caller serves the normal 404, never a
/// 304 for a non-existent commit).
///
/// The commit oid IS the content identity (a commit hashes its tree + parents + metadata), so
/// the canonical sha is a faithful strong ETag: it changes iff the commit content changes. This
/// is deliberately CHEAP — it mirrors [`build_commit_detail`]'s resolution (log-row match, else
/// the bounded CAS history walk) but SKIPS the numstat diff + VM build, so a matching
/// `If-None-Match` yields a 304 without the expensive render. Read-only.
#[must_use]
pub fn resolve_commit_etag(
    log: &EventLog,
    sha: &str,
    git_source: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    head_commit: Option<ObjectId>,
) -> Option<String> {
    let machine = project_machine(log).ok()?;
    let sha_lower = sha.to_ascii_lowercase();
    // A log row owns this sha → its target is the canonical identity (same match rule as
    // `build_commit_detail`: exact or 6-char prefix).
    if let Some(row) = machine.rows().iter().find(|row| {
        let target_lower = row_target(row).to_ascii_lowercase();
        target_lower == sha_lower || sha_prefix(&target_lower, 6) == sha_lower
    }) {
        let target = row_target(row);
        // An external change may carry no target oid — no stable identity, no ETag.
        return (!target.is_empty()).then_some(target);
    }
    // No log row → resolve directly from the CAS (a git-pushed-but-never-landed commit),
    // returning the FULL canonical oid. Bounded + budgeted (same guard as the detail path).
    let src = git_source?;
    let head = head_commit?;
    let deadline = Instant::now() + WALK_BUDGET;
    let budgeted = BudgetedSource::new(src.as_ref(), deadline);
    let oid = resolve_sha_in_history(&budgeted, head, &sha_lower, deadline)?;
    Some(oid.to_hex().to_string())
}

/// Resolve a URL `sha_lower` (full 40-hex oid OR a short prefix) to a full commit
/// oid present in history. A full oid resolves directly; a prefix is matched by a
/// BOUNDED first-parent walk from `head` (count [`COMMITS_CAP`] + the shared
/// wall-clock `deadline` — the same single-thread latency guard as the commits walk).
/// `None` when nothing in the bounded window matches (no existence oracle).
fn resolve_sha_in_history(
    src: &dyn hugit_proto::ObjectSource,
    head: ObjectId,
    sha_lower: &str,
    deadline: Instant,
) -> Option<ObjectId> {
    // A full 40-hex sha references a commit directly (no walk needed).
    if sha_lower.len() == 40
        && let Ok(oid) = ObjectId::from_hex(sha_lower.as_bytes())
    {
        return Some(oid);
    }
    // Otherwise walk the first-parent chain for a commit whose oid hex == / begins
    // with the prefix. Decoding each commit yields its first parent to continue.
    let mut current = Some(head);
    let mut walked = 0usize;
    while let Some(oid) = current {
        if walked >= COMMITS_CAP || Instant::now() >= deadline {
            break;
        }
        walked += 1;
        let hex = oid.to_hex().to_string();
        if hex == sha_lower || hex.starts_with(sha_lower) {
            return Some(oid);
        }
        // A missing/garbled commit stops the walk (fail-closed → 404, never a fake).
        let meta = commit_meta_from_cas(src, &oid)?;
        current = meta.parents.first().copied();
    }
    None
}

/// The commit-vs-first-parent numstat for a target `sha` string. A non-oid /
/// empty `sha` (an external change with no oid) or no git source → the
/// honest-empty diff (never fabricated).
fn commit_vs_parent_diff(
    git_source: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    sha: &str,
) -> hugit_http_contracts::common::DiffVm {
    let Some(commit) = ObjectId::from_hex(sha.as_bytes()).ok() else {
        return empty_diff();
    };
    diff_against_first_parent(git_source, Some(&commit))
}

/// The target oid of a projection row (empty string when an external change has none).
fn row_target(row: &ProjectionRow) -> String {
    match row {
        ProjectionRow::Intent(c) => c.target.clone(),
        ProjectionRow::ExternalChange { target, .. } => target.clone().unwrap_or_default(),
    }
}

/// The seq of a projection row.
fn row_seq(row: &ProjectionRow) -> u64 {
    match row {
        ProjectionRow::Intent(c) => c.seq,
        ProjectionRow::ExternalChange { seq, .. } => *seq,
    }
}

fn build_checks_ok_set(log: &EventLog) -> std::collections::HashSet<String> {
    use hugit_cli::checks::CHECK_RECORDED_KIND;
    let mut set = std::collections::HashSet::new();
    for record in log.records() {
        if record.kind != CHECK_RECORDED_KIND {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&record.payload) else {
            continue;
        };
        if v.get("exit").and_then(serde_json::Value::as_i64) != Some(0) {
            continue;
        }
        if let Some(target) = v.get("target").and_then(serde_json::Value::as_str) {
            let prefix = sha_prefix(target, 6);
            if !prefix.is_empty() {
                set.insert(prefix);
            }
            if target.len() > 6 {
                set.insert(target.to_string());
            }
        }
    }
    set
}

fn extract_author(log: &EventLog, seq: u64) -> String {
    let Some(record) = log.records().get(seq as usize) else {
        return String::new();
    };
    let chain = &record.principal_chain;
    for entry in chain {
        if entry.contains("agent:") || entry.contains("orchestrator:") {
            return entry.clone();
        }
    }
    chain.first().cloned().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_proto::{CasObjectSource, GitObject, ObjectKind};

    fn no_git() -> Option<&'static Arc<dyn hugit_proto::ObjectSource + Send + Sync>> {
        None
    }

    // git-object fixture builders (mirror diff.rs / compare.rs).
    fn blob(src: &mut CasObjectSource, body: &str) -> ObjectId {
        src.insert(GitObject::new(ObjectKind::Blob, body.as_bytes().to_vec()))
    }
    fn tree(src: &mut CasObjectSource, mut entries: Vec<(&str, &str, ObjectId)>) -> ObjectId {
        entries.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));
        let mut out = Vec::new();
        for (mode, name, oid) in &entries {
            out.extend_from_slice(mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(oid.as_bytes());
        }
        src.insert(GitObject::new(ObjectKind::Tree, out))
    }
    fn commit(src: &mut CasObjectSource, tree: ObjectId, parent: Option<ObjectId>) -> ObjectId {
        let mut body = format!("tree {tree}\n");
        if let Some(p) = parent {
            body.push_str(&format!("parent {p}\n"));
        }
        body.push_str("author a <a@x> 0 +0000\ncommitter a <a@x> 0 +0000\n\nmsg\n");
        src.insert(GitObject::new(ObjectKind::Commit, body.into_bytes()))
    }

    /// Seed a `ref.update` (external-change) event whose `target` is `oid_hex`, so
    /// the machine projection yields a row matched by `build_commit_detail(sha=oid)`.
    fn ref_update_log(oid_hex: &str) -> EventLog {
        let mut log = EventLog::new();
        log.append_for_test(
            "ref.update",
            vec!["agent:a".to_string()],
            format!(r#"{{"ref":"refs/heads/main","target":"{oid_hex}"}}"#),
            0,
        );
        log
    }

    /// REAL numstat: a 2-commit fixture; the commit-detail diff is the matched
    /// commit vs its first parent (one line added).
    #[test]
    fn commit_detail_real_numstat_vs_first_parent() {
        let mut s = CasObjectSource::new();
        let b0 = blob(&mut s, "a\nb\n");
        let b1 = blob(&mut s, "a\nb\nc\n");
        let t0 = tree(&mut s, vec![("100644", "f.txt", b0)]);
        let t1 = tree(&mut s, vec![("100644", "f.txt", b1)]);
        let c0 = commit(&mut s, t0, None);
        let c1 = commit(&mut s, t1, Some(c0));
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);

        let log = ref_update_log(&c1.to_string());
        let vm = build_commit_detail(&log, "hugit", &c1.to_string(), Some(&src), None)
            .expect("the seeded commit row is found");
        assert_eq!(vm.diff.files.len(), 1);
        assert_eq!(vm.diff.files[0].path, "f.txt");
        assert_eq!((vm.diff.files[0].added, vm.diff.files[0].removed), (1, 0));
    }

    /// HONEST-EMPTY: a ROOT commit (no first parent) → an empty diff, never a
    /// fabricated all-added wall.
    #[test]
    fn commit_detail_root_commit_is_honest_empty() {
        let mut s = CasObjectSource::new();
        let b = blob(&mut s, "x\n");
        let t = tree(&mut s, vec![("100644", "f", b)]);
        let c = commit(&mut s, t, None); // root commit
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);

        let log = ref_update_log(&c.to_string());
        let vm = build_commit_detail(&log, "hugit", &c.to_string(), Some(&src), None)
            .expect("the seeded commit row is found");
        assert!(vm.diff.files.is_empty(), "root commit → empty diff");
        assert!(vm.diff.hunks.is_empty());
    }

    /// HONEST-EMPTY: no git source → an empty diff (the row is still found).
    #[test]
    fn commit_detail_no_git_source_is_honest_empty() {
        // A 40-hex target that has no CAS object (no git source threaded anyway).
        let oid = "a".repeat(40);
        let log = ref_update_log(&oid);
        let vm = build_commit_detail(&log, "hugit", &oid, no_git(), None)
            .expect("the seeded commit row is found");
        assert!(vm.diff.files.is_empty());
        assert!(vm.diff.hunks.is_empty());
    }

    /// A secret-shaped FILE PATH in the commit diff must be scrubbed at the read
    /// boundary (the numstat is a new echo surface).
    #[test]
    fn commit_detail_secret_shaped_path_is_scrubbed() {
        let mut s = CasObjectSource::new();
        let b = blob(&mut s, "z\n");
        let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        let name = format!("{secret}.key");
        let t0 = tree(&mut s, vec![]);
        let t1 = tree(&mut s, vec![("100644", name.as_str(), b)]);
        let c0 = commit(&mut s, t0, None);
        let c1 = commit(&mut s, t1, Some(c0));
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);

        let log = ref_update_log(&c1.to_string());
        let vm = build_commit_detail(&log, "hugit", &c1.to_string(), Some(&src), None).unwrap();
        let j = serde_json::to_string(&vm.diff).unwrap();
        assert!(!j.contains("ghp_"));
        assert!(j.contains("REDACTED"));
    }

    // ── WP3/WP5: the CAS-fallback path (git-pushed-but-never-landed commit) ────

    /// A commit that exists ONLY in CAS (EMPTY event log — no `project_machine` row)
    /// resolves via the WP3 CAS fallback: REAL title/author/age, a REAL diff-vs-first-
    /// parent, `external: true`, and `intent_id: None` (no fabricated landing). The
    /// URL sha is the SHORT prefix, resolved by the bounded head-walk.
    #[test]
    fn git_only_commit_detail_resolves_from_cas_via_short_prefix() {
        let mut s = CasObjectSource::new();
        let b0 = blob(&mut s, "a\n");
        let b1 = blob(&mut s, "a\nb\n");
        let t0 = tree(&mut s, vec![("100644", "f.txt", b0)]);
        let t1 = tree(&mut s, vec![("100644", "f.txt", b1)]);
        let c0 = commit(&mut s, t0, None);
        let c1 = commit(&mut s, t1, Some(c0)); // head
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);

        let short = sha_prefix(&c1.to_string(), 6);
        let vm = build_commit_detail(&EventLog::new(), "githugr", &short, Some(&src), Some(c1))
            .expect("the CAS commit resolves with an empty log");
        assert_eq!(
            vm.sha,
            c1.to_string(),
            "detail carries the full resolved oid"
        );
        assert_eq!(vm.title, "msg", "REAL commit subject");
        assert!(vm.external, "a git-pushed-not-landed commit is external");
        assert!(vm.intent_id.is_none(), "no fabricated landing provenance");
        // REAL numstat vs the first parent (one line added to f.txt).
        assert_eq!(vm.diff.files.len(), 1);
        assert_eq!(vm.diff.files[0].path, "f.txt");
        assert_eq!((vm.diff.files[0].added, vm.diff.files[0].removed), (1, 0));
    }

    /// The full 40-hex oid URL resolves DIRECTLY from CAS (no walk needed).
    #[test]
    fn git_only_commit_detail_resolves_full_oid_directly() {
        let mut s = CasObjectSource::new();
        let b = blob(&mut s, "x\n");
        let t = tree(&mut s, vec![("100644", "f", b)]);
        let c = commit(&mut s, t, None); // root
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);

        let vm = build_commit_detail(
            &EventLog::new(),
            "githugr",
            &c.to_string(),
            Some(&src),
            Some(c),
        )
        .expect("the full-oid URL resolves directly");
        assert_eq!(vm.sha, c.to_string());
        assert!(vm.diff.files.is_empty(), "root commit → honest-empty diff");
    }

    /// HONEST 404: with no git seam OR no head, a git-only sha (empty log) → `None`
    /// (no existence oracle) — NEVER a fabricated commit.
    #[test]
    fn git_only_commit_detail_404_without_seam_or_head() {
        let mut s = CasObjectSource::new();
        let b = blob(&mut s, "x\n");
        let t = tree(&mut s, vec![("100644", "f", b)]);
        let c = commit(&mut s, t, None);
        let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(s);
        let short = sha_prefix(&c.to_string(), 6);

        // No head_commit → no walk anchor → 404.
        assert!(
            build_commit_detail(&EventLog::new(), "githugr", &short, Some(&src), None).is_none()
        );
        // No git source → 404.
        assert!(
            build_commit_detail(&EventLog::new(), "githugr", &short, no_git(), Some(c)).is_none()
        );
        // An unknown prefix not in history → 404 (no oracle).
        assert!(
            build_commit_detail(&EventLog::new(), "githugr", "ffffff", Some(&src), Some(c))
                .is_none()
        );
    }
}
