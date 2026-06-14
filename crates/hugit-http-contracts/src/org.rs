//! `GET /v1/orgs/{name}` → `OrgVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All Eq (no f64).
//! `OrgVm.repos` reuses the shared `DashboardRepoVm` atom.

use serde::{Deserialize, Serialize};

use crate::common::DashboardRepoVm;

/// One row of the Pessoas panel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgPersonVm {
    pub name: String,
    pub handle: String,
    pub role: String,
    pub is_owner: bool,
}

/// Org page view-model: header + Repositórios/Pessoas tabs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgVm {
    pub name: String,
    pub handle: String,
    pub avatar_letter: String,
    pub bio: String,
    pub location: String,
    pub github_app_line: String,
    /// Repositórios tab — reused wave-2 row.
    pub repos: Vec<DashboardRepoVm>,
    pub people: Vec<OrgPersonVm>,
    pub agent_note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn org_vm_round_trips() {
        let vm = OrgVm {
            name: "humangr".into(),
            handle: "@humangr".into(),
            avatar_letter: "H".into(),
            bio: "A família HuGR — hugit · githugr · CoreLink.".into(),
            location: "São Paulo".into(),
            github_app_line: "GitHub App instalado · $1.412 poupados →".into(),
            repos: vec![DashboardRepoVm {
                org: "humangr".into(),
                name: "corelink-server".into(),
                main_green: true,
                status: "main verde".into(),
                stack: "Rust".into(),
                visibility: "privado".into(),
                open_prs: 3,
                last_activity: "há 8 min".into(),
            }],
            people: vec![OrgPersonVm {
                name: "Gustavo Schneiter".into(),
                handle: "@gustavo".into(),
                role: "Owner".into(),
                is_owner: true,
            }],
            agent_note: "Agentes não são membros — PR e campanha têm sempre dono humano.".into(),
        };
        let json = serde_json::to_string(&vm).expect("OrgVm serializes");
        let reparsed: OrgVm = serde_json::from_str(&json).expect("OrgVm deserializes");
        assert_eq!(vm, reparsed, "OrgVm round-trip is lossless");
    }
}
