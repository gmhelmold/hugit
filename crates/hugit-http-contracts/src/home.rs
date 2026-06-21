//! `GET /v1/repos/{repo}/home` → `RepoHomeVm` and its home-specific nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical
//! companion web frontend's view-model definitions. All-`Eq` (no floats).

use serde::{Deserialize, Serialize};

/// One row of the repo file tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeRowVm {
    pub name: String,
    pub is_dir: bool,
    /// Last-touch intent id (the `.ix` link), when fleet-authored.
    pub intent_id: Option<String>,
    pub message: String,
    pub age: String,
}

/// The repo "About" sidebar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AboutVm {
    pub description: String,
    pub topics: Vec<String>,
    pub release: Option<String>,
    pub contributors: Vec<String>,
    #[serde(default)]
    pub stars: String,
    #[serde(default)]
    pub forks: String,
    #[serde(default)]
    pub updated_ago: String,
    #[serde(default)]
    pub license: String,
    #[serde(default)]
    pub releases_count: u32,
    /// Language-bar legend: `(name, percent)` pairs, e.g. `[("Rust","94%")]`.
    #[serde(default)]
    pub languages: Vec<(String, String)>,
    #[serde(default)]
    pub contributors_suffix: String,
}

/// The `.fcommit` last-commit header row above the file tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct LastCommitVm {
    pub author: String,
    pub intent_id: String,
    pub message: String,
    pub short_sha: String,
    pub age: String,
}

/// The synergy panel (the hugit layer at a glance on the code home).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SynergyVm {
    /// `(label, value)` lines, e.g. `("espelho GitHub", "sincronizado · 2 min")`.
    pub lines: Vec<(String, String)>,
}

/// `GET /v1/repos/{repo}/home` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoHomeVm {
    pub repo: String,
    pub branch: String,
    pub branch_count: usize,
    pub files: Vec<TreeRowVm>,
    /// Provider-rendered README body.
    pub readme_html: String,
    pub about: AboutVm,
    pub synergy: SynergyVm,
    #[serde(default)]
    pub tag_count: u32,
    #[serde(default)]
    pub commit_count: String,
    #[serde(default)]
    pub last_commit: LastCommitVm,
    #[serde(default)]
    pub branches: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The canonical Appendix-A `home` JSON round-trips losslessly through
    /// `RepoHomeVm` — proving our transcribed type accepts and re-emits EXACTLY
    /// the shape the frozen `githugr-live` client speaks.
    #[test]
    fn home_vm_round_trips_canonical_json() {
        let canonical = r#"{
          "repo": "hugit", "branch": "main", "branch_count": 12,
          "files": [{ "name": "src", "is_dir": true, "intent_id": null, "message": "refactor: split modules", "age": "há 2h" }],
          "readme_html": "<h1>hugit</h1><p>…</p>",
          "about": { "description": "…", "topics": ["rust"], "release": "v1.0.0", "contributors": ["gustavo"], "stars": "1.2k", "forks": "84", "updated_ago": "há 8 min", "license": "Apache-2.0", "releases_count": 8, "languages": [["Rust", "94%"]], "contributors_suffix": "· 6 + 4 agentes" },
          "synergy": { "lines": [["espelho GitHub", "sincronizado · 2 min"]] },
          "tag_count": 7, "commit_count": "4.832",
          "last_commit": { "author": "opus-4.8", "intent_id": "a31", "message": "fix: sessão expira cedo", "short_sha": "a31f9c", "age": "há 8 min" },
          "branches": ["feat/sessions", "fix/refresh-expiry"]
        }"#;
        let vm: RepoHomeVm =
            serde_json::from_str(canonical).expect("home JSON parses into RepoHomeVm");
        assert_eq!(vm.repo, "hugit");
        assert_eq!(vm.branch_count, 12);
        assert_eq!(vm.files[0].intent_id, None);
        assert_eq!(vm.about.release.as_deref(), Some("v1.0.0"));
        assert_eq!(
            vm.about.languages[0],
            ("Rust".to_string(), "94%".to_string())
        );
        assert_eq!(vm.last_commit.short_sha, "a31f9c");
        let reparsed: RepoHomeVm =
            serde_json::from_str(&serde_json::to_string(&vm).unwrap()).unwrap();
        assert_eq!(vm, reparsed, "RepoHomeVm round-trip is lossless");
    }
}
