//! Integration test for the `/v1` server's routing + auth + dispatch, via the
//! socket-free [`hugit_serve::server::route`] (deterministic — no port binding).
//!
//! Proves the transport law end-to-end over real handlers + a real on-disk log:
//! `/readyz` unauth → 200; `/v1/*` requires Bearer (401 without / wrong);
//! a present repo → 200 + a contract-valid VM; an absent repo or PR → 404
//! (`{code:"NOT_FOUND"}`, no existence leak); path-traversal → 404; non-GET → 404;
//! a TAMPERED log → 503 (`ENGINE_UNAVAILABLE`, fail-honest).

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use hugit_http_contracts::RepoHomeVm;
use hugit_refstore::EventLog;
use hugit_serve::server::{route, serve_on};
use hugit_serve::state::AppState;
use tiny_http::{Header, Method, Server};

const TOKEN: &str = "dev-token-abc";

/// A temp log dir unique to this test process AND to each call; `<dir>/<repo>.json`
/// is a repo log. The per-call `AtomicU64` is load-bearing: tests run in parallel
/// threads and several use the same repo name (`hugit`), so a clock-only suffix
/// collided when two `scratch_dir()` calls landed in the same nanosecond bucket —
/// two tests then shared one `hugit.json` and clobbered each other's fixture
/// (a valid `[]` log vs. a tampered one), a real flake seen locally and on CI. The
/// monotonic counter guarantees a distinct dir regardless of clock resolution.
fn scratch_dir() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "hugit-serve-it-{}-{}-{}",
        std::process::id(),
        nanos,
        seq
    ));
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
fn admin_control_plane_reads_are_wired_and_contract_valid() {
    use hugit_http_contracts::admin::{AdminOverviewVm, AuditVm, ErasureHistoryVm};
    let (state, _d) = state_with_repo("hugit", "[]");

    // audit — paginated timeline (empty log → honest empty page).
    let (s, b) = route(
        &state,
        &Method::Get,
        "/v1/repos/hugit/audit?since=0&limit=50",
        &bearer(TOKEN),
    );
    assert_eq!(s, 200, "audit body={b}");
    let audit: AuditVm = serde_json::from_str(&b).expect("AuditVm");
    assert_eq!(audit.returned, 0);
    assert_eq!(audit.next_since, None);

    // erasure — governance history.
    let (s, b) = route(
        &state,
        &Method::Get,
        "/v1/repos/hugit/erasure",
        &bearer(TOKEN),
    );
    assert_eq!(s, 200, "erasure body={b}");
    let er: ErasureHistoryVm = serde_json::from_str(&b).expect("ErasureHistoryVm");
    assert_eq!(er.approved_count, 0);

    // admin overview — one-call snapshot.
    let (s, b) = route(
        &state,
        &Method::Get,
        "/v1/repos/hugit/admin/overview",
        &bearer(TOKEN),
    );
    assert_eq!(s, 200, "overview body={b}");
    let ov: AdminOverviewVm = serde_json::from_str(&b).expect("AdminOverviewVm");
    assert_eq!(ov.log_depth, 0);
    assert_eq!(ov.last_activity_age, "—");

    // admin reads require Bearer like every other /v1 read.
    let (s, _b) = route(&state, &Method::Get, "/v1/repos/hugit/audit", &[]);
    assert_eq!(s, 401);

    // admin tokens — active engine-token sessions (account-level, store-backed).
    use hugit_http_contracts::admin::AdminTokensVm;
    let (s, b) = route(&state, &Method::Get, "/v1/admin/tokens", &bearer(TOKEN));
    assert_eq!(s, 200, "tokens body={b}");
    let toks: AdminTokensVm = serde_json::from_str(&b).expect("AdminTokensVm");
    assert_eq!(toks.count, 0, "no minted sessions yet");
    let (s, _b) = route(&state, &Method::Get, "/v1/admin/tokens", &[]);
    assert_eq!(s, 401, "tokens read requires Bearer");
}

#[test]
fn per_tenant_read_gate_enforces_visibility_and_owner() {
    use hugit_serve::token::ClerkPrincipal;
    // A PRIVATE repo owned by org-a (a real repo.meta record on a valid chain).
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility":"private","owner_tenant":"org-a"}).to_string(),
        0,
    );
    let log_json = serde_json::to_string_pretty(log.records()).unwrap();
    let (state, _d) = state_with_repo("acme", &log_json);

    let tok_a = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u-a".into(),
            org: "org-a".into(),
            fresh_auth: false,
        })
        .expect("mint a");
    let tok_b = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u-b".into(),
            org: "org-b".into(),
            fresh_auth: false,
        })
        .expect("mint b");

    // Owning tenant → 200.
    let (s, _b) = route(&state, &Method::Get, "/v1/repos/acme/home", &bearer(&tok_a));
    assert_eq!(s, 200, "owning tenant reads its private repo");
    // Cross-tenant → 404 (no existence leak, NEVER 403).
    let (s, b) = route(&state, &Method::Get, "/v1/repos/acme/home", &bearer(&tok_b));
    assert_eq!(s, 404, "cross-tenant denied as 404: {b}");
    assert!(b.contains("NOT_FOUND"));
    // Operator (dev-token) → 200 (bypass keeps single-tenant/dev working).
    let (s, _b) = route(&state, &Method::Get, "/v1/repos/acme/home", &bearer(TOKEN));
    assert_eq!(s, 200, "operator bypass");
    // The gate covers EVERY repo read — including the admin reads.
    let (s, _b) = route(
        &state,
        &Method::Get,
        "/v1/repos/acme/audit",
        &bearer(&tok_b),
    );
    assert_eq!(s, 404, "gate covers admin reads too (cross-tenant)");
    let (s, _b) = route(
        &state,
        &Method::Get,
        "/v1/repos/acme/audit",
        &bearer(&tok_a),
    );
    assert_eq!(s, 200, "owning tenant reads admin too");
}

