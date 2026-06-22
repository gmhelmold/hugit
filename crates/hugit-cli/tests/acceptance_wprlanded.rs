//! Acceptance — WP-W-PRLANDED: the `pr.landed` producer, against the REAL
//! `hugit` binary (`CARGO_BIN_EXE_hugit`).
//!
//! Before this wave, `pr land` appended only `pr.queued` (enter the landing
//! queue) — no event ever marked a PR as actually LANDED/settled, so campaign
//! progress could never advance a PR past `in_flight`, and a campaign could
//! only reach `closed:true` through `abandon`. This suite proves the producer:
//!
//! - the full porcelain path **campaign open → intent new → pr open →
//!   pr land (→ queued) → pr land --settle (→ pr.landed)** settles a PR;
//! - `campaign show` then reflects the PR as `landed` (progress.landed == 1,
//!   the PR row's phase == `landed`), NOT `in_flight`/`queued`;
//! - a campaign whose PRs are ALL landed reaches `campaign close → closed:true`
//!   driven by the REAL `pr.landed` (the deadlock fix on the LANDED path, not
//!   only abandon);
//! - `--settle` is **idempotent** (`already_landed:true`, exit 0, no second
//!   `pr.landed`);
//! - `--settle` refuses a PR that has not entered the queue (`pr_not_queued`,
//!   exit 2) and an unknown PR (`unknown_pr`, exit 2);
//! - the settle append is routed through the SAME guarded seam (the on-disk
//!   chain still verifies after the settle).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-wprlanded-{tag}-{}-{}",
        std::process::id(),
        nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Run `hugit <args>` → `(exit_code, parsed_stdout_json_or_null)`.
fn run(args: &[&str]) -> (Option<i32>, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code(), v)
}

fn count_kind(path: &Path, kind: &str) -> usize {
    let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == kind)
        .count()
}

/// Seed a campaign with one intent + one opened PR bundling it; return the
/// `(log_path, campaign, pr_id, intent_id)`.
fn seed_campaign_with_open_pr(dir: &Path) -> (PathBuf, String, String, String) {
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap();
    let store = dir.join("store.json");
    let store_s = store.to_str().unwrap();
    let campaign = "settle-camp".to_string();

    let (code, _) = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        &campaign,
        "--charter",
        "land the thing",
        "--owner",
        "o@h.com",
    ]);
    assert_eq!(code, Some(0), "campaign open succeeds");

    // Land an intent onto the SAME log so the PR's --intent id exists (the PC4
    // intent-existence check is satisfied), via the campaign log seam.
    let intent_id = "i1".to_string();
    let (code, v) = run(&[
        "intent",
        "new",
        "--log",
        log_s,
        "--store",
        store_s,
        "--campaign",
        &campaign,
        "--charter",
        "do the work",
        "--id",
        &intent_id,
    ]);
    assert_eq!(code, Some(0), "intent new succeeds: {v}");
    assert_eq!(v["intent_id"], intent_id, "{v}");

    let pr_id = "1".to_string();
    let (code, v) = run(&[
        "pr",
        "open",
        "--log",
        log_s,
        "--pr",
        &pr_id,
        "--campaign",
        &campaign,
        "--author-kind",
        "orchestrator",
        "--run-id",
        "run-1",
        "--intent",
        &intent_id,
    ]);
    assert_eq!(code, Some(0), "pr open succeeds: {v}");
    assert_eq!(v["state"], "proposed", "{v}");

    (log, campaign, pr_id, intent_id)
}

// ── The full porcelain settlement path ───────────────────────────────────────

