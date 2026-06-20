//! Acceptance — `hugit fleet` (Phase-D machine-readable fleet state).
//!
//! Drives the REAL binary over a REAL chained log built through
//! `EventLog::append_for_test`. Pins: the versioned `FleetState` schema is
//! projected from `ws.state.*` / `agent.*` events (reusing `hugit_ledger`'s own
//! fold), and the WB0 one-error/one-exit law (missing log ⇒ exit 2).

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::{Value, json};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-fleet-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

fn write_log(path: &Path, events: &[(&str, Value)]) {
    let mut log = EventLog::new();
    for (kind, payload) in events {
        log.append_for_test(*kind, vec![], payload.to_string(), 0);
    }
    std::fs::write(path, serde_json::to_string_pretty(log.records()).unwrap()).unwrap();
}

#[test]
fn fleet_projects_workspaces_and_agents_from_the_log() {
    let dir = scratch("project");
    let log = dir.join("log.json");
    write_log(
        &log,
        &[
            ("ws.state.active", json!({"workspace_id":"ws1"})),
            (
                "agent.assigned",
                json!({"agent_id":"a1","workspace_id":"ws1"}),
            ),
            ("agent.completed", json!({"agent_id":"a1"})),
        ],
    );

    let (code, v) = run(&["fleet", "--log", log.to_str().unwrap()]);
    assert_eq!(code, 0, "fleet exits 0 on a valid log: {v}");

    assert!(v["schema_version"].is_string(), "versioned schema: {v}");
    assert_eq!(v["event_count"], 3);

    let workspaces = v["workspaces"].as_array().unwrap();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0]["workspace_id"], "ws1");
    assert_eq!(workspaces[0]["state"], "active");

    let agents = v["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["agent_id"], "a1");
    assert_eq!(agents[0]["workspace_id"], "ws1");
    assert_eq!(agents[0]["state"], "completed");
}

#[test]
fn fleet_is_honestly_empty_on_a_log_without_fleet_events() {
    let dir = scratch("empty");
    let log = dir.join("log.json");
    write_log(
        &log,
        &[("journal.note", json!({"note":"no fleet activity"}))],
    );

    let (code, v) = run(&["fleet", "--log", log.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(v["workspaces"].as_array().unwrap().is_empty());
    assert!(v["agents"].as_array().unwrap().is_empty());
    assert_eq!(v["event_count"], 1);
}

#[test]
fn fleet_distinct_secret_shaped_agents_do_not_collide_and_are_redacted() {
    let dir = scratch("secret-collision");
    let log = dir.join("log.json");
    // Two DISTINCT agents whose ids are secret-shaped (both redact to the SAME
    // marker). Before the raw-key fold fix they collapsed to ONE `[REDACTED]` map
    // key — the second `assigned` overwrote the first and the `completed` landed
    // on the wrong (single) entry. Now keyed on the raw id, redacted at emission.
    write_log(
        &log,
        &[
            ("ws.state.active", json!({"workspace_id":"SECRET:ws-alpha"})),
            (
                "agent.assigned",
                json!({"agent_id":"SECRET:agent-alpha-key","workspace_id":"SECRET:ws-alpha"}),
            ),
            (
                "agent.assigned",
                json!({"agent_id":"SECRET:agent-beta-key","workspace_id":"SECRET:ws-alpha"}),
            ),
            (
                "agent.completed",
                json!({"agent_id":"SECRET:agent-alpha-key"}),
            ),
        ],
    );

    let (code, v) = run(&["fleet", "--log", log.to_str().unwrap()]);
    assert_eq!(code, 0, "fleet exits 0: {v}");

    // Collision fix: TWO distinct agents survive (were collapsed to 1 before).
    let agents = v["agents"].as_array().unwrap();
    assert_eq!(
        agents.len(),
        2,
        "distinct secret-shaped agents must not collide to one key: {v}"
    );

    // Per-agent state preserved (no cross-entry overwrite): one completed, one assigned.
    let states: std::collections::BTreeSet<&str> = agents
        .iter()
        .map(|a| a["state"].as_str().unwrap())
        .collect();
    assert!(
        states.contains("completed") && states.contains("assigned"),
        "per-agent state preserved across the two entries: {v}"
    );

    // Verb-boundary redaction: no raw secret body appears anywhere in the output.
    let raw = serde_json::to_string(&v).unwrap();
    for secret in ["agent-alpha-key", "agent-beta-key", "ws-alpha"] {
        assert!(
            !raw.contains(secret),
            "secret `{secret}` must be redacted at the view boundary: {raw}"
        );
    }
}

#[test]
fn fleet_missing_log_obeys_the_error_law() {
    let (code, v) = run(&["fleet", "--log", "/no/such/fleet/log.json"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["kind"], "log_not_found");
}
