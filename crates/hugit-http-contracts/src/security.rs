//! `GET /v1/repos/{repo}/security` → `SecurityVm` and its nested types.
//! Transcribed BYTE-FOR-FIELD from the canonical
//! companion web frontend's view-model definitions.

use serde::{Deserialize, Serialize};

use crate::common::KpiVm;

/// Proveniência por intent — the chain of the last landed intent + coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestationVm {
    pub intent_id: String,
    pub tree_hash: String,
    pub ref_line: String,
    pub model: String,
    pub prompt_ref: String,
    pub runner: String,
    pub principal: String,
    pub signature: String,
    pub cost_note: String,
    pub artifacts_attested: u32,
    pub artifacts_total: u32,
    pub signatures_valid: u32,
    pub slsa_level: String,
    pub public_key: String,
}

/// One row of the transparency log (Histórico de atestação).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttestationLogRowVm {
    pub item: String,
    pub entry: String,
    pub sub: String,
    pub log_index: u32,
    pub state: String,
    pub date: String,
    /// Hover tooltip with the full proof data; empty = no tooltip.
    #[serde(default)]
    pub tooltip: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyRowVm {
    pub name: String,
    pub digest: String,
    pub clean: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SupplyChainVm {
    pub audit_label: String,
    pub deny_label: String,
    pub pinning_label: String,
    pub toolchain_label: String,
    pub deps: Vec<DependencyRowVm>,
    pub more_note: String,
}

/// One pattern line of the secret gate ("API keys · 0 hits").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretGateLineVm {
    pub pattern: String,
    pub hits: u32,
}

/// One incident in the fail-closed scan history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretScanEventVm {
    pub severity_class: String,
    pub title: String,
    pub meta: String,
    pub badge: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecretGateVm {
    pub last_diff_label: String,
    pub patterns_label: String,
    pub policy_label: String,
    pub last_scan_label: String,
    pub gate_title: String,
    pub pass: bool,
    pub lines: Vec<SecretGateLineVm>,
    pub scan_status: String,
    pub events: Vec<SecretScanEventVm>,
}

/// One step of the erasure pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasureStepVm {
    pub name: String,
    pub sub: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasureVm {
    pub pending_label: String,
    pub request_id: String,
    pub requester: String,
    pub target: String,
    pub reason: String,
    pub steps: Vec<ErasureStepVm>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityVm {
    pub repo: String,
    /// The posture strip — reused [`KpiVm`].
    pub posture: Vec<KpiVm>,
    pub attestation: AttestationVm,
    pub history: Vec<AttestationLogRowVm>,
    pub history_older_count: usize,
    pub supply_chain: SupplyChainVm,
    pub secret_gate: SecretGateVm,
    pub erasure: Option<ErasureVm>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::common::KpiSubKind;

    /// `SecurityVm` (fully populated, every optional present) round-trips losslessly.
    #[test]
    fn security_vm_round_trips() {
        let vm = SecurityVm {
            repo: "acme/myrepo".to_string(),
            posture: vec![KpiVm {
                label: "SLSA".to_string(),
                value: "L2".to_string(),
                delta: None,
                unit: "".to_string(),
                sub_kind: KpiSubKind::Plain,
                sub_text: "build isolado + proveniência".to_string(),
                label_has_period: false,
            }],
            attestation: AttestationVm {
                intent_id: "a31".to_string(),
                tree_hash: "a31f9c4d".to_string(),
                ref_line: "corelink-server@main".to_string(),
                model: "claude-opus-4.8 (anthropic/2026-05)".to_string(),
                prompt_ref: "cas:3b7d… (redactado — tenant-private)".to_string(),
                runner: "cl-rnr-02 (ephemeral · destruído após run)".to_string(),
                principal: "gustavo → opus-4.8 (cadeia de delegação)".to_string(),
                signature: "ed25519:9f3c1a… ✓ verificada".to_string(),
                cost_note: "$0.04 · cache poupou $3.10".to_string(),
                artifacts_attested: 67,
                artifacts_total: 67,
                signatures_valid: 67,
                slsa_level: "L2 — build isolado + proveniência".to_string(),
                public_key: "ed25519:pub:a2c0…".to_string(),
            },
            history: vec![AttestationLogRowVm {
                item: "a31".to_string(),
                entry: "intent landed".to_string(),
                sub: "tree a31f9c… · sig ed25519:9f3c… ✓".to_string(),
                log_index: 38,
                state: "atestado".to_string(),
                date: "09 jun 14:22".to_string(),
                tooltip: "tree: a31f9c… · sig: ed25519:9f3c… · log-index: #38".to_string(),
            }],
            history_older_count: 37,
            supply_chain: SupplyChainVm {
                audit_label: "limpo — 0 CVEs".to_string(),
                deny_label: "verde — 0 advisories, 0 bans".to_string(),
                pinning_label: "por digest sha256 · Cargo.lock commitado".to_string(),
                toolchain_label: "1.96.0 · pinado em rust-toolchain.toml".to_string(),
                deps: vec![DependencyRowVm {
                    name: "tokio 1.44.2".to_string(),
                    digest: "sha256:4c2f…e8b1".to_string(),
                    clean: true,
                }],
                more_note: "… +186 crates · todas pinadas".to_string(),
            },
            secret_gate: SecretGateVm {
                last_diff_label: "nenhum segredo detectado".to_string(),
                patterns_label: "tokens, chaves, dsns, certs, envs".to_string(),
                policy_label: "fail-closed — bloqueia merge se detectar".to_string(),
                last_scan_label: "há 3min · intent a31".to_string(),
                gate_title: "gate de secrets · intent a31 · diff +420 −90".to_string(),
                pass: true,
                lines: vec![SecretGateLineVm {
                    pattern: "API keys".to_string(),
                    hits: 0,
                }],
                scan_status: "✓ ativo em push, seal e import".to_string(),
                events: vec![SecretScanEventVm {
                    severity_class: "dim".to_string(),
                    title: "redigido em import".to_string(),
                    meta: "intent a29 · 09 jun 12:10".to_string(),
                    badge: "redigido".to_string(),
                }],
            },
            erasure: Some(ErasureVm {
                pending_label: "1 pedido aguardando".to_string(),
                request_id: "ER-7".to_string(),
                requester: "@joao · 09 jun 16:02".to_string(),
                target: "secrets/.env.staging · commit e9aa21f".to_string(),
                reason: "segredo vazado".to_string(),
                steps: vec![ErasureStepVm {
                    name: "CAS scrub".to_string(),
                    sub: "remove blob do object store".to_string(),
                    state: "done".to_string(),
                }],
            }),
        };

        let json = serde_json::to_string(&vm).expect("SecurityVm serializes");
        let reparsed: SecurityVm = serde_json::from_str(&json).expect("SecurityVm deserializes");
        assert_eq!(vm, reparsed, "SecurityVm round-trip is lossless");
    }
}
