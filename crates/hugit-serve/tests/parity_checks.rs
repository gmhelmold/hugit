//! Parity tests for `build_checks`: empty log → honest defaults; populated log →
//! real KPI aggregation; serialization round-trip through `ChecksVm` is lossless.

use hugit_http_contracts::ChecksVm;
use hugit_refstore::EventLog;
use hugit_serve::handlers::build_checks;

/// Empty log → KPIs are the honest "NO DATA" defaults; fleet-KPI stubs are zero/empty;
/// culprit/bisect/hero_red are None; round-trip through JSON is lossless.
#[test]
fn empty_log_produces_honest_no_data_defaults() {
    let log = EventLog::new();
    let vm = build_checks(&log, "hugit");

    // repo passes through
    assert_eq!(vm.repo, "hugit");

    // shape must be "NO DATA" — not "NONE" or a fabricated rate
    assert_eq!(vm.kpis.shape, "NO DATA");
    assert_eq!(vm.kpis.hits, 0);
    assert_eq!(vm.kpis.executed, 0);
    assert_eq!(vm.kpis.hit_rate_pct, 0.0);
    assert_eq!(vm.kpis.saved_ms, 0);

    // no rows, no pills
    assert!(vm.checks.is_empty());
    assert!(vm.cpills.is_empty());

    // P2 stubs — honest None / 0 / ""
    assert!(vm.culprit.is_none(), "culprit must be None (P2 STUB)");
    assert!(vm.hero_red.is_none(), "hero_red must be None (P2 STUB)");
    assert!(vm.bisect.is_none(), "bisect must be None (P2 STUB)");

    // Fleet KPI stubs (P2) — honest zero / empty string
    assert_eq!(
        vm.cache_hit_rate_pct, 0,
        "fleet cache_hit_rate_pct must be 0 (P2 STUB)"
    );
    assert_eq!(
        vm.cache_saved_usd, "",
        "fleet cache_saved_usd must be empty (P2 STUB)"
    );
    assert_eq!(
        vm.cache_saved_runner_h, "",
        "fleet cache_saved_runner_h must be empty (P2 STUB)"
    );

    // Serialize + deserialize → identical (lossless round-trip)
    let json = serde_json::to_string(&vm).expect("ChecksVm serializes");
    let reparsed: ChecksVm = serde_json::from_str(&json).expect("ChecksVm deserializes");
    assert_eq!(vm, reparsed, "ChecksVm round-trip must be lossless");
}

