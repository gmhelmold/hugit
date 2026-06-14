//! `GET /v1/repos/{repo}/security` → [`SecurityVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. REAL backbone:
//! - `policy.set` records — latest-wins per `rule_id` (records iterate in seq
//!   order, so the last seen is the current state); the 3 house rules
//!   (`dco`/`changelog`/`secrets`) always appear, defaulting enabled when no
//!   operator override exists.
//! - `erasure.decided` records — latest decision per `erasure_id`; the latest
//!   `approved` one (if any) surfaces as `ErasureVm` (execution is the P2 seam —
//!   the CAS-scrub step is always `pending`, never `executed`).
//!
//! HONEST-DEFAULT (no local seam): attestation / history / supply-chain / scan
//! rows — P2.

use std::collections::HashMap;

use hugit_http_contracts::common::{KpiSubKind, KpiVm};
use hugit_http_contracts::security::{
    AttestationVm, ErasureStepVm, ErasureVm, SecretGateLineVm, SecretGateVm, SecurityVm,
    SupplyChainVm,
};
use hugit_refstore::EventLog;
use serde_json::Value;

use crate::fmt::humanize_age;

const POLICY_SET_KIND: &str = "policy.set";
const ERASURE_DECIDED_KIND: &str = "erasure.decided";

/// House gate ids + default-enabled + description (mirrors `hugit_policy::Engine::house`).
const HOUSE_RULES: &[(&str, bool, &str)] = &[
    (
        "dco",
        true,
        "Todo commit não-merge precisa do trailer Signed-off-by: (DCO).",
    ),
    (
        "changelog",
        true,
        "Commits feat/fix exigem uma seção ## [Unreleased] não-vazia no CHANGELOG.md.",
    ),
    (
        "secrets",
        true,
        "Nenhum commit pode introduzir padrões óbvios de segredo (chaves, tokens, senhas).",
    ),
];

struct PolicyRule {
    enabled: bool,
    recorded_at: u64,
}

/// Latest-wins per `rule_id`. Records iterate in seq (chain) order, so a plain
/// overwrite keeps the LAST (= latest) `policy.set` per rule.
fn latest_policy_rules(log: &EventLog) -> HashMap<String, PolicyRule> {
    let mut map: HashMap<String, PolicyRule> = HashMap::new();
    for record in log.records().iter().filter(|r| r.kind == POLICY_SET_KIND) {
        let Ok(v) = serde_json::from_str::<Value>(&record.payload) else {
            continue;
        };
        let Some(rule_id) = v.get("rule_id").and_then(Value::as_str) else {
            continue;
        };
        let enabled = v.get("enabled").and_then(Value::as_bool).unwrap_or(true);
        map.insert(
            rule_id.to_string(),
            PolicyRule {
                enabled,
                recorded_at: record.recorded_at,
            },
        );
    }
    map
}

struct ErasureDecision {
    erasure_id: String,
    state: String,
    recorded_at: u64,
    seq: u64,
}

/// Latest decision per `erasure_id` (by seq).
fn latest_erasure_decisions(log: &EventLog) -> Vec<ErasureDecision> {
    let mut map: HashMap<String, ErasureDecision> = HashMap::new();
    for record in log
        .records()
        .iter()
        .filter(|r| r.kind == ERASURE_DECIDED_KIND)
    {
        let Ok(v) = serde_json::from_str::<Value>(&record.payload) else {
            continue;
        };
        let Some(erasure_id) = v.get("erasure_id").and_then(Value::as_str) else {
            continue;
        };
        let state = v
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("denied")
            .to_string();
        map.insert(
            erasure_id.to_string(),
            ErasureDecision {
                erasure_id: erasure_id.to_string(),
                state,
                recorded_at: record.recorded_at,
                seq: record.seq,
            },
        );
    }
    let mut d: Vec<ErasureDecision> = map.into_values().collect();
    d.sort_by(|a, b| a.erasure_id.cmp(&b.erasure_id));
    d
}

