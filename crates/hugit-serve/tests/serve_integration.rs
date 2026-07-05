//! Integration test for the `/v1` server's routing + auth + dispatch, via the
//! socket-free [`hugit_serve::server::route`] (deterministic — no port binding).
//!
//! Proves the transport law end-to-end over real handlers + a real on-disk log:
//! `/readyz` unauth → 200; a repo READ derives the anonymous principal on a
//! missing/invalid Bearer (W-ANON-V1) and is gated by `authorize_read` — a PUBLIC
//! repo serves anon (200), a PRIVATE/absent one → 404 (no oracle), never a 401 on the
//! credential; the 401 stays on the WRITE door, `/v1/me/*`, `/v1/admin/*`, `/v1/token`;
//! a present readable repo → 200 + a contract-valid VM; an absent repo or PR → 404
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
fn readyz_git_serving_false_when_no_git_source() {
    // A state with no git seam loaded (no HUGIT_SERVE_GIT_DIR wired) must report
    // git_serving:false + git_repos:0 in the readyz response — visible in monitoring.
    let (state, _d) = state_with_repo("hugit", "[]");
    assert_eq!(
        state.git_serving_count(),
        0,
        "AppState::new() must have no git seam (test precondition)"
    );
    let (status, body) = route(&state, &Method::Get, "/readyz", &[]);
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).expect("readyz body is JSON");
    assert_eq!(v["ready"], true, "ready must be true");
    assert_eq!(
        v["git_serving"], false,
        "git_serving must be false when no git dir is wired, body={body}"
    );
    assert_eq!(v["git_repos"], 0, "git_repos must be 0, body={body}");
}

#[test]
fn readyz_git_serving_true_when_git_source_wired() {
    // A state with a repo's git seam set must report git_serving:true + the count.
    use hugit_proto::CasObjectSource;
    use std::sync::Arc;
    let (mut state, _d) = state_with_repo("hugit", "[]");
    state.set_repo_git(
        "hugit",
        Arc::new(CasObjectSource::new()),
        gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
        std::collections::BTreeMap::new(),
    );
    let (status, body) = route(&state, &Method::Get, "/readyz", &[]);
    assert_eq!(status, 200);
    let v: serde_json::Value = serde_json::from_str(&body).expect("readyz body is JSON");
    assert_eq!(
        v["git_serving"], true,
        "git_serving must be true when a git seam is wired, body={body}"
    );
    assert_eq!(
        v["git_repos"], 1,
        "git_repos must count the loaded repos, body={body}"
    );
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

    // A no-Bearer admin read degrades to anon (W-ANON-V1). This repo is PRIVATE
    // (default meta), so `authorize_read` denies the anon read → 404 BEFORE the
    // operator gate is even reached (no existence oracle). (On a PUBLIC repo the
    // read-gate passes but the operator-gate then 404s anon — proven in
    // `admin_control_plane_is_operator_only_even_on_a_public_repo`.)
    let (s, b) = route(&state, &Method::Get, "/v1/repos/hugit/audit", &[]);
    assert_eq!(
        s, 404,
        "no-Bearer admin read on a PRIVATE repo → anon → 404"
    );
    assert!(b.contains("NOT_FOUND"), "uniform no-oracle 404 body: {b}");

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
    // The read gate covers EVERY repo read — including the admin reads — for a
    // cross-tenant caller.
    let (s, _b) = route(
        &state,
        &Method::Get,
        "/v1/repos/acme/audit",
        &bearer(&tok_b),
    );
    assert_eq!(s, 404, "gate covers admin reads too (cross-tenant)");
    // The admin/audit control plane is OPERATOR-only (audit 2026-06-20): even the
    // OWNING tenant — which passes the read gate for normal screens — gets 404 on
    // the control plane. Only the operator reads /audit.
    let (s, _b) = route(
        &state,
        &Method::Get,
        "/v1/repos/acme/audit",
        &bearer(&tok_a),
    );
    assert_eq!(s, 404, "owning tenant is NOT the operator → no admin plane");
    let (s, _b) = route(&state, &Method::Get, "/v1/repos/acme/audit", &bearer(TOKEN));
    assert_eq!(s, 200, "only the operator reads /audit");
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

/// Seed an `AppState` (Local source) with N repos, each BOTH a `<slug>.json` log
/// AND an entry in the `repos` map (the git seam the W-METENANT me/* index
/// iterates). Mirrors the prod shape where every served repo is git-wired.
fn state_with_repos(repos: &[(&str, &str)]) -> (AppState, PathBuf) {
    use hugit_proto::CasObjectSource;
    use std::sync::Arc;
    let dir = scratch_dir();
    let mut state = AppState::new(dir.clone(), TOKEN.to_string());
    for (slug, json) in repos {
        std::fs::write(dir.join(format!("{slug}.json")), json).unwrap();
        state.set_repo_git(
            *slug,
            Arc::new(CasObjectSource::new()),
            gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
            std::collections::BTreeMap::new(),
        );
    }
    (state, dir)
}

/// A chain-valid serialized log carrying a single `repo.meta{visibility,owner_tenant}`.
fn repo_meta_log(visibility: &str, owner_tenant: &str) -> String {
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility":visibility,"owner_tenant":owner_tenant}).to_string(),
        0,
    );
    serde_json::to_string_pretty(log.records()).unwrap()
}