/// Populated log (3 HIT rows + 1 EXECUTED row) → real KPI aggregation, PARTIAL
/// shape, correct hit_rate_pct, saved_ms, cpills, and hero.green == true.
///
/// This tests the REAL path that was entirely untested before (the handler read
/// `check.recorded` events from a live log and aggregated KPIs — never exercised
/// by the empty-log test).
#[test]
fn populated_log_aggregates_kpis_correctly() {
    let mut log = EventLog::new();

    // 3 HIT rows: cache_hit:true, exit:0, real duration_ms, memo_key.
    let hit_rows = [
        (
            "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
            1_200_u64,
            "fmt",
        ),
        (
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            850_u64,
            "clippy",
        ),
        (
            "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
            300_u64,
            "test",
        ),
    ];
    for (memo_key, duration_ms, name) in &hit_rows {
        let payload = serde_json::json!({
            "name": name,
            "memo_key": memo_key,
            "cache_hit": true,
            "exit": 0,
            "duration_ms": duration_ms,
        })
        .to_string();
        log.append_for_test(
            "check.recorded",
            vec!["agent".to_string()],
            payload,
            1_000_000,
        );
    }

    // 1 EXECUTED row: cache_hit:false, exit:0 (no duration saved).
    let executed_payload = serde_json::json!({
        "name": "audit",
        "memo_key": "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
        "cache_hit": false,
        "exit": 0,
        "duration_ms": 5_000_u64,
    })
    .to_string();
    log.append_for_test(
        "check.recorded",
        vec!["agent".to_string()],
        executed_payload,
        1_000_001,
    );

    let vm = build_checks(&log, "hugit");

    // KPI aggregation
    assert_eq!(vm.kpis.hits, 3, "hits");
    assert_eq!(vm.kpis.executed, 1, "executed");
    assert_eq!(vm.kpis.shape, "PARTIAL", "shape");

    // hit_rate_pct: 3*10000/4/100.0 = 30000/4/100.0 = 7500/100.0 = 75.0
    assert_eq!(vm.kpis.hit_rate_pct, 75.0, "hit_rate_pct");

    // saved_ms = sum of the 3 HIT durations
    let expected_saved_ms: u64 = hit_rows.iter().map(|(_, ms, _)| ms).sum();
    assert_eq!(vm.kpis.saved_ms, expected_saved_ms, "saved_ms");

    // cpills: one per HIT row, hash == first 8 chars of memo_key
    assert_eq!(vm.cpills.len(), 3, "cpills.len");
    for (i, (memo_key, _, name)) in hit_rows.iter().enumerate() {
        assert_eq!(vm.cpills[i].hash, &memo_key[..8], "cpills[{i}].hash");
        assert_eq!(vm.cpills[i].key, *name, "cpills[{i}].key");
        assert!(vm.cpills[i].cached, "cpills[{i}].cached");
    }

    // hero.green: all rows have exit:0 → true
    assert!(vm.hero.green, "hero.green");

    // rows projected
    assert_eq!(vm.checks.len(), 4, "checks.len");

    // round-trip
    let json = serde_json::to_string(&vm).expect("ChecksVm serializes");
    let reparsed: ChecksVm = serde_json::from_str(&json).expect("ChecksVm deserializes");
    assert_eq!(vm, reparsed, "ChecksVm round-trip must be lossless");
}

/// RED variant: a log with one row with exit != 0 → hero.green == false.
///
/// This tests the P1 honesty fix: before the fix, `unwrap_or(true)` caused a
/// failed check (exit=1) to count as green. Now `unwrap_or(false)` is conservative.
#[test]
fn failed_exit_row_makes_hero_not_green() {
    let mut log = EventLog::new();

    let payload = serde_json::json!({
        "name": "clippy",
        "memo_key": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "cache_hit": false,
        "exit": 1,
        "duration_ms": 3_000_u64,
    })
    .to_string();
    log.append_for_test(
        "check.recorded",
        vec!["agent".to_string()],
        payload,
        1_000_000,
    );

    let vm = build_checks(&log, "hugit");

    assert!(!vm.hero.green, "hero.green must be false when exit != 0");
    assert_eq!(vm.kpis.hits, 0);
    assert_eq!(vm.kpis.executed, 1);
    assert_eq!(vm.kpis.shape, "NONE");
}

/// SECRET-MATRIX (read-boundary): a `check.recorded` whose `name` is secret-shaped,
/// pushed RAW via `append_for_test` (bypassing the write-path scrub), is scrubbed at
/// the READ boundary into the check row + cpill — defence-in-depth per the doctrine.
#[test]
fn check_name_is_scrubbed_at_read_boundary() {
    let mut log = EventLog::new();
    let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
    log.append_for_test(
        "check.recorded",
        vec!["test".to_string()],
        serde_json::json!({
            "name": secret,
            "exit": 0,
            "duration_ms": 10_u64,
            "cache_hit": true,
            "memo_key": "abc123def4567890",
        })
        .to_string(),
        1_000,
    );

    let vm = build_checks(&log, "hugit");
    assert!(
        vm.checks[0].name.contains("[REDACTED]"),
        "check row name scrubbed at read boundary, got: {}",
        vm.checks[0].name
    );
    assert!(
        !vm.checks[0].name.contains("ghp_"),
        "the raw PAT never reaches the check row name"
    );
    assert!(
        vm.cpills[0].key.contains("[REDACTED]"),
        "cpill key scrubbed, got: {}",
        vm.cpills[0].key
    );
    // memo_key is a content-address — NOT scrubbed.
    assert!(!vm.cpills[0].hash.contains("[REDACTED]"));
}
