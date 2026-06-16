//! `GET /v1/repos/{repo}/chrome` → [`RepoChromeVm`].
//!
//! The `log` is ALREADY chain-verified by the caller — do NOT re-load or
//! re-verify. No free-text from the log is echoed here, so `crate::fmt::scrub`
//! is not needed: string fields are either derived from the caller-supplied
//! `repo` slug (structural), projected from `repo.meta` (a closed machine value),
//! or honest-default empty (no engine seam).

use hugit_http_contracts::RepoChromeVm;
use hugit_refstore::EventLog;
use hugit_refstore::intent::intents_from_log;

/// Build the repo-chrome view-model from a verified event log.
pub fn build_repo_chrome(log: &EventLog, repo: &str) -> RepoChromeVm {
    // REAL: number of landed intents in the log.
    let intents_count = intents_from_log(log)
        .map(|il| il.intents().len() as u32)
        .unwrap_or(0);

    // REAL: the MACHINE visibility, from the SAME `repo.meta` projection the read
    // gate decides on (one source of truth). The window maps it to a localized
    // display string (githugr TL decision 2026-06-15) — the wire stays machine.
    let visibility = crate::authz::project_repo_meta(log)
        .visibility
        .as_machine_str()
        .to_string();

    RepoChromeVm {
        intents_count,                            // REAL
        visibility,                               // REAL — machine value from repo.meta
        clone_cmd: format!("hugit clone {repo}"), // DERIVED (structural)
        issues_count: 0,                          // HONEST-DEFAULT — no issues seam (P2)
        attention_count: 0,                       // HONEST-DEFAULT — no attention seam
        stars: String::new(),                     // HONEST-DEFAULT — GitHub-mirror (P2)
        forks: String::new(),                     // HONEST-DEFAULT — GitHub-mirror (P2)
        description_html_free: String::new(),     // HONEST-DEFAULT — GitHub-mirror (P2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authz::REPO_META_KIND;

    fn log_with_meta(visibility: &str) -> EventLog {
        let mut log = EventLog::new();
        let payload = serde_json::json!({"visibility": visibility, "owner_tenant": "org-a"});
        let body = hugit_refstore::canonical_json(&payload.to_string())
            .unwrap_or_else(|| payload.to_string());
        log.append_for_test(REPO_META_KIND, vec!["o".into()], body, 0);
        log
    }

    #[test]
    fn visibility_is_the_machine_value_from_repo_meta() {
        assert_eq!(
            build_repo_chrome(&log_with_meta("public"), "r").visibility,
            "public"
        );
        assert_eq!(
            build_repo_chrome(&log_with_meta("private"), "r").visibility,
            "private"
        );
    }

    #[test]
    fn visibility_defaults_private_on_a_pre_meta_repo() {
        // Fail-safe: a repo with no repo.meta projects PRIVATE (matches the gate).
        assert_eq!(
            build_repo_chrome(&EventLog::new(), "r").visibility,
            "private"
        );
    }
}