#[test]
fn private_repo_with_no_owner_denies_tenant_allows_operator() {
    use hugit_serve::token::ClerkPrincipal;
    // No repo.meta → fail-safe default PRIVATE, no owner.
    let (state, _d) = state_with_repo("acme", "[]");
    let tok = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u".into(),
            org: "org-x".into(),
            fresh_auth: false,
        })
        .expect("mint");
    // A tenant cannot read a private repo with no owner_tenant (fail-safe).
    let (s, _b) = route(&state, &Method::Get, "/v1/repos/acme/home", &bearer(&tok));
    assert_eq!(s, 404, "private-no-owner denies a tenant");
    // The operator still can (bypass — the dev/launch bootstrap).
    let (s, _b) = route(&state, &Method::Get, "/v1/repos/acme/home", &bearer(TOKEN));
    assert_eq!(s, 200, "operator bypass on private-no-owner");
}

#[test]
fn me_reads_are_tenant_gated_not_a_launch_repo_leak() {
    use hugit_serve::token::ClerkPrincipal;
    // hugit (the launch repo) has no repo.meta → private, no owner.
    let (state, _d) = state_with_repo("hugit", "[]");
    let tok = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u".into(),
            org: "org-x".into(),
            fresh_auth: false,
        })
        .expect("mint");
    // A non-owner tenant must NOT read the launch repo via /v1/me/* → 404.
    let (s, _b) = route(&state, &Method::Get, "/v1/me/dashboard", &bearer(&tok));
    assert_eq!(
        s, 404,
        "me/dashboard must not leak the launch repo to a tenant"
    );
    let (s, _b) = route(&state, &Method::Get, "/v1/me/attention", &bearer(&tok));
    assert_eq!(
        s, 404,
        "me/attention must not leak the launch repo to a tenant"
    );
    // The operator still sees their own me/* view (bypass).
    let (s, _b) = route(&state, &Method::Get, "/v1/me/dashboard", &bearer(TOKEN));
    assert_eq!(s, 200, "operator me/dashboard works");
}

#[test]
fn public_repo_is_readable_cross_tenant() {
    use hugit_serve::token::ClerkPrincipal;
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility":"public","owner_tenant":"org-a"}).to_string(),
        0,
    );
    let log_json = serde_json::to_string_pretty(log.records()).unwrap();
    let (state, _d) = state_with_repo("acme", &log_json);
    let tok_b = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u-b".into(),
            org: "org-b".into(),
            fresh_auth: false,
        })
        .expect("mint b");
    // A public repo is readable by ANY authenticated tenant.
    let (s, _b) = route(&state, &Method::Get, "/v1/repos/acme/home", &bearer(&tok_b));
    assert_eq!(s, 200, "public repo readable cross-tenant");
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

/// One raw HTTP/1.1 GET over a TcpStream → the full response string (headers+body).
/// `Connection: close` so the server closes and read-to-EOF terminates.
fn http_get(addr: &str, path: &str, bearer: Option<&str>) -> String {
    let mut stream = TcpStream::connect(addr).expect("connect to serve_on");
    let auth = bearer
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let req = format!("GET {path} HTTP/1.1\r\nHost: test\r\n{auth}Connection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).unwrap();
    resp
}

/// REAL-SOCKET smoke: bind `:0`, run the actual `serve_on` loop in a thread, and
/// drive real HTTP over a TcpStream — proving the loop wiring (`Server::http`,
/// `incoming_requests`, `respond`, the Content-Type header) the socket-free
/// `route` tests cannot reach.
#[test]
fn live_socket_serves_readyz_and_authed_read() {
    let (state, _d) = state_with_repo("hugit", "[]");
    let server = Server::http("127.0.0.1:0").expect("bind ephemeral port");
    let addr = server
        .server_addr()
        .to_ip()
        .expect("ip listen addr")
        .to_string();
    std::thread::spawn(move || {
        let _ = serve_on(state, server);
    });

    // /readyz — unauthenticated, real socket → 200 + JSON content-type + body.
    let r = http_get(&addr, "/readyz", None);
    assert!(r.starts_with("HTTP/1.1 200"), "readyz response: {r}");
    assert!(r.contains("application/json"), "content-type set: {r}");
    assert!(r.contains("\"ready\":true"), "readyz body: {r}");

    // Authed read over the real socket → 200 + a contract-valid RepoHomeVm body.
    let r2 = http_get(&addr, "/v1/repos/hugit/home", Some(TOKEN));
    assert!(r2.starts_with("HTTP/1.1 200"), "home response: {r2}");
    let body = r2.split("\r\n\r\n").nth(1).unwrap_or("");
    let vm: RepoHomeVm = serde_json::from_str(body).expect("home body parses as RepoHomeVm");
    assert_eq!(vm.repo, "hugit");

    // No token over the real socket → 401.
    let r3 = http_get(&addr, "/v1/repos/hugit/home", None);
    assert!(
        r3.starts_with("HTTP/1.1 401"),
        "missing-bearer response: {r3}"
    );
}
