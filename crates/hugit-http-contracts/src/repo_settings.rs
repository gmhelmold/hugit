//! `GET /v1/repos/{repo}/settings` → `RepoSettingsVm` and its module-local types.
//! All fields are `String`/`bool`/`Vec`/`Option`/tuple — no `f64` → full `Eq`.
//! Transcribed BYTE-FOR-FIELD from the canonical
//! `../githugr/crates/githugr-vm/src/provider.rs`; derives copied verbatim.

use serde::{Deserialize, Serialize};

/// One plain-language requirement of the policy summary card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyReqVm {
    pub text: String,
    /// True renders the ⊘ permanent-rule glyph instead of the ✓.
    pub locked: bool,
}

/// One adjustable policy rule row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyRuleVm {
    pub label: String,
    pub badge: Option<String>,
    pub description: String,
    pub param: Option<String>,
    pub effect: String,
    pub enabled: bool,
    pub state_label: String,
    pub locked: bool,
}

/// One titled group of rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuleGroupVm {
    pub title: String,
    pub rules: Vec<PolicyRuleVm>,
}

/// One collaborator row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollaboratorVm {
    pub name: String,
    pub role: String,
}

/// The Sync-GitHub section — status only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncGithubVm {
    pub mirror_repo: String,
    pub connection_note: String,
    pub direction: String,
    pub direction_note: String,
    /// Status kv rows: (key, value, gray note) — note "" when absent.
    pub rows: Vec<(String, String, String)>,
    pub how_note: String,
}

/// One secret row — METADATA ONLY (broker holds the value).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretRowVm {
    pub name: String,
    pub meta: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoSettingsVm {
    pub repo: String,
    /// Sub-nav entries: (label, external ↗).
    pub nav: Vec<(String, bool)>,
    pub policy_intro: String,
    pub policy_title: String,
    pub policy_requirements: Vec<PolicyReqVm>,
    pub policy_foot: String,
    pub rule_groups: Vec<RuleGroupVm>,
    pub save_hint: String,
    pub collaborators: Vec<CollaboratorVm>,
    pub sync: SyncGithubVm,
    pub secrets_note: String,
    pub secrets: Vec<SecretRowVm>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `RepoSettingsVm` (fully-populated) round-trips losslessly.
    #[test]
    fn repo_settings_vm_round_trips() {
        let vm = RepoSettingsVm {
            repo: "humangr/corelink-server".into(),
            nav: vec![("Webhooks".into(), true), ("Integrações".into(), false)],
            policy_intro: "Estas regras decidem quando um intent pode pousar na main.".into(),
            policy_title: "Hoje, para pousar na main, um intent precisa de:".into(),
            policy_requirements: vec![
                PolicyReqVm {
                    text: "2 aprovações de revisores distintos".into(),
                    locked: false,
                },
                PolicyReqVm {
                    text: "checks verdes (fmt · clippy · test · deny)".into(),
                    locked: true,
                },
            ],
            policy_foot: "É só isso. Falhou alguma? O intent não pousa.".into(),
            rule_groups: vec![RuleGroupVm {
                title: "Gates de qualidade".into(),
                rules: vec![PolicyRuleVm {
                    label: "Exigir aprovações".into(),
                    badge: None,
                    description: "Nenhum intent pousa sem N verdicts APPROVE.".into(),
                    param: Some("mínimo 2 aprovações".into()),
                    effect: "→ intent bloqueado até atingir o limiar".into(),
                    enabled: true,
                    state_label: "ativa".into(),
                    locked: false,
                }],
            }],
            save_hint: "Entra em vigor no próximo intent aberto.".into(),
            collaborators: vec![
                CollaboratorVm { name: "gustavo".into(), role: "Maintainer".into() },
                CollaboratorVm { name: "ana".into(), role: "Arquiteto".into() },
            ],
            sync: SyncGithubVm {
                mirror_repo: "humangr/corelink-server".into(),
                connection_note: "Conectado · GitHub App #488201".into(),
                direction: "Bidirecional seamless".into(),
                direction_note: "githugr → GitHub · GitHub → githugr".into(),
                rows: vec![
                    ("último push".into(), "há 2 min".into(), "".into()),
                    ("último pull".into(), "há 5 min".into(), "sem conflito".into()),
                ],
                how_note: "O espelho corre em background; conflitos param na fila.".into(),
            },
            secrets_note: "Apenas metadados. Para rotacionar, use o botão.".into(),
            secrets: vec![SecretRowVm {
                name: "CLOUDFLARE_API_TOKEN".into(),
                meta: "escopo deploy · rotacionado 05 jun".into(),
            }],
        };

        let json = serde_json::to_string(&vm).expect("RepoSettingsVm serializes");
        let reparsed: RepoSettingsVm =
            serde_json::from_str(&json).expect("RepoSettingsVm deserializes");
        assert_eq!(vm, reparsed, "RepoSettingsVm round-trip is lossless");
    }
}
