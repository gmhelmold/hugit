//! Acceptance — WP-WH-DOCS: end-to-end wedge chain test.
//!
//! Drives the REAL `hugit` binary through the FULL observable wedge chain as
//! ONE sequence on ONE log, asserting that every projection agrees:
//!
//!   1. `campaign open`  — opens a campaign on the log.
//!   2. `intent new`     — lands an intent bound to that campaign.
//!   3. `pr open`        — opens a PR bundling the intent.
//!   4. `pr land`        — enqueues the PR (pr.queued).
//!   5. `pr land --settle` — settles the PR as landed (pr.landed).
//!   6. `hugit check --def fmt --store` — cold run, cache MISS, records
//!      `check.recorded` with `cache_hit:false`.
//!   7. Re-run `hugit check` with identical inputs — warm HIT or
//!      `already_recorded:true` (either is correct per shipped dedup behaviour).
//!   8. `checks show` — real `hit_rate_pct > 0` (the wedge KPI is non-null).
//!   9. `hugit verdict --intent <id> --lens c --result approve --store` —
//!      records `verdict.recorded`, aggregate == "approve".
//!  10. `campaign show` — `proven >= 1`, `landed >= 1`, `ledger.done >= 1`,
//!      and the projections are coherent (proven == done for this scenario).
//!
//! The chain is driven entirely through the binary; no internal hugit crate is
//! imported beyond what is needed for the chain-verify helper.
//!
//! **Behaviour note (WH-CHECK dedup):** if `hugit check` has idempotency dedup
//! landed (the fix where a re-run with an already-recorded memo_key returns
//! `already_recorded:true, stored:false` rather than appending a duplicate), the
//! warm-run assertion adapts: we assert the STABLE invariants:
//!   - a cold run records successfully;
//!   - `checks show` reports a non-null `hit_rate_pct > 0` (at least one hit
//!     OR at least the cold record, depending on what the re-run appended);
//!   - a `verdict.recorded` flows to `proven` in `campaign show`.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-wh-chain-{tag}-{}-{}",
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
fn run(args: &[&str]) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

/// Bootstrap a tiny source tree (one `lib.rs`) that the tree-axis is scoped
/// over.  Returns the root path to pass as `--root`.
fn seed_tree(dir: &Path) -> PathBuf {
    let root = dir.join("src");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("lib.rs"), b"fn main() {}\n").unwrap();
    root
}

/// Re-read the log and verify the hash chain end-to-end.
fn assert_chain_verifies(path: &Path) {
    let bytes = std::fs::read(path).expect("log exists");
    let records: Vec<hugit_contracts::event_record::EventRecord> =
        serde_json::from_slice(&bytes).expect("log is valid canonical JSON");
    let mut log = hugit_refstore::EventLog::new();
    for r in records {
        log.push_record(r)
            .expect("record rehydrates into a gap-free chain");
    }
    hugit_refstore::verify_chain(log.records())
        .expect("the log's hash chain must verify after every append");
}

// ── The full observable wedge chain — one log, one sequence ─────────────────

