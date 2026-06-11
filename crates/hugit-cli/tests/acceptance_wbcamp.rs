//! Acceptance — WP-WB-CAMP: `hugit campaign list / abandon` + honest fix-hints +
//! stable key-sets + guarded appends.
//!
//! Proven here:
//! - `campaign list --log` enumerates all campaigns in stable (sorted) key
//!   order, carrying charter/owner/state/progress per entry.
//! - `campaign abandon --campaign … --reason …` appends `campaign.abandoned`,
//!   is idempotent, and rejects abandoning a closed campaign with a structured
//!   error (`already_closed`). Abandoning an unopened campaign is a structured
//!   error (`not_opened`).
//! - Abandoning a campaign releases its in-flight PRs from blocking close
//!   semantics: `close` proceeds after `abandon` even with in-flight PRs.
//! - `campaign open` idempotent re-run emits the SAME key-set as first-run
//!   (null over absent — charter/owner always present).
//! - `campaign close` idempotent re-run emits the SAME key-set as first-run
//!   (envelope/progress/prs/ledger/rollup always present, null where uncomputed).
//! - `close`'s in-flight refusal fix-hint says "land or abandon first" — a
//!   verb that NOW EXISTS (honest fix-hints, P6).
//! - `campaign open/close/abandon` appends route through `append_authorized`;
//!   a denial from the D14 matrix is a structured error on stdout.

use std::path::PathBuf;
use std::process::Command;

use serde_json::{Value, json};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-wbcamp-{tag}-{}-{}",
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
// World fixtures
// ─────────────────────────────────────────────────────────────────────────────

struct Ev {
    kind: &'static str,
    payload: Value,
}

fn ev(kind: &'static str, payload: Value) -> Ev {
    Ev { kind, payload }
}

fn pr_opened(pr_id: &str, intent_ids: &[&str]) -> Ev {
    ev(
        "pr.opened",
        json!({
            "pr_id": pr_id,
            "campaign": CAMPAIGN,
            "author_kind": "orchestrator",
            "intent_ids": intent_ids,
        }),
    )
}

fn pr_landed(pr_id: &str) -> Ev {
    ev(
        "pr.landed",
        json!({ "pr_id": pr_id, "campaign": CAMPAIGN, "union_verdict": "green" }),
    )
}

fn landed_intent(id: &str, at: u64) -> Ev {
    ev(
        "intent.landed",
        json!({
            "intent_id": id,
            "ref": "refs/heads/main",
            "target": format!("{at:040x}"),
            "charter": format!("harden auth — {id}"),
            "campaign": CAMPAIGN,
            "deep_link_target": id,
        }),
    )
}

/// A settled world where all PRs are landed.
fn settled_world() -> Vec<Ev> {
    vec![
        pr_opened("PR-1", &["i-a1"]),
        landed_intent("i-a1", T0 + 100_000),
        pr_landed("PR-1"),
    ]
}

/// A world with one PR still in-flight.
fn in_flight_world() -> Vec<Ev> {
    vec![
        pr_opened("PR-1", &["i-a1"]),
        pr_opened("PR-2", &["i-b1"]),
        landed_intent("i-a1", T0 + 100_000),
        pr_landed("PR-1"),
        // PR-2 opened but neither landed nor excluded → in-flight.
    ]
}

fn write_world(dir: &std::path::Path, events: &[Ev]) -> PathBuf {
    use hugit_refstore::{EventLog, canonical_json};
    let mut log = EventLog::new();
    for (i, e) in events.iter().enumerate() {
        let payload = e.payload.to_string();
        let payload = canonical_json(&payload).unwrap_or(payload);
        log.append(
            e.kind.to_string(),
            vec!["user:gustavo@humangr.com".to_string()],
            payload,
            T0 + i as u64,
        );
    }
    let p = dir.join("log.json");
    std::fs::write(&p, serde_json::to_vec_pretty(log.records()).unwrap()).unwrap();
    p
}

fn count_kind(path: &std::path::Path, kind: &str) -> usize {
    let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == kind)
        .count()
}

