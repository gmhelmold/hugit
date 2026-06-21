//! `GET /v1/repos/{repo}/intents/{id}` → `IntentDetailVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical
//! companion web frontend's view-model definitions; derives copied verbatim.

use serde::{Deserialize, Serialize};

use crate::common::{CampaignChipVm, DiffVm, VerdictVm};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorshipVm {
    pub model: String,
    pub principal_chain: Vec<String>,
    pub operator: String,
    /// The agent type shown in the Autoria `tipo` row. Empty = none.
    #[serde(default)]
    pub agent_type: String,
    /// The run id (Autoria `run` row). Empty = no run row.
    #[serde(default)]
    pub run_id: String,
    /// The parent run id shown after the run id. Empty = no parent.
    #[serde(default)]
    pub parent_run_id: String,
    /// Human-formatted wall time in the Autoria `viveu` row. Empty = none.
    #[serde(default)]
    pub wall_time_human: String,
}

/// Fixture-illustrative until WP-F2.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricsVm {
    pub tokens: u64,
    pub wall_ms: u64,
    pub tool_calls: u64,
    /// Intent envelope cost in micro-dollars (ADR-0001 1.2.0: `cost_usd_micros: u64`).
    #[serde(rename = "cost_usd_micros")]
    pub cost_usd_micros: u64,
    /// Active (non-idle) agent time in ms. 0 = row hidden.
    #[serde(default)]
    pub active_ms: u64,
    /// Model turns in the Métricas row. 0 = row hidden.
    #[serde(default)]
    pub model_turns: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotVm {
    pub tree: String,
    pub toolchain: String,
    pub workspace: String,
    /// The files the snapshot read. Empty = no read-files row.
    #[serde(default)]
    pub files_read: Vec<String>,
    /// The `prompt →` policy, e.g. "redactado". Empty = no prompt row.
    #[serde(default)]
    pub prompt_policy: String,
    /// The `ambiente →` toolchain string. Empty = no ambiente row.
    #[serde(default)]
    pub ambiente: String,
}

/// One altitude of the envelope trajectory: a row with a CAS-ref pill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeAltitudeVm {
    pub label: String,
    pub desc: String,
    pub cas_ref: String,
    /// Expanded body lines (turns). Empty = the inline resumo (no body).
    pub body: Vec<String>,
    /// Privacy note inside the "transcript completo" accordion. None = none.
    #[serde(default)]
    pub priv_note: Option<String>,
    /// Tool-invocation substrings wrapped in `<span class="tool">`. Empty = flat.
    #[serde(default)]
    pub body_tool_terms: Vec<String>,
    /// Tool-RESULT substrings wrapped in `<span class="res">`. Empty = flat.
    #[serde(default)]
    pub body_res_terms: Vec<String>,
}