/// W-METENANT: a non-owner tenant's `/v1/me/*` view is an honest EMPTY (200 with
/// no rows) — the launch repo is ABSENT, never leaked, never a 404-vs-200 oracle.
/// The frozen contract chose empty-not-404: identity-scoped, no default repo.
#[test]
fn me_reads_are_tenant_scoped_empty_not_a_launch_repo_leak() {
    use hugit_http_contracts::attention::AttentionVm;
    use hugit_http_contracts::dashboard::DashboardVm;
    use hugit_serve::token::ClerkPrincipal;
    // "hugit" (the launch repo) has no repo.meta → private, no owner. It IS loaded
    // (git-wired) so the index considers it — and excludes it for a non-owner.
    let (state, _d) = state_with_repos(&[("hugit", "[]")]);
    let tok = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u".into(),
            org: "org-x".into(),
            fresh_auth: false,
        })
        .expect("mint");
    // A non-owner tenant → 200 with an EMPTY dashboard (the private launch repo is
    // ABSENT, not leaked).
    let (s, b) = route(&state, &Method::Get, "/v1/me/dashboard", &bearer(&tok));
    assert_eq!(s, 200, "me/dashboard is identity-scoped 200, body={b}");
    let vm: DashboardVm = serde_json::from_str(&b).expect("DashboardVm");
    assert!(
        vm.repos.is_empty(),
        "the private launch repo must NOT appear for a non-owner tenant"
    );
    assert!(vm.inbox.is_empty());
    assert_eq!(vm.attention_count, 0);
    // Attention likewise empty (no cross-tenant feed leak).
    let (s, b) = route(&state, &Method::Get, "/v1/me/attention", &bearer(&tok));
    assert_eq!(s, 200);
    let av: AttentionVm = serde_json::from_str(&b).expect("AttentionVm");
    assert!(av.decisions.is_empty(), "no cross-tenant attention leak");
    // The operator still sees the launch repo (bypass).
    let (s, b) = route(&state, &Method::Get, "/v1/me/dashboard", &bearer(TOKEN));
    assert_eq!(s, 200, "operator me/dashboard works");
    let vm: DashboardVm = serde_json::from_str(&b).expect("DashboardVm");
    assert_eq!(vm.repos.len(), 1, "operator sees the launch repo");
    assert_eq!(vm.repos[0].name, "hugit");
}

/// W-METENANT CORE: two tenants with disjoint PRIVATE repos each see ONLY their
/// own via `/v1/me/dashboard` — the cross-principal isolation proof, end-to-end
/// through the real route + auth.
#[test]
fn me_dashboard_two_tenants_disjoint_private_repos_isolated() {
    use hugit_http_contracts::dashboard::DashboardVm;
    use hugit_serve::token::ClerkPrincipal;
    let (state, _d) = state_with_repos(&[
        ("alpha", &repo_meta_log("private", "org-a")),
        ("beta", &repo_meta_log("private", "org-b")),
    ]);
    let tok_a = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "ua".into(),
            org: "org-a".into(),
            fresh_auth: false,
        })
        .expect("mint a");
    let tok_b = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "ub".into(),
            org: "org-b".into(),
            fresh_auth: false,
        })
        .expect("mint b");

    let (s, b) = route(&state, &Method::Get, "/v1/me/dashboard", &bearer(&tok_a));
    assert_eq!(s, 200);
    let vm: DashboardVm = serde_json::from_str(&b).unwrap();
    let names_a: Vec<&str> = vm.repos.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names_a, vec!["alpha"], "org-a sees ONLY alpha");

    let (s, b) = route(&state, &Method::Get, "/v1/me/dashboard", &bearer(&tok_b));
    assert_eq!(s, 200);
    let vm: DashboardVm = serde_json::from_str(&b).unwrap();
    let names_b: Vec<&str> = vm.repos.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names_b, vec!["beta"], "org-b sees ONLY beta — never alpha");
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

