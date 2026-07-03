//! `write_usage` — the pure `ctx.usage` write verb. Records `ctx.usage`.
//!
//! The ENGINE mirror of `hugit ctx usage`: it appends a canonical `ctx.usage`
//! record — byte-identical to the CLI's payload (`crates/hugit-cli/src/ctx/usage.rs`)
//! — against a landed intent, through the SAME authorized+persisted write door the
//! other verbs use. This is the cost-killer's CAPTURE seam over the wire: the
//! authoring harness reads the LLM provider's `/usage` figures and records them
//! VERBATIM; a companion read-fold prices them into `/insights`.
//!
//! hugit records, it does NOT price. This verb computes NOTHING but the
//! trivially-consistent `total` (`input + output + cache_read + cache_write`,
//! fail-closed on overflow) — no network call, no rate math. The provider figures
//! land as-submitted.
//!
//! ## No phantom rows (the #113 misattribution law)
//! A capture against an intent id that has NOT landed is a `404` — structurally
//! forbidding a real-but-misattributed figure landing on a non-existent target.
//!
//! ## Fail-closed + structural secret-scrub
//! The intent id, model id, and optional model digest are ADDRESSES, not free
//! text — but a secret-shaped value is scrubbed at the write boundary so it can
//! never land verbatim on the forever hash-chained log.

use hugit_cli::ctx::CTX_USAGE_KIND;
use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::UsageReq;
use hugit_refstore::{Endpoint, EventLog};
use serde_json::json;

use crate::error::EngineErr;
use crate::fmt::scrub;

/// The provider-usage source marker recorded on every `ctx.usage` payload — the
/// figures came from the LLM provider's `/usage` field, recorded verbatim.
/// Byte-identical to the CLI's `USAGE_SOURCE` (`ctx/usage.rs`).
const USAGE_SOURCE: &str = "provider_usage";

/// Whether an intent `id` has a landed event on the log (the existence oracle for
/// the 404 gate). A chain-broken log fails closed (no phantom acceptance).
fn intent_exists(log: &EventLog, id: &str) -> Result<bool, EngineErr> {
    let intents = hugit_refstore::intent::intents_from_log(log)
        .map_err(|e| EngineErr::unavailable(format!("intent projection: {e}")))?;
    Ok(intents.by_id(id).is_some())
}

