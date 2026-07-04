//! End-to-end integration test for the `/v1` WRITE path, via the socket-free
//! [`hugit_serve::server::route_with_body`] over a real on-disk Local source.
//!
//! Proves the write transport law (backend-API-v1 §3) over the real door +
//! LogSink + a real verb: a POST with a Bearer + `Idempotency-Key` lands the
//! event AND persists it (200 `{accepted:true,…}`); a replay of the same key is
//! byte-identical and appends NOTHING new (the land one-position invariant,
//! end-to-end through load→mutate→persist→reload); a missing key → 400; a
//! step-up verb without fresh auth → 403; a wrong Bearer → 401.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use hugit_refstore::EventLog;
use hugit_serve::server::route_with_body;
use hugit_serve::state::AppState;
use tiny_http::{Header, Method};

const TOKEN: &str = "dev-token-abc";

/// A temp log dir unique per call (atomic counter — no clock-collision flake).
fn scratch_dir() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "hugit-serve-wr-{}-{nanos}-{seq}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A Local-source state whose `hugit.json` already carries a chain-valid
/// `pr.opened` for PR `1` (so `land`/`verdict`/`comment` have a target).
fn state_with_open_pr() -> (AppState, PathBuf) {
    let dir = scratch_dir();
    let mut log = EventLog::new();
    let payload = hugit_refstore::canonical_json(
        &serde_json::json!({
            "author_kind":"orchestrator","campaign":"c","intent_ids":["i"],
            "pr_id":"1","principal":null,"run_id":"r"
        })
        .to_string(),
    )
    .unwrap();
    log.append_for_test("pr.opened", vec!["orchestrator:t".into()], payload, 1);
    let bytes = serde_json::to_vec(log.records()).unwrap();
    std::fs::write(dir.join("hugit.json"), bytes).unwrap();
    (AppState::new(dir.clone(), TOKEN.to_string()), dir)
}

fn hdr(name: &str, val: &str) -> Header {
    Header::from_bytes(name.as_bytes(), val.as_bytes()).unwrap()
}

fn post(state: &AppState, url: &str, headers: &[Header], body: &[u8]) -> (u16, String) {
    route_with_body(state, &Method::Post, url, headers, body)
}

#[test]
fn cross_tenant_write_is_denied_owner_allowed() {
    use hugit_serve::token::ClerkPrincipal;
    // A private repo "acme" owned by org-a, with an open PR 1 to land.
    let dir = scratch_dir();
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility":"private","owner_tenant":"org-a"}).to_string(),
        0,
    );
    let pr = hugit_refstore::canonical_json(
        &serde_json::json!({"author_kind":"orchestrator","campaign":"c","intent_ids":["i"],"pr_id":"1","principal":null,"run_id":"r"}).to_string(),
    )
    .unwrap();
    log.append_for_test("pr.opened", vec!["orchestrator:t".into()], pr, 1);
    std::fs::write(
        dir.join("acme.json"),
        serde_json::to_vec(log.records()).unwrap(),
    )
    .unwrap();
    let state = AppState::new(dir.clone(), TOKEN.to_string());

    let tok_a = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u-a".into(),
            org: "org-a".into(),
            fresh_auth: false,
        })
        .unwrap();
    let tok_b = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u-b".into(),
            org: "org-b".into(),
            fresh_auth: false,
        })
        .unwrap();
    let land = |tok: &str, key: &str| {
        post(
            &state,
            "/v1/repos/acme/prs/1/land",
            &[
                hdr("Authorization", &format!("Bearer {tok}")),
                hdr("Idempotency-Key", key),
            ],
            br#"{"mode":"union"}"#,
        )
    };

    // Cross-tenant (org-b) WRITE → 404, denied BEFORE any effect (the P0 fix).
    let (s, b) = land(&tok_b, "kb");
    assert_eq!(s, 404, "cross-tenant write must be denied: {b}");
    assert!(b.contains("NOT_FOUND"));
    // The owning tenant (org-a) → 200 (not a blanket deny).
    let (s, b) = land(&tok_a, "ka");
    assert_eq!(s, 200, "owning tenant can write: {b}");
    // The cross-tenant write left NO trace on the log.
    let recs: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(dir.join("acme.json")).unwrap()).unwrap();
    let from_b = recs.iter().any(|r| {
        r["principal_chain"]
            .as_array()
            .is_some_and(|c| c.iter().any(|p| p == "clerk:org-b:u-b"))
    });
    assert!(!from_b, "no org-b record may exist on org-a's repo");
}

