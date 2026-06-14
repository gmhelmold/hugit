//! `write_comment` — the pure `comment` write verb. Introduces `pr.comment`.
//!
//! A free-text PR comment, optionally anchored to a diff line. The body (and the
//! anchor — also user-supplied) is scrubbed at the write boundary before it
//! reaches the hash chain. The write-door wraps this (idempotency, HTTP).

use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::CommentReq;
use hugit_refstore::{Endpoint, EventLog, PrincipalClass};

use crate::error::EngineErr;
use crate::fmt::scrub;

/// The new event kind appended by this verb (additive over the D1 log).
pub const PR_COMMENT_KIND: &str = "pr.comment";

/// Append a `pr.comment` for PR `pr` in `repo`.
///
/// # Errors
/// - `400 INVALID_REQUEST` — `req.body` is empty after trim.
/// - `404 NOT_FOUND` — no `pr.opened` names `pr`.
/// - `503 ENGINE_UNAVAILABLE` — `append_authorized` denied (mapped fail-honest).
pub fn write_comment(
    log: &mut EventLog,
    _repo: &str,
    pr: u32,
    req: &CommentReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    if req.body.trim().is_empty() {
        return Err(EngineErr::invalid_request("comentário vazio"));
    }
    hugit_cli::pr::find_pr_opened(log, &pr.to_string()).ok_or_else(EngineErr::not_found)?;

    let safe_body = scrub(&req.body);
    let safe_anchor: Option<String> = req.anchor.as_deref().map(scrub);

    let raw = match safe_anchor {
        Some(ref anchor) => {
            serde_json::json!({"anchor": anchor, "body": safe_body, "pr_id": u64::from(pr)})
        }
        None => serde_json::json!({"body": safe_body, "pr_id": u64::from(pr)}),
    };
    let payload =
        hugit_refstore::canonical_json(&raw.to_string()).unwrap_or_else(|| raw.to_string());

    // `Push` is the universal forge endpoint (all classes allowed); a comment is
    // not a land/undo/policy op. Route deterministically as Orchestrator — NOT
    // classify-with-Human-fallback (that fragile default inverted fail-closed,
    // P1 audit). The caller-asserted class is the disclosed P2 identity seam.
    let record = log
        .append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Push,
            PR_COMMENT_KIND,
            principal_chain,
            payload,
            at,
        )
        .map_err(|d| EngineErr::unavailable(format!("append negado: {}", d.reason.code())))?;

    Ok(Accepted {
        seq: record.seq,
        note: "comentário publicado".to_string(),
        extra: None,
        queue_pos: None,
        pr_number: Some(u64::from(pr)),
        branch: None,
        state: None,
        charter_preview: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_with_opened_pr(pr_number: u32) -> EventLog {
        let mut log = EventLog::new();
        let raw = serde_json::json!({
            "author_kind":"orchestrator","campaign":"c","intent_ids":[],
            "pr_id":pr_number.to_string(),"principal":null,"run_id":"r"
        });
        let canonical =
            hugit_refstore::canonical_json(&raw.to_string()).unwrap_or_else(|| raw.to_string());
        log.append_for_test(
            hugit_cli::pr::PR_OPENED_KIND,
            vec!["orchestrator:t".into()],
            canonical,
            1,
        );
        log
    }
    fn chain() -> Vec<String> {
        vec!["orchestrator:t".to_string()]
    }

    #[test]
    fn comment_on_existing_pr_appends_pr_comment() {
        let mut log = log_with_opened_pr(42);
        let req = CommentReq {
            body: "lgtm".into(),
            anchor: Some("lib.rs:1".into()),
        };
        let a = write_comment(&mut log, "r", 42, &req, chain(), 2).expect("ok");
        assert_eq!(a.pr_number, Some(42));
        let r = log
            .records()
            .iter()
            .find(|r| r.kind == PR_COMMENT_KIND)
            .expect("present");
        assert_eq!(r.seq, a.seq);
    }

    #[test]
    fn nonexistent_pr_is_404() {
        let mut log = log_with_opened_pr(1);
        let req = CommentReq {
            body: "x".into(),
            anchor: None,
        };
        let e = write_comment(&mut log, "r", 999, &req, chain(), 2).expect_err("404");
        assert_eq!(e.status, 404);
    }

    #[test]
    fn empty_body_is_400() {
        let mut log = log_with_opened_pr(7);
        let req = CommentReq {
            body: "   ".into(),
            anchor: None,
        };
        assert_eq!(
            write_comment(&mut log, "r", 7, &req, chain(), 2)
                .expect_err("400")
                .status,
            400
        );
    }

    #[test]
    fn body_with_pat_is_redacted() {
        let mut log = log_with_opened_pr(5);
        let pat = "ghp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let req = CommentReq {
            body: format!("token {pat}"),
            anchor: None,
        };
        let a = write_comment(&mut log, "r", 5, &req, chain(), 2).expect("ok");
        let r = log.records().iter().find(|r| r.seq == a.seq).unwrap();
        assert!(!r.payload.contains(pat), "raw PAT must be absent");
        assert!(r.payload.contains("[REDACTED]"));
    }
}
