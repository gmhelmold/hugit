//! `cost-attest` — the attested cost figures for a repo, read from the live
//! engine's `GET /v1/repos/{repo}/insights`.
//!
//! ## Honesty law, mechanized
//!
//! This tool exists to surface ATTESTED cost — figures the engine measured and
//! (where available) bound to a signed spend-proof. Two structural guarantees:
//!
//! 1. **No hand-stamp override.** There is NO argument, env var, or code path by
//!    which a caller can inject a cost figure. The tool reads ONLY what the
//!    engine serves. The `#113 $14k demo stamp` reversal (a real-but-
//!    misattributed number still failed the honesty law) is why: a figure must
//!    be the engine's own attested value or it is not shown.
//!
//! 2. **Per-PR cost is honest-null until the runner fabric.** hugit's dispatch
//!    submits `cost_usd_micros: None` (honest-zero floor) until a real
//!    provider-`/usage` source exists (merge-as-re-execution, P2). So per-PR
//!    drill rows whose `cost_total_micros` is 0 / whose `spend_proof` is null are
//!    reported as `attested: false` with the reason — NEVER dressed up as a real
//!    measured spend.
//!
//! We project the integer micro-USD fields (ADR-0001, no float rounding) and the
//! `spend_proof` CAS refs from the `InsightsVm`, and we explicitly classify each
//! campaign/PR row as attested-or-not by whether the engine carried a non-null
//! `spend_proof` AND a non-zero `cost_*_micros`.

use serde_json::{Value, json};

use super::{ToolOutcome, req_str};
use crate::http;

/// Args:
/// `{ "engine_base": "https://engine.githugr.com", "repo": "hugit",
///    "token": "<bearer>" }`.
///
/// `token` is REQUIRED: insights is auth-gated (401 unauthed). We deliberately
/// take NO cost/override argument — the engine is the sole source.
pub fn run(args: &Value) -> ToolOutcome {
    let base = match req_str(args, "engine_base") {
        Ok(b) => b.trim_end_matches('/'),
        Err(e) => return ToolOutcome::err(e),
    };
    if !base.starts_with("https://") && !base.starts_with("http://") {
        return ToolOutcome::err("`engine_base` must be an http(s) URL");
    }
    let repo = match req_str(args, "repo") {
        Ok(r) => r,
        Err(e) => return ToolOutcome::err(e),
    };
    let token = match req_str(args, "token") {
        Ok(t) => t,
        Err(_) => {
            return ToolOutcome::err(
                "`token` is required — `GET /v1/repos/{repo}/insights` is auth-gated (401 \
                 unauthed). Supply a real session Bearer (audience = the repo's tenant).",
            );
        }
    };

    // HARD STRUCTURAL GUARD: refuse any attempt to pass a cost override. There is
    // no honest reason to send one; its presence signals a hand-stamp attempt.
    for forbidden in [
        "cost",
        "cost_usd_micros",
        "cost_micros",
        "override",
        "stamp",
        "spend_proof",
    ] {
        if args.get(forbidden).is_some() {
            return ToolOutcome::err(format!(
                "refused: `{forbidden}` is not an accepted argument. cost-attest reads ONLY the \
                 engine's attested figures — a caller-supplied cost is a hand-stamp and violates \
                 the honesty law (the #113 demo-stamp reversal)."
            ));
        }
    }

    let url = format!("{base}/v1/repos/{repo}/insights");
    let agent = http::agent();
    match http::get_classified(&agent, &url, Some(token)) {
        http::HttpClass::Ok { body, .. } => project_insights(repo, &body),
        http::HttpClass::Unauthorized => ToolOutcome::err(
            "401 from insights — the token is missing/invalid/expired, or its audience is not \
             this repo's tenant. (401 is the auth-gate, NOT route-absence.)",
        ),
        http::HttpClass::Forbidden { .. } => ToolOutcome::err(
            "403 from insights — Cloudflare bot-protection (error 1010) or an authz denial. The \
             probe sends a git UA; a 403 here is an authz/edge denial.",
        ),
        http::HttpClass::NotFound => ToolOutcome::err(format!(
            "404 from insights for repo `{repo}` — the repo is unknown to the engine OR your \
             principal may not see it (no-oracle). Verify the repo name and the token's tenant."
        )),
        http::HttpClass::Other { status, body } => ToolOutcome::err(format!(
            "unexpected status {status} from insights: {}",
            http::snippet(&body)
        )),
        http::HttpClass::Transport { message } => {
            ToolOutcome::err(format!("could not reach the engine: {message}"))
        }
    }
}

