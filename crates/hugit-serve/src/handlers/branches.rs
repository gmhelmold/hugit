//! `GET /v1/repos/{repo}/branches` → [`BranchesVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. REAL: branch/tag counts,
//! default-branch pick, branch rows (name/head_sha/intent_id/checks_ok) from
//! `replay`/`project_machine`/checks. Honest defaults for fields with no local
//! seam (protected/ahead/behind/pr/activity), documented inline.

use std::collections::{HashMap, HashSet};

use hugit_http_contracts::{BranchRowVm, BranchesVm};
use hugit_refstore::EventLog;
use hugit_refstore::intent::projection::{ProjectionRow, project_machine};
use hugit_refstore::replay::replay;

use crate::fmt::{scrub, sha_prefix};

/// Maximum branch rows emitted before capping; `more_count` reports the tail.
const BRANCHES_CAP: usize = 100;

/// Build the branches view-model from a verified event log.
pub fn build_branches(log: &EventLog, repo: &str) -> BranchesVm {
    let ref_state = replay(log).unwrap_or_default();

    let mut heads: Vec<(String, String)> = Vec::new(); // (short_name, target_sha)
    let mut tag_count: usize = 0;
    for (name, target) in ref_state.iter() {
        if let Some(short) = name.strip_prefix("refs/heads/") {
            heads.push((short.to_string(), target.to_string()));
        } else if name.starts_with("refs/tags/") {
            tag_count += 1;
        }
    }
    let branch_count = heads.len();

    // pick_head — prefer main > master > first lexicographic (non-intent).
    let plain: Vec<&(String, String)> = heads
        .iter()
        .filter(|(name, _)| !name.starts_with("intent/"))
        .collect();
    let default_name: String = if plain.iter().any(|(n, _)| n == "main") {
        "main".to_string()
    } else if plain.iter().any(|(n, _)| n == "master") {
        "master".to_string()
    } else {
        plain.first().map(|(n, _)| n.clone()).unwrap_or_default()
    };

    // ref_name → intent_id index from machine-altitude rows.
    let machine = project_machine(log).unwrap_or_default();
    let intent_ref_index: HashMap<String, String> = machine
        .rows()
        .iter()
        .filter_map(|row| match row {
            ProjectionRow::Intent(gc) => Some((gc.ref_name.clone(), gc.intent_id.clone())),
            ProjectionRow::ExternalChange { .. } => None,
        })
        .collect();

    let checks_ok_shas = build_checks_ok_set(log);

    let mut rows: Vec<BranchRowVm> = Vec::with_capacity(heads.len().min(BRANCHES_CAP));
    for (short_name, target) in &heads {
        let name = scrub(short_name);
        let head_sha = sha_prefix(target, 6);
        let is_default = short_name == &default_name;
        let fq_ref = format!("refs/heads/{short_name}");
        let intent_id: Option<String> = intent_ref_index.get(&fq_ref).cloned();
        let checks_ok: Option<bool> = if head_sha.is_empty() {
            None
        } else {
            Some(
                checks_ok_shas.contains(head_sha.as_str())
                    || checks_ok_shas.contains(target.as_str()),
            )
        };
        rows.push(BranchRowVm {
            name,
            is_default,
            protected: false, // HONEST-DEFAULT — no protection-rules seam
            head_sha,
            attr: String::new(), // HONEST-DEFAULT — no last-push timestamp seam
            intent_id,
            model: None,  // HONEST-DEFAULT — no per-branch model seam
            ahead: None,  // HONEST-DEFAULT — no graph-walk seam
            behind: None, // HONEST-DEFAULT — no graph-walk seam
            checks_ok,
            pr: None, // HONEST-DEFAULT — no GitHub PR-link seam (P2)
        });
        if rows.len() >= BRANCHES_CAP {
            break;
        }
    }

    let default_branch: BranchRowVm =
        rows.iter()
            .find(|r| r.is_default)
            .cloned()
            .unwrap_or_else(|| BranchRowVm {
                name: default_name.clone(),
                is_default: true,
                protected: false,
                head_sha: String::new(),
                attr: String::new(),
                intent_id: None,
                model: None,
                ahead: None,
                behind: None,
                checks_ok: None,
                pr: None,
            });

    let more_count = branch_count.saturating_sub(rows.len());

    BranchesVm {
        repo: repo.to_string(),
        branch_count,
        tag_count,
        active_count: 0, // HONEST-DEFAULT — no activity seam
        yours_count: 0,  // HONEST-DEFAULT — no caller-principal seam (P2)
        default_branch,
        branches: rows,
        inactive_count: 0,            // HONEST-DEFAULT — no staleness seam
        inactive_note: String::new(), // HONEST-DEFAULT — no staleness seam
        more_count,
    }
}

/// shas (full + 6-char prefix) for commits with ≥1 successful `check.recorded`.
/// Mirrors the helper in `handlers::commits`.
fn build_checks_ok_set(log: &EventLog) -> HashSet<String> {
    use hugit_cli::checks::CHECK_RECORDED_KIND;
    let mut set = HashSet::new();
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
