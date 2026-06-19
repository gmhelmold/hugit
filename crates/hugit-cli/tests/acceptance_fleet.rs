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
fn fleet_missing_log_obeys_the_error_law() {
    let (code, v) = run(&["fleet", "--log", "/no/such/fleet/log.json"]);
    assert_eq!(code, 2);
    assert_eq!(v["error"]["kind"], "log_not_found");
}
