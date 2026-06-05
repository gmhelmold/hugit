//! WP-D1c acceptance oracle — hugit-refstore concurrency/perf.
//!
//! Owned item (VERBATIM from decomposition v2.0 — D1):
//!   ⑤ 100 concurrent ops: serialized, 0 loss, p99<500ms
//!
//! Driven by `tests/acceptance/wp-d1c/run.sh`. The Rust test itself drives 100
//! concurrent operations through the single-writer serialization point, measures
//! per-op latency, and asserts the p99 < 500ms bound plus the no-loss invariant.
//!
//! The load/perf mechanics live in the contract-owned `tests/concurrency_perf/`
//! directory; this file is the assertion surface for item ⑤.

#[path = "concurrency_perf/harness.rs"]
mod harness;

use harness::run_concurrent;
use hugit_refstore::concurrency::Serializer;
use hugit_refstore::tamper::verify_chain;
use std::collections::BTreeSet;
use std::time::Duration;

/// The contract budget: p99 latency at 100 concurrent ops must stay under 500ms.
const P99_BUDGET: Duration = Duration::from_millis(500);
/// The contract load: 100 concurrent operations.
const CONCURRENCY: usize = 100;

/// ⑤ 100 concurrent ops: serialized, 0 loss, p99 < 500ms.
///
/// Drives 100 submitters at one shared single-writer [`Serializer`]
/// simultaneously and proves the three contract invariants together:
///
/// - **Serialized:** the 100 ops land as one gap-free, monotonically-sequenced,
///   hash-linked chain — `verify_chain` passes and seqs are exactly `0..100`.
/// - **0 loss:** every accepted op produces exactly one record
///   (`records == accepted`), no op vanishes, and (with unbounded admission)
///   every launched op is accepted (`accepted == 100`, `rejected == 0`). Each
///   op is a distinct ref mutation, so the 100 distinct ref names all survive.
/// - **p99 < 500ms:** the measured 99th-percentile submit latency is under the
///   500ms budget.
#[test]
fn item_5_concurrent_serialized_zero_loss_p99() {
    // Unbounded admission: every concurrent op is accepted (no back-pressure),
    // which is the strict zero-loss case the contract measures.
    let serializer = Serializer::new();
    let run = run_concurrent(CONCURRENCY, serializer);

    // ── Launched accounting: 100 ops issued, all accounted for. ──────────────
    assert_eq!(run.launched, CONCURRENCY, "must launch 100 concurrent ops");
    assert_eq!(
        run.accepted() + run.rejected,
        CONCURRENCY,
        "every launched op must be accounted for (accepted xor rejected) — none lost"
    );

    // ── 0 loss: unbounded admission accepts all 100, none rejected. ──────────
    assert_eq!(
        run.rejected, 0,
        "unbounded admission must reject nothing (back-pressure off)"
    );
    assert_eq!(
        run.accepted(),
        CONCURRENCY,
        "all 100 concurrent ops must be accepted"
    );

    // ── 0 loss: records on the chain == accepted ops (exactly one each). ─────
    let chain_len = run
        .serializer
        .len()
        .expect("writer must not be poisoned after a clean run");
    assert_eq!(
        chain_len,
        run.accepted(),
        "records appended must equal accepted ops — exactly one record per op, zero loss"
    );
    assert_eq!(
        chain_len, CONCURRENCY,
        "the chain must hold all 100 records"
    );

    // ── Serialized: the chain is gap-free, monotonic, and hash-intact. ───────
    let log = run.serializer.snapshot().expect("snapshot under lock");
    verify_chain(log.records())
        .expect("the 100 serialized records must form one intact hash chain");
    for (i, rec) in log.records().iter().enumerate() {
        assert_eq!(
            rec.seq, i as u64,
            "seqs must be exactly 0..100 with no gaps or duplicates (single total order)"
        );
    }

    // ── 0 loss, distinct-payload check: every op's unique ref survives. ──────
    // Each submitter wrote a distinct ref name; all 100 must be present, proving
    // no accepted op was overwritten/coalesced away under contention.
    let names: BTreeSet<String> = log
        .records()
        .iter()
        .map(|r| {
            // payload is `{"ref":"refs/heads/op-NNN","target":...}`
            let needle = "\"ref\":\"";
            let start = r.payload.find(needle).expect("payload has a ref") + needle.len();
            let rest = &r.payload[start..];
            let end = rest.find('"').expect("ref name is quoted");
            rest[..end].to_string()
        })
        .collect();
    assert_eq!(
        names.len(),
        CONCURRENCY,
        "all 100 distinct ref mutations must survive — none coalesced or lost"
    );

    // ── p99 < 500ms: the measured latency budget. ────────────────────────────
    let p99 = run.p99();
    assert!(
        p99 < P99_BUDGET,
        "p99 latency {p99:?} must be under the {P99_BUDGET:?} budget at {CONCURRENCY} concurrent ops \
         (max observed {:?})",
        run.max()
    );

    // Evidence line (visible with `cargo test -- --nocapture`).
    println!(
        "WP-D1c ⑤: {} concurrent ops · accepted {} · rejected {} · chain_len {} · p99 {:?} (budget {:?}) · max {:?}",
        CONCURRENCY,
        run.accepted(),
        run.rejected,
        chain_len,
        p99,
        P99_BUDGET,
        run.max()
    );
}

/// Back-pressure proof (supports item ⑤'s "0 loss" clause): under a bounded
/// admission gate, the overflow is **explicitly rejected**, never silently
/// dropped — `accepted + rejected == launched`, and every accepted op is on the
/// chain (`records == accepted`). This is the not-silently-lost half of the
/// zero-loss invariant when the writer is saturated.
#[test]
fn backpressure_rejects_explicitly_never_drops() {
    let capacity = 8;
    let serializer = Serializer::with_capacity(capacity);
    let run = run_concurrent(CONCURRENCY, serializer);

    assert_eq!(
        run.accepted() + run.rejected,
        CONCURRENCY,
        "every op is either accepted or explicitly rejected — none silently dropped"
    );
    assert!(
        run.accepted() >= 1,
        "at least one op must make progress under back-pressure"
    );

    let chain_len = run.serializer.len().expect("writer not poisoned");
    assert_eq!(
        chain_len,
        run.accepted(),
        "records appended must equal accepted ops even under back-pressure (zero loss)"
    );

    let log = run.serializer.snapshot().expect("snapshot under lock");
    verify_chain(log.records()).expect("accepted records form an intact chain under back-pressure");
}
