//! `write_dispatch` — the pure `dispatch` write verb.
//!
//! Records the INTENT to do work: an `intent.landed` (charter from the scrubbed
//! ask) + a `pr.opened` (DRAFT). Returns `{pr_number, charter_preview}`.
//!
//! LEAD DECISION — NO auto-spawn: a web POST never starts a fleet. Spawning is the
//! P2 runner seam; this verb records the demand + a draft PR for human confirm.

use hugit_cli::pr::PR_OPENED_KIND;
use hugit_contracts::event_record::EventRecord;
use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::DispatchReq;
use hugit_refstore::{Endpoint, EventLog, PrincipalClass};
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::error::EngineErr;
use crate::fmt::scrub;

const INTENT_LANDED_KIND: &str = "intent.landed";
const AUTHORED_REF: &str = "refs/hugit/intents";
const AUTHOR_KIND: &str = "orchestrator";

fn derive_intent_id(scrubbed_ask: &str, at: u64) -> String {
    let at_str = at.to_string();
    let mut h = Sha256::new();
    h.update((scrubbed_ask.len() as u64).to_be_bytes());
    h.update(scrubbed_ask.as_bytes());
    h.update((at_str.len() as u64).to_be_bytes());
    h.update(at_str.as_bytes());
    let digest = hex::encode(h.finalize());
    format!("dispatch-{}", &digest[..16])
}

fn charter_preview(scrubbed_ask: &str) -> String {
    let first = scrubbed_ask.lines().next().unwrap_or("").trim();
    let chars: Vec<char> = first.chars().collect();
    if chars.len() <= 80 {
        first.to_string()
    } else {
        chars[..80].iter().collect::<String>() + "…"
    }
}

/// `POST /v1/repos/{repo}/dispatch`.
///
/// # Errors
/// - `400 INVALID_REQUEST` — `req.ask` blank.
/// - `503 ENGINE_UNAVAILABLE` — append denied (fail-honest).
pub fn write_dispatch(
    log: &mut EventLog,
    repo: &str,
    req: &DispatchReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    let _ = repo;
    if req.ask.trim().is_empty() {
        return Err(EngineErr::invalid_request("ask vazio"));
    }
    let charter = scrub(req.ask.trim());
    let intent_id = derive_intent_id(&charter, at);
    // P0 (audit): scrub the user-supplied campaign before it reaches the chain —
    // a secret-shaped campaign key would otherwise persist verbatim.
    let campaign = scrub(req.campaign.as_deref().unwrap_or("dispatch"));
    let pr_id = log
        .records()
        .iter()
        .filter(|r| r.kind == PR_OPENED_KIND)
        .count() as u64
        + 1;

    let intent_payload_value = json!({
        "charter": charter,
        "intent_id": intent_id,
        "ref": AUTHORED_REF,
        "target": format!("authored:{intent_id}"),
    });
    let intent_payload = hugit_refstore::canonical_json(&intent_payload_value.to_string())
        .unwrap_or_else(|| intent_payload_value.to_string());
    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        INTENT_LANDED_KIND,
        principal_chain.clone(),
        intent_payload,
        at,
    )
    .map_err(|d| {
        EngineErr::unavailable(format!("intent.landed append denied: {}", d.reason.code()))
    })?;

    let pr_payload_value = json!({
        "author_kind": AUTHOR_KIND,
        "campaign": campaign,
        "draft": req.draft,
        "intent_ids": [&intent_id],
        "pr_id": pr_id.to_string(),
        "principal": null,
        "run_id": null,
    });
    let pr_payload = hugit_refstore::canonical_json(&pr_payload_value.to_string())
        .unwrap_or_else(|| pr_payload_value.to_string());
    let pr_record: EventRecord = log
        .append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            PR_OPENED_KIND,
            principal_chain,
            pr_payload,
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!("pr.opened append denied: {}", d.reason.code()))
        })?;

    Ok(Accepted {
        seq: pr_record.seq,
        note: "despacho registrado (rascunho)".to_string(),
        extra: None,
        queue_pos: None,
        pr_number: Some(pr_id),
        branch: None,
        state: None,
        charter_preview: Some(charter_preview(&charter)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req(ask: &str) -> DispatchReq {
        DispatchReq {
            ask: ask.to_string(),
            campaign: None,
            draft: true,
        }
    }
    fn chain() -> Vec<String> {
        vec!["orchestrator:t".to_string()]
    }

    #[test]
    fn dispatch_records_intent_and_draft_pr() {
        let mut log = EventLog::new();
        let a = write_dispatch(&mut log, "r", &req("add feature branches"), chain(), 1_000)
            .expect("ok");
        assert_eq!(a.pr_number, Some(1));
        assert!(a.charter_preview.is_some());
        assert!(log.records().iter().any(|r| r.kind == INTENT_LANDED_KIND));
        assert!(log.records().iter().any(|r| r.kind == PR_OPENED_KIND));
    }

    #[test]
    fn empty_ask_is_400() {
        let mut log = EventLog::new();
        assert_eq!(
            write_dispatch(&mut log, "r", &req("  "), chain(), 1)
                .expect_err("400")
                .status,
            400
        );
        assert!(log.records().is_empty());
    }

    #[test]
    fn pat_in_ask_is_redacted() {
        let pat = "ghp_ABCdef1234567890ABCdef1234567890AB";
        let mut log = EventLog::new();
        let a = write_dispatch(&mut log, "r", &req(&format!("fix auth {pat}")), chain(), 1)
            .expect("ok");
        assert!(!a.charter_preview.unwrap().contains(pat));
        for r in log.records() {
            assert!(!r.payload.contains(pat), "raw PAT in {}", r.kind);
        }
    }
}
