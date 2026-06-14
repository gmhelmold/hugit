//! `GET /v1/repos/{repo}/chrome` → [`RepoChromeVm`].
//!
//! The `log` is ALREADY chain-verified by the caller — do NOT re-load or
//! re-verify. No free-text from the log is echoed here, so `crate::fmt::scrub`
//! is not needed: string fields are either derived from the caller-supplied
//! `repo` slug (structural) or honest-default empty (no engine seam).

use hugit_http_contracts::RepoChromeVm;
use hugit_refstore::EventLog;
use hugit_refstore::intent::intents_from_log;

/// Build the repo-chrome view-model from a verified event log.
pub fn build_repo_chrome(log: &EventLog, repo: &str) -> RepoChromeVm {
    // REAL: number of landed intents in the log.
    let intents_count = intents_from_log(log)
        .map(|il| il.intents().len() as u32)
        .unwrap_or(0);

    RepoChromeVm {
        intents_count,                       // REAL
        clone_cmd: format!("hugit clone {repo}"), // DERIVED (structural)
        issues_count: 0,                     // HONEST-DEFAULT — no issues seam (P2)
        attention_count: 0,                  // HONEST-DEFAULT — no attention seam
        stars: String::new(),                // HONEST-DEFAULT — GitHub-mirror (P2)
        forks: String::new(),                // HONEST-DEFAULT — GitHub-mirror (P2)
        visibility: String::new(),           // HONEST-DEFAULT — tenant provisioning (P2)
        description_html_free: String::new(), // HONEST-DEFAULT — GitHub-mirror (P2)
    }
}
