//! `GET /v1/repos/{repo}/commits` → [`CommitsVm`]. FROZEN signature; body filled
//! by the fleet per master-plan §5 (REAL: commit rows via `project_machine`,
//! branches via `replay`; PRESENTATION: age/avatar_class; honest: `checks_ok`).

use hugit_http_contracts::CommitsVm;
use hugit_refstore::EventLog;

/// Build the commits view-model from a verified event log.
pub fn build_commits(_log: &EventLog, _repo: &str) -> CommitsVm {
    todo!("Wave-1 commits handler — fleet fills per master-plan §5 source map")
}