/// Project the attested cost view from a raw `InsightsVm` JSON body.
///
/// We do NOT depend on `hugit-http-contracts` (it is a serve-side crate); we
/// read the documented integer-micro-USD fields by name from the JSON so the MCP
/// crate stays lean and forward-compatible (an unknown extra field is ignored,
/// a missing one is honest-null).
fn project_insights(repo: &str, body: &str) -> ToolOutcome {
    let vm: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => {
            return ToolOutcome::err(format!(
                "insights body was not valid JSON: {e}; snippet: {}",
                http::snippet(body)
            ));
        }
    };

    // Grand totals (integer micro-USD). Honest-null when the engine omitted the
    // totals block (older build / no data).
    let totals = vm.get("cost_xray_totals");
    let total_cost_micros = totals
        .and_then(|t| t.get("cost_micros"))
        .and_then(Value::as_u64);
    let total_waste_micros = totals
        .and_then(|t| t.get("waste_micros"))
        .and_then(Value::as_u64);

    // Per-campaign rows: classify each as attested-or-not.
    let empty = vec![];
    let rows = vm
        .get("cost_xray")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let mut campaigns = Vec::with_capacity(rows.len());
    for row in rows {
        let campaign_id = row
            .get("campaign")
            .and_then(|c| c.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let cost_micros = row
            .get("cost_total_micros")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        let spend_proof = row.get("spend_proof").and_then(Value::as_str);
        let (attested, reason) = attested_status(cost_micros, spend_proof);

        // Per-PR drill rows — per-PR cost is honest-null until the runner fabric.
        let drill_empty = vec![];
        let drills = row
            .get("drill_rows")
            .and_then(Value::as_array)
            .unwrap_or(&drill_empty);
        let per_pr: Vec<Value> = drills
            .iter()
            .map(|d| {
                let pr_ref = d.get("pr_ref").and_then(Value::as_str).unwrap_or("");
                json!({
                    "pr_ref": pr_ref,
                    // Per-PR raw cost is NOT yet sourced (runner fabric, P2) — the
                    // drill row carries a display string only; we surface it as
                    // honest-null for the raw figure.
                    "cost_micros": Value::Null,
                    "cost_display": d.get("cost").and_then(Value::as_str).unwrap_or(""),
                    "attested": false,
                    "attestation_note": "per-PR raw cost is honest-null until the runner fabric \
                        supplies a provider-billed figure (merge-as-re-execution, P2). The \
                        display string is the engine's formatted value, not a re-attested figure.",
                })
            })
            .collect();

        campaigns.push(json!({
            "campaign": campaign_id,
            "cost_total_micros": cost_micros,
            "spend_proof": spend_proof,
            "attested": attested,
            "attestation_reason": reason,
            "per_pr": per_pr,
        }));
    }

    ToolOutcome::Ok(json!({
        "repo": repo,
        "source": format!("GET /v1/repos/{repo}/insights"),
        "total_cost_micros": total_cost_micros,
        "total_waste_micros": total_waste_micros,
        "campaigns": campaigns,
        "honesty": {
            "hand_stamp_override": "structurally impossible — this tool reads only the engine's \
                attested figures; no cost argument is accepted.",
            "per_pr_cost": "honest-null until the runner fabric (provider-/usage source); hugit \
                dispatch submits None today, never a derived/misattributed COGS.",
            "unit": "integer micro-USD (1 USD = 1_000_000); null = the engine did not attest this \
                figure (older build or no captured spend-proof envelope).",
        },
    }))
}

