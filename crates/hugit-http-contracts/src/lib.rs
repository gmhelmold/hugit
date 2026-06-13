//! Frozen wire view-models for the `/v1` HTTP surface (githugr window ⇄ hugit
//! engine), backend-API-v1 §1 reads.
//!
//! Transcribed BYTE-FOR-FIELD from the canonical source
//! `../githugr/crates/githugr-vm/src/provider.rs` — field names, types, and
//! serde attributes (`#[serde(default)]`, `#[serde(rename)]`) match exactly, so
//! `hugit-serve` serializes precisely what the frozen `githugr-live` client
//! deserializes. Changing a field here is a WIRE BREAK; make it only in lock-step
//! with the window (same discipline as the byte-identical `conformance/` vectors).
//!
//! Scope: **Wave 1** = `home` + `commits` (the first two reads landed). The other
//! Wave-1 reads (`landing`, `checks`, `prs/{n}`) land as their verticals are
//! built; each is transcribed from the same canonical source and round-trip-tested
//! against the contract's Appendix-A JSON.
//!
//! Server-impl note: the `/v1` server (`hugit-serve`) is a minimal SYNCHRONOUS
//! HTTP server (not axum/tokio) — consistent with this sync, supply-chain-strict
//! workspace; the wire contract is server-impl-agnostic.

use serde::{Deserialize, Serialize};

// ===========================================================================
// GET /v1/repos/{repo}/home  →  RepoHomeVm   (repo-home.html)
// ===========================================================================

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

// ===========================================================================
// GET /v1/repos/{repo}/commits  →  CommitsVm   (commits.html)
// ===========================================================================

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

    /// The canonical Appendix-A `home` JSON (backend-API-v1 §1) round-trips
    /// LOSSLESSLY through `RepoHomeVm` — proving our transcribed type accepts and
    /// re-emits EXACTLY the shape the frozen `githugr-live` client speaks.
    #[test]
    fn home_vm_round_trips_canonical_json() {
        let canonical = r#"{
          "repo": "hugit", "branch": "main", "branch_count": 12,
          "files": [{ "name": "src", "is_dir": true, "intent_id": null, "message": "refactor: split modules", "age": "há 2h" }],
          "readme_html": "<h1>hugit</h1><p>…</p>",
          "about": { "description": "…", "topics": ["rust"], "release": "v1.0.0", "contributors": ["gustavo"], "stars": "1.2k", "forks": "84", "updated_ago": "há 8 min", "license": "proprietary", "releases_count": 8, "languages": [["Rust", "94%"]], "contributors_suffix": "· 6 + 4 agentes" },
          "synergy": { "lines": [["espelho GitHub", "sincronizado · 2 min"]] },
          "tag_count": 7, "commit_count": "4.832",
          "last_commit": { "author": "opus-4.8", "intent_id": "a31", "message": "fix: sessão expira cedo", "short_sha": "a31f9c", "age": "há 8 min" },
          "branches": ["feat/sessions", "fix/refresh-expiry"]
        }"#;
        let vm: RepoHomeVm =
            serde_json::from_str(canonical).expect("home JSON parses into RepoHomeVm");
        // Representative field assertions (names + types + nesting are correct).
        assert_eq!(vm.repo, "hugit");
        assert_eq!(vm.branch_count, 12);
        assert_eq!(vm.files[0].intent_id, None);
        assert_eq!(vm.about.release.as_deref(), Some("v1.0.0"));
        assert_eq!(
            vm.about.languages[0],
            ("Rust".to_string(), "94%".to_string())
        );
        assert_eq!(vm.last_commit.short_sha, "a31f9c");
        // Lossless round-trip: re-serialize → re-parse → structurally identical.
        let reparsed: RepoHomeVm =
            serde_json::from_str(&serde_json::to_string(&vm).unwrap()).unwrap();
        assert_eq!(vm, reparsed, "RepoHomeVm round-trip is lossless");
    }

    /// The canonical Appendix-A `commits` JSON round-trips losslessly through
    /// `CommitsVm`.
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