fn build_posture(rules: &HashMap<String, PolicyRule>) -> Vec<KpiVm> {
    HOUSE_RULES
        .iter()
        .map(|(id, default_enabled, description)| {
            let (enabled, last) = rules
                .get(*id)
                .map(|r| (r.enabled, Some(r.recorded_at)))
                .unwrap_or((*default_enabled, None));
            let value = if enabled { "ativo" } else { "inativo" }.to_string();
            let sub_text = match last {
                Some(at) => format!("{description} — {}", humanize_age(at)),
                None => (*description).to_string(),
            };
            KpiVm {
                label: (*id).to_string(),
                value,
                delta: None,
                unit: String::new(),
                sub_kind: if enabled {
                    KpiSubKind::Plain
                } else {
                    KpiSubKind::DeltaDn
                },
                sub_text,
                label_has_period: false,
            }
        })
        .collect()
}

fn build_secret_gate(rules: &HashMap<String, PolicyRule>) -> SecretGateVm {
    let enabled = rules.get("secrets").map(|r| r.enabled).unwrap_or(true);
    SecretGateVm {
        last_diff_label: String::new(),
        patterns_label: "tokens, chaves, dsns, certs, envs".to_string(),
        policy_label: if enabled {
            "fail-closed — bloqueia merge se detectar".to_string()
        } else {
            "DESABILITADO pelo operador — gate de secrets inativo".to_string()
        },
        last_scan_label: if enabled {
            "automático em push, seal e import".to_string()
        } else {
            String::new()
        },
        gate_title: String::new(),
        pass: enabled,
        lines: vec![
            SecretGateLineVm {
                pattern: "API keys".to_string(),
                hits: 0,
            },
            SecretGateLineVm {
                pattern: "tokens".to_string(),
                hits: 0,
            },
            SecretGateLineVm {
                pattern: "DSNs / certificados".to_string(),
                hits: 0,
            },
        ],
        scan_status: if enabled {
            "ativo em push, seal e import".to_string()
        } else {
            "inativo — regra desabilitada".to_string()
        },
        events: vec![],
    }
}

fn build_erasure(decisions: &[ErasureDecision]) -> Option<ErasureVm> {
    let approved = decisions
        .iter()
        .filter(|d| d.state == "approved")
        .max_by_key(|d| d.seq)?;
    Some(ErasureVm {
        pending_label: "1 pedido aprovado".to_string(),
        request_id: approved.erasure_id.clone(),
        requester: String::new(),
        target: String::new(),
        reason: String::new(),
        steps: vec![
            ErasureStepVm {
                name: "decisão registrada".to_string(),
                sub: format!("aprovada — {}", humanize_age(approved.recorded_at)),
                state: "done".to_string(),
            },
            ErasureStepVm {
                name: "CAS scrub".to_string(),
                sub: "remove blob do object store".to_string(),
                state: "pending".to_string(),
            },
        ],
    })
}

fn empty_attestation() -> AttestationVm {
    AttestationVm {
        intent_id: String::new(),
        tree_hash: String::new(),
        ref_line: String::new(),
        model: String::new(),
        prompt_ref: String::new(),
        runner: String::new(),
        principal: String::new(),
        signature: String::new(),
        cost_note: String::new(),
        artifacts_attested: 0,
        artifacts_total: 0,
        signatures_valid: 0,
        slsa_level: String::new(),
        public_key: String::new(),
    }
}

