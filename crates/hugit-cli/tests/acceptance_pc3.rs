//! WP-PC3 acceptance — `hugit pr open | land | show`.
//!
//! Every test drives the verbs over the REAL event log
//! ([`hugit_refstore::EventLog`]) and, for `land`, the REAL union-testing
//! landing queue ([`hugit_contracts::LandableEntry`] folded through
//! [`hugit_queue::core::Batch`] inside `pr::land`). No hand-faked queue or
//! projection state: open appends `pr.opened`, land appends `pr.queued`,
//! show projects them back out, and the F3 cost block is computed by the real
//! `pr_record` rollup from a captured `pr.envelope` on the log.
//!
//! Idempotency is proven by double-run tests for BOTH `open` and `land`
//! (re-run → no second event, `already_*: true`, identical position).

use hugit_cli::pr::{
    self, AuthorKind, INTENT_ENVELOPE_KIND, LandArgs, OpenArgs, PR_ENVELOPE_KIND, PR_OPENED_KIND,
    PR_QUEUED_KIND, ShowArgs,
};
use hugit_contracts::context_envelope::{
    Altitude, Authorship, ContextEnvelope, IntentMetrics, Snapshot, Spawn, TokenCounts, Trajectory,
};
use hugit_refstore::EventLog;
use serde_json::{Value, json};

// ── fixtures ──────────────────────────────────────────────────────────────────

fn open_args(pr: &str, campaign: &str, intents: &[&str]) -> OpenArgs {
    OpenArgs {
        pr_id: pr.to_string(),
        campaign: campaign.to_string(),
        author_kind: AuthorKind::Orchestrator,
        run_id: Some("run-orch".to_string()),
        principal: None,
        intent_ids: intents.iter().map(|s| s.to_string()).collect(),
        recorded_at: 1_000,
    }
}

fn count_kind(log: &EventLog, kind: &str) -> usize {
    log.records().iter().filter(|r| r.kind == kind).count()
}

// ── open ────────────────────────────────────────────────────────────────────

#[test]
fn open_bundles_intents_into_a_proposed_pr() {
    let mut log = EventLog::new();
    let out = pr::open(&mut log, &open_args("7", "camp-a", &["i1", "i2"])).unwrap();

    assert_eq!(out["pr_id"], json!("7"));
    assert_eq!(out["campaign"], json!("camp-a"));
    assert_eq!(out["author_kind"], json!("orchestrator"));
    assert_eq!(out["state"], json!("proposed"));
    assert_eq!(out["intent_count"], json!(2));
    assert_eq!(out["already_exists"], json!(false));
    // The real append path fired exactly once.
    assert_eq!(count_kind(&log, PR_OPENED_KIND), 1);
}

