//! `liveness-probe` — is the engine up, and what does a real request see?
//!
//! Two bounded calls, both with a git UA (Cloudflare bot-protection returns
//! `403 error 1010` to unknown UAs):
//!
//! 1. **`GET /readyz`** — unauthenticated readiness. Reports `ready`,
//!    `git_serving`, `git_repos`, `version` from the engine's own JSON.
//!
//! 2. **An OPTIONAL bounded authed probe** of a LIGHT endpoint when a `token`
//!    and `repo` are given — to disambiguate the auth/edge classes a hugit
//!    caller must tell apart:
//!    - **401** → the Worker auth-gate (token required/invalid). NOT
//!      route-absence (the "probe 401 ≠ route exists" lesson).
//!    - **403** → Cloudflare bot-protection / authz denial.
//!    - **404 with a valid token** → the repo is genuinely absent OR a
//!      no-oracle cross-tenant denial — we report `denied_or_missing` (the
//!      engine deliberately does not distinguish, and neither do we — no
//!      oracle).
//!    - **404 without a token** → could be a private repo hidden from anon;
//!      reported as `anon_not_found` (try a token).
//!    - **2xx** → reachable + visible.
//!
//! ## The heavy-read REFUSAL (a real safety property)
//!
//! The engine is single-threaded + lazy-CAS: a read bounded by RESULT COUNT (not
//! wall-clock) is a latency DoS — each object is a synchronous R2 fetch that
//! blocks the WHOLE accept loop, including `/readyz`, for minutes (I wedged prod
//! just by probing `/search`). So this tool probes ONLY a known-light endpoint
//! (a repo metadata/landing read) and **REFUSES** to probe any heavy path
//! (`search`, `diff`, `compare`, `blob` history, `insights`) — it returns a tool
//! error naming the refused path rather than risk wedging the single engine.

use serde_json::{Value, json};

use super::{ToolOutcome, opt_str, req_str};
use crate::http;

/// Endpoint path-substrings that are known to drive an unbounded (count-bounded,
/// not wall-clock-bounded) lazy-CAS walk — REFUSED for probing.
const HEAVY_PATHS: &[&str] = &[
    "search", "diff", "compare", "history", "blob", "insights", "tree", "edit",
];

/// The single LIGHT endpoint we authed-probe: the repo landing/home read. It is
/// O(1) projection over already-resolved refs, not a per-object CAS walk.
fn light_probe_path(repo: &str) -> String {
    format!("/v1/repos/{repo}/home")
}

/// Pure guard: `Some(reason)` when `path` matches a heavy-read class (REFUSED),
/// `None` when it is safe to probe. No I/O — this MUST be checked before any
/// network call so the refusal fires even when the engine is unreachable.
fn heavy_path_refusal(path: &str) -> Option<String> {
    let lower = path.to_ascii_lowercase();
    HEAVY_PATHS.iter().find(|h| lower.contains(**h)).map(|h| {
        format!(
            "refused to probe `{path}`: it matches the heavy-read class `{h}`. The engine is \
             single-threaded + lazy-CAS; a count-bounded (not wall-clock-bounded) read blocks the \
             whole accept loop — including /readyz — for minutes. liveness-probe only touches the \
             light landing read."
        )
    })
}

/// Args:
/// `{ "engine_base": "https://engine.githugr.com",
///    "repo"?: "hugit", "token"?: "<bearer>",
///    "probe_path"?: "<override — REFUSED if heavy>" }`.
pub fn run(args: &Value) -> ToolOutcome {
    let base = match req_str(args, "engine_base") {
        Ok(b) => b.trim_end_matches('/'),
        Err(e) => return ToolOutcome::err(e),
    };
    if !base.starts_with("https://") && !base.starts_with("http://") {
        return ToolOutcome::err("`engine_base` must be an http(s) URL");
    }

    // Refuse any heavy probe-path override BEFORE issuing ANY network call — the
    // refusal is a pure input-validation guard (a heavy read wedges the engine),
    // so it must fire even when the engine is unreachable. This precedes /readyz.
    if let Some(custom) = opt_str(args, "probe_path")
        && let Some(reason) = heavy_path_refusal(custom)
    {
        return ToolOutcome::err(reason);
    }

    let agent = http::agent();

    // (1) /readyz — unauthenticated.
    let readyz_url = format!("{base}/readyz");
    let readyz = match http::get_classified(&agent, &readyz_url, None) {
        http::HttpClass::Ok { body, .. } => match serde_json::from_str::<Value>(&body) {
            Ok(v) => json!({ "reachable": true, "body": v }),
            Err(_) => json!({ "reachable": true, "raw": http::snippet(&body) }),
        },
        http::HttpClass::Forbidden { .. } => json!({
            "reachable": true,
            "classification": "forbidden_bot_protection",
            "note": "403 — Cloudflare bot-protection (error 1010). The probe sent a git UA; if \
                this persists the edge config changed."
        }),
        http::HttpClass::Transport { message } => {
            return ToolOutcome::err(format!(
                "engine unreachable at {readyz_url}: {message}. (/readyz is unauthenticated; a \
                 transport error means down/DNS/connect, not an auth issue.)"
            ));
        }
        other => json!({ "reachable": true, "classification": classify_unexpected(&other) }),
    };

    // (2) Optional authed light-probe to disambiguate the auth/edge classes.
    let repo = opt_str(args, "repo");
    let token = opt_str(args, "token");

    let auth_probe = match repo {
        None => json!({
            "ran": false,
            "reason": "no `repo` supplied — skipped the authed disambiguation probe; only /readyz \
                was checked."
        }),
        Some(repo) => {
            let path = light_probe_path(repo);
            // Defensive: never let the chosen light path be heavy (future-proof).
            debug_assert!(!HEAVY_PATHS.iter().any(|h| path.contains(h)));
            let url = format!("{base}{path}");
            let class = http::get_classified(&agent, &url, token);
            classify_auth_probe(repo, token.is_some(), &class, &path)
        }
    };

    ToolOutcome::Ok(json!({
        "engine_base": base,
        "readyz": readyz,
        "auth_probe": auth_probe,
        "refusal_policy": {
            "heavy_paths_refused": HEAVY_PATHS,
            "why": "single-threaded lazy-CAS engine — a count-bounded read is a latency DoS that \
                wedges the whole accept loop (including /readyz). Probe light reads only.",
        },
    }))
}