/// THE hole this closes (pre-open-source audit 2026-06-20): the operator/admin
/// control-plane reads (audit timeline, erasure governance, admin overview) were
/// gated ONLY by read-visibility. The moment a repo is set `public` (the
/// documented anonymous-`git clone` gate), any anonymous/any-tenant caller could
/// read the admin plane. The fix gates these on OPERATOR status, not visibility:
/// a non-operator gets the uniform 404 EVEN on a public repo, while normal repo
/// screens (home) stay open to anonymous/any-tenant on a public repo.
#[test]
fn admin_control_plane_is_operator_only_even_on_a_public_repo() {
    use hugit_serve::token::ClerkPrincipal;
    // A PUBLIC repo owned by org-a — the exact config that opens anonymous clone.
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility":"public","owner_tenant":"org-a"}).to_string(),
        0,
    );
    let log_json = serde_json::to_string_pretty(log.records()).unwrap();
    let (state, _d) = state_with_repo("acme", &log_json);

    // A normal (non-operator) tenant — even the OWNING tenant (org-a) is NOT the
    // operator, so the control plane is closed to it too.
    let tok_owner = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u-a".into(),
            org: "org-a".into(),
            fresh_auth: false,
        })
        .expect("mint owner");
    let tok_other = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u-b".into(),
            org: "org-b".into(),
            fresh_auth: false,
        })
        .expect("mint other");

    let admin_routes = [
        "/v1/repos/acme/audit?since=0&limit=50",
        "/v1/repos/acme/erasure",
        "/v1/repos/acme/admin/overview",
    ];

    for url in admin_routes {
        // (a) ANONYMOUS (no Bearer, W-ANON-V1) → the anon principal PASSES the
        // read-gate on this PUBLIC repo, but the control-plane OPERATOR gate then
        // denies → uniform 404 (never the admin plane, no oracle). The key security
        // property holds: anon on a public repo can NEVER reach the control plane.
        let (s, b) = route(&state, &Method::Get, url, &[]);
        assert_eq!(
            s, 404,
            "anonymous is NOT the operator → 404 on public admin: {url}"
        );
        assert!(
            b.contains("NOT_FOUND"),
            "uniform 404 body for anon admin: {url}"
        );

        // (a) A normal tenant — owner AND a different tenant — gets the uniform
        // 404 on the control plane even though the repo is PUBLIC (no oracle).
        let (s, b) = route(&state, &Method::Get, url, &bearer(&tok_owner));
        assert_eq!(s, 404, "owning tenant is NOT the operator → 404: {url}");
        assert!(b.contains("NOT_FOUND"), "uniform 404 body: {url}");
        let (s, _b) = route(&state, &Method::Get, url, &bearer(&tok_other));
        assert_eq!(s, 404, "a different tenant → 404 on public admin: {url}");

        // (b) The OPERATOR (dev-token) still gets the control plane.
        let (s, _b) = route(&state, &Method::Get, url, &bearer(TOKEN));
        assert_eq!(s, 200, "operator reads the control plane: {url}");
    }

    // (c) A normal repo screen (home) is STILL open to any tenant on a public
    // repo — only the admin/audit/erasure/overview plane became operator-only.
    let (s, _b) = route(
        &state,
        &Method::Get,
        "/v1/repos/acme/home",
        &bearer(&tok_other),
    );
    assert_eq!(s, 200, "public home still readable cross-tenant");
}