/// Record provider token usage against intent `id` (`POST …/intents/{id}/usage`).
///
/// # Errors
/// - `400 INVALID_REQUEST` — the token counts sum beyond `u64::MAX` (overflow).
/// - `404 NOT_FOUND` — no `intent.landed` names `id` (no phantom target).
/// - `503 ENGINE_UNAVAILABLE` — append denied / projection fault (fail-honest).
pub fn write_usage(
    log: &mut EventLog,
    repo: &str,
    id: &str,
    req: &UsageReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    let _ = repo;
    // 404 the phantom target BEFORE any effect — a usage figure can never land on
    // a non-existent intent (the misattribution law), and 404-no-oracle mirrors
    // the other verbs' existence gate.
    if !intent_exists(log, id)? {
        return Err(EngineErr::not_found());
    }

    // The trivially-consistent total — fail-closed on overflow (never a wrong or
    // panicking figure). Identical to the CLI's TokenCounts identity.
    let total = req
        .input
        .checked_add(req.output)
        .and_then(|s| s.checked_add(req.cache_read))
        .and_then(|s| s.checked_add(req.cache_write))
        .ok_or_else(|| {
            EngineErr::invalid_request("as contagens de tokens somam além de u64::MAX")
        })?;

    // Scrub the id/model/digest at the write boundary: a secret-shaped value must
    // redact before it reaches the forever-log (a real slug/digest survives).
    let target_id = scrub(id);
    let model = scrub(&req.model);
    let model_digest = req.model_digest.as_deref().map(scrub);
    let recorded_at = req.recorded_at.unwrap_or(0);

    // Build the payload byte-identical to the CLI (`ctx/usage.rs`): sorted-key
    // canonical bytes, the same target_kind/source/tokens shape the read-fold parses.
    let mut payload_value = json!({
        "target_id": target_id,
        "target_kind": "intent",
        "source": USAGE_SOURCE,
        "model": model,
        "recorded_at": recorded_at,
        "tokens": {
            "input": req.input,
            "output": req.output,
            "cache_read": req.cache_read,
            "cache_write": req.cache_write,
            "total": total,
        },
    });
    if let Some(d) = &model_digest {
        payload_value["model_digest"] = json!(d);
    }
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());

    // D14: assert the REAL caller's class (chain-derived, fail-closed) — the same
    // Orchestrator/Land cell the CLI capture + verdict use. A worker/model is
    // recognized then matrix-denied; an unclassifiable principal 404s.
    let class = crate::writes::asserted_class(&principal_chain)?;
    let record = log
        .append_authorized(
            class,
            Endpoint::Land,
            CTX_USAGE_KIND,
            principal_chain,
            payload,
            at,
        )
        .map_err(|d| EngineErr::unavailable(format!("usage append denied: {}", d.reason.code())))?;

    Ok(Accepted {
        seq: record.seq,
        note: "uso registrado".to_string(),
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

    fn log_with_intent(id: &str) -> EventLog {
        let mut log = EventLog::new();
        let raw = serde_json::json!({
            "intent_id": id, "ref": "refs/heads/main", "target": "0".repeat(40), "charter": "c"
        });
        let c = hugit_refstore::canonical_json(&raw.to_string()).unwrap_or_else(|| raw.to_string());
        log.append_for_test(
            hugit_refstore::intent::INTENT_LANDED_KIND,
            vec!["orchestrator:t".into()],
            c,
            0,
        );
        log
    }
    fn chain() -> Vec<String> {
        vec!["orchestrator:t".to_string()]
    }
    fn req() -> UsageReq {
        UsageReq {
            model: "claude-opus-4-8".into(),
            input: 10,
            output: 20,
            cache_read: 30,
            cache_write: 40,
            model_digest: None,
            recorded_at: None,
        }
    }

    #[test]
    fn well_formed_appends_one_canonical_ctx_usage() {
        let mut log = log_with_intent("i-1");
        let a = write_usage(&mut log, "r", "i-1", &req(), chain(), 7).expect("ok");
        // Exactly one ctx.usage record appended.
        let recs: Vec<_> = log
            .records()
            .iter()
            .filter(|r| r.kind == CTX_USAGE_KIND)
            .collect();
        assert_eq!(recs.len(), 1);
        let r = recs[0];
        assert_eq!(r.seq, a.seq);
        // The payload round-trips the token fields incl. the derived total.
        let v: serde_json::Value = serde_json::from_str(&r.payload).expect("json");
        assert_eq!(v["target_id"], "i-1");
        assert_eq!(v["target_kind"], "intent");
        assert_eq!(v["source"], "provider_usage");
        assert_eq!(v["tokens"]["input"], 10);
        assert_eq!(v["tokens"]["output"], 20);
        assert_eq!(v["tokens"]["cache_read"], 30);
        assert_eq!(v["tokens"]["cache_write"], 40);
        assert_eq!(v["tokens"]["total"], 100);
        // The hash chain still verifies after the append.
        assert!(
            hugit_refstore::verify_chain(log.records()).is_ok(),
            "chain must verify post-append"
        );
    }

    #[test]
    fn unknown_intent_is_404() {
        let mut log = EventLog::new();
        assert_eq!(
            write_usage(&mut log, "r", "nope", &req(), chain(), 0)
                .expect_err("404")
                .status,
            404
        );
    }

    #[test]
    fn token_overflow_is_400() {
        let mut log = log_with_intent("i-2");
        let bad = UsageReq {
            model: "m".into(),
            input: u64::MAX,
            output: 1,
            cache_read: 0,
            cache_write: 0,
            model_digest: None,
            recorded_at: None,
        };
        assert_eq!(
            write_usage(&mut log, "r", "i-2", &bad, chain(), 0)
                .expect_err("400")
                .status,
            400
        );
    }

    #[test]
    fn secret_shaped_model_is_redacted() {
        let pat = format!("ghp_{}", "x".repeat(36));
        let mut log = log_with_intent("i-3");
        let r = UsageReq {
            model: pat.clone(),
            ..req()
        };
        let a = write_usage(&mut log, "r", "i-3", &r, chain(), 0).expect("ok");
        let rec = log.records().iter().find(|r| r.seq == a.seq).unwrap();
        assert!(!rec.payload.contains(&pat), "raw PAT must be absent");
        assert!(rec.payload.contains("[REDACTED]"));
    }
}
