//! `write_undo` — the pure `undo` write verb. Appends a compensating `op.undone`.
//!
//! LEAD DECISION (soundness-critical): the log is append-only + hash-chained —
//! `undo` MUST NOT rewrite/delete history. It appends a compensating `op.undone`
//! naming the reverted `op_seq`; projections honor it (a soft-undo — the
//! projection-side honoring across read handlers is a tracked follow-up). undo is
//! a HUMAN verb (`Human + Undo` — the matrix's undo cell). Genesis (seq 0) and an
//! `op.undone` itself are not undoable (no re-do in v1).

use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::UndoReq;
use hugit_refstore::{Endpoint, EventLog};
use serde_json::json;

use crate::error::EngineErr;

/// The soft-undo compensator kind. Payload (canonical JSON): `{"op_seq":<u64>}`.
pub const OP_UNDONE_KIND: &str = "op.undone";

/// `POST /v1/repos/{repo}/undo`.
///
/// # Errors
/// - `400 INVALID_REQUEST` — `op_seq` is 0 (genesis) or names an `op.undone` (re-do).
/// - `404 NOT_FOUND` — no record with `seq == op_seq`.
/// - `503 ENGINE_UNAVAILABLE` — append denied (`undo` requires a Human principal).
pub fn write_undo(
    log: &mut EventLog,
    repo: &str,
    req: &UndoReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    let _ = repo;
    let op_seq = req.op_seq;
    if op_seq == 0 {
        return Err(EngineErr::invalid_request(
            "seq 0 é o genesis do log e não pode ser revertido",
        ));
    }
    let target = log
        .records()
        .iter()
        .find(|r| r.seq == op_seq)
        .ok_or_else(EngineErr::not_found)?;
    if target.kind == OP_UNDONE_KIND {
        return Err(EngineErr::invalid_request(format!(
            "seq {op_seq} é um 'op.undone'; reverter um compensador (re-do) não é suportado em v1"
        )));
    }
    let payload_value = json!({ "op_seq": op_seq });
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());
    // D14: assert the REAL caller's class (chain-derived, fail-closed), never a
    // hardcoded `Human` — else any caller would pass the undo cell.
    let class = crate::writes::asserted_class(&principal_chain)?;
    let record = log
        .append_authorized(
            class,
            Endpoint::Undo,
            OP_UNDONE_KIND,
            principal_chain,
            payload,
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!(
                "undo requer principal Human ({}); verifique a cadeia de identidade",
                d.reason.code()
            ))
        })?;
    Ok(Accepted {
        seq: record.seq,
        note: format!("operação {op_seq} revertida"),
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

    fn human() -> Vec<String> {
        vec!["user:alice".to_string()]
    }
    fn seed(log: &mut EventLog, kind: &str, at: u64) -> u64 {
        log.append_for_test(kind, human(), r#"{"x":1}"#.to_string(), at)
            .seq
    }

    #[test]
    fn undo_existing_seq_appends_op_undone() {
        let mut log = EventLog::new();
        let _s0 = seed(&mut log, "pr.opened", 1); // seq 0
        let s1 = seed(&mut log, "pr.queued", 2); // seq 1
        let a = write_undo(&mut log, "r", &UndoReq { op_seq: s1 }, human(), 3).expect("ok");
        let r = log
            .records()
            .iter()
            .find(|r| r.kind == OP_UNDONE_KIND)
            .expect("present");
        assert_eq!(r.seq, a.seq);
        let v: serde_json::Value = serde_json::from_str(&r.payload).unwrap();
        assert_eq!(v["op_seq"].as_u64(), Some(s1));
    }

    #[test]
    fn nonexistent_seq_is_404() {
        let mut log = EventLog::new();
        seed(&mut log, "pr.opened", 1);
        assert_eq!(
            write_undo(&mut log, "r", &UndoReq { op_seq: 99 }, human(), 2)
                .expect_err("404")
                .status,
            404
        );
    }

    #[test]
    fn genesis_is_400() {
        let mut log = EventLog::new();
        seed(&mut log, "pr.opened", 1);
        assert_eq!(
            write_undo(&mut log, "r", &UndoReq { op_seq: 0 }, human(), 2)
                .expect_err("400")
                .status,
            400
        );
    }

    #[test]
    fn worker_subagent_principal_cannot_undo() {
        // THE D14 hole this closes: the verb used to assert a HARDCODED `Human`
        // class, so any caller passed the undo cell. Now the class is derived from
        // the real principal — a worker/subagent (`agent:`/`worker:`) is DENIED.
        let mut log = EventLog::new();
        let _s0 = seed(&mut log, "pr.opened", 1);
        let s1 = seed(&mut log, "pr.queued", 2);
        let before = log.records().len();
        let err = write_undo(
            &mut log,
            "r",
            &UndoReq { op_seq: s1 },
            vec!["agent:runner-03".to_string()],
            3,
        )
        .expect_err("a worker must NOT undo");
        assert_eq!(err.status, 503, "denied append maps fail-honest to 503");
        // No compensating op.undone landed; only the D14 authz.denied audit record.
        assert!(
            log.records().iter().all(|r| r.kind != OP_UNDONE_KIND),
            "the undo must not have taken effect"
        );
        assert!(log.records().len() > before, "the denial was audited");
    }

    #[test]
    fn unclassifiable_principal_cannot_undo_fail_closed() {
        // An unknown bearer prefix / empty chain must fail CLOSED (404, no oracle),
        // never default into Human.
        let mut log = EventLog::new();
        let _s0 = seed(&mut log, "pr.opened", 1);
        let s1 = seed(&mut log, "pr.queued", 2);
        assert_eq!(
            write_undo(
                &mut log,
                "r",
                &UndoReq { op_seq: s1 },
                vec!["weird:x".into()],
                3
            )
            .expect_err("unknown principal denied")
            .status,
            404
        );
        assert_eq!(
            write_undo(&mut log, "r", &UndoReq { op_seq: s1 }, vec![], 3)
                .expect_err("empty chain denied")
                .status,
            404
        );
        assert!(log.records().iter().all(|r| r.kind != OP_UNDONE_KIND));
    }

    #[test]
    fn double_undo_is_400() {
        let mut log = EventLog::new();
        let _s0 = seed(&mut log, "pr.opened", 1);
        let s1 = seed(&mut log, "pr.queued", 2);
        let first = write_undo(&mut log, "r", &UndoReq { op_seq: s1 }, human(), 3).expect("ok");
        assert_eq!(
            write_undo(&mut log, "r", &UndoReq { op_seq: first.seq }, human(), 4)
                .expect_err("400")
                .status,
            400
        );
    }
}
