//! `GET /v1/me/account` → `AccountVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical source. All Eq (no f64).

use serde::{Deserialize, Serialize};

/// One PAT row — METADATA ONLY (token value never reaches the browser — ADR-0002).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PatVm {
    pub name: String,
    pub meta: String,
}

/// Account settings page — ALL render data, no Option.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountVm {
    pub name: String,
    pub user: String,
    pub subtitle: String,
    pub org: String,
    pub bio: String,
    pub company: String,
    pub user_lock_note: String,
    pub email: String,
    pub email_verified: bool,
    pub email_help: String,
    pub language: String,
    pub languages: Vec<String>,
    pub pat_note: String,
    pub pats: Vec<PatVm>,
    pub notifications_note: String,
    pub repos_note: String,
    pub danger_title: String,
    pub danger_note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_vm_round_trips() {
        let vm = AccountVm {
            name: "Gustavo Schneiter".into(),
            user: "gustavo".into(),
            subtitle: "Sua conta HuGR — uma conta pra família toda".into(),
            org: "humangr".into(),
            bio: "fundador · HuGR".into(),
            company: "HuGR".into(),
            user_lock_note: "fixo — identidade HuGR".into(),
            email: "owner@example.com".into(),
            email_verified: true,
            email_help: "É o e-mail da conta HuGR.".into(),
            language: "Português (Brasil)".into(),
            languages: vec!["Português (Brasil)".into(), "English".into()],
            pat_note: "PATs são criados via CLI e nunca aparecem no navegador.".into(),
            pats: vec![PatVm {
                name: "ci-runner-hetzner".into(),
                meta: "escopo org · criado 05 jun · último uso há 2 h".into(),
            }],
            notifications_note: "githugr não tem notificações — tem a Atenção ◎".into(),
            repos_note: "4 repositórios na conta · gerencie no dashboard →".into(),
            danger_title: "Excluir conta".into(),
            danger_note: "Remove permanentemente sua conta e dados.".into(),
        };
        let json = serde_json::to_string(&vm).expect("serialize");
        let reparsed: AccountVm = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(vm, reparsed, "AccountVm round-trip is lossless");
    }
}
