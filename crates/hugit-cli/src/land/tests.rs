//! Unit tests for the `hugit land queue` batch-land engine driver.
//!
//! These exercise the library core ([`batch_land`]) directly over a synthetic
//! event log + a fresh [`hugit_checks::client::ac::InMemoryAc`], so the
//! union-test + bisect + memoize wire is proven WITHOUT the CLI shell or the
//! filesystem. The CLI shell (`run_queue_land`) is a thin lock+load+persist
//! wrapper over this core; the cross-process file-backed-AC path is covered by
//! the integration acceptance suite.

use std::collections::BTreeMap;

use hugit_checks::client::ac::InMemoryAc;
use hugit_refstore::EventLog;
use serde_json::{Value, json};

use super::*;

/// Seed a `pr.opened` + `pr.queued` for a PR onto a synthetic log via the
/// test-support raw shim (the canonical fixture path).
fn seed_pr(log: &mut EventLog, pr_id: &str, campaign: &str, intents: &[&str], order: u64) {
    log.append_for_test(
        "pr.opened",
        vec!["orchestrator:hugit".to_string()],
        json!({
            "pr_id": pr_id,
            "campaign": campaign,
            "author_kind": "orchestrator",
            "run_id": "run-1",
            "principal": null,
            "intent_ids": intents,
        })
        .to_string(),
        0,
    );
    log.append_for_test(
        "pr.queued",
        vec!["orchestrator:hugit".to_string()],
        json!({
            "pr_id": pr_id,
            "item_id": format!("{pr_id}#{order}"),
            "order_index": order,
            "mode": "union",
        })
        .to_string(),
        0,
    );
}

/// The set of pr_ids the log records as terminally landed (`pr.landed`).
fn landed_set(log: &EventLog) -> std::collections::BTreeSet<String> {
    log.records()
        .iter()
        .filter(|r| r.kind == "pr.landed")
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter_map(|v| v.get("pr_id").and_then(Value::as_str).map(str::to_string))
        .collect()
}

/// A batch of 2 PRs whose union is GREEN → BOTH land.
#[test]
fn green_union_lands_both() {
    let mut log = EventLog::new();
    seed_pr(&mut log, "PR-1", "camp-a", &["I-1"], 0);
    seed_pr(&mut log, "PR-2", "camp-a", &["I-2"], 1);
    let ac = InMemoryAc::new();

    let out = batch_land(&mut log, &ac, Some("camp-a"), 0).expect("batch land");

    assert_eq!(out["verdict"], "green");
    assert_eq!(out["queued"], 2);
    assert_eq!(out["landed"], json!(["PR-1", "PR-2"]));
    assert_eq!(out["excluded"], json!([]));
    assert!(out["failing_pair"].is_null());
    // Both PRs are now terminally landed on the log.
    let landed = landed_set(&log);
    assert!(landed.contains("PR-1") && landed.contains("PR-2"));
    // No union-fail recorded on a green union.
    assert!(
        !log.records()
            .iter()
            .any(|r| r.kind == QUEUE_UNION_FAIL_KIND)
    );
}

