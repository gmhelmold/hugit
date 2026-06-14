//! `GET /v1/repos/{repo}/branches` → `BranchesVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All-`Eq` (no floats).

use serde::{Deserialize, Serialize};

/// The PR cell of a branch row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchPrVm {
    pub number: u32,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchRowVm {
    pub name: String,
    pub is_default: bool,
    pub protected: bool,
    pub head_sha: String,
    pub attr: String,
    pub intent_id: Option<String>,
    pub model: Option<String>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub checks_ok: Option<bool>,
    pub pr: Option<BranchPrVm>,
}

/// `GET /v1/repos/{repo}/branches` response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BranchesVm {
    pub repo: String,
    pub branch_count: usize,
    pub tag_count: usize,
    pub active_count: usize,
    pub yours_count: usize,
    pub default_branch: BranchRowVm,
    pub branches: Vec<BranchRowVm>,
    pub inactive_count: usize,
    pub inactive_note: String,
    #[serde(default)]
    pub more_count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn branches_vm_round_trips() {
        let default_row = BranchRowVm {
            name: "main".to_string(),
            is_default: true,
            protected: true,
            head_sha: "a31f9c".to_string(),
            attr: "há 8 min".to_string(),
            intent_id: Some("a31".to_string()),
            model: Some("opus-4.8".to_string()),
            ahead: None,
            behind: None,
            checks_ok: Some(true),
            pr: None,
        };
        let feat_row = BranchRowVm {
            name: "feat/sessions".to_string(),
            is_default: false,
            protected: false,
            head_sha: "c12d70".to_string(),
            attr: "você empurrou há 2 h".to_string(),
            intent_id: None,
            model: None,
            ahead: Some(3),
            behind: Some(1),
            checks_ok: Some(false),
            pr: Some(BranchPrVm {
                number: 129,
                state: "aberto".to_string(),
            }),
        };
        let vm = BranchesVm {
            repo: "hugit".to_string(),
            branch_count: 8,
            tag_count: 3,
            active_count: 5,
            yours_count: 2,
            default_branch: default_row,
            branches: vec![feat_row],
            inactive_count: 33,
            inactive_note: "33 branches sem atividade.".to_string(),
            more_count: 37,
        };
        let json = serde_json::to_string(&vm).expect("BranchesVm serializes");
        let reparsed: BranchesVm = serde_json::from_str(&json).expect("BranchesVm round-trips");
        assert_eq!(vm, reparsed, "BranchesVm round-trip is lossless");
        let without_more = r#"{
          "repo":"r","branch_count":1,"tag_count":0,"active_count":1,"yours_count":0,
          "default_branch":{"name":"main","is_default":true,"protected":false,
            "head_sha":"000000","attr":"","intent_id":null,"model":null,
            "ahead":null,"behind":null,"checks_ok":null,"pr":null},
          "branches":[],"inactive_count":0,"inactive_note":""
        }"#;
        let minimal: BranchesVm =
            serde_json::from_str(without_more).expect("absent more_count → 0");
        assert_eq!(minimal.more_count, 0);
    }
}