/// A campaign row is ATTESTED only when the engine carried BOTH a non-null
/// `spend_proof` (a content-addressed signed record) AND a non-zero measured
/// cost. A zero cost or a null proof is honest-unattested, never dressed up.
fn attested_status(cost_micros: u64, spend_proof: Option<&str>) -> (bool, &'static str) {
    match (cost_micros, spend_proof) {
        (0, _) => (
            false,
            "cost is honest-zero — no provider-billed figure captured yet (runner fabric / P2).",
        ),
        (_, None) => (
            false,
            "no spend_proof — the engine measured a cost but captured no signed attestation \
             envelope (provability hook absent for this row).",
        ),
        (_, Some(_)) => (true, "non-zero cost bound to a signed spend_proof CAS ref."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_token_is_a_tool_error() {
        let args = json!({ "engine_base": "https://e.test", "repo": "hugit" });
        match run(&args) {
            ToolOutcome::Err(e) => assert!(e.contains("token")),
            ToolOutcome::Ok(_) => panic!("expected token error"),
        }
    }

    #[test]
    fn a_cost_override_argument_is_refused() {
        for k in [
            "cost",
            "cost_usd_micros",
            "override",
            "stamp",
            "spend_proof",
        ] {
            let args = json!({
                "engine_base": "https://e.test", "repo": "hugit", "token": "t", k: 9999
            });
            match run(&args) {
                ToolOutcome::Err(e) => assert!(e.contains("refused"), "key {k}: {e}"),
                ToolOutcome::Ok(_) => panic!("override `{k}` must be refused"),
            }
        }
    }

    #[test]
    fn non_url_engine_base_is_rejected() {
        let args = json!({ "engine_base": "engine.test", "repo": "hugit", "token": "t" });
        assert!(matches!(run(&args), ToolOutcome::Err(_)));
    }

    #[test]
    fn attested_status_classifies_honestly() {
        assert!(!attested_status(0, Some("cas:x")).0);
        assert!(!attested_status(100, None).0);
        assert!(attested_status(100, Some("cas:x")).0);
    }

    #[test]
    fn projects_a_real_insights_body_with_honest_attestation() {
        // A row with cost + proof → attested; a row with cost but no proof →
        // not attested; per-PR cost is always honest-null.
        let body = json!({
            "repo": "hugit",
            "cost_xray_totals": { "cost_micros": 94_000_000u64, "waste_micros": 11_000_000u64 },
            "cost_xray": [
                {
                    "campaign": { "id": "auth" },
                    "cost_total_micros": 24_000_000u64,
                    "spend_proof": "cas:abc",
                    "drill_rows": [ { "pr_ref": "#129", "cost": "$0.02" } ]
                },
                {
                    "campaign": { "id": "noproof" },
                    "cost_total_micros": 5_000_000u64,
                    "spend_proof": null,
                    "drill_rows": []
                }
            ]
        })
        .to_string();

        match project_insights("hugit", &body) {
            ToolOutcome::Ok(v) => {
                assert_eq!(v["total_cost_micros"], json!(94_000_000u64));
                let camps = v["campaigns"].as_array().unwrap();
                assert_eq!(camps[0]["attested"], json!(true));
                assert_eq!(camps[1]["attested"], json!(false));
                // per-PR raw cost is always honest-null
                assert!(camps[0]["per_pr"][0]["cost_micros"].is_null());
                assert_eq!(camps[0]["per_pr"][0]["attested"], json!(false));
            }
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
    }

    #[test]
    fn missing_totals_block_is_honest_null_not_zero() {
        let body = json!({ "repo": "hugit", "cost_xray": [] }).to_string();
        match project_insights("hugit", &body) {
            ToolOutcome::Ok(v) => {
                assert!(v["total_cost_micros"].is_null(), "no totals → null, not 0");
            }
            ToolOutcome::Err(e) => panic!("unexpected error: {e}"),
        }
    }
}
