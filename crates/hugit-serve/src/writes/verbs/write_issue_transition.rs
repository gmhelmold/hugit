//! `write_issue_transition` — the pure `issue.transition` write verb.
//!
//! Move an issue's state to `backlog|open|closed|dispatch`. Introduces the new
//! `issue.transition` kind. As of Wave 2 there is NO `issue.opened` seam (the
//! `repo_chrome` read reports `issues_count:0`, P2), so transitions are
//! free-standing: an `issue.transition` may be recorded for issue `n` without a
//! pre-existing issue record. A P2 existence gate slots in here with zero
//! interface change.

use hugit_contracts::event_record::EventRecord;
use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::IssueTransitionReq;
use hugit_refstore::{Endpoint, EventLog, PrincipalClass};
use serde_json::json;

use crate::error::EngineErr;
use crate::fmt::scrub;

/// The new event kind appended by this verb.
pub const ISSUE_TRANSITION_KIND: &str = "issue.transition";

const VALID_STATES: &[&str] = &["backlog", "open", "closed", "dispatch"];

/// Move issue `n` in `repo` to a new state (`POST …/issues/{n}/transition`).
///
/// # Errors
/// - `400 INVALID_REQUEST` — `req.to` ∉ {backlog,open,closed,dispatch}.
/// - `503 ENGINE_UNAVAILABLE` — append denied (mapped fail-honest).
pub fn write_issue_transition(
    log: &mut EventLog,
    repo: &str,
    n: u32,
    req: &IssueTransitionReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    let _ = repo;
    if !VALID_STATES.contains(&req.to.as_str()) {
        return Err(EngineErr::invalid_request(format!(
            "estado inválido '{}': aceito backlog|open|closed|dispatch",
            req.to
        )));
    }
    let priority: Option<String> = req.priority.as_deref().map(scrub);
    let payload_value = match &priority {
        Some(p) => json!({"issue_id": n, "priority": p, "to": req.to}),
        None => json!({"issue_id": n, "to": req.to}),
    };
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());
    let record: EventRecord = log
        .append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            ISSUE_TRANSITION_KIND,
            principal_chain,
            payload,
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!(
                "issue.transition append denied: {}",
                d.reason.code()
            ))
        })?;
    Ok(Accepted {
        seq: record.seq,
        note: format!("issue movido para {}", req.to),
        extra: None,
        queue_pos: None,
        pr_number: None,
        branch: None,
        state: Some(req.to.clone()),
        charter_preview: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain() -> Vec<String> {
        vec!["orchestrator:t".to_string()]
    }
    fn req(to: &str) -> IssueTransitionReq {
        IssueTransitionReq {
            to: to.to_string(),
            priority: None,
        }
    }

    #[test]
    fn valid_transition_appends_record() {
        let mut log = EventLog::new();
        let a = write_issue_transition(&mut log, "r", 42, &req("open"), chain(), 1).expect("ok");
        assert_eq!(a.state.as_deref(), Some("open"));
        let r = log
            .records()
            .iter()
            .find(|r| r.kind == ISSUE_TRANSITION_KIND)
            .expect("present");
        assert_eq!(r.seq, a.seq);
        let p: serde_json::Value = serde_json::from_str(&r.payload).unwrap();
        assert_eq!(p["issue_id"].as_u64(), Some(42));
        assert_eq!(p["to"].as_str(), Some("open"));
    }

    #[test]
    fn invalid_to_is_400() {
        let mut log = EventLog::new();
        assert_eq!(
            write_issue_transition(&mut log, "r", 7, &req("in_review"), chain(), 1)
                .expect_err("400")
                .status,
            400
        );
        assert!(
            log.records()
                .iter()
                .all(|r| r.kind != ISSUE_TRANSITION_KIND)
        );
    }

    #[test]
    fn free_standing_on_empty_log() {
        let mut log = EventLog::new();
        assert!(write_issue_transition(&mut log, "r", 99, &req("closed"), chain(), 2).is_ok());
    }

    #[test]
    fn all_states_accepted() {
        for s in VALID_STATES {
            let mut log = EventLog::new();
            assert!(write_issue_transition(&mut log, "r", 1, &req(s), chain(), 1).is_ok());
        }
    }
}
