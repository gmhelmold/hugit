//! `write_policy` — the pure `policy` write verb. Records `policy.set`.
//!
//! STEP-UP is the DOOR's job (the door gates policy/erasure on fresh-auth before
//! calling this). This verb just records the toggle. LIVE enforcement against
//! `hugit-policy` is the P2 seam — v1 records the operator's decision faithfully,
//! never fabricating that the rule took effect. `rule_id` is structural (not
//! scrubbed); `param` is free text (scrubbed).

use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::PolicyReq;
use hugit_refstore::{Endpoint, EventLog};
use serde_json::json;

use crate::error::EngineErr;
use crate::fmt::scrub;

/// The event kind appended by this verb (the HTTP serve-layer policy event;
/// distinct from the CLI's `policy.change`).
pub const POLICY_SET_KIND: &str = "policy.set";

/// `POST /v1/repos/{repo}/policy` (STEP-UP gated by the door).
///
/// # Errors
/// - `400 INVALID_REQUEST` — `req.rule_id` blank.
/// - `503 ENGINE_UNAVAILABLE` — append denied (fail-honest).
pub fn write_policy(
    log: &mut EventLog,
    repo: &str,
    req: &PolicyReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    let _ = repo;
    // `rule_id` is a structural identifier (a rule key like `dco`, `changelog`,
    // `org.custom-gate`): non-empty, ≤64 bytes, charset `[A-Za-z0-9-_.:]`. NOTE the
    // charset allows `_`, so a credential-shaped value (`ghp_…`, `sk-…`) PASSES the
    // shape check — therefore it is ALSO scrubbed at the boundary before it is
    // echoed/persisted (audit 2026-06-16 P3: a secret-shaped rule_id was leaking
    // verbatim into the payload + note; a legit rule key is not secret-shaped, so
    // scrub is a no-op for it).
    let rid = req.rule_id.trim();
    if rid.is_empty() {
        return Err(EngineErr::invalid_request("rule_id não pode ser vazio"));
    }
    if rid.len() > 64
        || rid
            .chars()
            .any(|c| !matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | ':'))
    {
        return Err(EngineErr::invalid_request(
            "rule_id inválido (≤64 chars, apenas ASCII alfanumérico e - _ . :)",
        ));
    }
    let rule_id_safe = scrub(rid);
    let enabled = req.enabled.unwrap_or(true);
    let param_scrubbed: Option<String> = req.param.as_deref().map(scrub);
    let payload_value = match &param_scrubbed {
        Some(p) => json!({"enabled": enabled, "param": p, "rule_id": rule_id_safe}),
        None => json!({"enabled": enabled, "rule_id": rule_id_safe}),
    };
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());
    // D14: assert the REAL caller's class (chain-derived, fail-closed), never a
    // hardcoded `Orchestrator` — else any caller would pass the policy write cell.
    let class = crate::writes::asserted_class(&principal_chain)?;
    let record = log
        .append_authorized(
            class,
            Endpoint::Land,
            POLICY_SET_KIND,
            principal_chain,
            payload,
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!("policy.set append denied: {}", d.reason.code()))
        })?;
    Ok(Accepted {
        seq: record.seq,
        note: format!("regra {rule_id_safe} atualizada"),
        extra: None,
        queue_pos: None,
        pr_number: None,
        branch: None,
        state: None,
        charter_preview: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(rule_id: &str, enabled: Option<bool>, param: Option<&str>) -> PolicyReq {
        PolicyReq {
            rule_id: rule_id.to_string(),
            enabled,
            param: param.map(str::to_string),
        }
    }
    fn chain() -> Vec<String> {
        vec!["orchestrator:t".to_string()]
    }

    #[test]
    fn valid_toggle_appends_policy_set() {
        let mut log = EventLog::new();
        let a =
            write_policy(&mut log, "r", &req("dco", Some(false), None), chain(), 1).expect("ok");
        let r = log
            .records()
            .iter()
            .find(|r| r.kind == POLICY_SET_KIND)
            .expect("present");
        assert_eq!(r.seq, a.seq);
        let p: serde_json::Value = serde_json::from_str(&r.payload).unwrap();
        assert_eq!(p["rule_id"].as_str(), Some("dco"));
        assert_eq!(p["enabled"].as_bool(), Some(false));
    }

    #[test]
    fn secret_shaped_rule_id_is_scrubbed_in_payload_and_note() {
        // The shape check allows `_`, so a `ghp_…` rule_id PASSES it — it must then
        // be scrubbed at the boundary (audit 2026-06-16 P3), never persisted/echoed
        // verbatim. (A 40-hex/`ghp_` token is the canonical secret shape.)
        let mut log = EventLog::new();
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let a = write_policy(&mut log, "r", &req(pat, Some(true), None), chain(), 1).expect("ok");
        // Persisted payload: the raw PAT must NOT appear.
        let r = log
            .records()
            .iter()
            .find(|r| r.kind == POLICY_SET_KIND)
            .expect("present");
        assert!(
            !r.payload.contains(pat),
            "raw PAT must not be persisted: {}",
            r.payload
        );
        // The success note must NOT echo the raw PAT either.
        assert!(
            !a.note.contains(pat),
            "raw PAT must not be echoed: {}",
            a.note
        );
    }

    #[test]
    fn empty_rule_id_is_400() {
        let mut log = EventLog::new();
        assert_eq!(
            write_policy(&mut log, "r", &req("", None, None), chain(), 1)
                .expect_err("400")
                .status,
            400
        );
    }

    #[test]
    fn pat_in_param_is_redacted() {
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let mut log = EventLog::new();
        write_policy(&mut log, "r", &req("g", Some(true), Some(pat)), chain(), 1).expect("ok");
        let r = log
            .records()
            .iter()
            .find(|r| r.kind == POLICY_SET_KIND)
            .unwrap();
        assert!(!r.payload.contains(pat));
        assert!(r.payload.contains("[REDACTED]"));
    }
}
