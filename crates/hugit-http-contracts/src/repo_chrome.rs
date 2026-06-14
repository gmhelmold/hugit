//! `GET /v1/repos/{repo}/chrome` → `RepoChromeVm`.
//! Transcribed BYTE-FOR-FIELD from the canonical source.

use serde::{Deserialize, Serialize};

/// The per-repo chrome view-model: repohdr counters + tabbar counts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoChromeVm {
    pub issues_count: u32,
    pub intents_count: u32,
    pub attention_count: u32,
    pub stars: String,
    pub forks: String,
    pub visibility: String,
    /// Repo description as plain text (HTML-free).
    pub description_html_free: String,
    pub clone_cmd: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_chrome_vm_round_trips() {
        let vm = RepoChromeVm {
            issues_count: 8,
            intents_count: 24,
            attention_count: 3,
            stars: "1.2k".into(),
            forks: "84".into(),
            visibility: "Privado".into(),
            description_html_free: "Content-addressed cache + compute da CoreLink.".into(),
            clone_cmd: "hugit clone humangr/corelink-server".into(),
        };
        let json = serde_json::to_string(&vm).unwrap();
        let reparsed: RepoChromeVm = serde_json::from_str(&json).unwrap();
        assert_eq!(vm, reparsed, "RepoChromeVm round-trip is lossless");
    }
}
