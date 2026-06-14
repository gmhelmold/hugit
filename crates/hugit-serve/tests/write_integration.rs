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
