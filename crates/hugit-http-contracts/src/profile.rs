//! `GET /v1/users/{user}` → `ProfileVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All Eq (no f64).

use serde::{Deserialize, Serialize};

/// One pinned-repo card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedRepoVm {
    pub name: String,
    pub visibility: String,
    pub description: String,
    pub lang: String,
    pub lang_class: String,
    pub status: String,
    pub prs_note: String,
}

/// One monthly activity bar.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivityMonthVm {
    pub label: String,
    pub count: u32,
    pub height_pct: u8,
    pub current: bool,
    #[serde(default)]
    pub year: String,
}

/// One Destaques row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HighlightVm {
    pub icon: String,
    pub text: String,
    pub href: Option<String>,
    pub age: String,
    #[serde(default)]
    pub verb: String,
    #[serde(default)]
    pub entity_id: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub mid: String,
}

/// Public profile view-model: pinned repos + monthly activity bars + highlights.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileVm {
    pub user: String,
    pub name: String,
    pub bio: String,
    pub company: String,
    pub location: String,
    pub email: String,
    pub orgs: Vec<String>,
    pub pinned: Vec<PinnedRepoVm>,
    pub contributions_total: u32,
    pub activity_legend: String,
    pub months: Vec<ActivityMonthVm>,
    pub what_counts: Vec<String>,
    pub activity_note: String,
    pub highlights: Vec<HighlightVm>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_vm_round_trips() {
        let vm = ProfileVm {
            user: "gustavo".into(),
            name: "Test User".into(),
            bio: "fundador · HuGR".into(),
            company: "HuGR".into(),
            location: "São Paulo".into(),
            email: "owner@example.com".into(),
            orgs: vec!["humangr".into()],
            pinned: vec![PinnedRepoVm {
                name: "corelink-server".into(),
                visibility: "privado".into(),
                description: "o CAS de produção — Workers/R2/D1/DO".into(),
                lang: "Rust".into(),
                lang_class: "rust".into(),
                status: "main verde".into(),
                prs_note: "3 PRs abertos".into(),
            }],
            contributions_total: 888,
            activity_legend: "jun (até dia 10)".into(),
            months: vec![ActivityMonthVm {
                label: "jul".into(),
                count: 21,
                height_pct: 72,
                current: false,
                year: "2025".into(),
            }],
            what_counts: vec!["834 intents orquestrados".into(), "71 PRs".into()],
            activity_note: "jun é o sprint hugit — fundado dia 5".into(),
            highlights: vec![HighlightVm {
                icon: "◈".into(),
                text: "assinou a campanha auth-hardening · 3 PRs · 5 intents".into(),
                href: Some("/r/corelink-server/campaign/auth-hardening".into()),
                age: "há 38 min".into(),
                verb: "assinou".into(),
                entity_id: "auth-hardening".into(),
                detail: " · 3 PRs · 5 intents".into(),
                mid: " a campanha".into(),
            }],
        };
        let json = serde_json::to_string(&vm).expect("ProfileVm serializes");
        let reparsed: ProfileVm = serde_json::from_str(&json).expect("ProfileVm deserializes");
        assert_eq!(vm, reparsed, "ProfileVm round-trip is lossless");
    }
}
