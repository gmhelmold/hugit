//! `GET /v1/repos/{repo}/checks` → [`ChecksVm`]. FROZEN signature; body filled by
//! the fleet per master-plan §5 (REAL: local hit-rate KPIs + check rows via the
//! `hugit checks show` projection; STUB: bisect/culprit + FLEET KPIs = P2).

use hugit_http_contracts::ChecksVm;
use hugit_refstore::EventLog;

/// Build the checks view-model from a verified event log.
pub fn build_checks(_log: &EventLog, _repo: &str) -> ChecksVm {
    todo!("Wave-1 checks handler — fleet fills per master-plan §5 source map")
}