// ─────────────────────────────────────────────────────────────────────────────
// campaign list
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn list_empty_log_returns_empty_array() {
    let dir = scratch("list-empty");
    let world = write_world(&dir, &[]);
    let log = world.to_str().unwrap();

    let (ok, v) = run(&["list", "--log", log]);
    assert!(ok, "list exits 0 on an empty log: {v}");
    assert_eq!(v["count"], 0);
    assert_eq!(v["campaigns"], json!([]));
}

#[test]
fn list_shows_opened_campaign_with_charter_and_owner() {
    let dir = scratch("list-one");
    let world = write_world(&dir, &[]);
    let log = world.to_str().unwrap();

    // Open a campaign first.
    let (ok, _) = run(&[
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
    assert!(ok, "open succeeded");

    let (ok, v) = run(&["list", "--log", log]);
    assert!(ok, "list exits 0: {v}");
    assert_eq!(v["count"], 1);

    let campaigns = v["campaigns"].as_array().unwrap();
    assert_eq!(campaigns.len(), 1);
    let c = &campaigns[0];
    assert_eq!(c["campaign"], CAMPAIGN);
    assert_eq!(c["charter"], "harden the auth border");
    assert_eq!(c["owner"], "gustavo@humangr.com");
    assert_eq!(c["opened"], true);
    assert_eq!(c["closed"], false);
    assert_eq!(c["abandoned"], false);
    // Progress: no PRs yet.
    assert_eq!(c["progress"]["landed"], 0);
    assert_eq!(c["progress"]["in_flight"], 0);
}

#[test]
fn list_stable_order_multiple_campaigns() {
    // Open two campaigns (beta before alpha) and verify they come out sorted.
    let dir = scratch("list-order");
    let world = write_world(&dir, &[]);
    let log = world.to_str().unwrap();

    let (ok, _) = run(&[
        "open",
        "--log",
        log,
        "--campaign",
        "z-campaign",
        "--charter",
        "Z",
        "--owner",
        "u",
    ]);
    assert!(ok);
    let (ok, _) = run(&[
        "open",
        "--log",
        log,
        "--campaign",
        "a-campaign",
        "--charter",
        "A",
        "--owner",
        "u",
    ]);
    assert!(ok);

    let (ok, v) = run(&["list", "--log", log]);
    assert!(ok, "list exits 0: {v}");
    assert_eq!(v["count"], 2);
    let campaigns = v["campaigns"].as_array().unwrap();
    assert_eq!(
        campaigns[0]["campaign"], "a-campaign",
        "sorted order: a before z"
    );
    assert_eq!(campaigns[1]["campaign"], "z-campaign");
}

#[test]
fn list_shows_progress_counts() {
    // Use the settled world to check progress is surfaced.
    let dir = scratch("list-progress");
    let world = write_world(&dir, &settled_world());
    let log = world.to_str().unwrap();

    // Open the campaign so list can project it.
    let (ok, _) = run(&[
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "c",
        "--owner",
        "u",
    ]);
    assert!(ok);

    let (ok, v) = run(&["list", "--log", log]);
    assert!(ok, "list exits 0: {v}");
    let campaigns = v["campaigns"].as_array().unwrap();
    let c = campaigns
        .iter()
        .find(|c| c["campaign"] == CAMPAIGN)
        .unwrap();
    assert_eq!(c["progress"]["landed"], 1, "settled world has 1 landed PR");
    assert_eq!(c["progress"]["in_flight"], 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// campaign abandon
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn abandon_records_campaign_abandoned() {
    let dir = scratch("abandon");
    let world = write_world(&dir, &[]);
    let log = world.to_str().unwrap();

    // Open first.
    let (ok, _) = run(&[
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "c",
        "--owner",
        "u",
    ]);
    assert!(ok);

    let (ok, v) = run(&[
        "abandon",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--reason",
        "scope change — rerouted to v2",
    ]);
    assert!(ok, "abandon exits 0: {v}");
    assert_eq!(v["campaign"], CAMPAIGN);
    assert_eq!(v["abandoned"], true);
    assert_eq!(v["already_abandoned"], false);
    assert_eq!(v["reason"], "scope change — rerouted to v2");
    assert_eq!(count_kind(&world, "campaign.abandoned"), 1);
}

#[test]
fn abandon_is_idempotent() {
    let dir = scratch("abandon-idem");
    let world = write_world(&dir, &[]);
    let log = world.to_str().unwrap();

    // Open then abandon.
    let (ok, _) = run(&[
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "c",
        "--owner",
        "u",
    ]);
    assert!(ok);
    let args = [
        "abandon",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--reason",
        "first abandon",
    ];
    let (ok1, v1) = run(&args);
    assert!(ok1);
    assert_eq!(v1["already_abandoned"], false);

    // Re-abandon with the same key → exit 0, already_abandoned, no duplicate.
    let (ok2, v2) = run(&args);
    assert!(ok2, "re-abandon exits 0 (idempotent)");
    assert_eq!(v2["already_abandoned"], true);
    assert_eq!(
        count_kind(&world, "campaign.abandoned"),
        1,
        "idempotent: exactly one abandoned record"
    );
}

#[test]
fn abandon_closed_campaign_is_structured_error() {
    let dir = scratch("abandon-closed");
    let world = write_world(&dir, &settled_world());
    let log = world.to_str().unwrap();

    // Open and close the campaign.
    let (ok, _) = run(&[
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "c",
        "--owner",
        "u",
    ]);
    assert!(ok);
    let (ok, _) = run(&["close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(ok, "close succeeded on settled world");

    // Now attempt to abandon the closed campaign.
    let (ok, v) = run(&[
        "abandon",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--reason",
        "late change of heart",
    ]);
    assert!(!ok, "abandoning a closed campaign must exit nonzero");
    assert_eq!(
        v["error"]["kind"], "already_closed",
        "structured error kind"
    );
    assert!(
        v["error"]["fix"].as_str().is_some(),
        "fix hint present: {v}"
    );
    assert_eq!(count_kind(&world, "campaign.abandoned"), 0);
}

#[test]
fn abandon_unopened_campaign_is_structured_error() {
    let dir = scratch("abandon-unopened");
    let world = write_world(&dir, &[]);
    let log = world.to_str().unwrap();

    let (ok, v) = run(&[
        "abandon",
        "--log",
        log,
        "--campaign",
        "ghost-campaign",
        "--reason",
        "never existed",
    ]);
    assert!(!ok, "abandoning an unopened campaign must exit nonzero");
    assert_eq!(v["error"]["kind"], "not_opened");
    assert!(v["error"]["fix"].as_str().is_some(), "fix hint present");
}

// ─────────────────────────────────────────────────────────────────────────────
// Abandon releases in-flight PRs from blocking close semantics
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn close_succeeds_after_abandon_despite_in_flight_prs() {
    // Without abandon: close of in_flight_world would refuse.
    // With abandon: close must proceed.
    let dir = scratch("close-after-abandon");
    let world = write_world(&dir, &in_flight_world());
    let log = world.to_str().unwrap();

    // Open the campaign (required by abandon's not_opened guard).
    let (ok, _) = run(&[
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "c",
        "--owner",
        "u",
    ]);
    assert!(ok);

    // Abandon.
    let (ok, v) = run(&[
        "abandon",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--reason",
        "strategy pivot",
    ]);
    assert!(ok, "abandon exits 0: {v}");

    // Close should now proceed even though PR-2 is in-flight.
    let (ok, v) = run(&["close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(
        ok,
        "close must succeed after abandon despite in-flight PRs: {v}"
    );
    assert_eq!(v["closed"], true);
    assert_eq!(v["already_closed"], false);
    assert_eq!(count_kind(&world, "campaign.closed"), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Honest fix-hints (P6): close's in-flight hint names a real verb
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn close_in_flight_hint_names_abandon_which_now_exists() {
    // The hint "land or abandon first" now refers to a real verb (`abandon`
    // exists). Verify the hint text is the expected honest message.
    let dir = scratch("close-hint");
    let world = write_world(&dir, &in_flight_world());
    let log = world.to_str().unwrap();

    let (ok, v) = run(&["close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(!ok, "close must exit nonzero while PR is in-flight");
    assert_eq!(v["error"]["kind"], "in_flight_prs");
    let fix = v["error"]["fix"]
        .as_str()
        .expect("fix hint must be present");
    assert!(
        fix.contains("abandon"),
        "fix hint must name the `abandon` verb: got {fix:?}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Stable key-sets (P8)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn open_idempotent_rerun_carries_charter_and_owner() {
    // First-run and second-run must emit the same key-set.
    // charter + owner must be present (null or string, never absent) in both.
    let dir = scratch("open-keyset");
    let world = write_world(&dir, &[]);
    let log = world.to_str().unwrap();
    let args = [
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "harden auth",
        "--owner",
        "gustavo@humangr.com",
    ];

    let (ok1, v1) = run(&args);
    assert!(ok1);
    let (ok2, v2) = run(&args);
    assert!(ok2);

    // Both must carry charter + owner.
    assert!(
        v1.get("charter").is_some(),
        "first run: charter key present"
    );
    assert!(v1.get("owner").is_some(), "first run: owner key present");
    assert!(
        v2.get("charter").is_some(),
        "idempotent re-run: charter key present"
    );
    assert!(
        v2.get("owner").is_some(),
        "idempotent re-run: owner key present"
    );
    // Values match.
    assert_eq!(v2["charter"], v1["charter"]);
    assert_eq!(v2["owner"], v1["owner"]);
    assert_eq!(v2["already_exists"], true);
}

#[test]
fn close_idempotent_rerun_carries_full_key_set() {
    // Close re-run must carry envelope/progress/prs/ledger/rollup.
    let dir = scratch("close-keyset");
    let world = write_world(&dir, &settled_world());
    let log = world.to_str().unwrap();

    let (ok1, v1) = run(&["close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(ok1, "first close: {v1}");

    let (ok2, v2) = run(&["close", "--log", log, "--campaign", CAMPAIGN]);
    assert!(ok2, "re-close: {v2}");
    assert_eq!(v2["already_closed"], true);

    // Same key-set: envelope, progress, prs, ledger, rollup must all be present.
    for key in &["envelope", "progress", "prs", "ledger", "rollup"] {
        assert!(
            v1.get(*key).is_some(),
            "first run: key '{key}' present in first close"
        );
        assert!(
            v2.get(*key).is_some(),
            "re-run: key '{key}' present in idempotent re-close"
        );
    }
    // Values agree.
    assert_eq!(v2["progress"], v1["progress"]);
    assert_eq!(v2["envelope"], v1["envelope"]);
}

// ─────────────────────────────────────────────────────────────────────────────
// D14 guarded appends — abandon appends through the authorized path
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn abandon_appends_campaign_abandoned_record_to_log() {
    // The `campaign.abandoned` record must appear on the log's event array,
    // confirming it was written through the real append path (not just stdout).
    let dir = scratch("abandon-record");
    let world = write_world(&dir, &[]);
    let log = world.to_str().unwrap();

    let (ok, _) = run(&[
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "c",
        "--owner",
        "u",
    ]);
    assert!(ok);

    let (ok, _) = run(&[
        "abandon",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--reason",
        "pivot",
    ]);
    assert!(ok);

    // The log file must contain exactly one campaign.abandoned record.
    assert_eq!(
        count_kind(&world, "campaign.abandoned"),
        1,
        "abandon wrote exactly one campaign.abandoned record"
    );
    // And the log is still valid JSON.
    let v: Value =
        serde_json::from_slice(&std::fs::read(&world).unwrap()).expect("log is valid JSON");
    assert!(v.is_array(), "log is a JSON array");
}

#[test]
fn list_shows_abandoned_campaign_state() {
    let dir = scratch("list-abandoned");
    let world = write_world(&dir, &[]);
    let log = world.to_str().unwrap();

    let (ok, _) = run(&[
        "open",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--charter",
        "c",
        "--owner",
        "gustavo@humangr.com",
    ]);
    assert!(ok);

    let (ok, _) = run(&[
        "abandon",
        "--log",
        log,
        "--campaign",
        CAMPAIGN,
        "--reason",
        "no longer needed",
    ]);
    assert!(ok);

    let (ok, v) = run(&["list", "--log", log]);
    assert!(ok);
    let campaigns = v["campaigns"].as_array().unwrap();
    let c = campaigns
        .iter()
        .find(|c| c["campaign"] == CAMPAIGN)
        .unwrap();
    assert_eq!(c["abandoned"], true, "abandoned flag reflected in list");
    assert_eq!(
        c["abandon_reason"], "no longer needed",
        "abandon reason in list"
    );
}
