//! dock::insights — the per-branch cost projection (WP-DOCK-3 F5).
//!
//! F5 (design §3.2/§7): the unit of insight is the **branch** (what the
//! business asks — "how much did rate-limiting cost?"). The dock is the
//! physical sub-unit; multi-head worktrees on one branch **aggregate** under
//! that branch, never duplicating cost (R2).
//!
//! The projection reads ONLY the canonical log projection
//! ([`super::reconcile::attribute`] — impl note 4, no parallel store):
//!
//! - per-branch — branch total = Σ(docks on branch) (matched + investigated);
//! - residual buckets (R3 — never hidden, never silently zero):
//!   `reconciled` (samples whose dock id no record places), `unlabeled`
//!   (empty-dock samples, or committed work with no dock cost).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::reconcile::{Bucket, attribute_on, records_from_event_log};
use hugit_refstore::EventLog;

/// The `hugit dock insight` args (WP-DOCK-3 F5).
#[derive(clap::Args, Debug)]
pub struct InsightArgs {
    /// Restrict the projection to one branch (else all branches).
    #[arg(long)]
    pub branch: Option<String>,
    /// The canonical log path.
    #[arg(long)]
    pub log: Option<PathBuf>,
}

/// The per-branch projection (F5) — one entry per branch, docks aggregated.
#[derive(Debug, Clone)]
pub struct BranchInsight {
    pub branch: String,
    /// Σ(docks on branch) — matched + investigated (R2: never duplicated).
    pub cost_usd_micros: u64,
    pub commit_count: u64,
    /// matched / investigated (the dock-visible cost split).
    pub matched_cost_usd_micros: u64,
    pub investigated_cost_usd_micros: u64,
    /// The per-dock rows that make up this branch.
    pub docks: Vec<Value>,
}

/// The full insight document (branches + honest residuals).
#[derive(Debug, Clone)]
pub struct InsightDocument {
    pub branches: Vec<BranchInsight>,
    /// Σ cost of samples with an unplaceable dock id (A2) — visible, never
    /// fabricated onto a branch.
    pub reconciled_cost_usd_micros: u64,
    /// Σ cost of samples that raced AHEAD of their dock's coinage (A2 micro-
    /// window) — the `reconciled-late` bucket, never hidden.
    pub reconciled_late_cost_usd_micros: u64,
    /// Σ cost of samples with NO dock binding (empty dock_id).
    pub unlabeled_cost_usd_micros: u64,
    /// Commits on branches with no dock cost and no repo-scope link (R3).
    pub unlabeled_commit_count: u64,
    /// Branches whose unbound intents linked to the repo-scope dock (M5).
    pub repo_scope_linked_branches: Vec<String>,
}

/// Compute the F5 per-branch projection + residuals from a log PATH (the CLI).
pub fn compute_insights(log: &Path, only_branch: Option<&str>) -> Result<InsightDocument, String> {
    let event_log =
        crate::checks::load_event_log(log).map_err(|e| format!("load log: {}", e.to_json()))?;
    compute_insights_on(&event_log, only_branch)
}

/// Compute the F5 per-branch projection + residuals from an IN-MEMORY log (the
/// server) — shares the single attribution core with the CLI (I1, L5).
pub fn compute_insights_on(
    log: &EventLog,
    only_branch: Option<&str>,
) -> Result<InsightDocument, String> {
    let att = attribute_on(&records_from_event_log(log))?;

    // Aggregate per branch: each dock row appears under its branch, its cost
    // counted once (multi-head worktrees sum — R2 never duplicates).
    let mut by_branch: BTreeMap<String, BranchInsight> = BTreeMap::new();
    for d in &att.docks {
        if let Some(b) = only_branch
            && b != d.branch
        {
            continue;
        }
        let entry = by_branch.entry(d.branch.clone()).or_insert(BranchInsight {
            branch: d.branch.clone(),
            cost_usd_micros: 0,
            commit_count: d.commit_count,
            matched_cost_usd_micros: 0,
            investigated_cost_usd_micros: 0,
            docks: Vec::new(),
        });
        entry.cost_usd_micros += d.cost_usd_micros;
        entry.commit_count += d.commit_count;
        match d.bucket {
            Bucket::Matched => entry.matched_cost_usd_micros += d.cost_usd_micros,
            Bucket::Investigated => entry.investigated_cost_usd_micros += d.cost_usd_micros,
            _ => {}
        }
        entry.docks.push(json!({
            "dock_id": d.dock_id,
            "state": d.state,
            "origin": d.origin,
            "cost_usd_micros": d.cost_usd_micros,
            "commit_count": d.commit_count,
            "bucket": d.bucket.as_str(),
        }));
    }

    Ok(InsightDocument {
        branches: by_branch.into_values().collect(),
        reconciled_cost_usd_micros: att.reconciled_cost_usd_micros,
        reconciled_late_cost_usd_micros: att.reconciled_late_cost_usd_micros,
        unlabeled_cost_usd_micros: att.unlabeled_cost_usd_micros,
        unlabeled_commit_count: att.unlabeled_commit_count,
        repo_scope_linked_branches: att.repo_scope_linked_branches,
    })
}

impl InsightDocument {
    /// The SEAL-able wire shape.
    fn to_json(&self) -> Value {
        let branches: Vec<Value> = self
            .branches
            .iter()
            .map(|b| {
                json!({
                    "branch": b.branch,
                    "cost_usd_micros": b.cost_usd_micros,
                    "commit_count": b.commit_count,
                    "buckets": {
                        "matched_usd_micros": b.matched_cost_usd_micros,
                        "investigated_usd_micros": b.investigated_cost_usd_micros,
                    },
                    "docks": b.docks,
                })
            })
            .collect();
        json!({
            "branches": branches,
            "residual": {
                "reconciled_usd_micros": self.reconciled_cost_usd_micros,
                "reconciled_late_usd_micros": self.reconciled_late_cost_usd_micros,
                "unlabeled_usd_micros": self.unlabeled_cost_usd_micros,
                "unlabeled_commit_count": self.unlabeled_commit_count,
                "repo_scope_linked_branches": self.repo_scope_linked_branches,
            },
        })
    }
}

/// Run `hugit dock insight` (F5) — the per-branch + residual projection.
pub fn run_insight(args: InsightArgs) -> std::process::ExitCode {
    let log = crate::log_resolve::resolve_log(args.log.clone());
    match compute_insights(&log, args.branch.as_deref()) {
        Ok(doc) => {
            println!("{}", doc.to_json());
            std::process::ExitCode::SUCCESS
        }
        Err(e) => {
            let esc = serde_json::to_string(&e).unwrap_or_else(|_| format!("\"{e}\""));
            println!("{{\"error\":{}}}", esc);
            std::process::ExitCode::FAILURE
        }
    }
}

/// The log path helper (used by the CLI wiring; mirrors `ls`).
pub fn resolve_log(path: Option<PathBuf>) -> PathBuf {
    crate::log_resolve::resolve_log(path)
}