/// `PartialEq` only — matches the canonical source derive (no `Eq`); copy verbatim.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentDetailVm {
    pub repo: String,
    pub id: String,
    pub title: String,
    pub status: String,
    pub pr_number: Option<u64>,
    pub summary: String,
    pub charter: String,
    /// Code-identifier substrings of `summary` wrapped in mono. Empty = flat.
    #[serde(default)]
    pub summary_mono_terms: Vec<String>,
    /// Code-identifier substrings of `charter` wrapped in mono. Empty = flat.
    #[serde(default)]
    pub charter_mono_terms: Vec<String>,
    pub acceptance: Vec<String>,
    /// Accordion bodies; `None` renders the honest "não capturado" state.
    pub task_transcript: Option<String>,
    pub full_transcript: Option<String>,
    pub journal: Option<String>,
    pub context_json: String,
    /// The envelope trajectory. Empty when not captured.
    pub trajectory: Vec<EnvelopeAltitudeVm>,
    pub context_cas: String,
    pub compact_context_ref: String,
    pub bundle_ref: String,
    pub compact_transcript_ref: String,
    /// The campaign this intent's PR belongs to. None = not bundled.
    pub campaign: Option<CampaignChipVm>,
    pub diff: DiffVm,
    pub authorship: AuthorshipVm,
    pub metrics: MetricsVm,
    pub snapshot: SnapshotVm,
    pub verdicts: Vec<VerdictVm>,
    /// The short commit SHA in the t2 line. Empty = no commit hash shown.
    #[serde(default)]
    pub commit_hash: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `IntentDetailVm` round-trips losslessly; the `cost_usd_micros` rename is
    /// verified on the wire.
    #[test]
    fn intent_detail_vm_round_trips() {
        let metrics = MetricsVm {
            tokens: 1_234_567,
            wall_ms: 874_000,
            tool_calls: 42,
            cost_usd_micros: 60_000,
            active_ms: 544_000,
            model_turns: 11,
        };
        let authorship = AuthorshipVm {
            model: "opus-4.8".to_string(),
            principal_chain: vec!["humangr/gustavo".to_string(), "orq-014".to_string()],
            operator: "gustavo".to_string(),
            agent_type: "implementer".to_string(),
            run_id: "r-9f2a".to_string(),
            parent_run_id: "orq-014".to_string(),
            wall_time_human: "14m37s".to_string(),
        };
        let snapshot = SnapshotVm {
            tree: "a31f9c".to_string(),
            toolchain: "1.96.0".to_string(),
            workspace: "corelink-server".to_string(),
            files_read: vec!["auth/token.rs".to_string(), "session.rs".to_string()],
            prompt_policy: "redactado".to_string(),
            ambiente: "rustc 1.96.0".to_string(),
        };
        let altitude = EnvelopeAltitudeVm {
            label: "resumo compactado".to_string(),
            desc: "o essencial da sessão, compactado — pra ler".to_string(),
            cas_ref: "cas:55c1… · 9 KB".to_string(),
            body: vec!["turn 1".to_string(), "turn 2".to_string()],
            priv_note: Some("tenant-private · redactado na escrita · TTL 90d".to_string()),
            body_tool_terms: vec!["Read auth/token.rs".to_string()],
            body_res_terms: vec!["→ 88 linhas".to_string()],
        };
        let vm = IntentDetailVm {
            repo: "acme/myrepo".to_string(),
            id: "a31".to_string(),
            title: "fix: sessão expira cedo no refresh".to_string(),
            status: "landed".to_string(),
            pr_number: Some(129),
            summary: "corrige o cálculo de iat no refresh".to_string(),
            charter: "token expira antes do TTL no refresh path".to_string(),
            summary_mono_terms: vec!["iat".to_string(), "now()".to_string()],
            charter_mono_terms: vec!["iat".to_string()],
            acceptance: vec![
                "test_refresh_full_ttl verde".to_string(),
                "nenhum token expira antes de TTL".to_string(),
            ],
            task_transcript: Some("task transcript text".to_string()),
            full_transcript: Some("full transcript text".to_string()),
            journal: Some("journal text".to_string()),
            context_json: r#"{"version":"1.2.0"}"#.to_string(),
            trajectory: vec![altitude],
            context_cas: "cas:7e1a…".to_string(),
            compact_context_ref: "cas:2c91… · 2 KB".to_string(),
            bundle_ref: "cas:8d40… · 1.6 MB".to_string(),
            compact_transcript_ref: "cas:55c1… · 9 KB".to_string(),
            campaign: Some(CampaignChipVm {
                id: "auth-hardening".to_string(),
                label: "auth hardening".to_string(),
                color_class: "c-auth".to_string(),
                display_label: "auth".to_string(),
            }),
            diff: DiffVm {
                files: vec![],
                hunks: vec![],
            },
            authorship,
            metrics,
            snapshot,
            verdicts: vec![VerdictVm {
                verdict: "APPROVE".to_string(),
                reviewer: "painel-adversarial".to_string(),
                summary: "correctness held".to_string(),
                adversarial: true,
                lens: "correctness".to_string(),
                evidence_mono_terms: vec!["iat".to_string()],
            }],
            commit_hash: "a31f9c".to_string(),
        };

        let s = serde_json::to_string(&vm).unwrap();
        let back: IntentDetailVm = serde_json::from_str(&s).unwrap();
        assert_eq!(vm, back);
        assert!(s.contains("\"cost_usd_micros\""));
    }
}
