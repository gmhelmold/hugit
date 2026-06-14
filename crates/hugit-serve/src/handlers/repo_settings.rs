//! `GET /v1/repos/{repo}/settings` → [`RepoSettingsVm`].
//!
//! The `log` is ALREADY chain-verified by the caller. REAL backbone:
//! - `repo` — DERIVED (structural): the caller-supplied slug, set verbatim. Not
//!   scrubbed (it is a path identifier, not free text).
//! - `rule_groups` — REAL: folded from `policy.set` records, latest-wins per
//!   `rule_id` (records iterate in seq/chain order, last-seen wins — the SAME
//!   `latest_policy_rules` fold `security.rs` uses). The 3 house rules
//!   (`dco`/`changelog`/`secrets`) ALWAYS appear (default `enabled=true` when no
//!   override) as one `RuleGroupVm`; any OTHER `rule_id` seen becomes an
//!   additional operator-override row.
//! - `policy_requirements` — REAL: DERIVED from the SAME fold — one row per
//!   ENABLED rule, so a disabled rule drops out truthfully.
//!
//! REDACTION: the one free-text field that originates from a payload is a rule's
//! `param`; it is re-scrubbed at the read boundary (defense-in-depth) via
//! [`crate::fmt::scrub`]. Every other string is a structural id or a canned
//! non-secret constant.
//!
//! HONEST-DEFAULT (no local seam — never faked): `nav`, the canned-label prose
//! fields (`policy_intro`/`policy_title`/`policy_foot`/`save_hint`/
//! `secrets_note`), `collaborators` (P2 identity seam), `sync` (P2 GitHub-mirror
//! seam — `mirror_repo` kept `""`, not `repo`, to avoid implying a connection
//! that does not exist), and `secrets` (P2 broker seam).

use std::collections::HashMap;

use hugit_http_contracts::repo_settings::{
    CollaboratorVm, PolicyReqVm, PolicyRuleVm, RepoSettingsVm, RuleGroupVm, SecretRowVm,
    SyncGithubVm,
};
use hugit_refstore::EventLog;
use serde_json::Value;

const POLICY_SET_KIND: &str = "policy.set";

/// House gate ids + default-enabled + description (mirrors
/// `hugit_policy::Engine::house` / `security.rs::HOUSE_RULES`). Duplicated
/// locally to keep the handlers decoupled.
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
    /// `param` is the one free-text payload field — already scrubbed at fold time
    /// (re-scrubbed defense-in-depth, mirrors `issues.rs` priority).
    param: Option<String>,
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
        let param = v
            .get("param")
            .and_then(Value::as_str)
            .map(crate::fmt::scrub);
        map.insert(rule_id.to_string(), PolicyRule { enabled, param });
    }
    map
}

/// Canned house-rule description, or `""` honest-default for an operator override.
fn house_description(rule_id: &str) -> String {
    HOUSE_RULES
        .iter()
        .find(|(id, _, _)| *id == rule_id)
        .map(|(_, _, desc)| (*desc).to_string())
        .unwrap_or_default()
}

fn rule_row(rule_id: &str, enabled: bool, param: Option<String>) -> PolicyRuleVm {
    PolicyRuleVm {
        // `rule_id` is payload-derived free text (the write door only checks
        // charset+len, NOT secret-shape — a `ghp_…` PAT passes verbatim), so it
        // MUST be scrubbed at the read boundary. House ids scrub to themselves
        // (no-op); only a secret-shaped override id is redacted for display.
        label: crate::fmt::scrub(rule_id),
        badge: None,
        description: house_description(rule_id),
        param,
        effect: String::new(),
        enabled,
        state_label: if enabled { "ativa" } else { "inativa" }.to_string(),
        locked: false,
    }
}

/// The single "Gates de qualidade" group: the 3 house rules (folded or
/// defaulted) followed by any operator-override `rule_id` seen in the log.
fn build_rule_groups(rules: &HashMap<String, PolicyRule>) -> Vec<RuleGroupVm> {
    let mut group_rules: Vec<PolicyRuleVm> = HOUSE_RULES
        .iter()
        .map(|(id, default_enabled, _)| {
            let (enabled, param) = rules
                .get(*id)
                .map(|r| (r.enabled, r.param.clone()))
                .unwrap_or((*default_enabled, None));
            rule_row(id, enabled, param)
        })
        .collect();

    // Operator overrides: any rule_id NOT in the house set, sorted for stability.
    let mut overrides: Vec<&String> = rules
        .keys()
        .filter(|id| !HOUSE_RULES.iter().any(|(h, _, _)| *h == id.as_str()))
        .collect();
    overrides.sort();
    for id in overrides {
        let r = &rules[id];
        group_rules.push(rule_row(id, r.enabled, r.param.clone()));
    }

    vec![RuleGroupVm {
        title: "Gates de qualidade".to_string(),
        rules: group_rules,
    }]
}

/// One `PolicyReqVm` per ENABLED rule (DERIVED from the same fold — a disabled
/// rule drops out truthfully). `text` is the canned house description (or the
/// override `rule_id`, structural); `locked=false` (no permanent ⊘ rule exists).
fn build_requirements(groups: &[RuleGroupVm]) -> Vec<PolicyReqVm> {
    groups
        .iter()
        .flat_map(|g| g.rules.iter())
        .filter(|r| r.enabled)
        .map(|r| {
            let text = if r.description.is_empty() {
                r.label.clone()
            } else {
                r.description.clone()
            };
            PolicyReqVm {
                text,
                locked: false,
            }
        })
        .collect()
}

