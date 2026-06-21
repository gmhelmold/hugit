//! `GET /v1/github-app` → `GithubAppVm` and its module-local nested types.
//! Transcribed BYTE-FOR-FIELD from `githugr-vm/src/provider.rs`; `GithubAppVm`
//! carries an f64 via `strip` → `PartialEq` only, never `Eq`.

use serde::{Deserialize, Serialize};

use crate::common::GithubAppStripVm;

/// One installed repo with its MEASURED hit-rate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRepoRowVm {
    pub org: String,
    pub name: String,
    /// Measured hit-rate percentage (98 → "98%").
    pub hit_pct: u8,
    pub partial: bool,
    pub saved: String,
    pub agent_prs: u32,
    pub human_prs: u32,
    pub queue_note: String,
    pub queue_class: String,
    pub detail: String,
}

/// One row of the live union queue (over GitHub PRs).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppQueueRowVm {
    pub pos: u8,
    pub pr: u32,
    pub title: String,
    pub state: String,
    pub active: bool,
}

/// The bot comment as posted on GitHub — the mini-frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppCommentVm {
    pub green: bool,
    pub url: String,
    pub verdict_line: String,
    pub verdict_note: String,
    pub body: String,
    pub code_lines: Vec<String>,
    pub link_label: String,
}

/// One row of "o que está por baixo" — the family as facts with state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppStackRowVm {
    pub name: String,
    pub state: String,
    pub active: bool,
    pub detail: String,
    pub action: Option<String>,
    #[serde(default)]
    pub detail_segments: Vec<(String, bool)>,
}

/// GitHub App dashboard view-model. `PartialEq` only — `strip` holds an f64.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GithubAppVm {
    pub org: String,
    pub identity_note: String,
    #[serde(default)]
    pub identity_arrival: Option<String>,
    /// Reused strip atom (saved $ + CI minutes this month).
    pub strip: GithubAppStripVm,
    pub proved_checks: u32,
    pub executed_checks: u32,
    pub hero_note: String,
    pub repos: Vec<AppRepoRowVm>,
    pub union_repo: String,
    pub union_note: String,
    pub queue: Vec<AppQueueRowVm>,
    pub queue_foot: String,
    pub culprit_note: String,
    pub comment_green: AppCommentVm,
    pub comment_red: AppCommentVm,
    pub stack: Vec<AppStackRowVm>,
    pub stack_note: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `GithubAppVm` (the f64-bearing top-level VM) round-trips losslessly.
    #[test]
    fn github_app_vm_round_trips() {
        let vm = GithubAppVm {
            org: "humangr".into(),
            identity_note: "instalado na org humangr · 4 repos · desde 2 jun".into(),
            identity_arrival: Some("· chega-se pelo dashboard, pela org ou ⌘K".into()),
            strip: GithubAppStripVm {
                saved_usd: 612.40,
                saved_ci_minutes: 1480,
            },
            proved_checks: 11480,
            executed_checks: 218,
            hero_note: "um check verde nunca re-roda".into(),
            repos: vec![AppRepoRowVm {
                org: "humangr".into(),
                name: "corelink-server".into(),
                hit_pct: 98,
                partial: false,
                saved: "$612".into(),
                agent_prs: 14,
                human_prs: 3,
                queue_note: "3 em união — pousando".into(),
                queue_class: "g".into(),
                detail: "Rust · hermético · checks: fmt · clippy · test · audit".into(),
            }],
            union_repo: "humangr / corelink-server".into(),
            union_note: "união verde · 1 execução · $0.04".into(),
            queue: vec![AppQueueRowVm {
                pos: 1,
                pr: 205,
                title: "fix: sessão expira cedo no refresh".into(),
                state: "pousando…".into(),
                active: true,
            }],
            queue_foot: "provado junto, pousa em ordem — 1 execução pra 3 PRs".into(),
            culprit_note: "acme / storefront · união vermelha → bisect → culpado #412".into(),
            comment_green: AppCommentVm {
                green: true,
                url: "github.com/acme/myrepo/pull/205".into(),
                verdict_line: "hugit / memoized-ci — verde".into(),
                verdict_note: "provado por $0.02 · 14s".into(),
                body: "9 de 11 checks provados pela cache.".into(),
                code_lines: vec!["✓ fmt [cache] $0.00".into(), "✓ test [exec] $0.02".into()],
                link_label: "ver a substância no githugr →".into(),
            },
            comment_red: AppCommentVm {
                green: false,
                url: "github.com/acme/myrepo/pull/412".into(),
                verdict_line: "hugit / memoized-ci — vermelho".into(),
                verdict_note: "culpado isolado pelo bisect · $0.01".into(),
                body: "1 de 9 falhou — bisect isolou o culpado.".into(),
                code_lines: vec!["✗ test -p edge [exec] $0.01 1/9 falhou".into()],
                link_label: "ver o culpado no githugr →".into(),
            },
            stack: vec![AppStackRowVm {
                name: "CoreLink · Cache".into(),
                state: "em uso".into(),
                active: true,
                detail: "11.480 provas este mês · 412 GB dedup · $9.80 poupado".into(),
                action: None,
                detail_segments: vec![
                    ("".into(), false),
                    ("11.480".into(), true),
                    (" provas este mês".into(), false),
                ],
            }],
            stack_note: "uma conta, uma fatura previsível.".into(),
        };

        let json = serde_json::to_string(&vm).expect("GithubAppVm serializes");
        let reparsed: GithubAppVm = serde_json::from_str(&json).expect("GithubAppVm deserializes");
        assert_eq!(vm, reparsed, "GithubAppVm round-trip is lossless");
        assert_eq!(reparsed.strip.saved_usd, 612.40);
        assert!(!reparsed.comment_red.green);
    }
}
