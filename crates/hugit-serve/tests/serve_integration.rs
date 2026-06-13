//! Integration test for the `/v1` server's routing + auth + dispatch, via the
//! socket-free [`hugit_serve::server::route`] (deterministic — no port binding).
//!
//! Proves the transport law end-to-end over real handlers + a real on-disk log:
//! `/readyz` unauth → 200; `/v1/*` requires Bearer (401 without / wrong);
//! a present repo → 200 + a contract-valid VM; an absent repo or PR → 404
//! (`{code:"NOT_FOUND"}`, no existence leak); path-traversal → 404; non-GET → 404;
//! a TAMPERED log → 503 (`ENGINE_UNAVAILABLE`, fail-honest).

use std::path::PathBuf;

use hugit_http_contracts::RepoHomeVm;
use hugit_serve::server::route;
use hugit_serve::state::AppState;
use tiny_http::{Header, Method};

const TOKEN: &str = "dev-token-abc";

/// A temp log dir unique to this test process; `<dir>/<repo>.json` is a repo log.
fn scratch_dir() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("hugit-serve-it-{}-{}", std::process::id(), nanos));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn state_with_repo(repo: &str, log_json: &str) -> (AppState, PathBuf) {
    let dir = scratch_dir();
    std::fs::write(dir.join(format!("{repo}.json")), log_json).unwrap();
    (AppState::new(dir.clone(), TOKEN.to_string()), dir)
}

fn bearer(tok: &str) -> Vec<Header> {
    vec![Header::from_bytes(&b"Authorization"[..], format!("Bearer {tok}").as_bytes()).unwrap()]
}

#[test]
fn readyz_is_unauthenticated_200() {
    let (state, _d) = state_with_repo("hugit", "[]");
    // No auth header, /readyz still 200.
    let (status, body) = route(&state, &Method::Get, "/readyz", &[]);
    assert_eq!(status, 200);
    assert!(body.contains("ready"));
}

#[test]
fn present_repo_home_with_bearer_is_200_contract_valid() {
    // An empty (but valid, chain-verifies) log → honest RepoHomeVm.
    let (state, _d) = state_with_repo("hugit", "[]");
    let (status, body) = route(&state, &Method::Get, "/v1/repos/hugit/home", &bearer(TOKEN));
    assert_eq!(status, 200, "body={body}");
    // The body MUST deserialize through the frozen contract type.
    let vm: RepoHomeVm = serde_json::from_str(&body).expect("home body parses as RepoHomeVm");
    assert_eq!(vm.repo, "hugit");
}

#[test]
fn missing_bearer_is_401() {
    let (state, _d) = state_with_repo("hugit", "[]");
    let (status, body) = route(&state, &Method::Get, "/v1/repos/hugit/home", &[]);
    assert_eq!(status, 401);
    assert!(body.contains("TOKEN_INVALID"));
}

#[test]
fn wrong_bearer_is_401() {
    let (state, _d) = state_with_repo("hugit", "[]");
    let (status, _b) = route(
        &state,
        &Method::Get,
        "/v1/repos/hugit/home",
        &bearer("WRONG"),
    );
    assert_eq!(status, 401);
}

#[test]
fn absent_repo_is_404_no_leak() {
    let (state, _d) = state_with_repo("hugit", "[]");
    // A different repo has no log file → 404, identical shape to a no-access 404.
    let (status, body) = route(
        &state,
        &Method::Get,
        "/v1/repos/does-not-exist/home",
        &bearer(TOKEN),
    );
    assert_eq!(status, 404);
    assert!(body.contains("NOT_FOUND"));
    assert!(
        !body.to_lowercase().contains("exist"),
        "no existence oracle in the body"
    );
}

#[test]
fn absent_pr_is_404() {
    let (state, _d) = state_with_repo("hugit", "[]");
    let (status, body) = route(
        &state,
        &Method::Get,
        "/v1/repos/hugit/prs/999",
        &bearer(TOKEN),
    );
    assert_eq!(status, 404);
    assert!(body.contains("NOT_FOUND"));
}

#[test]
fn path_traversal_repo_is_404() {
    let (state, _d) = state_with_repo("hugit", "[]");
    // Auth passes, but the slug guard rejects traversal → 404 (never reads outside).
    let (status, _b) = route(
        &state,
        &Method::Get,
        "/v1/repos/..%2F..%2Fetc%2Fpasswd/home",
        &bearer(TOKEN),
    );
    assert_eq!(status, 404);
}

#[test]
fn non_get_is_404() {
    let (state, _d) = state_with_repo("hugit", "[]");
    let (status, _b) = route(
        &state,
        &Method::Post,
        "/v1/repos/hugit/home",
        &bearer(TOKEN),
    );
    assert_eq!(status, 404);
}

#[test]
fn tampered_log_is_503_fail_honest() {
    // Valid JSON array shape but NOT a well-formed/verifiable chain → the verified
    // loader rejects it → 503 ENGINE_UNAVAILABLE (never a fake-empty 200 VM).
    let bogus = r#"[{"seq":7,"kind":"pr.opened","payload":"{}","prev_hash":"deadbeef","this_hash":"00","recorded_at":1,"principal_chain":["x"]}]"#;
    let (state, _d) = state_with_repo("hugit", bogus);
    let (status, body) = route(&state, &Method::Get, "/v1/repos/hugit/home", &bearer(TOKEN));
    assert_eq!(
        status, 503,
        "tampered chain must be 503, got {status}: {body}"
    );
    assert!(body.contains("ENGINE_UNAVAILABLE"));
}

#[test]
fn all_five_reads_route_with_bearer() {
    let (state, _d) = state_with_repo("hugit", "[]");
    for path in [
        "/v1/repos/hugit/home",
        "/v1/repos/hugit/landing",
        "/v1/repos/hugit/checks",
        "/v1/repos/hugit/commits",
    ] {
        let (status, body) = route(&state, &Method::Get, path, &bearer(TOKEN));
        assert_eq!(status, 200, "{path} → {status}: {body}");
        // Each body must be valid JSON.
        let _: serde_json::Value = serde_json::from_str(&body).expect("valid JSON body");
    }
}
