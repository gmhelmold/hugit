//! Acceptance — WP-PC1: `hugit campaign open / close / show`.
//!
//! Drives the REAL `hugit` binary (`CARGO_BIN_EXE_hugit`) over a hermetic
//! world file. The world is built from REAL projection inputs — a real event
//! log (un-hashed events the binary hash-chains through
//! `hugit_refstore::EventLog::append`) plus frozen `ContextEnvelope`s (the
//! WP-F1 shape) — never hand-faked sums; `close` drives the WP-F3
//! `campaign_rollup` over them, exactly as `hugit-ledger`'s F3 acceptance does.
//!
//! Proven here, per the DoD:
//! - `open` records `campaign.opened` (charter + human owner, D14) and is
//!   IDEMPOTENT (double-run → `already_exists:true`, no duplicate record);
//! - `close` REFUSES while a PR is in-flight (structured error + suggested
//!   fix), and on a settled campaign appends `campaign.closed` and prints the
//!   real F3 rollup (cost decomposition that adds up + progress) + honest
//!   envelope seal; `close` is idempotent too;
//! - `show` projects landed / in-flight / blocked + the PR list;
//! - every output is a single JSON object on stdout; errors are
//!   `{"error":{...}}` with a nonzero exit code.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-pc1-{tag}-{}-{}",
        std::process::id(),
        tag.len()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const CAMPAIGN: &str = "auth-hardening";
const T0: u64 = 1_760_000_000_000;

/// Run `hugit campaign <args>` and return `(exit_success, parsed_stdout_json)`.
fn run(args: &[&str]) -> (bool, Value) {
    let out = Command::new(hugit_bin())
        .arg("campaign")
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("stdout must be a single JSON object: {e}\nstdout was: {stdout}")
    });
    (out.status.success(), v)
}

// ─────────────────────────────────────────────────────────────────────────────
// World fixtures — real event log inputs + frozen envelopes
// ─────────────────────────────────────────────────────────────────────────────

/// An un-hashed event (the binary computes the hash chain).
fn event(kind: &str, payload: Value, at: u64) -> Value {
    json!({
        "kind": kind,
        "principal_chain": ["user:gustavo@humangr.com", "orchestrator:opus"],
        "payload": payload.to_string(),
        "recorded_at": at,
    })
}

fn pr_event(kind: &str, pr_id: &str, at: u64) -> Value {
    event(
        kind,
        json!({ "pr_id": pr_id, "campaign": CAMPAIGN, "union_verdict": "green" }),
        at,
    )
}

fn landed_intent(id: &str, at: u64) -> Value {
    event(
        "intent.landed",
        json!({
            "intent_id": id,
            "ref": "refs/heads/main",
            "target": format!("{:040x}", at),
            "charter": format!("harden auth — {id}"),
            "campaign": CAMPAIGN,
            "deep_link_target": id,
        }),
        at,
    )
}

fn metrics(tokens: u64, active_ms: u64, tool_calls: u64, turns: u64, cost: f64) -> Value {
    json!({
        "tokens": {
            "input": tokens / 2,
            "output": tokens - tokens / 2,
            "cache_read": 0,
            "cache_write": 0,
            "total": tokens,
        },
        "wall_ms": active_ms + 2_000,
        "active_ms": active_ms,
        "tool_calls": tool_calls,
        "tool_breakdown": [],
        "model_turns": turns,
        "cost_usd": cost,
    })
}