#[test]
fn full_wedge_chain_campaign_intent_pr_check_verdict_proven() {
    let dir = scratch("chain");
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    let store = dir.join("store.json");
    let log_s = log.to_str().unwrap();
    let store_s = store.to_str().unwrap();
    let ac_s = ac.to_str().unwrap();
    let campaign = "wh-chain-camp";
    let intent_id = "wh-chain-intent";
    let pr_id = "wh-chain-pr-1";

    let root = seed_tree(&dir);
    let root_s = root.to_str().unwrap();

    // ── Step 1: campaign open ────────────────────────────────────────────────
    let (code, v) = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        campaign,
        "--charter",
        "wh chain e2e test",
        "--owner",
        "test@hugit.dev",
    ]);
    assert_eq!(code, 0, "step 1: campaign open exits 0: {v}");
    assert_chain_verifies(&log);

    // ── Step 2: intent new ───────────────────────────────────────────────────
    let (code, v) = run(&[
        "intent",
        "new",
        "--log",
        log_s,
        "--store",
        store_s,
        "--campaign",
        campaign,
        "--charter",
        "drive the wedge chain",
        "--id",
        intent_id,
    ]);
    assert_eq!(code, 0, "step 2: intent new exits 0: {v}");
    assert_eq!(v["intent_id"], intent_id, "intent id matches: {v}");
    assert_chain_verifies(&log);

    // ── Step 3: pr open ──────────────────────────────────────────────────────
    let (code, v) = run(&[
        "pr",
        "open",
        "--log",
        log_s,
        "--pr",
        pr_id,
        "--campaign",
        campaign,
        "--author-kind",
        "orchestrator",
        "--run-id",
        "run-wh-chain",
        "--intent",
        intent_id,
    ]);
    assert_eq!(code, 0, "step 3: pr open exits 0: {v}");
    assert_eq!(v["state"], "proposed", "PR is proposed: {v}");
    assert_chain_verifies(&log);

    // ── Step 4: pr land (enqueue) ────────────────────────────────────────────
    let (code, v) = run(&["pr", "land", "--log", log_s, "--pr", pr_id]);
    assert_eq!(code, 0, "step 4: pr land exits 0: {v}");
    assert_eq!(v["queued"], true, "PR is queued: {v}");
    assert_chain_verifies(&log);

    // ── Step 5: pr land --settle (land) ─────────────────────────────────────
    let (code, v) = run(&["pr", "land", "--log", log_s, "--pr", pr_id, "--settle"]);
    assert_eq!(code, 0, "step 5: pr land --settle exits 0: {v}");
    assert_eq!(v["landed"], true, "PR landed: {v}");
    assert_eq!(v["already_landed"], false, "first settle: {v}");
    assert_chain_verifies(&log);

    // ── Step 6: hugit check --def fmt --store (cold run) ────────────────────
    //
    // We use the `fmt` built-in def here, matching the spec. The test runs
    // `cargo fmt --check` on a minimal tree; the exit may be 0 (formatted) or
    // non-zero (would reformat), but the VERB exit is always 0 (it reports the
    // check outcome, not panics).  We assert the structural invariants — the
    // verb runs, records, and the key is a 64-char digest — not the gate result.
    let check_args = &[
        "check",
        "--def",
        "fmt",
        "--log",
        log_s,
        "--store",
        "--ac",
        ac_s,
        "--root",
        root_s,
        "--toolchain",
        "wh-chain-tc",
        "--timeout-secs",
        "60",
    ];
    let (code, cold) = run(check_args);
    assert_eq!(code, 0, "step 6: check --def fmt --store exits 0: {cold}");
    assert_eq!(cold["cache_hit"], false, "cold run is a MISS: {cold}");
    assert_eq!(
        cold["local_executions"], 1,
        "cold run executes once: {cold}"
    );
    assert_eq!(cold["stored"], true, "--store recorded the check: {cold}");
    let cold_key = cold["memo_key"].as_str().expect("memo_key present");
    assert_eq!(cold_key.len(), 64, "memo key is a 64-char sha256: {cold}");
    assert_chain_verifies(&log);

    // ── Step 7: warm re-run (identical inputs) ───────────────────────────────
    //
    // The wedge's CORE promise: a 2nd identical `check --store` is a genuine
    // cache HIT — serviced from the AC store with ZERO local execution, same
    // memo key. (The cold MISS is (key,false); this warm run is (key,true) — a
    // distinct dedup tuple, so WH-CHECK records it as the HIT row, NOT an
    // already_recorded no-op. A no-op/miss here would be the wedge FAILING, so
    // we assert the HIT positively rather than accepting "hit OR deduped".)
    let (warm_code, warm) = run(check_args);
    assert_eq!(warm_code, 0, "step 7: warm re-run exits 0: {warm}");
    assert_eq!(
        warm["cache_hit"].as_bool(),
        Some(true),
        "step 7: warm re-run with identical inputs MUST be a cache HIT (the wedge): {warm}"
    );
    assert_eq!(
        warm["local_executions"], 0,
        "step 7: a HIT performs zero local execution: {warm}"
    );
    assert_eq!(
        warm["memo_key"].as_str().unwrap_or(""),
        cold_key,
        "step 7: identical inputs key to the same memo key: {warm}"
    );
    assert_chain_verifies(&log);

    // ── Step 8: checks show — real hit_rate > 0 ──────────────────────────────
    //
    // At least one `check.recorded` event is on the log (from step 6).  Whether
    // step 7 appended a HIT row or returned `already_recorded`, the cold MISS
    // row is enough for `checks show` to report a non-null `hit_rate_pct`.
    // The rate is either 50% (miss + hit rows) or 0% / null if only the MISS
    // row landed.  We assert what is STABLE: the KPIs are non-null and
    // `checks show` exits 0 with a positive check_count.
    let (code, show) = run(&["checks", "show", "--log", log_s]);
    assert_eq!(code, 0, "step 8: checks show exits 0: {show}");
    assert!(
        show["check_count"].as_u64().unwrap_or(0) >= 1,
        "step 8: at least one check.recorded on the log: {show}"
    );
    let kpis = &show["kpis"];
    // Step 7 proved a HIT row landed (cold MISS + warm HIT), so the wedge KPI
    // is UNCONDITIONALLY non-null and > 0 here — no is_hit/deduped escape hatch
    // (that branch masked a possible warm-miss; killed in WI-TESTS follow-up).
    assert!(
        !kpis["hit_rate_pct"].is_null(),
        "step 8: with a HIT row on the log, hit_rate_pct is non-null: {kpis}"
    );
    let rate = kpis["hit_rate_pct"].as_f64().unwrap_or(0.0);
    assert!(rate > 0.0, "step 8: hit_rate_pct > 0 after a HIT: {kpis}");
    // Regardless of the warm-run outcome, executed is >= 1 (the cold run).
    assert!(
        !kpis["executed"].is_null(),
        "step 8: executed is non-null (cold run was real): {kpis}"
    );
    assert!(
        kpis["executed"].as_u64().unwrap_or(0) >= 1,
        "step 8: at least one execution recorded: {kpis}"
    );

    // ── Step 9: hugit verdict --intent --lens c --result approve --store ─────
    let (code, v) = run(&[
        "verdict", "--log", log_s, "--store", "--intent", intent_id, "--lens", "c", "--result",
        "approve",
    ]);
    assert_eq!(code, 0, "step 9: verdict exits 0: {v}");
    assert_eq!(
        v["verdict_recorded"], true,
        "step 9: verdict_recorded true: {v}"
    );
    assert_eq!(v["already_recorded"], false, "step 9: first record: {v}");
    assert_eq!(v["stored"], true, "step 9: --store recorded: {v}");
    assert_eq!(v["aggregate"], "approve", "step 9: aggregate approve: {v}");
    assert_chain_verifies(&log);

    // ── Step 10: campaign show — proven >= 1, landed >= 1, coherent ──────────
    //
    // The WG-COHERENCE fix (B3: intent.landed carries campaign; B4: verdict →
    // proven) must hold: proven >= 1, done >= 1, landed >= 1.
    let (code, v) = run(&["campaign", "show", "--log", log_s, "--campaign", campaign]);
    assert_eq!(code, 0, "step 10: campaign show exits 0: {v}");

    let landed = v["progress"]["landed"].as_u64().unwrap_or(0);
    let proven = v["ledger"]["proven"].as_u64().unwrap_or(0);
    let done = v["ledger"]["done"].as_u64().unwrap_or(0);

    assert!(
        landed >= 1,
        "step 10: campaign show must report progress.landed >= 1 after pr land --settle: {v}"
    );
    assert!(
        done >= 1,
        "step 10: campaign show must report ledger.done >= 1 (intent landed): {v}"
    );
    assert!(
        proven >= 1,
        "step 10: campaign show must report ledger.proven >= 1 after approve verdict (WG-COHERENCE): {v}"
    );
    // Coherence: proven == done for this single-intent, single-approve scenario.
    assert_eq!(
        proven, done,
        "step 10: proven == done — the approve verdict accounts for every landed intent: {v}"
    );
}
