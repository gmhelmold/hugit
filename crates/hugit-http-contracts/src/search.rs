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

/// The structured lifecycle state of a [`SearchIntentVm`], beside the localized
/// `status` prose — githugr styles from this discriminant. The variants match the
/// intent states the search projection can know: an intent that landed but is not
/// yet proven/rejected is `Landed`; the proven/rejected terminal states; `Unknown`
/// (`#[default]`) when no status is on the record (the current `intent.landed`
/// projection carries no status, so this is the honest live value today).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum IntentState {
    #[default]
    Unknown,
    /// In flight / proposed — not yet landed.
    InFlight,
    /// Landed onto a ref (the `intent.landed` projection's value).
    Landed,
    /// Verdict-rejected.
    Rejected,
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
    /// Structured lifecycle discriminant beside the localized `status` prose
    /// (githugr styles from this). Derived from the same source as `status`.
    /// Additive + forward-compat: an old payload without it defaults to
    /// [`IntentState::Unknown`]. The prose `status` stays.
    #[serde(default)]
    pub state: IntentState,
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
                state: IntentState::Landed,
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
        assert_eq!(reparsed.intents[0].state, IntentState::Landed);
    }

    /// `SearchIntentVm.state` (the new structured discriminant beside `status`)
    /// round-trips AND forward-compat-defaults to `Unknown` when absent.
    #[test]
    fn search_intent_vm_state_round_trips_and_defaults() {
        let it = SearchIntentVm {
            id: "i-9".to_string(),
            charter: "c".to_string(),
            model: String::new(),
            pr: 0,
            status: String::new(),
            age: "há 1 dia".to_string(),
            state: IntentState::Rejected,
        };
        let reparsed: SearchIntentVm =
            serde_json::from_str(&serde_json::to_string(&it).unwrap()).unwrap();
        assert_eq!(reparsed.state, IntentState::Rejected);

        let legacy =
            r#"{ "id": "i-1", "charter": "c", "model": "", "pr": 0, "status": "", "age": "" }"#;
        let parsed: SearchIntentVm = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            parsed.state,
            IntentState::Unknown,
            "absent state defaults to Unknown (forward-compat)"
        );
    }
}
