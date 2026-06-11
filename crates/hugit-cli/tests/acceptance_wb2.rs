//! Acceptance — WP-WB2: the wedge made visible (`hugit checks` / `hugit queue`).
//!
//! SOTA-audit Tier-2 P1 ("the wedge is invisible"). These tests drive the REAL
//! `hugit` binary (`CARGO_BIN_EXE_hugit`) over REAL logs — the queue case builds
//! its log through the canonical `pr open` / `pr land` append paths (the PC4
//! pattern); the checks case crafts a `[EventRecord, …]` log via the engine's
//! own `EventLog::append` (the same primitive every verb writes through) so the
//! `check.recorded` rows are honestly-built, never hand-forged hashes.
//!
//! What is pinned:
//!   - `checks key` is byte-identical to `hugit_refstore::compute_memo_key` for
//!     the same three axes (engine parity — the agent can predict cache behavior).
//!   - `checks show` aggregates the hit-rate KPIs correctly over a crafted log,
//!     and is honest-null (never a fabricated 0%) on a log with no check records.
//!   - `queue show` orders by the queue's `order_index` and groups entries by
//!     campaign (union-batch composition), with the verdict honestly `null`.
//!   - every output is stable JSON under the WB0 one-error/one-exit law
//!     (`log_not_found` / `parse_log` are the canonical envelopes, exit 2).

