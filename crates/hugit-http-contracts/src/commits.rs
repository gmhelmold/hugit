//! `GET /v1/repos/{repo}/commits` → `CommitsVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All-`Eq` (no floats).

use serde::{Deserialize, Serialize};

/// One row of the raw git log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitRowVm {
    pub message: String,
    /// `"opus-4.8"` | `"sonnet-4.6"` | a human handle (`"ana"`).
    pub author: String,
    /// Avatar class: `"opus"` | `"sonnet"` | a human class (`"ana"`).
    pub avatar_class: String,
    pub age: String,
    /// The intent chip (`"← a31"`); `None` = external (manual push) commit.
    pub intent_id: Option<String>,
    /// Short sha, e.g. `"a31f9c"`.
    pub sha: String,
    pub checks_ok: bool,
}

/// One day group (`"Commits em 9 de jun de 2026"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitDayVm {
    pub label: String,
    pub commits: Vec<CommitRowVm>,
}

/// `GET /v1/repos/{repo}/commits` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitsVm {
    pub repo: String,
    /// The selected branch (selector label).
    pub branch: String,
    /// Other plain branches in the selector.
    pub other_branches: Vec<String>,
    /// `"geradas ← intent"` branches in the selector (e.g. `"intent/a31"`).
    pub generated_branches: Vec<String>,
    pub days: Vec<CommitDayVm>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical Appendix-A `commits` JSON round-trips losslessly.
    #[test]
    fn commits_vm_round_trips_canonical_json() {
        let canonical = r#"{
          "repo": "hugit", "branch": "main",
          "other_branches": ["feat/sessions"], "generated_branches": ["intent/a31"],
          "days": [{
            "label": "Commits em 9 de jun de 2026",
            "commits": [{ "message": "fix: sessão expira cedo no refresh", "author": "opus-4.8", "avatar_class": "opus", "age": "há 38 min", "intent_id": "a31", "sha": "a31f9c", "checks_ok": true }]
          }]
        }"#;
        let vm: CommitsVm =
            serde_json::from_str(canonical).expect("commits JSON parses into CommitsVm");
        assert_eq!(vm.repo, "hugit");
        assert_eq!(vm.generated_branches, vec!["intent/a31".to_string()]);
        assert_eq!(vm.days[0].commits[0].sha, "a31f9c");
        assert!(vm.days[0].commits[0].checks_ok);
        assert_eq!(vm.days[0].commits[0].intent_id.as_deref(), Some("a31"));
        let reparsed: CommitsVm =
            serde_json::from_str(&serde_json::to_string(&vm).unwrap()).unwrap();
        assert_eq!(vm, reparsed, "CommitsVm round-trip is lossless");
    }
}