// W-ANON-V1: a repo READ no longer 401s on the credential — a missing/invalid Bearer
// degrades to the ANONYMOUS principal (empty chain), the JSON twin of the anon `git
// clone` door. The repo here (`hugit`, log `[]`) defaults to PRIVATE, so the anon
// read is denied by `authorize_read` → a uniform 404 (no existence oracle). The 401
// still guards the WRITE door, `/v1/me/*`, `/v1/admin/*`, and `/v1/token` (proven
// elsewhere). Both cases are a DENY — the change is only 401→404 on a private read.
#[test]
fn missing_bearer_private_read_is_404_no_oracle() {
    let (state, _d) = state_with_repo("hugit", "[]");
    let (status, body) = route(&state, &Method::Get, "/v1/repos/hugit/home", &[]);
    assert_eq!(status, 404, "no-Bearer read of a PRIVATE repo → anon → 404");
    assert!(
        body.contains("NOT_FOUND"),
        "uniform no-oracle 404 body: {body}"
    );
}

#[test]
fn wrong_bearer_private_read_is_404() {
    let (state, _d) = state_with_repo("hugit", "[]");
    let (status, body) = route(
        &state,
        &Method::Get,
        "/v1/repos/hugit/home",
        &bearer("WRONG"),
    );
    assert_eq!(
        status, 404,
        "a garbage Bearer degrades to anon (never a 401 on a read); PRIVATE → 404"
    );
    assert!(
        body.contains("NOT_FOUND"),
        "uniform no-oracle 404 body: {body}"
    );
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
fn tampered_log_is_404_not_503_for_a_non_operator() {
    use hugit_serve::token::ClerkPrincipal;
    // The existence/integrity-oracle fix (audit 2026-06-16): a load/verify failure
    // (503 with integrity detail) must NOT reveal to a non-operator that a repo
    // exists / is tampered — it gets a uniform 404, identical to a non-existent
    // repo. The OPERATOR still gets the honest 503 (the integrity signal).
    let bogus = r#"[{"seq":7,"kind":"pr.opened","payload":"{}","prev_hash":"deadbeef","this_hash":"00","recorded_at":1,"principal_chain":["x"]}]"#;
    let (state, _d) = state_with_repo("acme", bogus);
    let tok_b = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u-b".into(),
            org: "org-b".into(),
            fresh_auth: false,
        })
        .expect("mint b");

    // Non-operator tenant → 404 (no existence/integrity oracle), NOT 503.
    let (s, b) = route(&state, &Method::Get, "/v1/repos/acme/home", &bearer(&tok_b));
    assert_eq!(s, 404, "non-operator must get 404 on a tampered repo: {b}");
    assert!(b.contains("NOT_FOUND"));
    assert!(
        !b.contains("ENGINE_UNAVAILABLE"),
        "no integrity detail leaks: {b}"
    );
    // Operator (dev-token) STILL gets the honest 503 integrity signal.
    let (s, b) = route(&state, &Method::Get, "/v1/repos/acme/home", &bearer(TOKEN));
    assert_eq!(s, 503, "operator keeps the honest 503: {b}");
    assert!(b.contains("ENGINE_UNAVAILABLE"));
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

    // No token over the real socket → the read degrades to anon (W-ANON-V1); this
    // repo is PRIVATE (log `[]`), so the anon read is denied → 404 (no oracle), not a
    // 401. A read never 401s on the credential; the 401 stays on the write/me/token
    // doors.
    let r3 = http_get(&addr, "/v1/repos/hugit/home", None);
    assert!(
        r3.starts_with("HTTP/1.1 404"),
        "missing-bearer PRIVATE read → anon → 404: {r3}"
    );
}

#[test]
fn me_account_requires_auth_and_returns_usage_shape() {
    // The per-principal account read (githugr's account page consumes it).
    let (state, _d) = state_with_repo("hugit", "[]");
    // No Bearer → 401 (a private identity read is never anonymous).
    let (s, _b) = route(&state, &Method::Get, "/v1/me/account", &[]);
    assert_eq!(s, 401, "me/account requires a session Bearer");
    // With a Bearer (operator dev-token) → 200 + the structured usage/pats shape.
    let (s, b) = route(&state, &Method::Get, "/v1/me/account", &bearer(TOKEN));
    assert_eq!(s, 200, "authed me/account is 200: {b}");
    let v: serde_json::Value = serde_json::from_str(&b).unwrap();
    assert!(
        v["usage"]["repos_count"].is_u64(),
        "usage.repos_count present: {b}"
    );
    assert!(
        v["usage"]["log_footprint_bytes"].is_u64(),
        "usage.log_footprint_bytes present"
    );
    assert!(
        v["pats"].as_array().unwrap().is_empty(),
        "pats empty until the PAT store lands"
    );
}