#[test]
fn open_rejects_subagent_author_kind_at_the_door() {
    // D14: `subagent` is not a parseable AuthorKind — the porcelain rejects it
    // before any event is appended, with the structured fix-carrying error.
    assert_eq!(AuthorKind::parse("subagent"), None);

    let err = pr::PrError::SubagentAuthor {
        got: "subagent".to_string(),
    };
    // Canonical WB0 error law: nested under `error`, `fix`-keyed, context flat.
    let j = err.to_json();
    assert_eq!(j["error"]["kind"], json!("subagent_author"));
    assert!(
        j["error"]["message"]
            .as_str()
            .unwrap()
            .contains("never a subagent")
    );
    assert!(
        j["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("--author-kind")
    );
    assert_eq!(j["error"]["got"], json!("subagent"));
}

#[test]
fn open_is_idempotent_double_run_same_campaign() {
    let mut log = EventLog::new();
    let first = pr::open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();
    assert_eq!(first["already_exists"], json!(false));
    let n = log.len();

    // Re-run with the same id + campaign: no second event, already_exists:true.
    let again = pr::open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();
    assert_eq!(again["already_exists"], json!(true));
    assert_eq!(again["intent_count"], json!(1));
    assert_eq!(log.len(), n, "idempotent re-open appends NO second event");
    assert_eq!(count_kind(&log, PR_OPENED_KIND), 1);
}

#[test]
fn open_rejects_campaign_mismatch_on_reopen() {
    let mut log = EventLog::new();
    pr::open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();
    let err = pr::open(&mut log, &open_args("7", "camp-b", &["i1"])).unwrap_err();
    assert_eq!(err.code(), "campaign_mismatch");
    let j = err.to_json();
    assert_eq!(j["error"]["existing_campaign"], json!("camp-a"));
    assert_eq!(j["error"]["attempted_campaign"], json!("camp-b"));
}

// ── land ────────────────────────────────────────────────────────────────────

#[test]
fn land_enters_the_real_queue_and_reports_position() {
    let mut log = EventLog::new();
    // Two PRs opened; queue them in order — positions come from the queue path.
    pr::open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();
    pr::open(&mut log, &open_args("8", "camp-a", &["i2"])).unwrap();

    let first = pr::land(
        &mut log,
        &LandArgs {
            pr_id: "7".to_string(),
            recorded_at: 2_000,
        },
    )
    .unwrap();
    assert_eq!(first["queued"], json!(true));
    assert_eq!(first["already_queued"], json!(false));
    assert_eq!(first["position"], json!(0));
    assert_eq!(first["mode"], json!("union"));

    let second = pr::land(
        &mut log,
        &LandArgs {
            pr_id: "8".to_string(),
            recorded_at: 2_001,
        },
    )
    .unwrap();
    assert_eq!(
        second["position"],
        json!(1),
        "second PR lands behind the first"
    );
    assert_eq!(count_kind(&log, PR_QUEUED_KIND), 2);
}

#[test]
fn land_refuses_empty_pr() {
    let mut log = EventLog::new();
    pr::open(&mut log, &open_args("7", "camp-a", &[])).unwrap();
    let err = pr::land(
        &mut log,
        &LandArgs {
            pr_id: "7".to_string(),
            recorded_at: 2_000,
        },
    )
    .unwrap_err();
    assert_eq!(err.code(), "empty_pr");
    assert_eq!(count_kind(&log, PR_QUEUED_KIND), 0);
}

#[test]
fn land_refuses_unknown_pr() {
    let mut log = EventLog::new();
    let err = pr::land(
        &mut log,
        &LandArgs {
            pr_id: "404".to_string(),
            recorded_at: 2_000,
        },
    )
    .unwrap_err();
    assert_eq!(err.code(), "unknown_pr");
}

#[test]
fn land_is_idempotent_double_run() {
    let mut log = EventLog::new();
    pr::open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();
    let first = pr::land(
        &mut log,
        &LandArgs {
            pr_id: "7".to_string(),
            recorded_at: 2_000,
        },
    )
    .unwrap();
    assert_eq!(first["already_queued"], json!(false));
    let n = log.len();

    // Re-run land on the same PR: no second pr.queued, already_queued:true,
    // identical position.
    let again = pr::land(
        &mut log,
        &LandArgs {
            pr_id: "7".to_string(),
            recorded_at: 2_002,
        },
    )
    .unwrap();
    assert_eq!(again["already_queued"], json!(true));
    assert_eq!(
        again["position"], first["position"],
        "position is stable across replay"
    );
    assert_eq!(log.len(), n, "idempotent re-land appends NO second event");
    assert_eq!(count_kind(&log, PR_QUEUED_KIND), 1);
}

// ── show ────────────────────────────────────────────────────────────────────

#[test]
fn show_reports_intents_and_queue_state() {
    let mut log = EventLog::new();
    pr::open(&mut log, &open_args("7", "camp-a", &["i1", "i2"])).unwrap();

    // Before landing: queued:false, cost honestly null (no envelope captured).
    let before = pr::show(
        &log,
        &ShowArgs {
            pr_id: "7".to_string(),
        },
    )
    .unwrap();
    assert_eq!(before["intent_count"], json!(2));
    assert_eq!(before["intent_ids"], json!(["i1", "i2"]));
    assert_eq!(before["queue"]["queued"], json!(false));
    assert_eq!(before["cost"], Value::Null);

    // After landing: queue state reflects the real position.
    pr::land(
        &mut log,
        &LandArgs {
            pr_id: "7".to_string(),
            recorded_at: 2_000,
        },
    )
    .unwrap();
    let after = pr::show(
        &log,
        &ShowArgs {
            pr_id: "7".to_string(),
        },
    )
    .unwrap();
    assert_eq!(after["queue"]["queued"], json!(true));
    assert_eq!(after["queue"]["position"], json!(0));
    assert_eq!(after["queue"]["mode"], json!("union"));
}

#[test]
fn show_refuses_unknown_pr() {
    let log = EventLog::new();
    let err = pr::show(
        &log,
        &ShowArgs {
            pr_id: "404".to_string(),
        },
    )
    .unwrap_err();
    assert_eq!(err.code(), "pr_not_found");
}

#[test]
fn show_surfaces_the_f3_pr_record_when_envelope_is_captured() {
    let mut log = EventLog::new();
    pr::open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();

    // Capture a PR-altitude envelope + its intent's envelope on the REAL log.
    append_envelope(&mut log, pr_envelope("7", "camp-a"));
    append_envelope(&mut log, intent_envelope("i1"));

    let shown = pr::show(
        &log,
        &ShowArgs {
            pr_id: "7".to_string(),
        },
    )
    .unwrap();
    let cost = &shown["cost"];
    assert!(
        !cost.is_null(),
        "F3 pr_record present once the envelope is captured"
    );
    assert_eq!(cost["pr_id"], json!("7"));
    // The rollup's author block enforces D14 again at the projection level.
    assert_eq!(cost["author"]["kind"], json!("orchestrator"));
    // Work was computed from the real intent-altitude envelope (1000 tokens).
    assert_eq!(cost["cost"]["work"]["tokens"], json!(1_000));
    // Orchestration is the PR session's own spend (500 tokens).
    assert_eq!(cost["cost"]["orchestration"]["tokens"], json!(500));
    // Honest gaps: CI has no porcelain seam → zero, never faked.
    assert_eq!(cost["cost"]["ci"]["exec"], json!(0));
}

#[test]
fn show_subagent_authored_pr_envelope_yields_null_cost() {
    // A subagent-authored PR-altitude envelope is rejected by the rollup
    // (D14, fail-closed) → the cost block is honestly null, never faked green.
    let mut log = EventLog::new();
    pr::open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();

    let mut env = pr_envelope("7", "camp-a");
    env.authorship.agent_type = "implementer".to_string(); // subagent marker
    append_envelope(&mut log, env);

    let shown = pr::show(
        &log,
        &ShowArgs {
            pr_id: "7".to_string(),
        },
    )
    .unwrap();
    assert_eq!(shown["cost"], Value::Null);
}

// ── envelope helpers (real ContextEnvelope, appended to the real log) ───────────

fn append_envelope(log: &mut EventLog, env: ContextEnvelope) {
    let kind = match env.altitude {
        Altitude::Pr => PR_ENVELOPE_KIND,
        Altitude::Intent => INTENT_ENVELOPE_KIND,
        _ => panic!("test only captures pr/intent envelopes"),
    };
    let payload = serde_json::to_string(&env).unwrap();
    let canonical = hugit_refstore::canonical_json(&payload).unwrap();
    log.append_for_test(kind, vec![], canonical, 3_000);
}

fn metrics(total: u64) -> IntentMetrics {
    IntentMetrics {
        tokens: TokenCounts {
            input: total,
            output: 0,
            cache_read: 0,
            cache_write: 0,
            total,
        },
        wall_ms: 0,
        active_ms: 0,
        tool_calls: 0,
        tool_breakdown: vec![],
        model_turns: 0,
        cost_usd_micros: 0,
    }
}

fn authorship(agent_type: &str, parent: Option<&str>, run_id: &str) -> Authorship {
    Authorship {
        model: "opus-4.8".to_string(),
        model_digest: "sha256:dead".to_string(),
        agent_type: agent_type.to_string(),
        spawn: Spawn {
            run_id: run_id.to_string(),
            parent_run_id: parent.map(str::to_string),
            born_at: 1_000,
            died_at: 2_000,
        },
        operator: "gustavo".to_string(),
    }
}

fn envelope(id: &str, altitude: Altitude, authorship: Authorship, total: u64) -> ContextEnvelope {
    ContextEnvelope {
        schema_version: "1.1.0".to_string(),
        altitude,
        intent_id: id.to_string(),
        commit: String::new(),
        tree_hash: String::new(),
        authorship,
        charter: "do the thing".to_string(),
        campaign: Some("camp-a".to_string()),
        constraints: vec![],
        acceptance: vec![],
        parent_intents: vec![],
        trajectory: Trajectory {
            raw_transcript_ref: None,
            task_transcript_ref: None,
            summary: None,
            journal_ref: None,
            redaction_policy: "default".to_string(),
        },
        snapshot: Snapshot {
            files_read: vec![],
            prompt_ref: None,
            env_manifest: "rustc 1.96.0".to_string(),
        },
        metrics: metrics(total),
        verdicts_ref: None,
    }
}

fn pr_envelope(pr_id: &str, _campaign: &str) -> ContextEnvelope {
    // Top-level orchestrator session: agent_type "main", no parent → not a
    // subagent (passes the D14 projection gate). 500 orchestration tokens.
    envelope(
        pr_id,
        Altitude::Pr,
        authorship("main", None, "run-orch"),
        500,
    )
}

fn intent_envelope(intent_id: &str) -> ContextEnvelope {
    // A subagent (implementer) authored the intent — 1000 work tokens.
    envelope(
        intent_id,
        Altitude::Intent,
        authorship("implementer", Some("run-orch"), "run-sub"),
        1_000,
    )
}

// ── binary e2e (WP-PC3b) ────────────────────────────────────────────────────────
//
// Drive the REAL compiled `hugit pr open|land|show` verb through the clap
// adapter, the way acceptance_rcli drives the other verbs (via
// `CARGO_BIN_EXE_hugit`). These exercise the full path: clap parse → load the
// `--log` event-log file → the PC3 engine → JSON on stdout → persist back.

use std::path::PathBuf;
use std::process::Command;

/// The built `hugit` binary (Cargo sets this env for bin-target tests).
fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

/// A per-test scratch dir under the OS temp.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-pc3b-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit pr …`, returning (exit-success, parsed-stdout-JSON, raw-stderr).
fn run_pr(args: &[&str]) -> (bool, Value, String) {
    let out = Command::new(hugit_bin())
        .arg("pr")
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let json: Value = serde_json::from_str(stdout.trim()).unwrap_or_else(|e| {
        panic!("stdout is not JSON ({e}): stdout={stdout:?} stderr={stderr:?}")
    });
    (out.status.success(), json, stderr)
}

#[test]
fn e2e_open_land_show_over_the_real_binary() {
    let dir = scratch("happy");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();

    // open — creates the log, appends pr.opened, exit 0.
    let (ok, opened, err) = run_pr(&[
        "open",
        "--log",
        log_s,
        "--pr",
        "7",
        "--campaign",
        "camp-a",
        "--author-kind",
        "orchestrator",
        "--run-id",
        "run-orch",
        "--intent",
        "i1",
        "--intent",
        "i2",
    ]);
    assert!(ok, "pr open must exit 0; stderr: {err}");
    assert_eq!(opened["pr_id"], json!("7"));
    assert_eq!(opened["state"], json!("proposed"));
    assert_eq!(opened["intent_count"], json!(2));
    assert_eq!(opened["already_exists"], json!(false));
    assert!(log.is_file(), "open persisted the event log");

    // land — enters the real queue, position 0, exit 0.
    let (ok, landed, err) = run_pr(&["land", "--log", log_s, "--pr", "7"]);
    assert!(ok, "pr land must exit 0; stderr: {err}");
    assert_eq!(landed["queued"], json!(true));
    assert_eq!(landed["already_queued"], json!(false));
    assert_eq!(landed["position"], json!(0));
    assert_eq!(landed["mode"], json!("union"));

    // show — projects intents + queue state, exit 0.
    let (ok, shown, err) = run_pr(&["show", "--log", log_s, "--pr", "7"]);
    assert!(ok, "pr show must exit 0; stderr: {err}");
    assert_eq!(shown["intent_count"], json!(2));
    assert_eq!(shown["queue"]["queued"], json!(true));
    assert_eq!(shown["queue"]["position"], json!(0));
    // No envelope captured on the log → honest null cost block.
    assert_eq!(shown["cost"], Value::Null);
}

#[test]
fn e2e_open_rejects_subagent_author_kind_with_structured_error() {
    let dir = scratch("d14");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();

    // D14 at the door: --author-kind subagent → structured error on stdout,
    // non-zero exit, and NO log written (rejected before any append).
    let (ok, j, _err) = run_pr(&[
        "open",
        "--log",
        log_s,
        "--pr",
        "7",
        "--campaign",
        "camp-a",
        "--author-kind",
        "subagent",
        "--intent",
        "i1",
    ]);
    assert!(!ok, "subagent author-kind MUST be refused (non-zero exit)");
    assert_eq!(j["error"]["kind"], json!("subagent_author"));
    assert!(
        j["error"]["message"]
            .as_str()
            .unwrap()
            .contains("never a subagent")
    );
    assert!(
        j["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("--author-kind")
    );
    assert_eq!(j["error"]["got"], json!("subagent"));
    assert!(!log.exists(), "a rejected open writes no event log");
}

#[test]
fn e2e_open_is_idempotent_across_two_binary_invocations() {
    let dir = scratch("idem");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    let open = |intent: &str| {
        run_pr(&[
            "open",
            "--log",
            log_s,
            "--pr",
            "9",
            "--campaign",
            "camp-z",
            "--author-kind",
            "human",
            "--principal",
            "gustavo",
            "--intent",
            intent,
        ])
    };

    // First open: fresh PR.
    let (ok, first, err) = open("i1");
    assert!(ok, "first open exits 0; stderr: {err}");
    assert_eq!(first["already_exists"], json!(false));

    // Second open of the SAME pr id + campaign: idempotent no-op, exit 0,
    // already_exists:true, and the persisted log still has exactly one pr.opened.
    let (ok, again, err) = open("i1");
    assert!(ok, "idempotent re-open exits 0; stderr: {err}");
    assert_eq!(again["already_exists"], json!(true));

    let bytes = std::fs::read(&log).unwrap();
    let records: Vec<Value> = serde_json::from_slice(&bytes).unwrap();
    let opened_count = records
        .iter()
        .filter(|r| r["kind"] == json!(PR_OPENED_KIND))
        .count();
    assert_eq!(
        opened_count, 1,
        "idempotent re-open appends no second event"
    );
}

#[test]
fn e2e_land_unknown_pr_is_structured_error_exit_two() {
    let dir = scratch("unknown");
    let log = dir.join("log.json");
    // An empty (but present) log: landing an unopened PR → structured unknown_pr.
    std::fs::write(&log, "[]").unwrap();
    let (ok, j, _err) = run_pr(&["land", "--log", log.to_str().unwrap(), "--pr", "404"]);
    assert!(!ok, "landing an unknown PR MUST be refused");
    assert_eq!(j["error"]["kind"], json!("unknown_pr"));
}
