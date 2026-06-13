//! Parity tests for `build_checks`: empty log → honest defaults; serialization
//! round-trip through `ChecksVm` is lossless.

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
