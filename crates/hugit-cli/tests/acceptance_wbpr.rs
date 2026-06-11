//! WP-WB-PR acceptance — `pr list` / `pr abandon`, referential symmetry,
//! stable key-sets, and the D14-guarded append routing (WA2b, pr side).
//!
//! Every test drives the verbs over the REAL engine: the library functions
//! ([`pr::list`] / [`pr::abandon`] / …) against an in-memory
//! [`hugit_refstore::EventLog`], and the new subcommands through the REAL
//! compiled `hugit pr …` binary (the clap adapter → `--log` file → engine →
//! JSON on stdout → persist). No hand-faked projections.

use hugit_cli::pr::{
    self, AbandonArgs, AuthorKind, CAMPAIGN_OPENED_KIND, LandArgs, ListArgs, OpenArgs,
    PR_ABANDONED_KIND, PR_LANDED_KIND, PR_QUEUED_KIND, ShowArgs,
};
use hugit_refstore::EventLog;
use serde_json::{Value, json};

// ── fixtures ──────────────────────────────────────────────────────────────────

fn orch_args(pr: &str, campaign: &str, intents: &[&str]) -> OpenArgs {
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

fn human_args(pr: &str, campaign: &str, principal: Option<&str>) -> OpenArgs {
    OpenArgs {
        pr_id: pr.to_string(),
        campaign: campaign.to_string(),
        author_kind: AuthorKind::Human,
        run_id: None,
        principal: principal.map(str::to_string),
        intent_ids: vec!["i1".to_string()],
        recorded_at: 1_000,
    }
}

fn count_kind(log: &EventLog, kind: &str) -> usize {
    log.records().iter().filter(|r| r.kind == kind).count()
}

/// Append a `campaign.opened` record naming `key` (the read-side vocabulary
/// `pr open --campaign` validates against — written here through the raw log).
fn open_campaign(log: &mut EventLog, key: &str) {
    let payload = format!("{{\"campaign\":{}}}", json!(key));
    log.append(CAMPAIGN_OPENED_KIND, vec![], payload, 500);
}

/// Append a `pr.landed` record naming `pr_id` (the disclosed landing seam — the
/// pr porcelain never writes it; `abandon` only READS it to refuse).
fn land_externally(log: &mut EventLog, pr_id: &str) {
    let payload = format!("{{\"pr_id\":{}}}", json!(pr_id));
    log.append(PR_LANDED_KIND, vec![], payload, 600);
}

// ── referential symmetry (audit P5) ─────────────────────────────────────────────

#[test]
fn open_orchestrator_requires_run_id() {
    let mut log = EventLog::new();
    let mut args = orch_args("7", "camp-a", &["i1"]);
    args.run_id = None; // orchestrator with no binding
    let err = pr::open(&mut log, &args).unwrap_err();
    assert_eq!(err.code(), "missing_author_binding");
    let j = err.to_json();
    // Canonical WB0 shape: nested, fix-keyed, context flat.
    assert_eq!(j["error"]["kind"], json!("missing_author_binding"));
    assert!(j["error"]["fix"].as_str().unwrap().contains("--run-id"));
    assert_eq!(j["error"]["author_kind"], json!("orchestrator"));
    assert_eq!(count_kind(&log, "pr.opened"), 0, "no event on a refusal");
}

#[test]
fn open_orchestrator_rejects_blank_run_id() {
    let mut log = EventLog::new();
    let mut args = orch_args("7", "camp-a", &["i1"]);
    args.run_id = Some(String::new()); // present but empty → still unbound
    let err = pr::open(&mut log, &args).unwrap_err();
    assert_eq!(err.code(), "missing_author_binding");
}

#[test]
fn open_human_requires_principal() {
    let mut log = EventLog::new();
    let err = pr::open(&mut log, &human_args("7", "camp-a", None)).unwrap_err();
    assert_eq!(err.code(), "missing_author_binding");
    let j = err.to_json();
    assert!(j["error"]["fix"].as_str().unwrap().contains("--principal"));
    assert_eq!(j["error"]["author_kind"], json!("human"));
}

#[test]
fn open_human_with_principal_succeeds() {
    let mut log = EventLog::new();
    let out = pr::open(&mut log, &human_args("7", "camp-a", Some("gustavo"))).unwrap();
    assert_eq!(out["already_exists"], json!(false));
    assert_eq!(count_kind(&log, "pr.opened"), 1);
}

// ── campaign existence symmetry (audit P5) ──────────────────────────────────────

#[test]
fn open_refuses_unknown_campaign_on_a_campaign_aware_log() {
    let mut log = EventLog::new();
    open_campaign(&mut log, "camp-real"); // the log now carries campaign vocabulary
    let err = pr::open(&mut log, &orch_args("7", "camp-ghost", &["i1"])).unwrap_err();
    assert_eq!(err.code(), "unknown_campaign");
    let j = err.to_json();
    assert_eq!(j["error"]["campaign"], json!("camp-ghost"));
    assert!(
        j["error"]["fix"]
            .as_str()
            .unwrap()
            .contains("campaign open")
    );
    assert_eq!(count_kind(&log, "pr.opened"), 0);
}

#[test]
fn open_accepts_an_existing_campaign_on_a_campaign_aware_log() {
    let mut log = EventLog::new();
    open_campaign(&mut log, "camp-real");
    let out = pr::open(&mut log, &orch_args("7", "camp-real", &["i1"])).unwrap();
    assert_eq!(out["already_exists"], json!(false));
}

#[test]
fn open_stays_permissive_on_a_log_with_no_campaign_vocabulary() {
    // The documented opt-out: a log that does not use the campaign seam accepts
    // any campaign key (an early fixture stays valid).
    let mut log = EventLog::new();
    let out = pr::open(&mut log, &orch_args("7", "anything-goes", &["i1"])).unwrap();
    assert_eq!(out["already_exists"], json!(false));
    assert_eq!(count_kind(&log, "pr.opened"), 1);
}

// ── list ────────────────────────────────────────────────────────────────────

#[test]
fn list_returns_every_pr_in_stable_open_order_with_full_rows() {
    let mut log = EventLog::new();
    pr::open(&mut log, &orch_args("7", "camp-a", &["i1", "i2"])).unwrap();
    pr::open(&mut log, &orch_args("8", "camp-b", &["i3"])).unwrap();
    pr::land(
        &mut log,
        &LandArgs {
            pr_id: "7".to_string(),
            recorded_at: 2_000,
        },
    )
    .unwrap();

    let out = pr::list(&log, &ListArgs::default());
    assert_eq!(out["count"], json!(2));
    assert_eq!(out["shown"], json!(2));
    let prs = out["prs"].as_array().unwrap();
    // Stable order: by open seq — 7 then 8.
    assert_eq!(prs[0]["pr_id"], json!("7"));
    assert_eq!(prs[1]["pr_id"], json!("8"));
    // Full row: every field present, including queue position + state.
    assert_eq!(prs[0]["campaign"], json!("camp-a"));
    assert_eq!(prs[0]["author_kind"], json!("orchestrator"));
    assert_eq!(prs[0]["intent_count"], json!(2));
    assert_eq!(prs[0]["state"], json!("queued"));
    assert_eq!(prs[0]["position"], json!(0));
    // PR 8 is proposed (not landed) → null position.
    assert_eq!(prs[1]["state"], json!("proposed"));
    assert_eq!(prs[1]["position"], Value::Null);
    // The row key-set is identical across rows.
    let keys = |v: &Value| {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(keys(&prs[0]), keys(&prs[1]), "every row has the same keys");
}

#[test]
fn list_filters_by_campaign_and_state() {
    let mut log = EventLog::new();
    pr::open(&mut log, &orch_args("7", "camp-a", &["i1"])).unwrap();
    pr::open(&mut log, &orch_args("8", "camp-b", &["i2"])).unwrap();
    pr::open(&mut log, &orch_args("9", "camp-a", &["i3"])).unwrap();

    let by_campaign = pr::list(
        &log,
        &ListArgs {
            campaign: Some("camp-a".to_string()),
            state: None,
        },
    );
    assert_eq!(by_campaign["shown"], json!(2));
    assert_eq!(
        by_campaign["count"],
        json!(3),
        "count is the unfiltered total"
    );

    // Abandon 7 → state filter narrows to it.
    pr::abandon(
        &mut log,
        &AbandonArgs {
            pr_id: "7".to_string(),
            reason: "superseded".to_string(),
            recorded_at: 3_000,
        },
    )
    .unwrap();
    let abandoned = pr::list(
        &log,
        &ListArgs {
            campaign: None,
            state: Some("abandoned".to_string()),
        },
    );
    assert_eq!(abandoned["shown"], json!(1));
    assert_eq!(abandoned["prs"][0]["pr_id"], json!("7"));
}

// ── abandon ───────────────────────────────────────────────────────────────────

#[test]
fn abandon_appends_pr_abandoned_through_the_real_event_path() {
    let mut log = EventLog::new();
    pr::open(&mut log, &orch_args("7", "camp-a", &["i1"])).unwrap();
    let out = pr::abandon(
        &mut log,
        &AbandonArgs {
            pr_id: "7".to_string(),
            reason: "obsolete".to_string(),
            recorded_at: 3_000,
        },
    )
    .unwrap();
    assert_eq!(out["abandoned"], json!(true));
    assert_eq!(out["already_abandoned"], json!(false));
    assert_eq!(out["reason"], json!("obsolete"));
    assert_eq!(count_kind(&log, PR_ABANDONED_KIND), 1);
}

#[test]
fn abandon_is_idempotent_double_run() {
    let mut log = EventLog::new();
    pr::open(&mut log, &orch_args("7", "camp-a", &["i1"])).unwrap();
    let abandon = |log: &mut EventLog| {
        pr::abandon(
            log,
            &AbandonArgs {
                pr_id: "7".to_string(),
                reason: "obsolete".to_string(),
                recorded_at: 3_000,
            },
        )
        .unwrap()
    };
    let first = abandon(&mut log);
    assert_eq!(first["already_abandoned"], json!(false));
    let n = log.len();

    let again = abandon(&mut log);
    assert_eq!(again["already_abandoned"], json!(true));
    assert_eq!(
        log.len(),
        n,
        "idempotent re-abandon appends NO second event"
    );
    assert_eq!(count_kind(&log, PR_ABANDONED_KIND), 1);
    // Same key-set on first-run and the idempotent re-run.
    assert_eq!(
        first.as_object().unwrap().keys().collect::<Vec<_>>(),
        again.as_object().unwrap().keys().collect::<Vec<_>>(),
        "abandon key-set is stable across first + re-run"
    );
}

#[test]
fn abandon_refuses_unknown_pr() {
    let mut log = EventLog::new();
    let err = pr::abandon(
        &mut log,
        &AbandonArgs {
            pr_id: "404".to_string(),
            reason: "x".to_string(),
            recorded_at: 3_000,
        },
    )
    .unwrap_err();
    assert_eq!(err.code(), "unknown_pr");
}

#[test]
fn abandon_refuses_a_landed_pr() {
    let mut log = EventLog::new();
    pr::open(&mut log, &orch_args("7", "camp-a", &["i1"])).unwrap();
    land_externally(&mut log, "7");
    let err = pr::abandon(
        &mut log,
        &AbandonArgs {
            pr_id: "7".to_string(),
            reason: "too late".to_string(),
            recorded_at: 3_000,
        },
    )
    .unwrap_err();
    assert_eq!(err.code(), "pr_already_landed");
    assert_eq!(count_kind(&log, PR_ABANDONED_KIND), 0);
}

#[test]
fn abandoned_pr_leaves_the_queue_projection() {
    // An abandoned PR is no longer in the proposed/queued lifecycle; list shows
    // it as `abandoned`, not as a live queue row.
    let mut log = EventLog::new();
    pr::open(&mut log, &orch_args("7", "camp-a", &["i1"])).unwrap();
    pr::land(
        &mut log,
        &LandArgs {
            pr_id: "7".to_string(),
            recorded_at: 2_000,
        },
    )
    .unwrap();
    pr::abandon(
        &mut log,
        &AbandonArgs {
            pr_id: "7".to_string(),
            reason: "pulled".to_string(),
            recorded_at: 3_000,
        },
    )
    .unwrap();
    let out = pr::list(&log, &ListArgs::default());
    assert_eq!(out["prs"][0]["state"], json!("abandoned"));
}

// ── stable key-sets (audit P8) ──────────────────────────────────────────────────

fn keys_of(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
    k.sort();
    k
}

#[test]
fn open_key_set_is_stable_first_run_and_idempotent_re_run() {
    let mut log = EventLog::new();
    let first = pr::open(&mut log, &orch_args("7", "camp-a", &["i1"])).unwrap();
    let again = pr::open(&mut log, &orch_args("7", "camp-a", &["i1"])).unwrap();
    assert_eq!(first["already_exists"], json!(false));
    assert_eq!(again["already_exists"], json!(true));
    assert_eq!(keys_of(&first), keys_of(&again), "open key-set is stable");
}

#[test]
fn land_key_set_is_stable_first_run_and_idempotent_re_run() {
    let mut log = EventLog::new();
    pr::open(&mut log, &orch_args("7", "camp-a", &["i1"])).unwrap();
    let land = |log: &mut EventLog| {
        pr::land(
            log,
            &LandArgs {
                pr_id: "7".to_string(),
                recorded_at: 2_000,
            },
        )
        .unwrap()
    };
    let first = land(&mut log);
    let again = land(&mut log);
    assert_eq!(first["already_queued"], json!(false));
    assert_eq!(again["already_queued"], json!(true));
    assert_eq!(keys_of(&first), keys_of(&again), "land key-set is stable");
}

#[test]
fn show_key_set_is_stable_before_and_after_landing() {
    let mut log = EventLog::new();
    pr::open(&mut log, &orch_args("7", "camp-a", &["i1"])).unwrap();
    let before = pr::show(
        &log,
        &ShowArgs {
            pr_id: "7".to_string(),
        },
    )
    .unwrap();
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
    assert_eq!(keys_of(&before), keys_of(&after), "show key-set is stable");
}

// ── D14-guarded append routing (WA2b, pr side) ──────────────────────────────────

#[test]
fn land_and_abandon_route_through_the_guarded_append() {
    // The pr.queued + pr.abandoned appends go through `append_authorized` under
    // the opened PR's author class (orchestrator → matrix-allowed) — the events
    // land, and NO `authz.denied` audit record is emitted (an allow path).
    let mut log = EventLog::new();
    pr::open(&mut log, &orch_args("7", "camp-a", &["i1"])).unwrap();
    pr::land(
        &mut log,
        &LandArgs {
            pr_id: "7".to_string(),
            recorded_at: 2_000,
        },
    )
    .unwrap();
    assert_eq!(count_kind(&log, PR_QUEUED_KIND), 1);
    assert_eq!(
        count_kind(&log, "authz.denied"),
        0,
        "the orchestrator land append is matrix-allowed (no denial audit)"
    );

    pr::abandon(
        &mut log,
        &AbandonArgs {
            pr_id: "7".to_string(),
            reason: "x".to_string(),
            recorded_at: 3_000,
        },
    )
    .unwrap();
    assert_eq!(count_kind(&log, PR_ABANDONED_KIND), 1);
    assert_eq!(
        count_kind(&log, "authz.denied"),
        0,
        "the orchestrator abandon append is matrix-allowed (no denial audit)"
    );
}

// ── binary e2e: the new subcommands through the REAL `hugit pr` binary ──────────

use std::path::PathBuf;
use std::process::Command;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-wbpr-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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

fn open_pr(log_s: &str, pr: &str, intent: &str) {
    let (ok, _, err) = run_pr(&[
        "open",
        "--log",
        log_s,
        "--pr",
        pr,
        "--campaign",
        "camp-a",
        "--author-kind",
        "orchestrator",
        "--run-id",
        "run-orch",
        "--intent",
        intent,
    ]);
    assert!(ok, "pr open exits 0; stderr: {err}");
}

#[test]
fn e2e_pr_list_over_the_real_binary() {
    let dir = scratch("list");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    open_pr(log_s, "7", "i1");
    open_pr(log_s, "8", "i2");

    let (ok, out, err) = run_pr(&["list", "--log", log_s]);
    assert!(ok, "pr list exits 0; stderr: {err}");
    assert_eq!(out["count"], json!(2));
    let prs = out["prs"].as_array().unwrap();
    assert_eq!(prs[0]["pr_id"], json!("7"));
    assert_eq!(prs[0]["state"], json!("proposed"));
}

#[test]
fn e2e_pr_abandon_over_the_real_binary_is_idempotent() {
    let dir = scratch("abandon");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    open_pr(log_s, "7", "i1");

    let (ok, first, err) = run_pr(&[
        "abandon", "--log", log_s, "--pr", "7", "--reason", "obsolete",
    ]);
    assert!(ok, "pr abandon exits 0; stderr: {err}");
    assert_eq!(first["abandoned"], json!(true));
    assert_eq!(first["already_abandoned"], json!(false));

    // Re-abandon over a second invocation: idempotent, exit 0, no second event.
    let (ok, again, err) = run_pr(&[
        "abandon", "--log", log_s, "--pr", "7", "--reason", "obsolete",
    ]);
    assert!(ok, "idempotent re-abandon exits 0; stderr: {err}");
    assert_eq!(again["already_abandoned"], json!(true));

    let bytes = std::fs::read(&log).unwrap();
    let records: Vec<Value> = serde_json::from_slice(&bytes).unwrap();
    let abandoned = records
        .iter()
        .filter(|r| r["kind"] == json!(PR_ABANDONED_KIND))
        .count();
    assert_eq!(
        abandoned, 1,
        "idempotent re-abandon appends no second event"
    );
}

#[test]
fn e2e_open_orchestrator_without_run_id_is_structured_refusal() {
    let dir = scratch("symmetry");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    // --author-kind orchestrator with NO --run-id → referential-symmetry refusal.
    let (ok, j, _err) = run_pr(&[
        "open",
        "--log",
        log_s,
        "--pr",
        "7",
        "--campaign",
        "camp-a",
        "--author-kind",
        "orchestrator",
        "--intent",
        "i1",
    ]);
    assert!(!ok, "orchestrator without --run-id MUST be refused");
    assert_eq!(j["error"]["kind"], json!("missing_author_binding"));
    assert!(j["error"]["fix"].as_str().unwrap().contains("--run-id"));
    assert!(!log.exists(), "a refused open writes no event log");
}