#[test]
fn write_on_a_public_repo_is_denied_to_a_non_owner() {
    use hugit_serve::token::ClerkPrincipal;
    // A PUBLIC repo owned by org-a. Reads are open to all; writes are NOT — only
    // the owner/operator. This is the public-write hole: the read gate would let
    // any tenant write; `authorize_write` must deny org-b end-to-end.
    let dir = scratch_dir();
    let mut log = EventLog::new();
    log.append_for_test(
        "repo.meta",
        vec![],
        serde_json::json!({"visibility":"public","owner_tenant":"org-a"}).to_string(),
        0,
    );
    let pr = hugit_refstore::canonical_json(
        &serde_json::json!({"author_kind":"orchestrator","campaign":"c","intent_ids":["i"],"pr_id":"1","principal":null,"run_id":"r"}).to_string(),
    )
    .unwrap();
    log.append_for_test("pr.opened", vec!["orchestrator:t".into()], pr, 1);
    std::fs::write(
        dir.join("oss.json"),
        serde_json::to_vec(log.records()).unwrap(),
    )
    .unwrap();
    let state = AppState::new(dir.clone(), TOKEN.to_string());

    let tok_b = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u-b".into(),
            org: "org-b".into(),
            fresh_auth: false,
        })
        .unwrap();
    let tok_a = state
        .token_store
        .mint(&ClerkPrincipal {
            user: "u-a".into(),
            org: "org-a".into(),
            fresh_auth: false,
        })
        .unwrap();
    let land = |tok: &str, key: &str| {
        post(
            &state,
            "/v1/repos/oss/prs/1/land",
            &[
                hdr("Authorization", &format!("Bearer {tok}")),
                hdr("Idempotency-Key", key),
            ],
            br#"{"mode":"union"}"#,
        )
    };

    // A non-owner tenant WRITE to a PUBLIC repo → 404 (the hole, closed).
    let (s, b) = land(&tok_b, "pb");
    assert_eq!(
        s, 404,
        "non-owner write to a public repo must be denied: {b}"
    );
    assert!(b.contains("NOT_FOUND"));
    // The owning tenant writes fine (public didn't lock the owner out).
    let (s, b) = land(&tok_a, "pa");
    assert_eq!(s, 200, "owner can write its public repo: {b}");
    // No org-b trace on the log.
    let recs: Vec<serde_json::Value> =
        serde_json::from_slice(&std::fs::read(dir.join("oss.json")).unwrap()).unwrap();
    assert!(
        !recs.iter().any(|r| r["principal_chain"]
            .as_array()
            .is_some_and(|c| c.iter().any(|p| p == "clerk:org-b:u-b"))),
        "no org-b record may exist on a public repo it doesn't own"
    );
}

#[test]
fn comment_route_is_plural_comments() {
    // githugr CORRECTION 2026-06-15: the comment route is `/prs/{pr}/comments`
    // (PLURAL), NOT `/comment`. Prove the engine serves the plural form (200) and
    // the old singular `/comment` is NOT a route (404).
    let (state, _d) = state_with_open_pr();
    let headers = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", "C1"),
    ];
    let (status, body) = post(
        &state,
        "/v1/repos/hugit/prs/1/comments",
        &headers,
        br#"{"body":"looks good"}"#,
    );
    assert_eq!(status, 200, "plural /comments must route: {body}");

    let headers2 = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", "C2"),
    ];
    let (singular, _b) = post(
        &state,
        "/v1/repos/hugit/prs/1/comment",
        &headers2,
        br#"{"body":"x"}"#,
    );
    assert_eq!(singular, 404, "singular /comment must NOT be a route");
}

