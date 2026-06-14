//! `GET /v1/repos/{repo}/new-pr` → `NewPrVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All Eq (no f64).

use serde::{Deserialize, Serialize};

use crate::commits::CommitRowVm;
use crate::common::CampaignChipVm;

/// One step of the "O que acontece quando você despacha" strip.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPrStepVm {
    pub title: String,
    pub detail: String,
    pub timing: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NewPrVm {
    pub repo: String,
    pub base: String,
    pub head: String,
    pub mergeable_note: String,
    pub branch_title: String,
    pub branch_description: String,
    /// Commits of the compare — reused wave-2 row.
    pub commits: Vec<CommitRowVm>,
    pub commits_note: String,
    pub reviewers_note: String,
    pub labels_note: String,
    pub checks_note: String,
    /// Campaign selector options — reused wave-1 chips.
    pub campaigns: Vec<CampaignChipVm>,
    pub ask_placeholder: String,
    pub examples: Vec<String>,
    pub steps: Vec<NewPrStepVm>,
    /// Executor options: (name, gray note).
    pub executors: Vec<(String, String)>,
    pub policy_note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_pr_vm_round_trips() {
        let vm = NewPrVm {
            repo: "hugit".to_string(),
            base: "main".to_string(),
            head: "feat/sessions".to_string(),
            mergeable_note: "3 à frente · 0 atrás".to_string(),
            branch_title: "fix: sessão expira cedo no refresh".to_string(),
            branch_description: "Corrige o iat reusado.".to_string(),
            commits: vec![CommitRowVm {
                message: "fix: sessão expira cedo".to_string(),
                author: "ana".to_string(),
                avatar_class: "ana".to_string(),
                age: "há 38 min".to_string(),
                intent_id: None,
                sha: "a31f9c".to_string(),
                checks_ok: true,
            }],
            commits_note: "3 commits · push manual de @ana.".to_string(),
            reviewers_note: "nenhum — a política indica security se tocar auth/**".to_string(),
            labels_note: "nenhum".to_string(),
            checks_note: "os do repo, memoizados — só a novidade executa".to_string(),
            campaigns: vec![CampaignChipVm {
                id: "auth-hardening".to_string(),
                label: "auth-hardening".to_string(),
                color_class: "c-auth".to_string(),
                display_label: "auth".to_string(),
            }],
            ask_placeholder: "Escreva como você pediria pra uma pessoa do time.".to_string(),
            examples: vec!["fix: refresh reusava o iat antigo…".to_string()],
            steps: vec![NewPrStepVm {
                title: "O orquestrador planeja — e abre o PR".to_string(),
                detail: "decompõe o pedido em intents atômicos".to_string(),
                timing: "agora".to_string(),
            }],
            executors: vec![
                ("fleet · opus-4.8".to_string(), "padrão".to_string()),
                ("fleet · sonnet-4.6".to_string(), "econômico".to_string()),
            ],
            policy_note: "a do repo aplica — verificação não é opcional".to_string(),
        };
        let json = serde_json::to_string(&vm).unwrap();
        let reparsed: NewPrVm = serde_json::from_str(&json).unwrap();
        assert_eq!(vm, reparsed, "NewPrVm round-trip is lossless");
        assert_eq!(reparsed.executors[0].0, "fleet · opus-4.8");
    }
}
