//! `GET /v1/repos/{repo}/commit/{sha}` → `CommitDetailVm` (None = 404).
//!
//! The `log` is ALREADY chain-verified by the caller. REAL: sha / external /
//! intent_id / author / age / title / checks_ok. Honest defaults for
//! parent_sha / description / checks_summary / checks_detail / diff (no diffstat
//! or git-parent seam — never faked). 404 (None) leaks nothing.

use hugit_http_contracts::CommitDetailVm;
use hugit_http_contracts::common::DiffVm;
use hugit_refstore::EventLog;
use hugit_refstore::intent::projection::{ProjectionRow, project_machine};

use crate::fmt::{humanize_age, scrub, sha_prefix};

/// Build the commit-detail view-model for the commit whose full or 6-char `sha`
/// prefix matches a projection row. Returns `None` (→ 404) when none matches.
pub fn build_commit_detail(log: &EventLog, repo: &str, sha: &str) -> Option<CommitDetailVm> {
    let machine = project_machine(log).ok()?;
    let checks_ok_shas = build_checks_ok_set(log);
    let sha_lower = sha.to_ascii_lowercase();

    let row = machine.rows().iter().find(|row| {
        let target = row_target(row);
        let target_lower = target.to_ascii_lowercase();
        target_lower == sha_lower || sha_prefix(&target_lower, 6) == sha_lower
    })?;

    let seq = row_seq(row);
    let recorded_at_ms = log
        .records()
        .get(seq as usize)
        .map(|r| r.recorded_at)
        .unwrap_or(0);
    let author_raw = extract_author(log, seq);
    let author = scrub(&author_raw);
    let age = humanize_age(recorded_at_ms);

    let empty_diff = DiffVm {
        files: vec![],
        hunks: vec![],
    };

    match row {
        ProjectionRow::Intent(c) => {
            let charter_first_line = c.message.lines().next().unwrap_or("").to_string();
            let title = scrub(&charter_first_line);
            let matched_sha = c.target.clone();
            let checks_ok = checks_ok_shas.contains(matched_sha.as_str())
                || checks_ok_shas.contains(sha_prefix(&matched_sha, 6).as_str());
            let provenance_note = scrub(&format!("intent {} via {}", c.intent_id, author_raw));
            Some(CommitDetailVm {
                repo: repo.to_string(),
                sha: matched_sha,
                parent_sha: String::new(), // HONEST-DEFAULT — no git-parent seam
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
                diff: empty_diff,              // HONEST-DEFAULT — no diffstat seam
            })
        }
        ProjectionRow::ExternalChange { kind, .. } => {
            let matched_sha = row_target(row);
            let checks_ok = !matched_sha.is_empty()
                && (checks_ok_shas.contains(matched_sha.as_str())
                    || checks_ok_shas.contains(sha_prefix(&matched_sha, 6).as_str()));
            let title = scrub(kind);
            let provenance_note = scrub(&format!("external {} via {}", kind, author_raw));
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
                diff: empty_diff,
            })
        }
    }
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
