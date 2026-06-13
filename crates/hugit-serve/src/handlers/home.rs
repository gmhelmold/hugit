//! `GET /v1/repos/{repo}/home` → [`RepoHomeVm`]. FROZEN signature; body filled by
//! the fleet per master-plan §5 (REAL: branches/last_commit/commit_count via
//! refstore `replay`/`project_machine`; STUB: file tree, readme, about-mirror).

use hugit_http_contracts::RepoHomeVm;
use hugit_refstore::EventLog;

/// Build the repo-home view-model from a verified event log.
pub fn build_home(_log: &EventLog, _repo: &str) -> RepoHomeVm {
    todo!("Wave-1 home handler — fleet fills per master-plan §5 source map")
}
