//! `GET /v1/repos/{repo}/search?q=` → `SearchVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All Eq (no f64).

use serde::{Deserialize, Serialize};

/// One code hit: path + raw mono snippet lines (no highlighter this wave).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchCodeVm {
    pub path: String,
    /// (1-based line number, raw text) pairs.
    pub lines: Vec<(u32, String)>,
    /// Per-line syntax-colored tokens — `(line, [(token_text, color_class)])`.
    #[serde(default)]
    pub lines_tokens: Vec<(u32, Vec<(String, String)>)>,
}

/// One PR or issue hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRefVm {
    pub number: u32,
    pub title: String,
    pub open: bool,
    pub meta: String,
    pub extra: String,
}

/// One intent hit — the search finds CHARTERS, not just code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchIntentVm {
    pub id: String,
    pub charter: String,
    pub model: String,
    pub pr: u32,
    pub status: String,
    pub age: String,
}

/// One commit hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchCommitVm {
    pub sha: String,
    pub message: String,
    pub who: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchVm {
    pub repo: String,
    pub q: String,
    pub index_note: String,
    pub code: Vec<SearchCodeVm>,
    pub prs: Vec<SearchRefVm>,
    pub issues: Vec<SearchRefVm>,
    pub intents_note: String,
    pub intents: Vec<SearchIntentVm>,
    pub commits: Vec<SearchCommitVm>,
    pub people_count: usize,
    /// The TRUE total of code hits across all pages.
    #[serde(default)]
    pub code_total: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_vm_round_trips() {
        let vm = SearchVm {
            repo: "hugit".to_string(),
            q: "skew".to_string(),
            index_note: "0,04s · índice do CAS".to_string(),
            code: vec![SearchCodeVm {
                path: "crates/auth/src/refresh.rs".to_string(),
                lines: vec![(42, "let skew = Duration::seconds(2);".to_string())],
                lines_tokens: vec![(42, vec![("let".to_string(), "c-auth".to_string())])],
            }],
            prs: vec![SearchRefVm {
                number: 128,
                title: "fix: sessão expira cedo".to_string(),
                open: true,
                meta: "aberto · @ana · há 3 dias".to_string(),
                extra: "intent a31".to_string(),
            }],
            issues: vec![SearchRefVm {
                number: 412,
                title: "clock skew no refresh".to_string(),
                open: false,
                meta: "fechado · @gustavo · há 6 dias".to_string(),
                extra: "auth · bug".to_string(),
            }],
            intents_note: "intents carregam o porquê.".to_string(),
            intents: vec![SearchIntentVm {
                id: "a31".to_string(),
                charter: "fix: refresh reusava o iat antigo".to_string(),
                model: "opus-4.8".to_string(),
                pr: 128,
                status: "pousou".to_string(),
                age: "há 3 dias".to_string(),
            }],
            commits: vec![SearchCommitVm {
                sha: "a31f9c".to_string(),
                message: "fix: sessão expira cedo".to_string(),
                who: "ana · há 3 dias".to_string(),
            }],
            people_count: 2,
            code_total: 6,
        };
        let json = serde_json::to_string(&vm).unwrap();
        let reparsed: SearchVm = serde_json::from_str(&json).unwrap();
        assert_eq!(vm, reparsed, "SearchVm round-trip is lossless");
        assert_eq!(reparsed.code_total, 6);
    }
}