#[allow(clippy::too_many_arguments)]
fn envelope(
    altitude: &str,
    id: &str,
    model: &str,
    agent_type: &str,
    run_id: &str,
    parent: Option<&str>,
    born: u64,
    died: u64,
    m: Value,
) -> Value {
    json!({
        "schema_version": "1.0.0",
        "altitude": altitude,
        "intent_id": id,
        "commit": format!("{:040x}", 7u64),
        "tree_hash": format!("{:064x}", 7u64),
        "authorship": {
            "model": model,
            "model_digest": if model.is_empty() { String::new() } else { format!("{:064x}", 0xABCDu64) },
            "agent_type": agent_type,
            "spawn": {
                "run_id": run_id,
                "parent_run_id": parent,
                "born_at": born,
                "died_at": died,
            },
            "operator": "gustavo@humangr.com",
        },
        "charter": format!("harden auth — {id}"),
        "campaign": CAMPAIGN,
        "constraints": [],
        "acceptance": [],
        "parent_intents": [],
        "trajectory": {
            "raw_transcript_ref": null,
            "task_transcript_ref": null,
            "summary": null,
            "journal_ref": null,
            "redaction_policy": "default-v1",
        },
        "snapshot": { "files_read": [], "prompt_ref": null, "env_manifest": "rustc 1.96.0" },
        "metrics": m,
        "verdicts_ref": null,
    })
}

/// A world with two LANDED PRs (PR-1: 2 intents, PR-2: 1 intent) and full
/// envelope capture — the settled, fully-captured happy path.
fn settled_world() -> Value {
    json!({
        "events": [
            pr_event("pr.submitted", "PR-1", T0),
            pr_event("pr.submitted", "PR-2", T0 + 1_000),
            landed_intent("i-a1", T0 + 100_000),
            landed_intent("i-a2", T0 + 101_000),
            landed_intent("i-b1", T0 + 120_000),
            pr_event("pr.landed", "PR-1", T0 + 110_000),
            pr_event("pr.landed", "PR-2", T0 + 130_000),
        ],
        "envelopes": [
            // campaign-altitude (top-level, human-driven orchestrator).
            envelope("campaign", CAMPAIGN, "opus-4.8", "main", "orq-c", None,
                     T0, T0 + 130_000, metrics(5_000, 10_000, 10, 6, 0.20)),
            envelope("pr", "PR-1", "opus-4.8", "main", "orq-1", None,
                     T0, T0 + 110_000, metrics(20_000, 30_000, 40, 12, 0.50)),
            envelope("pr", "PR-2", "opus-4.8", "main", "orq-2", None,
                     T0 + 1_000, T0 + 130_000, metrics(10_000, 15_000, 20, 8, 0.25)),
            envelope("intent", "i-a1", "opus-4.8", "implementer", "run-a1", Some("orq-1"),
                     T0 + 10_000, T0 + 80_000, metrics(50_000, 60_000, 30, 8, 1.00)),
            envelope("intent", "i-a2", "opus-4.8", "implementer", "run-a2", Some("orq-1"),
                     T0 + 10_000, T0 + 90_000, metrics(45_000, 50_000, 25, 8, 0.90)),
            envelope("intent", "i-b1", "opus-4.8", "implementer", "run-b1", Some("orq-2"),
                     T0 + 20_000, T0 + 100_000, metrics(40_000, 55_000, 20, 8, 0.80)),
        ],
        "bundles": [
            { "pr_id": "PR-1", "intent_ids": ["i-a1", "i-a2"], "phase": "landed", "landed_at": T0 + 110_000 },
            { "pr_id": "PR-2", "intent_ids": ["i-b1"], "phase": "landed", "landed_at": T0 + 130_000 },
        ],
        "campaign_envelope_ref": "cas:campaign-envelope/auth-hardening",
    })
}

/// The same campaign with PR-2 STILL IN-FLIGHT (submitted, not settled).
fn in_flight_world() -> Value {
    json!({
        "events": [
            pr_event("pr.submitted", "PR-1", T0),
            pr_event("pr.submitted", "PR-2", T0 + 1_000),
            landed_intent("i-a1", T0 + 100_000),
            pr_event("pr.landed", "PR-1", T0 + 110_000),
            // PR-2 submitted but neither landed nor excluded → in-flight.
        ],
        "envelopes": [],
        "bundles": [],
        "campaign_envelope_ref": null,
    })
}

fn write_world(dir: &std::path::Path, world: &Value) -> PathBuf {
    let p = dir.join("world.json");
    std::fs::write(&p, world.to_string()).unwrap();
    p
}

