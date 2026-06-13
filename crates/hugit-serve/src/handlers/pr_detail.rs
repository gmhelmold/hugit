//! `GET /v1/repos/{repo}/prs/{n}` → [`PrDetailVm`] (None = 404). FROZEN signature;
//! body filled by the fleet per master-plan §5 (REAL: PR projection + cost split
//! via ledger + intents + check_rows + envelope; STUB: diff/added/removed,
//! impact, reviewers, labels, mirror, branches).

use hugit_http_contracts::PrDetailVm;
use hugit_refstore::EventLog;

/// Build the PR-detail view-model for PR `pr_number`. `None` → HTTP 404 (the read
/// is `get_opt`: not-found and no-access are indistinguishable — no existence leak).
pub fn build_pr_detail(_log: &EventLog, _repo: &str, _pr_number: u32) -> Option<PrDetailVm> {
    todo!("Wave-1 pr_detail handler — fleet fills per master-plan §5 source map")
}