fn empty_sync() -> SyncGithubVm {
    SyncGithubVm {
        mirror_repo: String::new(),
        connection_note: String::new(),
        direction: String::new(),
        direction_note: String::new(),
        rows: vec![],
        how_note: String::new(),
    }
}

/// Build the repo-settings view-model (router calls `build_repo_settings(log, repo)`).
///
/// Returns the VM directly (NOT `Option`) — settings always exist for a repo; an
/// empty log yields the house-default rules, never a 404.
pub fn build_repo_settings(log: &EventLog, repo: &str) -> RepoSettingsVm {
    let rules = latest_policy_rules(log);
    let rule_groups = build_rule_groups(&rules);
    let policy_requirements = build_requirements(&rule_groups);
    RepoSettingsVm {
        repo: repo.to_string(),
        nav: vec![],
        policy_intro: String::new(),
        policy_title: String::new(),
        policy_requirements,
        policy_foot: String::new(),
        rule_groups,
        save_hint: String::new(),
        collaborators: Vec::<CollaboratorVm>::new(),
        sync: empty_sync(),
        secrets_note: String::new(),
        secrets: Vec::<SecretRowVm>::new(),
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

    fn house_rule<'a>(vm: &'a RepoSettingsVm, id: &str) -> &'a PolicyRuleVm {
        vm.rule_groups[0]
            .rules
            .iter()
            .find(|r| r.label == id)
            .expect("rule present")
    }

    #[test]
    fn empty_log_house_defaults() {
        let vm = build_repo_settings(&EventLog::new(), "r");
        assert_eq!(vm.rule_groups.len(), 1);
        assert_eq!(vm.rule_groups[0].title, "Gates de qualidade");
        assert_eq!(vm.rule_groups[0].rules.len(), 3);
        assert!(vm.rule_groups[0].rules.iter().all(|r| r.enabled));
        assert!(vm.rule_groups[0].rules.iter().all(|r| !r.locked));
        // All-enabled defaults → one requirement per house rule.
        assert_eq!(vm.policy_requirements.len(), 3);
        assert!(vm.policy_requirements.iter().all(|r| !r.locked));
        // Honest defaults — every P2 seam empty, nothing fabricated.
        assert!(vm.nav.is_empty());
        assert!(vm.collaborators.is_empty());
        assert!(vm.secrets.is_empty());
        assert_eq!(vm.sync.mirror_repo, "");
        assert!(vm.sync.rows.is_empty());
        assert_eq!(vm.policy_intro, "");
    }

    #[test]
    fn real_projection_from_records() {
        let mut log = EventLog::new();
        policy(&mut log, "secrets", false, None, 1000);
        policy(&mut log, "secrets", true, None, 2000); // latest-wins → enabled
        policy(&mut log, "dco", false, None, 3000);
        let vm = build_repo_settings(&log, "r");
        assert!(house_rule(&vm, "secrets").enabled);
        assert_eq!(house_rule(&vm, "secrets").state_label, "ativa");
        assert!(!house_rule(&vm, "dco").enabled);
        assert_eq!(house_rule(&vm, "dco").state_label, "inativa");
        // House descriptions are the canned constants.
        assert!(house_rule(&vm, "dco").description.contains("Signed-off-by"));
    }

    #[test]
    fn override_rule_and_disabled_drops_from_requirements() {
        let mut log = EventLog::new();
        policy(&mut log, "dco", false, None, 1000); // disabled house rule
        policy(&mut log, "require_approvals", true, Some("min 2"), 2000); // operator override
        let vm = build_repo_settings(&log, "r");
        // Override appears as an extra row with honest-default "" description.
        let ov = house_rule(&vm, "require_approvals");
        assert!(ov.enabled);
        assert_eq!(ov.description, "");
        assert_eq!(ov.param.as_deref(), Some("min 2"));
        // Disabled `dco` drops from requirements; enabled override is present.
        assert!(
            vm.policy_requirements
                .iter()
                .all(|r| !r.text.contains("Signed-off-by"))
        );
        assert!(
            vm.policy_requirements
                .iter()
                .any(|r| r.text == "require_approvals")
        );
    }

    #[test]
    fn param_secret_redacted_in_json() {
        let mut log = EventLog::new();
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        policy(&mut log, "dco", true, Some(pat), 1000);
        let j = serde_json::to_string(&build_repo_settings(&log, "r")).unwrap();
        assert!(!j.contains(pat), "PAT must not reach the wire");
    }

    #[test]
    fn rule_id_secret_redacted_in_json() {
        // The write door does NOT secret-shape-check rule_id (charset+len only),
        // so a PAT can land as a rule_id; the read boundary must redact it.
        let mut log = EventLog::new();
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        policy(&mut log, pat, true, None, 1000);
        let j = serde_json::to_string(&build_repo_settings(&log, "r")).unwrap();
        assert!(
            !j.contains(pat),
            "a secret-shaped rule_id must not reach the wire"
        );
    }

    #[test]
    fn vm_round_trips() {
        let mut log = EventLog::new();
        policy(&mut log, "changelog", false, None, 1000);
        policy(&mut log, "custom_gate", true, Some("x"), 2000);
        let vm = build_repo_settings(&log, "humangr/hugit");
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<RepoSettingsVm>(&j).unwrap());
    }
}