/// Count `kind` records in a world file's event list.
fn count_kind(path: &std::path::Path, kind: &str) -> usize {
    let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    v["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == kind)
        .count()
}

// ─────────────────────────────────────────────────────────────────────────────
// open
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn open_records_campaign_opened_with_charter_and_human_owner() {
    let dir = scratch("open");
    let world = write_world(&dir, &json!({ "events": [] }));
    let log = world.to_str().unwrap();

    let (ok, v) = run(&[
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "harden the auth border",
        "--owner",
        "gustavo@humangr.com",
    ]);
    assert!(ok, "open exits 0");
    assert_eq!(v["campaign"], CAMPAIGN);
    assert_eq!(v["opened"], true);
    assert_eq!(v["already_exists"], false);
    assert_eq!(v["owner"], "gustavo@humangr.com");
    assert_eq!(
        count_kind(&world, "campaign.opened"),
        1,
        "one opened record"
    );
}

#[test]
fn open_is_idempotent_no_duplicate_record() {
    let dir = scratch("open-idem");
    let world = write_world(&dir, &json!({ "events": [] }));
    let log = world.to_str().unwrap();
    let args = [
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "c",
        "--owner",
        "gustavo@humangr.com",
    ];

    let (ok1, v1) = run(&args);
    assert!(ok1);
    assert_eq!(v1["already_exists"], false);

    // Second run with the SAME key → exit 0, already_exists, NO duplicate.
    let (ok2, v2) = run(&args);
    assert!(ok2, "re-open exits 0 (idempotent retry)");
    assert_eq!(v2["already_exists"], true);
    assert_eq!(v2["campaign"], CAMPAIGN);
    assert_eq!(
        count_kind(&world, "campaign.opened"),
        1,
        "idempotent: still exactly one opened record after double-run"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// close
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn close_refuses_while_a_pr_is_in_flight() {
    let dir = scratch("close-inflight");
    let world = write_world(&dir, &in_flight_world());
    let log = world.to_str().unwrap();

    let (ok, v) = run(&["close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(!ok, "close must exit nonzero while a PR is in-flight");
    assert_eq!(v["error"]["kind"], "in_flight_prs");
    assert_eq!(v["error"]["fix"], "land or abandon first");
    let in_flight = v["error"]["detail"]["in_flight"].as_array().unwrap();
    assert!(
        in_flight.iter().any(|p| p == "PR-2"),
        "the in-flight PR is named: {in_flight:?}"
    );
    assert_eq!(
        count_kind(&world, "campaign.closed"),
        0,
        "no seal record appended on a refused close"
    );
}

#[test]
fn close_seals_settled_campaign_with_real_f3_rollup() {
    let dir = scratch("close-seal");
    let world = write_world(&dir, &settled_world());
    let log = world.to_str().unwrap();

    let (ok, v) = run(&["close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(ok, "close exits 0 on a settled campaign: {v}");
    assert_eq!(v["closed"], true);
    assert_eq!(v["already_closed"], false);
    // Envelope seal — the ref is present, included honestly.
    assert_eq!(v["envelope"], "cas:campaign-envelope/auth-hardening");

    // Progress: both PRs landed.
    assert_eq!(v["progress"]["landed"], 2);
    assert_eq!(v["progress"]["in_flight"], 0);

    // The REAL F3 rollup is printed — and the decomposition ADDS UP:
    // total = work + orchestration + verification + ci (ci=0 here).
    let cost = &v["rollup"]["cost"];
    let work = cost["work"]["tokens"].as_u64().unwrap();
    let orch = cost["orchestration"]["tokens"].as_u64().unwrap();
    let verif = cost["verification"]["tokens"].as_u64().unwrap();
    assert_eq!(
        cost["total"]["tokens"].as_u64().unwrap(),
        work + orch + verif,
        "the seal's cost decomposition adds up"
    );
    // work = Σ landed intents (i-a1 + i-a2 + i-b1) = 50_000 + 45_000 + 40_000.
    assert_eq!(
        work, 135_000,
        "work is the projected landed spend, not faked"
    );
    // orchestration = Σ PR-author sessions = 20_000 + 10_000.
    assert_eq!(orch, 30_000);
    assert_eq!(v["rollup"]["progress"]["landed"], 2);
    assert_eq!(v["rollup"]["intent_count"], 3);

    // The seal record is on the log exactly once.
    assert_eq!(count_kind(&world, "campaign.closed"), 1);
}

#[test]
fn close_is_idempotent() {
    let dir = scratch("close-idem");
    let world = write_world(&dir, &settled_world());
    let log = world.to_str().unwrap();

    let (ok1, _) = run(&["close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(ok1);
    let (ok2, v2) = run(&["close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(ok2, "re-close exits 0");
    assert_eq!(v2["already_closed"], true);
    assert_eq!(
        count_kind(&world, "campaign.closed"),
        1,
        "idempotent: still one seal record after double-run"
    );
}

#[test]
fn close_seals_progress_only_when_no_envelope_captured() {
    // A settled campaign (both PRs landed) but NO captured envelopes → the
    // rollup is honestly null, the envelope seal "not_captured", still exits 0.
    let dir = scratch("close-nocap");
    let world = write_world(
        &dir,
        &json!({
            "events": [
                pr_event("pr.submitted", "PR-1", T0),
                landed_intent("i-a1", T0 + 100_000),
                pr_event("pr.landed", "PR-1", T0 + 110_000),
            ],
        }),
    );
    let log = world.to_str().unwrap();

    let (ok, v) = run(&["close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(ok, "close seals progress-only without faking a rollup");
    assert_eq!(v["envelope"], "not_captured", "honest envelope seal");
    assert_eq!(v["rollup"], Value::Null, "no fabricated rollup");
    assert_eq!(v["progress"]["landed"], 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// show
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn show_projects_landed_in_flight_blocked_and_pr_list() {
    let dir = scratch("show");
    let world = write_world(&dir, &in_flight_world());
    let log = world.to_str().unwrap();

    let (ok, v) = run(&["show", "--log", log, "--campaign", CAMPAIGN]);
    assert!(ok, "show is read-only and exits 0: {v}");
    assert_eq!(v["progress"]["landed"], 1);
    assert_eq!(v["progress"]["in_flight"], 1);
    assert_eq!(v["progress"]["blocked"], 0);

    let prs = v["prs"].as_array().unwrap();
    assert_eq!(prs.len(), 2);
    let pr1 = prs.iter().find(|p| p["pr_id"] == "PR-1").unwrap();
    assert_eq!(pr1["phase"], "landed");
    let pr2 = prs.iter().find(|p| p["pr_id"] == "PR-2").unwrap();
    assert_eq!(pr2["phase"], "in_flight");

    // show never writes.
    assert_eq!(count_kind(&world, "campaign.closed"), 0);
}

#[test]
fn show_surfaces_rollup_summary_when_captured() {
    let dir = scratch("show-rollup");
    let world = write_world(&dir, &settled_world());
    let log = world.to_str().unwrap();

    let (ok, v) = run(&["show", "--log", log, "--campaign", CAMPAIGN]);
    assert!(ok);
    assert_eq!(v["rollup"]["intent_count"], 3);
    assert_eq!(v["rollup"]["pr_count"], 2);
    // headline cost surfaced (real, computed).
    assert!(v["rollup"]["tokens"].as_u64().unwrap() > 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// D14 — a subagent-authored campaign envelope is refused, fail-closed
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn close_refuses_subagent_authored_campaign_envelope() {
    let dir = scratch("close-d14");
    let mut world = settled_world();
    // Corrupt the campaign envelope's authorship into a subagent (spawned).
    let envs = world["envelopes"].as_array_mut().unwrap();
    for e in envs.iter_mut() {
        if e["altitude"] == "campaign" {
            e["authorship"]["agent_type"] = json!("implementer");
            e["authorship"]["spawn"]["parent_run_id"] = json!("orq-boss");
        }
    }
    let path = write_world(&dir, &world);
    let log = path.to_str().unwrap();

    let (ok, v) = run(&["close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(!ok, "a subagent-authored campaign must be refused (D14)");
    assert_eq!(v["error"]["kind"], "subagent_author");
    assert_eq!(count_kind(&path, "campaign.closed"), 0);
}
