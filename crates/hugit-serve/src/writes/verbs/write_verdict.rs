//! `write_verdict` — the pure `verdict` write verb. Records `verdict.recorded`.
//!
//! Records a reviewer verdict against a PR as a CLEAN `VerdictObject` payload (so
//! the `insights`/ledger read — which parses `verdict.recorded` as a
//! `VerdictObject` — counts it). The optional free-text `note` is preserved as a
//! SEPARATE `pr.comment` (scrubbed), NOT nested inside the VerdictObject (which is
//! `deny_unknown_fields` — an extra key would break the read parse). Lead decision:
//! a clean verdict that the read path sees + the note never lost.

use hugit_cli::verdict::VERDICT_RECORDED_KIND;
use hugit_contracts::VerdictObject;
use hugit_contracts::verdict_object::Verdict;
use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::VerdictReq;
use hugit_refstore::{Endpoint, EventLog, PrincipalClass};

use crate::error::EngineErr;
use crate::fmt::scrub;
use crate::writes::verbs::write_comment::PR_COMMENT_KIND;

fn pr_exists(log: &EventLog, pr: u32) -> bool {
    hugit_cli::pr::find_pr_opened(log, &pr.to_string()).is_some()
}

/// `"approve"|"request-changes"` (HTTP vocab) → the canonical [`Verdict`].
fn parse_verdict_req(s: &str) -> Option<Verdict> {
    match s {
        "approve" => Some(Verdict::Approve),
        "request-changes" => Some(Verdict::FixFirst),
        _ => None,
    }
}

/// Record a verdict against PR `pr` (`POST …/prs/{n}/verdict`).
///
/// # Errors
/// - `400 INVALID_REQUEST` — `req.verdict` ∉ {approve, request-changes}.
/// - `404 NOT_FOUND` — no `pr.opened` names `pr`.
/// - `503 ENGINE_UNAVAILABLE` — append denied / serialize fault (fail-honest).
pub fn write_verdict(
    log: &mut EventLog,
    repo: &str,
    pr: u32,
    req: &VerdictReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    let _ = repo;
    let verdict = parse_verdict_req(&req.verdict).ok_or_else(|| {
        EngineErr::invalid_request(format!(
            "veredito inválido {:?}; use \"approve\" ou \"request-changes\"",
            req.verdict
        ))
    })?;
    if !pr_exists(log, pr) {
        return Err(EngineErr::not_found());
    }

    let verdict_str = match verdict {
        Verdict::Approve => "approve",
        Verdict::FixFirst => "fix_first",
        Verdict::Reject => "reject",
    };
    let vo = VerdictObject {
        intent: pr.to_string(),
        tree_hash: String::new(),
        lens: "panel".to_string(),
        model: "http-write".to_string(),
        prompt_digest: "0".repeat(64),
        verdict: verdict.clone(),
        claims_checked: vec![format!("http-verdict:{verdict_str}")],
        evidence_refs: Vec::new(),
    };
    // A CLEAN VerdictObject — no extra keys (the read path parses it strictly).
    let vo_json = serde_json::to_string(&vo)
        .map_err(|e| EngineErr::unavailable(format!("verdict serialize: {e}")))?;
    let payload = hugit_refstore::canonical_json(&vo_json).unwrap_or(vo_json);

    let record = log
        .append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            VERDICT_RECORDED_KIND,
            principal_chain.clone(),
            payload,
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!("verdict append denied: {}", d.reason.code()))
        })?;
    let seq = record.seq;

    // Preserve the reviewer note as a SEPARATE pr.comment (scrubbed) — never lost,
    // never breaks the VerdictObject parse. Only when non-empty after scrub.
    if let Some(raw_note) = req.note.as_deref() {
        let safe = scrub(raw_note);
        if !safe.trim().is_empty() {
            let raw = serde_json::json!({"body": safe, "pr_id": u64::from(pr)});
            let cpayload =
                hugit_refstore::canonical_json(&raw.to_string()).unwrap_or_else(|| raw.to_string());
            log.append_authorized(
                PrincipalClass::Orchestrator,
                Endpoint::Push,
                PR_COMMENT_KIND,
                principal_chain,
                cpayload,
                at,
            )
            .map_err(|d| {
                EngineErr::unavailable(format!("note append denied: {}", d.reason.code()))
            })?;
        }
    }

    Ok(Accepted {
        seq,
        note: "veredito registrado".to_string(),
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

    fn log_with_pr(pr: u32) -> EventLog {
        let mut log = EventLog::new();
        let raw = serde_json::json!({
            "author_kind":"orchestrator","campaign":"c","intent_ids":[],"pr_id":pr.to_string()
        });
        let c = hugit_refstore::canonical_json(&raw.to_string()).unwrap_or_else(|| raw.to_string());
        log.append_for_test(
            hugit_cli::pr::PR_OPENED_KIND,
            vec!["orchestrator:t".into()],
            c,
            0,
        );
        log
    }
    fn chain() -> Vec<String> {
        vec!["orchestrator:t".to_string()]
    }

    #[test]
    fn approve_records_clean_verdict_object() {
        let mut log = log_with_pr(42);
        let req = VerdictReq {
            verdict: "approve".into(),
            note: None,
        };
        let a = write_verdict(&mut log, "r", 42, &req, chain(), 1).expect("ok");
        assert_eq!(a.pr_number, Some(42));
        let r = log
            .records()
            .iter()
            .find(|r| r.kind == VERDICT_RECORDED_KIND)
            .expect("present");
        // MUST parse as a clean VerdictObject (the read path depends on this).
        let vo: VerdictObject = serde_json::from_str(&r.payload).expect("parses as VerdictObject");
        assert_eq!(vo.verdict, Verdict::Approve);
        assert_eq!(vo.intent, "42");
    }

    #[test]
    fn nonexistent_pr_is_404() {
        let mut log = EventLog::new();
        let req = VerdictReq {
            verdict: "approve".into(),
            note: None,
        };
        assert_eq!(
            write_verdict(&mut log, "r", 9, &req, chain(), 0)
                .expect_err("404")
                .status,
            404
        );
    }

    #[test]
    fn invalid_verdict_is_400() {
        let mut log = log_with_pr(1);
        let req = VerdictReq {
            verdict: "fix_first".into(),
            note: None,
        };
        assert_eq!(
            write_verdict(&mut log, "r", 1, &req, chain(), 0)
                .expect_err("400")
                .status,
            400
        );
    }

    #[test]
    fn note_with_pat_is_redacted_and_kept_as_comment_not_in_verdict() {
        let pat = format!("ghp_{}", "x".repeat(36));
        let mut log = log_with_pr(7);
        let req = VerdictReq {
            verdict: "request-changes".into(),
            note: Some(format!("found {pat} in diff")),
        };
        let a = write_verdict(&mut log, "r", 7, &req, chain(), 0).expect("ok");
        // The verdict record itself parses clean.
        let vr = log.records().iter().find(|r| r.seq == a.seq).unwrap();
        let _vo: VerdictObject = serde_json::from_str(&vr.payload).expect("clean verdict");
        // The note lives in a pr.comment, scrubbed.
        let cmt = log
            .records()
            .iter()
            .find(|r| r.kind == PR_COMMENT_KIND)
            .expect("note kept as comment");
        assert!(!cmt.payload.contains(&pat), "raw PAT must be absent");
        assert!(cmt.payload.contains("[REDACTED]"));
    }
}
