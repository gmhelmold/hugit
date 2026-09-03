//! `hugit land queue` acceptance — the batch-land front door over the REAL
//! `hugit` binary (`CARGO_BIN_EXE_hugit`), driving the actual union-test +
//! bisect + memoize engine end to end through the operator-invocable verb.
//!
//! This proves the wedge is REACHABLE by an operator (the missing front door):
//! a real `hugit pr open` + `hugit pr queue` builds the queue, `hugit land
//! queue` folds it into a `Batch`, runs `evaluate_union` over a real `MemoCheck`
//! oracle backed by `run_memoized` + the file-backed AC (the SAME `<log>.ac`
//! seam `hugit check run --store` uses), lands the green set, and on a red union
//! bisects to the minimal failing pair (recording `queue.union_fail` so `hugit
//! queue show`'s `failing_pair` lights up).
//!
//! Honest scope: single-tenant LOCAL operator (file-backed AC, local memoized
//! execution); the distributed runner fabric swaps in behind the same MemoCheck
//! trait. No fabricated verdicts — a clean local union is honestly green.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-landq-{tag}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn run(args: &[&str]) -> Output {
    Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs")
}

fn json_of(out: &Output) -> Value {
    serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap_or(Value::Null)
}

/// Open a PR bundling `intents`, then queue it (the standard operator ceremony).
fn open_and_queue(log: &str, pr: &str, campaign: &str, intents: &[&str]) {
    let mut args = vec![
        "pr",
        "open",
        "--log",
        log,
        "--pr",
        pr,
        "--campaign",
        campaign,
        "--author-kind",
        "orchestrator",
        "--run-id",
        "run-1",
    ];
    for i in intents {
        args.push("--intent");
        args.push(i);
    }
    let out = run(&args);
    assert!(
        out.status.success(),
        "pr open {pr}: {}",
        String::from_utf8_lossy(&out.stdout)
    );
    let out = run(&["pr", "queue", "--log", log, "--pr", pr]);
    assert!(
        out.status.success(),
        "pr queue {pr}: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// A 2-PR batch whose union is GREEN → both land, no failure recorded.
#[test]
fn green_batch_lands_both_over_the_binary() {
    let dir = scratch("green");
    let log = dir.join("log.json");
    let log = log.to_str().unwrap();

    open_and_queue(log, "PR-1", "camp-a", &["I-1"]);
    open_and_queue(log, "PR-2", "camp-a", &["I-2"]);

    let out = run(&["land", "queue", "--log", log, "--campaign", "camp-a"]);
    assert!(out.status.success(), "land queue exits 0 on a green union");
    let v = json_of(&out);
    assert_eq!(v["verdict"], "green");
    assert_eq!(v["queued"], 2);
    assert_eq!(v["landed"], serde_json::json!(["PR-1", "PR-2"]));
    assert!(v["failing_pair"].is_null());

    // Both PRs now project as landed (they have left the active queue).
    let show = run(&["queue", "show", "--log", log, "--campaign", "camp-a"]);
    let sv = json_of(&show);
    assert_eq!(sv["queue_depth"], 0, "landed PRs leave the active queue");
}

/// A batch whose union is RED via a genuine pair interaction: `bisect_failure`
/// names the minimal failing pair, the green remainder lands, the culprits are
/// excluded, and `queue show`'s `failing_pair` lights up from the recorded
/// `queue.union_fail`.
#[test]
fn red_batch_bisects_and_lights_up_queue_show() {
    let dir = scratch("red");
    let log = dir.join("log.json");
    let log = log.to_str().unwrap();

    // PR-A declares a conflict with PR-B (a pair interaction; each green alone).
    open_and_queue(log, "PR-A", "camp-b", &["conflicts-with:PR-B"]);
    open_and_queue(log, "PR-B", "camp-b", &["I-B"]);
    open_and_queue(log, "PR-C", "camp-b", &["I-C"]);

    let out = run(&["land", "queue", "--log", log, "--campaign", "camp-b"]);
    assert!(
        out.status.success(),
        "land queue exits 0 (the green set lands)"
    );
    let v = json_of(&out);
    assert_eq!(v["verdict"], "red");
    assert_eq!(v["locus"], "pair");

    // The minimal failing pair is {PR-A, PR-B}.
    let a = v["failing_pair"]["item_a"].as_str().unwrap();
    let b = v["failing_pair"]["item_b"].as_str().unwrap();
    let mut pair = [a, b];
    pair.sort_unstable();
    assert_eq!(pair, ["PR-A", "PR-B"]);

    // The green remainder (PR-C) lands; the pair is excluded.
    assert_eq!(v["landed"], serde_json::json!(["PR-C"]));
    let excluded: Vec<&str> = v["excluded"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x.as_str().unwrap())
        .collect();
    assert!(excluded.contains(&"PR-A") && excluded.contains(&"PR-B"));

    // `queue show` now lights up the bisected failing pair (was hardcoded null).
    let show = run(&["queue", "show", "--log", log, "--campaign", "camp-b"]);
    let sv = json_of(&show);
    let batch = &sv["batches"][0];
    assert!(
        !batch["failing_pair"].is_null(),
        "queue show failing_pair lights up after a batch land bisects: {batch}"
    );
    let sa = batch["failing_pair"]["item_a"].as_str().unwrap();
    let sb = batch["failing_pair"]["item_b"].as_str().unwrap();
    let mut spair = [sa, sb];
    spair.sort_unstable();
    assert_eq!(spair, ["PR-A", "PR-B"]);
}

/// The memo wedge across PROCESSES: a second `hugit land queue` over the same
/// `<log>.ac` re-uses the cached per-PR check results — zero executions on the
/// re-run (the file-backed AC survives across binary invocations). We assert the
/// re-run's executed_count is 0 (cache hits) while the first run executed.
#[test]
fn rerun_is_a_cross_process_cache_hit() {
    let dir = scratch("memo");
    let log = dir.join("log.json");
    let log = log.to_str().unwrap();

    // Two same-campaign PRs that do NOT land yet: we read the union verdict twice
    // WITHOUT settling, by scoping the campaign so the engine re-evaluates the
    // same union. To keep them in the queue across runs we make the union RED via
    // a pair conflict (a red union excludes the pair and lands nothing here since
    // every member is a culprit), so the queue persists for the second probe.
    open_and_queue(log, "PR-X", "camp-c", &["conflicts-with:PR-Y"]);
    open_and_queue(log, "PR-Y", "camp-c", &["I-Y"]);

    let first = json_of(&run(&[
        "land",
        "queue",
        "--log",
        log,
        "--campaign",
        "camp-c",
    ]));
    assert_eq!(first["verdict"], "red");
    let first_exec = first["executed_count"].as_u64().unwrap();
    assert!(
        first_exec > 0,
        "the cold AC must execute the per-PR checks at least once (got {first_exec})"
    );

    // The pair is excluded (nothing landed) so both PRs remain queued. A second
    // batch land re-probes the SAME union over the now-warm `<log>.ac` → every
    // per-PR check is an AC HIT, zero executions (the cross-process memo wedge).
    let second = json_of(&run(&[
        "land",
        "queue",
        "--log",
        log,
        "--campaign",
        "camp-c",
    ]));
    assert_eq!(second["verdict"], "red");
    assert_eq!(
        second["executed_count"].as_u64().unwrap(),
        0,
        "a warm cross-process re-run must be all cache HITs — zero executions (the wedge)"
    );

    // The `<log>.ac` file exists (the wedge state persisted to disk).
    assert!(
        dir.join("log.json.ac").exists(),
        "the file-backed AC persisted across the binary invocations"
    );
}

// ── local-only determinism (owner decision) ────────────────────────────────

#[test]
fn corelink_env_is_inert_land_stays_local() {
    let dir = std::env::temp_dir().join(format!("hugit-land-localonly-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("log.json"), b"[]\n").unwrap();
    let log = dir.join("log.json").to_str().unwrap().to_string();

    // Set hostile CoreLink env — must have NO effect (the local FileAc is the
    // only backend; no network is ever attempted).
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args(["land", "queue", "--log", &log])
        .current_dir(&dir)
        .env("HUGIT_CORELINK_AC_URL", "https://hostile.example.invalid")
        .env("HUGIT_CORELINK_TENANT", "hugit")
        .env("HUGIT_CORELINK_PAT", "clp_secret")
        .output()
        .expect("land runs with hostile CoreLink env");
    assert_eq!(out.status.code(), Some(0), "land succeeds with hostile CoreLink env (local-only)");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.to_lowercase().contains("hostile"),
        "no network/hostile resolution happened: {stderr}"
    );
}
