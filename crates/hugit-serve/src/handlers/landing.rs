//! `GET /v1/repos/{repo}/landing` → [`LandingVm`]. FROZEN signature; body filled
//! by the fleet per master-plan §5 (REAL: PR list/state/queue + cost via
//! ledger rollup + campaigns; STUB: main_green/status, file_count, mirror, diff).

use hugit_http_contracts::LandingVm;
use hugit_refstore::EventLog;

/// Build the landing view-model from a verified event log.
pub fn build_landing(_log: &EventLog, _repo: &str) -> LandingVm {
    todo!("Wave-1 landing handler — fleet fills per master-plan §5 source map")
}