use std::path::PathBuf;
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::{Value, json};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-wb2-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit <args>` and return `(exit_code, parsed_stdout_json)`.
fn run(args: &[&str]) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

/// Write a `[EventRecord, …]` log to `path`, appending each `(kind, payload)`
/// through the engine's canonical [`EventLog::append`] — the same primitive
/// every porcelain verb writes through (so the hash chain is real, not forged).
fn write_log(path: &std::path::Path, events: &[(&str, Value)]) {
    let mut log = EventLog::new();
    for (kind, payload) in events {
        // `append` computes the hash chain; payload is canonical JSON (sorted by
        // serde_json::to_string over a json! object is sufficient for the read,
        // which re-parses the payload string).
        log.append(*kind, vec![], payload.to_string(), 0);
    }
    let json = serde_json::to_string_pretty(log.records()).unwrap();
    std::fs::write(path, json).unwrap();
}

// ─────────────────────────────────────────────────────────────────────────────
// checks key — engine parity (predict cache behavior before running).
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn checks_key_matches_the_engine_memo_key_primitive() {
    let (code, v) = run(&[
        "checks",
        "key",
        "--tree",
        "deadbeef",
        "--def",
        "cafef00d",
        "--toolchain",
        "0badc0de",
    ]);
    assert_eq!(code, 0, "checks key exits 0: {v}");

    // PARITY: the printed key IS the engine's own computation for the same axes.
    let expected = hugit_refstore::compute_memo_key("deadbeef", "cafef00d", "0badc0de");
    assert_eq!(v["memo_key"], expected, "memo key must match the engine");
    assert_eq!(v["axes"]["tree_hash"], "deadbeef");
    assert_eq!(v["axes"]["def_digest"], "cafef00d");
    assert_eq!(v["axes"]["toolchain_digest"], "0badc0de");
}

// ─────────────────────────────────────────────────────────────────────────────
// checks show — hit-rate aggregation over a crafted, canonically-built log.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn checks_show_aggregates_hit_rate_on_a_crafted_log() {
    let dir = scratch("checks-show");
    let log = dir.join("log.json");
    // 3 hits, 1 executed → hit-rate 75.00%; saved_ms = 100+50+30 = 180.
    write_log(
        &log,
        &[
            (
                "check.recorded",
                json!({"name":"fmt","exit":0,"duration_ms":100,"cache_hit":true,"memo_key":"aaaa"}),
            ),
            (
                "check.recorded",
                json!({"name":"clippy","exit":0,"duration_ms":50,"cache_hit":true,"memo_key":"bbbb"}),
            ),
            (
                "check.recorded",
                json!({"name":"test","exit":0,"duration_ms":30,"cache_hit":true,"memo_key":"cccc"}),
            ),
            (
                "check.recorded",
                json!({"name":"audit","exit":1,"duration_ms":200,"cache_hit":false,"memo_key":"dddd"}),
            ),
        ],
    );

    let (code, v) = run(&["checks", "show", "--log", log.to_str().unwrap()]);
    assert_eq!(code, 0, "checks show exits 0: {v}");
    assert_eq!(v["check_count"], 4);
    assert_eq!(v["kpis"]["hits"], 3);
    assert_eq!(v["kpis"]["executed"], 1);
    assert_eq!(v["kpis"]["hit_rate_pct"], json!(75.0));
    assert_eq!(v["kpis"]["saved_ms"], 180);

    // Per-check rows carry name, ok, cache_hit, and the truncated + full memo key.
    let rows = v["checks"].as_array().unwrap();
    assert_eq!(rows.len(), 4);
    let fmt = &rows[0];
    assert_eq!(fmt["name"], "fmt");
    assert_eq!(fmt["ok"], true);
    assert_eq!(fmt["cache_hit"], true);
    assert_eq!(fmt["memo_key"], "aaaa");
    assert_eq!(fmt["memo_key_short"], "aaaa");
    let audit = &rows[3];
    assert_eq!(audit["ok"], false, "exit 1 → ok false");
    assert_eq!(audit["cache_hit"], false);
}

#[test]
fn checks_show_is_honest_null_on_a_log_with_no_checks() {
    let dir = scratch("checks-empty");
    let log = dir.join("log.json");
    // A real log with a non-check event — checks projection must be honest-empty.
    write_log(&log, &[("pr.opened", json!({"pr_id":"1"}))]);

    let (code, v) = run(&["checks", "show", "--log", log.to_str().unwrap()]);
    assert_eq!(code, 0, "checks show exits 0 on a checkless log: {v}");
    assert_eq!(v["check_count"], 0);
    assert!(v["checks"].as_array().unwrap().is_empty());
    // Honest nulls — NEVER a fabricated 0% hit-rate.
    assert!(v["kpis"]["hit_rate_pct"].is_null());
    assert!(v["kpis"]["hits"].is_null());
    assert!(v["kpis"]["saved_ms"].is_null());
    // The gap is disclosed, not silent.
    assert!(v["note"].is_string());
}

#[test]
fn checks_show_scopes_to_pr_and_skips_unknown_cache_hits() {
    let dir = scratch("checks-pr");
    let log = dir.join("log.json");
    write_log(
        &log,
        &[
            (
                "check.recorded",
                json!({"name":"fmt","exit":0,"duration_ms":10,"cache_hit":true,"pr_id":"7"}),
            ),
            // A row with NO cache_hit captured — must not skew the KPIs.
            (
                "check.recorded",
                json!({"name":"clippy","exit":0,"duration_ms":99,"pr_id":"7"}),
            ),
            // A different PR — excluded under --pr 7.
            (
                "check.recorded",
                json!({"name":"test","exit":0,"cache_hit":false,"pr_id":"8"}),
            ),
        ],
    );

    let (code, v) = run(&[
        "checks",
        "show",
        "--log",
        log.to_str().unwrap(),
        "--pr",
        "7",
    ]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["check_count"], 2, "only PR-7 rows");
    // 1 known hit, 0 known executed (the second row's cache_hit is unknown).
    assert_eq!(v["kpis"]["hits"], 1);
    assert_eq!(v["kpis"]["executed"], 0);
    assert_eq!(v["kpis"]["hit_rate_pct"], json!(100.0));
}

// ─────────────────────────────────────────────────────────────────────────────
// queue show — real ordering + campaign batch composition over the append path.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn queue_show_orders_and_groups_by_campaign() {
    let dir = scratch("queue-show");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();

    // Build a real queue THROUGH the canonical append paths: open + land three
    // PRs across two campaigns (the PC4 pattern, real binary).
    for (pr, campaign) in [("1", "auth"), ("2", "billing"), ("3", "auth")] {
        let (code, v) = run(&[
            "pr",
            "open",
            "--log",
            log_s,
            "--pr",
            pr,
            "--campaign",
            campaign,
            "--author-kind",
            "orchestrator",
            "--run-id",
            "r",
            "--intent",
            &format!("i{pr}"),
        ]);
        assert_eq!(code, 0, "pr open {pr}: {v}");
        let (code, v) = run(&["pr", "land", "--log", log_s, "--pr", pr]);
        assert_eq!(code, 0, "pr land {pr}: {v}");
    }

    let (code, v) = run(&["queue", "show", "--log", log_s]);
    assert_eq!(code, 0, "queue show exits 0: {v}");
    assert_eq!(v["queue_depth"], 3);

    // Entries are in queue (order_index) order: PR 1 → 2 → 3 at positions 0,1,2.
    let entries = v["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0]["pr_id"], "1");
    assert_eq!(entries[0]["position"], 0);
    assert_eq!(entries[0]["mode"], "union");
    assert_eq!(entries[0]["campaign"], "auth");
    assert_eq!(entries[1]["pr_id"], "2");
    assert_eq!(entries[1]["position"], 1);
    assert_eq!(entries[2]["position"], 2);
    // The verdict is honestly null (no verdict seam yet) — never faked.
    assert!(entries[0]["verdict"].is_null());

    // Batches GROUP by campaign: auth = {1,3}, billing = {2}.
    let batches = v["batches"].as_array().unwrap();
    assert_eq!(batches.len(), 2, "two campaigns → two union batches");
    let auth = batches.iter().find(|b| b["campaign"] == "auth").unwrap();
    assert_eq!(auth["member_count"], 2);
    let members: Vec<&str> = auth["members"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m.as_str().unwrap())
        .collect();
    assert!(members.contains(&"1") && members.contains(&"3"));
    assert!(auth["verdict"].is_null());
    assert!(auth["implicated_pr"].is_null());
    // The null verdict is disclosed, not silent.
    assert!(v["verdict_note"].is_string());
}

#[test]
fn queue_show_scopes_to_one_campaign() {
    let dir = scratch("queue-scope");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();
    for (pr, campaign) in [("1", "auth"), ("2", "billing")] {
        run(&[
            "pr",
            "open",
            "--log",
            log_s,
            "--pr",
            pr,
            "--campaign",
            campaign,
            "--author-kind",
            "orchestrator",
            "--run-id",
            "r",
            "--intent",
            &format!("i{pr}"),
        ]);
        run(&["pr", "land", "--log", log_s, "--pr", pr]);
    }
    let (code, v) = run(&["queue", "show", "--log", log_s, "--campaign", "billing"]);
    assert_eq!(code, 0, "{v}");
    assert_eq!(v["queue_depth"], 1, "only the billing entry");
    assert_eq!(v["entries"][0]["pr_id"], "2");
    assert_eq!(v["batches"].as_array().unwrap().len(), 1);
}

// ─────────────────────────────────────────────────────────────────────────────
// The one error/exit law — log_not_found / parse_log, exit 2, canonical shape.
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn missing_log_is_the_canonical_log_not_found_envelope_exit_two() {
    for verb in [
        vec!["checks", "show", "--log", "/no/such/log.json"],
        vec!["queue", "show", "--log", "/no/such/log.json"],
    ] {
        let (code, v) = run(&verb);
        assert_eq!(code, 2, "log_not_found is exit 2 ({verb:?}): {v}");
        assert_eq!(v["error"]["kind"], "log_not_found");
        assert_eq!(v["error"]["path"], "/no/such/log.json");
        assert!(v["error"]["fix"].is_string());
        // Nested under "error", never flat.
        assert!(v.get("kind").is_none());
    }
}

#[test]
fn malformed_log_is_the_canonical_parse_log_envelope_exit_two() {
    let dir = scratch("parse");
    let log = dir.join("trunc.json");
    std::fs::write(&log, b"{not valid json").unwrap();
    let log_s = log.to_str().unwrap();

    for verb in [
        vec!["checks", "show", "--log", log_s],
        vec!["queue", "show", "--log", log_s],
    ] {
        let (code, v) = run(&verb);
        assert_eq!(code, 2, "parse_log is exit 2 ({verb:?}): {v}");
        assert_eq!(v["error"]["kind"], "parse_log");
        assert_eq!(v["error"]["path"], log_s);
    }
}