#[test]
fn edit_propose_route_is_path_scoped_multi_segment() {
    // githugr CORRECTION 2026-06-15: edit_propose is `/edit/{*path}/propose` (the
    // file path is IN the route, suffix `/propose`), NOT a bare `/edit`. Prove a
    // MULTI-SEGMENT path routes (200) and the bare/suffix-less forms are NOT routes.
    let (state, _d) = state_with_open_pr();
    let headers = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", "E1"),
    ];
    let (status, body) = post(
        &state,
        "/v1/repos/hugit/edit/src/foo/bar.rs/propose",
        &headers,
        br#"{"content":"fn main() {}","title":"edit bar"}"#,
    );
    assert_eq!(
        status, 200,
        "multi-segment /edit/.../propose must route: {body}"
    );

    // Bare `/edit` and a path WITHOUT the `/propose` suffix are NOT routes.
    for bad in ["/v1/repos/hugit/edit", "/v1/repos/hugit/edit/src/foo.rs"] {
        let h = vec![
            hdr("Authorization", &format!("Bearer {TOKEN}")),
            hdr("Idempotency-Key", "E2"),
        ];
        let (s, _b) = post(&state, bad, &h, br#"{"content":"x","title":"t"}"#);
        assert_eq!(s, 404, "non-propose edit shape must NOT route: {bad}");
    }
}

#[test]
fn land_with_key_is_200_accepted_and_persists() {
    let (state, _d) = state_with_open_pr();
    let headers = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", "K1"),
    ];
    let (status, body) = post(
        &state,
        "/v1/repos/hugit/prs/1/land",
        &headers,
        br#"{"mode":"union"}"#,
    );
    assert_eq!(status, 200, "body={body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["accepted"], serde_json::Value::Bool(true));
    assert_eq!(v["queue_pos"], 1);
    assert_eq!(v["pr_number"], 1);
}

#[test]
fn replay_same_key_appends_nothing_new_one_position() {
    let (state, dir) = state_with_open_pr();
    let headers = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", "K1"),
    ];
    let body = br#"{"mode":"union"}"#;
    let (s1, b1) = post(&state, "/v1/repos/hugit/prs/1/land", &headers, body);
    assert_eq!(s1, 200);
    // Replay (lost-response retry): same key + body → byte-identical, NO new queue pos.
    let (s2, b2) = post(&state, "/v1/repos/hugit/prs/1/land", &headers, body);
    assert_eq!(s2, 200);
    assert_eq!(b1, b2, "replay must be byte-identical");
    // The persisted log must hold EXACTLY ONE pr.queued (one land, one position).
    let persisted = std::fs::read(dir.join("hugit.json")).unwrap();
    let recs: Vec<serde_json::Value> = serde_json::from_slice(&persisted).unwrap();
    let queued = recs.iter().filter(|r| r["kind"] == "pr.queued").count();
    assert_eq!(queued, 1, "exactly ONE pr.queued despite the replay");
}

#[test]
fn same_key_different_body_is_409() {
    let (state, _d) = state_with_open_pr();
    let headers = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", "K1"),
    ];
    post(
        &state,
        "/v1/repos/hugit/prs/1/land",
        &headers,
        br#"{"mode":"union"}"#,
    );
    let (status, _b) = post(
        &state,
        "/v1/repos/hugit/prs/1/land",
        &headers,
        br#"{"mode":"serial"}"#,
    );
    assert_eq!(status, 409);
}

#[test]
fn missing_idempotency_key_is_400() {
    let (state, _d) = state_with_open_pr();
    let headers = vec![hdr("Authorization", &format!("Bearer {TOKEN}"))];
    let (status, body) = post(
        &state,
        "/v1/repos/hugit/prs/1/land",
        &headers,
        br#"{"mode":"union"}"#,
    );
    assert_eq!(status, 400);
    assert!(body.contains("IDEMPOTENCY_REQUIRED"));
}