/// Build the security view-model (router calls `build_security(log, repo)`).
pub fn build_security(log: &EventLog, repo: &str) -> SecurityVm {
    let rules = latest_policy_rules(log);
    let decisions = latest_erasure_decisions(log);
    SecurityVm {
        repo: repo.to_string(),
        posture: build_posture(&rules),
        attestation: empty_attestation(),
        history: vec![],
        history_older_count: 0,
        supply_chain: SupplyChainVm {
            audit_label: String::new(),
            deny_label: String::new(),
            pinning_label: String::new(),
            toolchain_label: String::new(),
            deps: vec![],
            more_note: String::new(),
        },
        secret_gate: build_secret_gate(&rules),
        erasure: build_erasure(&decisions),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};

    fn policy(log: &mut EventLog, rule: &str, enabled: bool, param: Option<&str>, at: u64) {
        let pv = match param {
            Some(p) => serde_json::json!({"enabled":enabled,"param":p,"rule_id":rule}),
            None => serde_json::json!({"enabled":enabled,"rule_id":rule}),
        };
        let payload =
            hugit_refstore::canonical_json(&pv.to_string()).unwrap_or_else(|| pv.to_string());
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            POLICY_SET_KIND,
            vec!["o".into()],
            payload,
            at,
        )
        .expect("p");
    }
    fn erasure(log: &mut EventLog, id: &str, approve: bool, at: u64) {
        let state = if approve { "approved" } else { "denied" };
        let pv = serde_json::json!({"approve":approve,"erasure_id":id,"state":state});
        let payload =
            hugit_refstore::canonical_json(&pv.to_string()).unwrap_or_else(|| pv.to_string());
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            ERASURE_DECIDED_KIND,
            vec!["o".into()],
            payload,
            at,
        )
        .expect("e");
    }

    #[test]
    fn empty_log_house_defaults() {
        let vm = build_security(&EventLog::new(), "r");
        assert_eq!(vm.posture.len(), 3);
        assert!(vm.posture.iter().all(|k| k.value == "ativo"));
        assert!(vm.secret_gate.pass);
        assert!(vm.erasure.is_none());
    }

    #[test]
    fn policy_disables_rule() {
        let mut log = EventLog::new();
        policy(&mut log, "dco", false, None, 1000);
        let vm = build_security(&log, "r");
        assert_eq!(
            vm.posture.iter().find(|k| k.label == "dco").unwrap().value,
            "inativo"
        );
    }

    #[test]
    fn policy_latest_wins() {
        let mut log = EventLog::new();
        policy(&mut log, "secrets", false, None, 1000);
        policy(&mut log, "secrets", true, None, 2000);
        let vm = build_security(&log, "r");
        assert_eq!(
            vm.posture
                .iter()
                .find(|k| k.label == "secrets")
                .unwrap()
                .value,
            "ativo"
        );
        assert!(vm.secret_gate.pass);
    }

    #[test]
    fn disabled_secrets_gate_not_pass() {
        let mut log = EventLog::new();
        policy(&mut log, "secrets", false, None, 1000);
        assert!(!build_security(&log, "r").secret_gate.pass);
    }

    #[test]
    fn approved_erasure_surfaces_never_executed() {
        let mut log = EventLog::new();
        erasure(&mut log, "er-42", true, 5000);
        let e = build_security(&log, "r").erasure.expect("some");
        assert_eq!(e.request_id, "er-42");
        assert!(e.steps.iter().all(|s| s.state != "executed"));
        assert!(e.steps.iter().any(|s| s.state == "pending"));
    }

    #[test]
    fn erasure_latest_wins_approve_then_deny() {
        let mut log = EventLog::new();
        erasure(&mut log, "er-1", true, 1000);
        erasure(&mut log, "er-1", false, 2000);
        assert!(build_security(&log, "r").erasure.is_none());
    }

    #[test]
    fn param_secret_redacted_in_json() {
        let mut log = EventLog::new();
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        policy(&mut log, "dco", true, Some(pat), 1000);
        let j = serde_json::to_string(&build_security(&log, "r")).unwrap();
        assert!(!j.contains(pat));
    }

    #[test]
    fn vm_round_trips() {
        let vm = build_security(&EventLog::new(), "humangr/hugit");
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<SecurityVm>(&j).unwrap());
    }
}
