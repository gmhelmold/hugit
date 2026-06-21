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
    /// MACHINE value: `"public" | "private"` (NOT a localized display string).
    /// The window maps it to a display label at render (githugr TL decision
    /// 2026-06-15 — locale lives in the window). Empty only on a pre-meta repo.
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
            visibility: "private".into(), // MACHINE value on the wire (display is window-side)
            description_html_free: "Content-addressed cache + compute da CoreLink.".into(),
            clone_cmd: "hugit clone acme/myrepo".into(),
        };
        let json = serde_json::to_string(&vm).unwrap();
        let reparsed: RepoChromeVm = serde_json::from_str(&json).unwrap();
        assert_eq!(vm, reparsed, "RepoChromeVm round-trip is lossless");
    }
}