#[test]
fn step_up_verb_without_fresh_auth_is_403() {
    let (state, _d) = state_with_open_pr();
    let headers = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", "K1"),
    ];
    // `policy` is step-up-gated; no X-Step-Up header → 403 before any effect.
    let (status, body) = post(
        &state,
        "/v1/repos/hugit/policy",
        &headers,
        br#"{"rule_id":"dco","enabled":false}"#,
    );
    assert_eq!(status, 403);
    assert!(body.contains("STEP_UP_REQUIRED"));
    // WITH the step-up header, it lands.
    let headers2 = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", "K2"),
        hdr("X-Step-Up", "true"),
    ];
    let (s2, _b2) = post(
        &state,
        "/v1/repos/hugit/policy",
        &headers2,
        br#"{"rule_id":"dco","enabled":false}"#,
    );
    assert_eq!(s2, 200);
}

#[test]
fn wrong_bearer_is_401() {
    let (state, _d) = state_with_open_pr();
    let headers = vec![
        hdr("Authorization", "Bearer WRONG"),
        hdr("Idempotency-Key", "K1"),
    ];
    let (status, _b) = post(
        &state,
        "/v1/repos/hugit/prs/1/land",
        &headers,
        br#"{"mode":"union"}"#,
    );
    assert_eq!(status, 401);
}

#[test]
fn invalid_body_is_400() {
    let (state, _d) = state_with_open_pr();
    let headers = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", "K1"),
    ];
    let (status, _b) = post(&state, "/v1/repos/hugit/prs/1/land", &headers, b"not json");
    assert_eq!(status, 400);
}

#[test]
fn idempotency_key_over_cap_is_rejected() {
    // An Idempotency-Key longer than 256 bytes must be rejected with 400
    // INVALID_REQUEST before anything is persisted (DoS / R2 log-bloat guard).
    let (state, _d) = state_with_open_pr();
    let long_key = "x".repeat(257);
    let headers = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", &long_key),
    ];
    let (status, body) = post(
        &state,
        "/v1/repos/hugit/prs/1/land",
        &headers,
        br#"{"mode":"union"}"#,
    );
    assert_eq!(status, 400, "an over-cap key must be rejected: {body}");
    assert!(
        body.contains("INVALID_REQUEST"),
        "error code must be INVALID_REQUEST: {body}"
    );

    // A key exactly at the cap (256 bytes) must be accepted.
    let exact_key = "y".repeat(256);
    let headers2 = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", &exact_key),
    ];
    let (status2, body2) = post(
        &state,
        "/v1/repos/hugit/prs/1/land",
        &headers2,
        br#"{"mode":"union"}"#,
    );
    assert_eq!(status2, 200, "a 256-byte key must be accepted: {body2}");
}

// ── GDPR1 POST /v1/account/erase — top-level route wiring ────────────────────

#[test]
fn account_erase_operator_is_refused_no_god_erase() {
    // The dev-token resolves to the OPERATOR (allow_dev_operator default-on in the
    // test constructor). The top-level erase route reaches the handler, which derives
    // the subject from the principal and REFUSES the operator → 401 (no god-erase).
    let (state, _dir) = state_with_open_pr();
    let headers = vec![
        hdr("Authorization", &format!("Bearer {TOKEN}")),
        hdr("Idempotency-Key", "E1"),
        hdr("X-Step-Up", "true"),
    ];
    let (status, body) = post(
        &state,
        "/v1/account/erase",
        &headers,
        br#"{"confirm":"hugit"}"#,
    );
    assert_eq!(status, 401, "operator cannot erase an account: {body}");
    assert!(
        body.contains("UNAUTHORIZED"),
        "code must be UNAUTHORIZED: {body}"
    );
}

#[test]
fn account_erase_anonymous_is_refused() {
    // No Bearer → the write path 401s at auth, before any effect (a write is never
    // anonymous; there is no anon-erase).
    let (state, _dir) = state_with_open_pr();
    let headers = vec![hdr("Idempotency-Key", "E2")];
    let (status, _body) = post(&state, "/v1/account/erase", &headers, br#"{"confirm":"x"}"#);
    assert_eq!(status, 401, "anonymous cannot erase an account");
}