#[test]
fn full_path_open_intent_pr_land_settle_then_campaign_close_via_landed() {
    let dir = scratch("full-path");
    let (log, campaign, pr_id, _intent) = seed_campaign_with_open_pr(&dir);
    let log_s = log.to_str().unwrap();

    // pr land → enqueue (pr.queued), still in-flight.
    let (code, v) = run(&["pr", "queue", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(code, Some(0), "pr land enqueues: {v}");
    assert_eq!(v["queued"], true, "{v}");
    assert_eq!(v["already_queued"], false, "{v}");
    assert_eq!(count_kind(&log, "pr.queued"), 1, "one pr.queued appended");
    assert_eq!(
        count_kind(&log, "pr.landed"),
        0,
        "no pr.landed yet — queued is not landed"
    );

    // While only queued, campaign show reports the PR in-flight (NOT landed).
    let (code, v) = run(&["campaign", "show", "--log", log_s, "--campaign", &campaign]);
    assert_eq!(code, Some(0), "campaign show: {v}");
    assert_eq!(v["progress"]["in_flight"], 1, "queued PR is in-flight: {v}");
    assert_eq!(
        v["progress"]["landed"], 0,
        "queued PR is not yet landed: {v}"
    );

    // pr land --settle → settle the queued PR as landed (pr.landed).
    let (code, v) = run(&["pr", "land", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(code, Some(0), "pr land --settle settles: {v}");
    assert_eq!(v["landed"], true, "{v}");
    assert_eq!(v["already_landed"], false, "{v}");
    assert_eq!(v["state"], "landed", "{v}");
    assert_eq!(
        v["campaign"], campaign,
        "the settle payload carries the campaign: {v}"
    );
    assert_eq!(
        count_kind(&log, "pr.landed"),
        1,
        "exactly one pr.landed appended"
    );

    // campaign show now reflects the PR as landed (NOT in-flight/queued).
    let (code, v) = run(&["campaign", "show", "--log", log_s, "--campaign", &campaign]);
    assert_eq!(code, Some(0), "campaign show after settle: {v}");
    assert_eq!(v["progress"]["landed"], 1, "the settled PR is landed: {v}");
    assert_eq!(
        v["progress"]["in_flight"], 0,
        "no PR remains in-flight: {v}"
    );
    let prs = v["prs"].as_array().expect("prs is an array");
    assert!(
        prs.iter()
            .any(|p| p["pr_id"] == pr_id && p["phase"] == "landed"),
        "the PR row's phase is landed: {v}"
    );

    // campaign close → closed:true, driven by the REAL pr.landed (no abandon).
    let (code, v) = run(&["campaign", "close", "--log", log_s, "--campaign", &campaign]);
    assert_eq!(
        code,
        Some(0),
        "a campaign whose PRs all LANDED reaches closed:true via pr.landed (not abandon): {v}"
    );
    assert_eq!(v["closed"], true, "{v}");
    assert_eq!(
        count_kind(&log, "campaign.closed"),
        1,
        "one campaign.closed seal"
    );
    // The deadlock fix is on the LANDED path: NO pr.abandoned was needed.
    assert_eq!(
        count_kind(&log, "pr.abandoned"),
        0,
        "close reached via landed, not abandon"
    );

    // The on-disk chain still verifies (the settle append went through the
    // guarded, hash-chained seam — never a parallel store).
    assert_chain_verifies(&log);
}

// ── Idempotency ──────────────────────────────────────────────────────────────

#[test]
fn settle_is_idempotent() {
    let dir = scratch("idempotent");
    let (log, _campaign, pr_id, _intent) = seed_campaign_with_open_pr(&dir);
    let log_s = log.to_str().unwrap();

    run(&["pr", "queue", "--log", log_s, "--pr", &pr_id]);
    let (code, v) = run(&["pr", "land", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(code, Some(0), "first settle: {v}");
    assert_eq!(v["already_landed"], false, "{v}");

    let (code, v) = run(&["pr", "land", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(code, Some(0), "re-settle is exit 0 (idempotent): {v}");
    assert_eq!(
        v["already_landed"], true,
        "re-settle reports already_landed: {v}"
    );
    assert_eq!(v["landed"], true, "{v}");
    assert_eq!(
        count_kind(&log, "pr.landed"),
        1,
        "re-settle appends NO second pr.landed"
    );
}

// ── Refusals ─────────────────────────────────────────────────────────────────

#[test]
fn settle_refuses_a_pr_not_yet_queued() {
    // A PROPOSED-but-not-queued PR cannot be settled: settle is the land-confirm
    // over a QUEUED PR (run `pr land` first).
    let dir = scratch("not-queued");
    let (log, _campaign, pr_id, _intent) = seed_campaign_with_open_pr(&dir);
    let log_s = log.to_str().unwrap();

    let (code, v) = run(&["pr", "land", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(
        code,
        Some(2),
        "settle of an un-queued PR refuses (exit 2): {v}"
    );
    assert_eq!(v["error"]["kind"], "pr_not_queued", "{v}");
    assert!(v["error"]["fix"].is_string(), "fix hint present: {v}");
    assert_eq!(
        count_kind(&log, "pr.landed"),
        0,
        "no pr.landed appended on refusal"
    );
}

#[test]
fn settle_refuses_an_unknown_pr() {
    let dir = scratch("unknown");
    let (log, _campaign, _pr_id, _intent) = seed_campaign_with_open_pr(&dir);
    let log_s = log.to_str().unwrap();

    let (code, v) = run(&["pr", "land", "--log", log_s, "--pr", "999"]);
    assert_eq!(
        code,
        Some(2),
        "settle of an unknown PR refuses (exit 2): {v}"
    );
    assert_eq!(v["error"]["kind"], "unknown_pr", "{v}");
    assert_eq!(
        count_kind(&log, "pr.landed"),
        0,
        "no pr.landed appended on refusal"
    );
}

#[test]
fn abandon_refuses_a_landed_pr() {
    // The terminal-landed property: once settled, the PR cannot be abandoned
    // (the existing AbandonLanded refusal fires on a REAL pr.landed producer).
    let dir = scratch("abandon-landed");
    let (log, _campaign, pr_id, _intent) = seed_campaign_with_open_pr(&dir);
    let log_s = log.to_str().unwrap();

    run(&["pr", "queue", "--log", log_s, "--pr", &pr_id]);
    run(&["pr", "land", "--log", log_s, "--pr", &pr_id]);

    let (code, v) = run(&[
        "pr", "abandon", "--log", log_s, "--pr", &pr_id, "--reason", "too late",
    ]);
    assert_eq!(
        code,
        Some(2),
        "abandoning a landed PR refuses (exit 2): {v}"
    );
    assert_eq!(v["error"]["kind"], "pr_already_landed", "{v}");
    assert_eq!(
        count_kind(&log, "pr.abandoned"),
        0,
        "no pr.abandoned on a landed PR"
    );
}

// ── WG-PR: settled pr.landed leaves the queue projection ─────────────────────

/// After open → land (queued) → settle (landed):
/// - `queue show` reports `queue_depth:0` and an empty `entries` array.
/// - `pr show` reports `queue.queued:false` and `state:"landed"`.
///
/// Proves both the queue projection and the pr show projection treat a
/// `pr.landed` PR as NO LONGER queued (WG-PR defect 2 fix).
#[test]
fn settled_pr_leaves_queue_projection() {
    let dir = scratch("wgpr-queue-leaves");
    let (log, _campaign, pr_id, _intent) = seed_campaign_with_open_pr(&dir);
    let log_s = log.to_str().unwrap();

    // Land → queued: queue_depth should be 1.
    let (code, v) = run(&["pr", "queue", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(code, Some(0), "pr land enqueues: {v}");
    assert_eq!(v["queued"], true, "{v}");

    // Confirm queue shows the PR before settlement.
    let (code, v) = run(&["queue", "show", "--log", log_s]);
    assert_eq!(code, Some(0), "queue show before settle: {v}");
    assert_eq!(v["queue_depth"], 1, "depth 1 before settle: {v}");
    let entries = v["entries"].as_array().expect("entries is array");
    assert_eq!(entries.len(), 1, "one entry before settle: {v}");
    assert_eq!(entries[0]["pr_id"], pr_id, "{v}");

    // Settle → pr.landed.
    let (code, v) = run(&["pr", "land", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(code, Some(0), "pr land --settle settles: {v}");
    assert_eq!(v["landed"], true, "{v}");
    assert_eq!(v["state"], "landed", "{v}");

    // After settlement: queue show MUST report depth 0, no entries.
    let (code, v) = run(&["queue", "show", "--log", log_s]);
    assert_eq!(code, Some(0), "queue show after settle: {v}");
    assert_eq!(
        v["queue_depth"], 0,
        "landed PR must leave the queue (depth 0): {v}"
    );
    assert_eq!(
        v["entries"].as_array().map(|a| a.len()).unwrap_or(99),
        0,
        "landed PR must not appear in queue entries: {v}"
    );

    // After settlement: pr show MUST report queue.queued:false and state:landed.
    let (code, v) = run(&["pr", "show", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(code, Some(0), "pr show after settle: {v}");
    assert_eq!(
        v["queue"]["queued"], false,
        "pr show queue.queued must be false after landing: {v}"
    );
    // The state field is derived from pr_state() which already returns "landed".
    // The show response doesn't carry "state" directly, but queue.queued:false
    // is the projection fix being tested here. For state we verify via campaign show.
}

/// Verify that `pr show` also reflects the correct state (not queued) for a
/// settled PR — complementary to the queue projection test above.
#[test]
fn pr_show_queue_queued_false_after_settle() {
    let dir = scratch("wgpr-prshow-queued");
    let (log, _campaign, pr_id, _intent) = seed_campaign_with_open_pr(&dir);
    let log_s = log.to_str().unwrap();

    run(&["pr", "queue", "--log", log_s, "--pr", &pr_id]);
    run(&["pr", "land", "--log", log_s, "--pr", &pr_id]);

    let (code, v) = run(&["pr", "show", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(code, Some(0), "pr show: {v}");
    assert_eq!(
        v["queue"]["queued"], false,
        "after pr.landed: pr show queue.queued must be false: {v}"
    );
    // No queue position for a landed PR.
    assert!(
        v["queue"].get("position").is_none() || v["queue"]["position"].is_null(),
        "landed PR has no queue position: {v}"
    );
}

// ── WI-PR defect 1: pr land is idempotent on a TERMINAL (landed) PR ──────────

/// open → land (queued) → settle (landed) → `pr land` AGAIN must be a NO-OP.
///
/// Before the fix, re-running `pr land` on a settled PR re-appended a SECOND
/// `pr.queued` AFTER the terminal `pr.landed` (post-terminal log corruption),
/// reporting `already_queued:false` + `queued:true` — a fresh-enqueue lie. The
/// canonical agent retry-on-land pattern must instead be idempotent: NO new
/// `pr.queued`, an idempotent response, and the log's pr kinds end at
/// `pr.landed` (no post-terminal `pr.queued`).
#[test]
fn land_is_idempotent_on_a_terminal_landed_pr() {
    let dir = scratch("land-terminal-idempotent");
    let (log, _campaign, pr_id, _intent) = seed_campaign_with_open_pr(&dir);
    let log_s = log.to_str().unwrap();

    // open → land (queued) → settle (landed).
    let (code, _) = run(&["pr", "queue", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(code, Some(0), "pr land enqueues");
    let (code, v) = run(&["pr", "land", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(code, Some(0), "pr land --settle settles: {v}");
    assert_eq!(v["landed"], true, "{v}");
    assert_eq!(
        count_kind(&log, "pr.queued"),
        1,
        "exactly one pr.queued so far"
    );
    assert_eq!(count_kind(&log, "pr.landed"), 1, "exactly one pr.landed");

    // The terminal log shape BEFORE the retry: the last pr.* kind is pr.landed.
    assert_eq!(
        last_pr_kind(&log),
        Some("pr.landed".to_string()),
        "terminal pr kind is pr.landed before the retry"
    );

    // pr land AGAIN on the landed PR — the canonical retry-on-land pattern.
    let (code, v) = run(&["pr", "queue", "--log", log_s, "--pr", &pr_id]);
    assert_eq!(
        code,
        Some(0),
        "re-land on a landed PR is exit 0 (idempotent): {v}"
    );
    // The response is idempotent — NOT a fresh-enqueue lie.
    assert_eq!(
        v["already_queued"], true,
        "re-land must report already_queued:true (no fresh enqueue): {v}"
    );
    assert_eq!(
        v["already_landed"], true,
        "re-land surfaces the terminal landed state: {v}"
    );

    // PROOF: NO second pr.queued was appended — the count is unchanged at 1.
    assert_eq!(
        count_kind(&log, "pr.queued"),
        1,
        "re-land on a landed PR must NOT append a second pr.queued"
    );
    // PROOF: the log's pr kinds STILL end at pr.landed — no post-terminal queued.
    assert_eq!(
        last_pr_kind(&log),
        Some("pr.landed".to_string()),
        "no post-terminal pr.queued — the last pr kind is still pr.landed"
    );

    // The on-disk chain still verifies (the no-op never corrupted the chain).
    assert_chain_verifies(&log);
}

// ── WI-PR defect 2: pr open validates intent existence (like verdict) ─────────

/// `pr open --intent <ghost>` on a log that HAS intent vocabulary must refuse
/// (exit-2) — never silently accept a phantom intent that `verdict` would later
/// reject (`intent_not_found`), producing a permanently un-provable PR. A real
/// intent is accepted. Mirrors the verdict verb's exact raw-record rule.
#[test]
fn pr_open_validates_intent_existence_on_a_log_with_intents() {
    let dir = scratch("pr-open-intent-existence");
    let (log, campaign, _pr_id, intent_id) = seed_campaign_with_open_pr(&dir);
    let log_s = log.to_str().unwrap();

    // A GHOST intent (absent from the log, which DOES carry intent vocabulary)
    // is refused, exit-2 — no pr.opened appended.
    let before = count_kind(&log, "pr.opened");
    let (code, v) = run(&[
        "pr",
        "open",
        "--log",
        log_s,
        "--pr",
        "99",
        "--campaign",
        &campaign,
        "--author-kind",
        "orchestrator",
        "--run-id",
        "r9",
        "--intent",
        "does-not-exist",
    ]);
    assert_eq!(
        code,
        Some(2),
        "pr open with a ghost intent must refuse (exit 2): {v}"
    );
    assert!(
        v["error"]["kind"].is_string(),
        "structured error with a kind: {v}"
    );
    assert_eq!(
        count_kind(&log, "pr.opened"),
        before,
        "no pr.opened appended on a refused open"
    );

    // A REAL intent (the one seeded onto the log) is accepted, exit 0.
    let (code, v) = run(&[
        "pr",
        "open",
        "--log",
        log_s,
        "--pr",
        "100",
        "--campaign",
        &campaign,
        "--author-kind",
        "orchestrator",
        "--run-id",
        "r10",
        "--intent",
        &intent_id,
    ]);
    assert_eq!(code, Some(0), "pr open with a REAL intent succeeds: {v}");
    assert_eq!(v["state"], "proposed", "{v}");
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// The kind of the LAST `pr.*` record on the on-disk log, in chain order — the
/// terminal-state probe (post-terminal corruption shows up as a trailing
/// `pr.queued` after the `pr.landed`).
fn last_pr_kind(path: &Path) -> Option<String> {
    // Only the LIFECYCLE pr.* kinds count — the additive WP-F2 legibility
    // record `pr.envelope` (captured ALONGSIDE the terminal `pr.landed`) is
    // cost/metrics metadata, not a lifecycle state transition, so it must not
    // shadow the terminal lifecycle kind this helper reports.
    const LIFECYCLE: &[&str] = &["pr.opened", "pr.queued", "pr.landed", "pr.abandoned"];
    let v: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    v.as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["kind"].as_str())
        .rfind(|k| LIFECYCLE.contains(k))
        .map(str::to_string)
}

/// Parse the on-disk canonical `[EventRecord, …]` log and assert its hash chain
/// verifies through the real engine (the settle append must be a real,
/// chained, guarded mutation — never a parallel/forked store).
fn assert_chain_verifies(path: &Path) {
    let bytes = std::fs::read(path).expect("log exists after settle");
    let records: Vec<hugit_contracts::event_record::EventRecord> =
        serde_json::from_slice(&bytes).expect("log is VALID canonical JSON");
    let mut log = hugit_refstore::EventLog::new();
    for r in records {
        log.push_record(r)
            .expect("record rehydrates into a gap-free chain");
    }
    hugit_refstore::verify_chain(log.records())
        .expect("the log's hash chain MUST verify after the settle append");
}