/// A batch whose union is RED via a genuine pair interaction (PR-A declares
/// `conflicts-with:PR-B`, but each is individually green): `bisect_failure`
/// identifies the minimal failing pair, the green remainder lands, the pair is
/// excluded, and a `queue.union_fail` carries the bisected pair.
#[test]
fn red_union_bisects_pair_lands_remainder_records_fail() {
    let mut log = EventLog::new();
    // PR-A and PR-B form a failing pair; PR-C is disjoint-green.
    seed_pr(&mut log, "PR-A", "camp-b", &["conflicts-with:PR-B"], 0);
    seed_pr(&mut log, "PR-B", "camp-b", &["I-B"], 1);
    seed_pr(&mut log, "PR-C", "camp-b", &["I-C"], 2);
    let ac = InMemoryAc::new();

    let out = batch_land(&mut log, &ac, Some("camp-b"), 0).expect("batch land");

    assert_eq!(out["verdict"], "red");
    assert_eq!(out["locus"], "pair");
    // The minimal failing pair is named (A,B in canonical order).
    let pair = &out["failing_pair"];
    let a = pair["item_a"].as_str().unwrap();
    let b = pair["item_b"].as_str().unwrap();
    assert_eq!(
        {
            let mut v = [a, b];
            v.sort_unstable();
            v
        },
        ["PR-A", "PR-B"]
    );
    // The green remainder (PR-C) lands; the pair is excluded.
    assert_eq!(out["landed"], json!(["PR-C"]));
    let excluded: Vec<&str> = out["excluded"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(excluded.contains(&"PR-A") && excluded.contains(&"PR-B"));

    // Only PR-C is landed; the culprits are NOT landed.
    let landed = landed_set(&log);
    assert_eq!(
        landed,
        std::iter::once("PR-C".to_string()).collect::<std::collections::BTreeSet<_>>()
    );

    // A `queue.union_fail` is recorded carrying the bisected pair.
    let fail = log
        .records()
        .iter()
        .find(|r| r.kind == QUEUE_UNION_FAIL_KIND)
        .expect("queue.union_fail recorded");
    let payload: Value = serde_json::from_str(&fail.payload).unwrap();
    assert_eq!(payload["locus"], "pair");
    assert_eq!(payload["campaign"], "camp-b");
    let recorded: Vec<&str> = [
        payload["item_a"].as_str().unwrap(),
        payload["item_b"].as_str().unwrap(),
    ]
    .into_iter()
    .collect();
    assert!(recorded.contains(&"PR-A") && recorded.contains(&"PR-B"));
}

/// The memo cache makes a RE-RUN a HIT: priming a shared AC with the first
/// evaluation means a second evaluation of the SAME union performs ZERO local
/// executions (the wedge — `run_memoized` returns before the runner runs).
#[test]
fn rerun_is_a_cache_hit_zero_execution() {
    // Build the oracle inputs directly so we can prove the executed-count drop
    // across two evaluations sharing ONE AC (the cross-call memo wedge).
    let content: BTreeMap<String, Vec<String>> = [
        ("PR-1".to_string(), vec!["I-1".to_string()]),
        ("PR-2".to_string(), vec!["I-2".to_string()]),
    ]
    .into_iter()
    .collect();
    let conflicts = std::collections::BTreeSet::new();
    let def = local_check_def();
    let ac = InMemoryAc::new();

    // First batch land: a cold AC → the per-PR checks EXECUTE (miss).
    let mut log1 = EventLog::new();
    seed_pr(&mut log1, "PR-1", "camp-a", &["I-1"], 0);
    seed_pr(&mut log1, "PR-2", "camp-a", &["I-2"], 1);
    let out1 = batch_land(&mut log1, &ac, Some("camp-a"), 0).expect("first land");
    assert_eq!(out1["verdict"], "green");
    let executed_first = out1["executed_count"].as_u64().unwrap();
    assert!(
        executed_first > 0,
        "a cold AC must execute the per-PR checks at least once (got {executed_first})"
    );

    // Re-evaluate the SAME union over the SAME (now-warm) AC directly through the
    // oracle: every per-PR check is served from the AC → ZERO executions.
    {
        let mut oracle = LogMemoOracle {
            ac: &ac,
            content: &content,
            conflicts: &conflicts,
            check_def: &def,
        };
        let (verdict, sources) = oracle.evaluate(&["PR-1", "PR-2"]);
        assert_eq!(verdict, hugit_queue::core::union::UnionVerdict::Green);
        let executed = sources
            .iter()
            .filter(|s| **s == hugit_queue::core::union::CheckSource::Executed)
            .count();
        assert_eq!(
            executed, 0,
            "a warm AC re-run must be a HIT — zero local executions (the wedge)"
        );
    }

    // And the AC actually holds memoized results (sanity: it is not empty).
    assert!(
        !ac.is_empty(),
        "the AC must have stored the cold-run results"
    );
}

/// Batch land is idempotent: re-running over a queue whose PRs are already
/// landed is a no-op (no second `pr.landed`, no `queue.union_fail`).
#[test]
fn rerun_after_land_is_idempotent() {
    let mut log = EventLog::new();
    seed_pr(&mut log, "PR-1", "camp-a", &["I-1"], 0);
    let ac = InMemoryAc::new();

    let _ = batch_land(&mut log, &ac, Some("camp-a"), 0).expect("first land");
    let landed_count_1 = log
        .records()
        .iter()
        .filter(|r| r.kind == "pr.landed")
        .count();
    assert_eq!(landed_count_1, 1);

    // PR-1 has left the active queue (landed) → the second run finds nothing.
    let out2 = batch_land(&mut log, &ac, Some("camp-a"), 0).expect("second land");
    assert_eq!(out2["queued"], 0);
    let landed_count_2 = log
        .records()
        .iter()
        .filter(|r| r.kind == "pr.landed")
        .count();
    assert_eq!(landed_count_2, 1, "no second pr.landed — idempotent");
}

/// An empty queue (or a campaign scope matching nothing) is an honest no-op:
/// green verdict, nothing landed, a disclosing note — never an error or a fake.
#[test]
fn empty_queue_is_honest_noop() {
    let mut log = EventLog::new();
    let ac = InMemoryAc::new();
    let out = batch_land(&mut log, &ac, Some("nope"), 0).expect("empty land");
    assert_eq!(out["queued"], 0);
    assert_eq!(out["verdict"], "green");
    assert_eq!(out["landed"], json!([]));
    assert!(out["note"].is_string());
}

/// `conflict_pairs` keeps only pairs where BOTH PRs are present and stores them
/// canonically (sorted, symmetric); a conflict with an absent PR is inert.
#[test]
fn conflict_pairs_are_canonical_and_bounded() {
    let content: BTreeMap<String, Vec<String>> = [
        ("PR-B".to_string(), vec!["conflicts-with:PR-A".to_string()]),
        ("PR-A".to_string(), vec!["I-A".to_string()]),
        // A conflict with an ABSENT PR is inert.
        (
            "PR-Z".to_string(),
            vec!["conflicts-with:PR-GONE".to_string()],
        ),
    ]
    .into_iter()
    .collect();
    let pairs = conflict_pairs(&content);
    assert_eq!(pairs.len(), 1);
    assert!(pairs.contains(&("PR-A".to_string(), "PR-B".to_string())));
}
