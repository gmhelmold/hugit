//! `write_erasure_decide` — the pure `erasure/{id}/decide` write verb (MOST SENSITIVE).
//!
//! LEAD DECISION (fail-closed): records the DECISION only (`approved`|`denied`) as
//! `erasure.decided`; NEVER executes the erasure. The tombstone write + mirror
//! obligation + X12 provenance maintenance = the P2 erasure seam, NEVER performed
//! from this v1 POST. `state` is NEVER `"executed"`. STEP-UP is the door's job.
//! Free-standing in v1 (no `erasure.requested` kind exists yet; the lifecycle
//! binding is P2).

use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::ErasureDecideReq;
use hugit_refstore::{Endpoint, EventLog, PrincipalClass};
use serde_json::json;

use crate::error::EngineErr;

/// The event kind appended by this verb.
pub const ERASURE_DECIDED_KIND: &str = "erasure.decided";

/// A structurally safe erasure id: non-empty, ≤128 bytes, `[A-Za-z0-9-_.:/]`.
fn validate_erasure_id(id: &str) -> Result<(), EngineErr> {
    if id.is_empty() {
        return Err(EngineErr::invalid_request("erasure id vazio"));
    }
    if id.len() > 128 {
        return Err(EngineErr::invalid_request(
            "erasure id muito longo (máx 128)",
        ));
    }
    if id
        .chars()
        .any(|c| !matches!(c, 'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | ':' | '/'))
    {
        return Err(EngineErr::invalid_request(
            "erasure id contém caractere inválido (apenas ASCII alfanumérico e - _ . : /)",
        ));
    }
    Ok(())
}

/// `POST /v1/repos/{repo}/erasure/{id}/decide` (STEP-UP gated by the door).
///
/// # Errors
/// - `400 INVALID_REQUEST` — `id` empty / too long / unsafe chars.
/// - `503 ENGINE_UNAVAILABLE` — append denied (fail-honest).
pub fn write_erasure_decide(
    log: &mut EventLog,
    repo: &str,
    id: &str,
    req: &ErasureDecideReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    let _ = repo;
    validate_erasure_id(id)?;
    // NEVER "executed" — execution is the P2 erasure seam.
    let state: &str = if req.approve { "approved" } else { "denied" };
    let payload_value = json!({"approve": req.approve, "erasure_id": id, "state": state});
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());
    let record = log
        .append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            ERASURE_DECIDED_KIND,
            principal_chain,
            payload,
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!(
                "erasure.decided append denied: {}",
                d.reason.code()
            ))
        })?;
    Ok(Accepted {
        seq: record.seq,
        note: format!("decisão de apagamento registrada: {state}"),
        extra: None,
        queue_pos: None,
        pr_number: None,
        branch: None,
        state: Some(state.to_string()),
        charter_preview: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> Vec<String> {
        vec!["orchestrator:t".to_string()]
    }
    fn req(approve: bool) -> ErasureDecideReq {
        ErasureDecideReq { approve }
    }

    #[test]
    fn approve_is_approved_deny_is_denied() {
        let mut log = EventLog::new();
        assert_eq!(
            write_erasure_decide(&mut log, "r", "e1", &req(true), chain(), 1)
                .unwrap()
                .state
                .as_deref(),
            Some("approved")
        );
        assert_eq!(
            write_erasure_decide(&mut log, "r", "e2", &req(false), chain(), 2)
                .unwrap()
                .state
                .as_deref(),
            Some("denied")
        );
    }

    #[test]
    fn state_is_never_executed() {
        for approve in [true, false] {
            let mut log = EventLog::new();
            let a = write_erasure_decide(&mut log, "r", "e", &req(approve), chain(), 1).unwrap();
            assert_ne!(a.state.as_deref(), Some("executed"));
            let r = log
                .records()
                .iter()
                .find(|r| r.kind == ERASURE_DECIDED_KIND)
                .unwrap();
            let v: serde_json::Value = serde_json::from_str(&r.payload).unwrap();
            assert_ne!(v["state"].as_str(), Some("executed"));
        }
    }

    #[test]
    fn empty_id_is_400() {
        let mut log = EventLog::new();
        assert_eq!(
            write_erasure_decide(&mut log, "r", "", &req(true), chain(), 1)
                .expect_err("400")
                .status,
            400
        );
    }

    #[test]
    fn unsafe_id_is_400() {
        let mut log = EventLog::new();
        for bad in &["a b", "a;b", "a$b"] {
            assert_eq!(
                write_erasure_decide(&mut log, "r", bad, &req(true), chain(), 1)
                    .expect_err("400")
                    .status,
                400
            );
        }
    }
}
