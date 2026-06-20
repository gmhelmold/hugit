//! `write_land` — the pure `land` write verb for the hugit-serve write-door.
//!
//! Enqueue an existing PR (`pr.queued`) into the union-testing landing queue,
//! mirroring the engine porcelain `hugit pr land` via `append_authorized`. The
//! write-door wraps this function (idempotency, D14 author assertion, HTTP). This
//! function is the PURE verb body — no I/O, no HTTP, no idempotency.

use hugit_cli::pr::{PR_QUEUED_KIND, all_pr_queued, find_pr_opened};
use hugit_contracts::event_record::EventRecord;
use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::LandReq;
use hugit_refstore::{Endpoint, EventLog};
use serde_json::json;

use crate::error::EngineErr;
use crate::fmt::scrub;

/// The valid landing modes (spec §3 / `LandReq.mode`).
const VALID_MODES: &[&str] = &["union", "serial", "window"];
/// The mode written into the `pr.queued` payload (v1 always `"union"`; the other
/// modes are reserved for future engine support — mirrors `hugit_cli::pr`).
const QUEUE_MODE: &str = "union";

/// Enqueue PR `pr` in `repo` into the landing queue (`POST …/prs/{n}/land`).
///
/// # Errors
/// - `400 INVALID_REQUEST` — `req.mode` not in `union|serial|window`.
/// - `404 NOT_FOUND` — no `pr.opened` names `pr`.
/// - `503 ENGINE_UNAVAILABLE` — `append_authorized` denied (unreachable for
///   `Orchestrator + Land`; mapped fail-honest).
pub fn write_land(
    log: &mut EventLog,
    repo: &str,
    pr: u32,
    req: &LandReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    if !VALID_MODES.contains(&req.mode.as_str()) {
        return Err(EngineErr::invalid_request(format!(
            "mode inválido '{}': aceito union|serial|window",
            scrub(&req.mode)
        )));
    }

    let pr_id = pr.to_string();
    find_pr_opened(log, &pr_id).ok_or_else(EngineErr::not_found)?;

    // order_index = active-queue tail (mirrors the CLI `land`).
    let order_index: u64 = all_pr_queued(log).len() as u64;
    let item_id = format!("{pr_id}#{order_index}");

    let payload_value = json!({
        "item_id": item_id,
        "mode": QUEUE_MODE,
        "order_index": order_index,
        "pr_id": pr_id,
    });
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());

    // D14: assert the REAL caller's class (chain-derived, fail-closed), never a
    // hardcoded `Orchestrator` — else a worker/model could land over the serve path.
    let class = crate::writes::asserted_class(&principal_chain)?;
    let record: EventRecord = log
        .append_authorized(
            class,
            Endpoint::Land,
            PR_QUEUED_KIND,
            principal_chain,
            payload,
            at,
        )
        .map_err(|denied| {
            EngineErr::unavailable(format!(
                "append_authorized negado inesperadamente ({}); consistência interna",
                denied.reason.code()
            ))
        })?;

    let queue_pos: u32 = (order_index + 1) as u32;
    let _ = repo; // owned by the door (routing); not in the response body.

    Ok(Accepted {
        seq: record.seq,
        note: "entrou na fila de união".to_string(),
        extra: Some(format!("fila #{queue_pos}")),
        queue_pos: Some(queue_pos),
        pr_number: Some(u64::from(pr)),
        branch: None,
        state: None,
        charter_preview: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn seed_pr_opened(log: &mut EventLog, pr_id: &str) {
        let payload = hugit_refstore::canonical_json(
            &json!({
                "author_kind": "orchestrator",
                "campaign": "camp-a",
                "intent_ids": ["i1"],
                "pr_id": pr_id,
                "principal": null,
                "run_id": "run-1",
            })
            .to_string(),
        )
        .unwrap();
        log.append_for_test(
            "pr.opened",
            vec!["orchestrator:test".into()],
            payload,
            1_000,
        );
    }

    fn land_req(mode: &str) -> LandReq {
        LandReq {
            mode: mode.to_string(),
        }
    }
    fn chain() -> Vec<String> {
        vec!["orchestrator:test".to_string()]
    }

    #[test]
    fn land_existing_pr_returns_accepted_with_queue_pos() {
        let mut log = EventLog::new();
        seed_pr_opened(&mut log, "42");
        let accepted = write_land(&mut log, "myrepo", 42, &land_req("union"), chain(), 2_000)
            .expect("land must succeed");
        assert_eq!(accepted.queue_pos, Some(1));
        assert_eq!(accepted.pr_number, Some(42));
        assert_eq!(accepted.extra.as_deref(), Some("fila #1"));
        let queued = log
            .records()
            .iter()
            .find(|r| r.kind == PR_QUEUED_KIND)
            .expect("pr.queued present");
        assert_eq!(queued.seq, accepted.seq);
        let payload: serde_json::Value = serde_json::from_str(&queued.payload).unwrap();
        assert_eq!(payload["pr_id"].as_str(), Some("42"));
        assert_eq!(payload["order_index"].as_u64(), Some(0));
    }

    #[test]
    fn second_pr_gets_position_2() {
        let mut log = EventLog::new();
        seed_pr_opened(&mut log, "1");
        seed_pr_opened(&mut log, "2");
        write_land(&mut log, "r", 1, &land_req("union"), chain(), 1_000).unwrap();
        let accepted = write_land(&mut log, "r", 2, &land_req("union"), chain(), 2_000).unwrap();
        assert_eq!(accepted.queue_pos, Some(2));
    }

    #[test]
    fn land_non_existent_pr_returns_not_found() {
        let mut log = EventLog::new();
        let err = write_land(&mut log, "myrepo", 99, &land_req("union"), chain(), 1_000)
            .expect_err("must 404");
        assert_eq!(err.status, 404);
        assert!(log.records().iter().all(|r| r.kind != PR_QUEUED_KIND));
    }

    #[test]
    fn invalid_mode_returns_invalid_request() {
        let mut log = EventLog::new();
        seed_pr_opened(&mut log, "7");
        let err = write_land(&mut log, "myrepo", 7, &land_req("squash"), chain(), 1_000)
            .expect_err("must 400");
        assert_eq!(err.status, 400);
        assert!(log.records().iter().all(|r| r.kind != PR_QUEUED_KIND));
    }

    #[test]
    fn worker_subagent_principal_cannot_land() {
        // THE D14 hole this closes: the verb used to assert a HARDCODED
        // `Orchestrator` class, so any caller passed the land cell. Now the class
        // is derived from the real principal — a worker/subagent is DENIED land.
        let mut log = EventLog::new();
        seed_pr_opened(&mut log, "42");
        let err = write_land(
            &mut log,
            "r",
            42,
            &land_req("union"),
            vec!["agent:runner-03".to_string()],
            2_000,
        )
        .expect_err("a worker must NOT land");
        assert_eq!(err.status, 503, "denied append maps fail-honest to 503");
        assert!(
            log.records().iter().all(|r| r.kind != PR_QUEUED_KIND),
            "the land must not have taken effect"
        );
    }

    #[test]
    fn model_principal_cannot_land() {
        let mut log = EventLog::new();
        seed_pr_opened(&mut log, "7");
        assert_eq!(
            write_land(
                &mut log,
                "r",
                7,
                &land_req("union"),
                vec!["model:claude".into()],
                1
            )
            .expect_err("a model must NOT land")
            .status,
            503
        );
        assert!(log.records().iter().all(|r| r.kind != PR_QUEUED_KIND));
    }

    #[test]
    fn unclassifiable_principal_cannot_land_fail_closed() {
        let mut log = EventLog::new();
        seed_pr_opened(&mut log, "9");
        assert_eq!(
            write_land(
                &mut log,
                "r",
                9,
                &land_req("union"),
                vec!["weird:x".into()],
                1
            )
            .expect_err("unknown principal denied")
            .status,
            404,
            "unclassifiable principal fails closed (404, no oracle)"
        );
        assert_eq!(
            write_land(&mut log, "r", 9, &land_req("union"), vec![], 1)
                .expect_err("empty chain denied")
                .status,
            404
        );
        assert!(log.records().iter().all(|r| r.kind != PR_QUEUED_KIND));
    }

    #[test]
    fn all_valid_modes_accepted() {
        for mode in &["union", "serial", "window"] {
            let mut log = EventLog::new();
            seed_pr_opened(&mut log, "5");
            assert!(write_land(&mut log, "r", 5, &land_req(mode), chain(), 1_000).is_ok());
        }
    }
}
