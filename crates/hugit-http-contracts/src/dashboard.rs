//! `GET /v1/me/dashboard` → `DashboardVm` and its nested types.
//! Shared atoms (`DashboardRepoVm`, `GithubAppStripVm`) come from [`crate::common`].
//! `DashboardVm` carries `GithubAppStripVm` which holds f64 → `PartialEq`-only.

use serde::{Deserialize, Serialize};

use crate::common::{DashboardRepoVm, GithubAppStripVm};

/// One row in the "Precisa de você" inbox.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxRowVm {
    pub signal_class: String,
    pub context: String,
    pub title: String,
    pub description: String,
    pub action_label: String,
}

/// `PartialEq` only — embeds `GithubAppStripVm` (f64).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DashboardVm {
    pub repos: Vec<DashboardRepoVm>,
    pub github_app: GithubAppStripVm,
    pub inbox_pending_total: usize,
    pub inbox: Vec<InboxRowVm>,
    pub attention_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dashboard_vm_round_trips() {
        let vm = DashboardVm {
            repos: vec![DashboardRepoVm {
                org: "humangr".into(),
                name: "corelink-server".into(),
                main_green: true,
                status: "main verde".into(),
                stack: "Rust · 14 crates".into(),
                visibility: "privado".into(),
                open_prs: 3,
                last_activity: "há 12 min".into(),
            }],
            github_app: GithubAppStripVm {
                saved_usd: 1412.0,
                saved_ci_minutes: 240,
            },
            inbox_pending_total: 5,
            inbox: vec![InboxRowVm {
                signal_class: "green".into(),
                context: "corelink-server · #128".into(),
                title: "PR pronto pra land".into(),
                description: "painel adversarial APPROVE · checks verdes".into(),
                action_label: "Aprovar e land →".into(),
            }],
            attention_count: 2,
        };
        let json = serde_json::to_string(&vm).expect("DashboardVm serializes");
        let reparsed: DashboardVm = serde_json::from_str(&json).expect("DashboardVm deserializes");
        assert_eq!(vm, reparsed, "DashboardVm round-trip is lossless");
    }
}