/// Map an unexpected `/readyz` HttpClass to a short label.
fn classify_unexpected(class: &http::HttpClass) -> &'static str {
    match class {
        http::HttpClass::Unauthorized => "unexpected_401_on_readyz",
        http::HttpClass::NotFound => "readyz_route_absent",
        http::HttpClass::Other { .. } => "unexpected_status",
        _ => "unexpected",
    }
}

/// Disambiguate the authed light-probe result into the classes a hugit caller
/// must tell apart, carrying the "no oracle" honesty on the 404 branch.
fn classify_auth_probe(repo: &str, had_token: bool, class: &http::HttpClass, path: &str) -> Value {
    let (classification, detail): (&str, String) = match class {
        http::HttpClass::Ok { status, .. } => (
            "reachable_and_visible",
            format!("{status} — the repo is reachable and visible to this principal."),
        ),
        http::HttpClass::Unauthorized => (
            "auth_gate_401",
            "401 — the Worker auth-gate: a token is required or the supplied one is \
             invalid/expired. This is NOT route-absence (probe 401 ≠ route exists)."
                .to_string(),
        ),
        http::HttpClass::Forbidden { .. } => (
            "forbidden_403",
            "403 — Cloudflare bot-protection (error 1010) or an authz denial. A git UA was sent."
                .to_string(),
        ),
        http::HttpClass::NotFound if had_token => (
            "denied_or_missing_404",
            format!(
                "404 WITH a valid-shaped token for `{repo}` — the engine returns 404 for BOTH a \
                 genuinely-absent repo AND a cross-tenant no-oracle denial; it deliberately does \
                 not distinguish, so neither do we. Verify the repo name and the token's tenant."
            ),
        ),
        http::HttpClass::NotFound => (
            "anon_not_found_404",
            format!(
                "404 with NO token for `{repo}` — could be absent OR a private repo hidden from \
                 anonymous callers (no oracle). Retry with a token scoped to the repo's tenant."
            ),
        ),
        http::HttpClass::Other { status, body } => (
            "unexpected_status",
            format!("unexpected status {status}: {}", http::snippet(body)),
        ),
        http::HttpClass::Transport { message } => {
            ("transport_error", format!("transport error: {message}"))
        }
    };
    json!({
        "ran": true,
        "path": path,
        "had_token": had_token,
        "classification": classification,
        "detail": detail,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_engine_base_is_a_tool_error() {
        assert!(matches!(run(&json!({})), ToolOutcome::Err(_)));
    }

    #[test]
    fn non_url_engine_base_is_rejected() {
        let args = json!({ "engine_base": "engine.test" });
        assert!(matches!(run(&args), ToolOutcome::Err(_)));
    }

    #[test]
    fn a_heavy_probe_path_is_refused_without_a_network_call() {
        for heavy in [
            "/v1/repos/hugit/search?q=x",
            "/v1/repos/hugit/diff/abc",
            "/v1/x/insights",
        ] {
            let args = json!({
                "engine_base": "https://e.test",
                "repo": "hugit",
                "probe_path": heavy
            });
            match run(&args) {
                ToolOutcome::Err(e) => assert!(e.contains("refused"), "{heavy}: {e}"),
                ToolOutcome::Ok(_) => panic!("heavy path `{heavy}` must be refused"),
            }
        }
    }

    #[test]
    fn heavy_path_refusal_is_a_pure_guard() {
        // Heavy substrings refused; a light path passes — no I/O involved.
        assert!(heavy_path_refusal("/v1/repos/hugit/search?q=x").is_some());
        assert!(heavy_path_refusal("/v1/repos/hugit/diff/abc").is_some());
        assert!(heavy_path_refusal("/v1/repos/hugit/blob/a/b.rs").is_some());
        assert!(heavy_path_refusal("/v1/repos/hugit/home").is_none());
        assert!(heavy_path_refusal("/readyz").is_none());
    }

    #[test]
    fn the_light_probe_path_is_never_heavy() {
        let path = light_probe_path("hugit");
        assert!(
            !HEAVY_PATHS.iter().any(|h| path.contains(h)),
            "light path went heavy: {path}"
        );
    }

    #[test]
    fn classify_404_with_token_is_no_oracle_denied_or_missing() {
        let v = classify_auth_probe(
            "hugit",
            true,
            &http::HttpClass::NotFound,
            "/v1/repos/hugit/home",
        );
        assert_eq!(v["classification"], json!("denied_or_missing_404"));
    }

    #[test]
    fn classify_404_without_token_is_anon_not_found() {
        let v = classify_auth_probe(
            "hugit",
            false,
            &http::HttpClass::NotFound,
            "/v1/repos/hugit/home",
        );
        assert_eq!(v["classification"], json!("anon_not_found_404"));
    }

    #[test]
    fn classify_401_is_the_auth_gate_not_route_absence() {
        let v = classify_auth_probe("hugit", true, &http::HttpClass::Unauthorized, "/p");
        assert_eq!(v["classification"], json!("auth_gate_401"));
        assert!(v["detail"].as_str().unwrap().contains("NOT route-absence"));
    }
}
