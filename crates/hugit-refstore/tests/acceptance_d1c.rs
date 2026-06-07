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
use hugit_refstore::concurrency::{Op, Serializer};
use hugit_refstore::tamper::verify_chain;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::thread;
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

    // ── Back-pressure bound: in-flight stays within capacity, then drains. ────
    // Unbounded admission ⇒ peak can reach 100, but must never exceed capacity,
    // and every permit must be returned once the run completes (no leak).
    assert!(
        run.peak_in_flight <= run.serializer.capacity(),
        "in-flight peak {} must never exceed capacity {} (admission bound)",
        run.peak_in_flight,
        run.serializer.capacity()
    );
    assert_eq!(
        run.serializer.in_flight(),
        0,
        "in-flight must return to 0 after the run — no admission permit leaked"
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

    // ── The bound held under contention, then drained. ───────────────────────
    // The high-contention peak of simultaneously-admitted ops must NEVER exceed
    // the capacity: this is the assertion that catches the TOCTOU where in-flight
    // could transiently exceed the bound under interleaved admission.
    assert!(
        run.peak_in_flight <= capacity,
        "in-flight peak {} exceeded capacity {} under contention — back-pressure bound breached",
        run.peak_in_flight,
        capacity
    );
    assert_eq!(
        run.serializer.in_flight(),
        0,
        "in-flight must drain to 0 after a bounded run — no permit leaked or double-returned"
    );
}

/// Deterministic TOCTOU probe of the admit/lock split (item ⑤ back-pressure).
///
/// The high-contention test above samples a peak and so is probabilistic about
/// hitting the exact interleave; this test makes the bound failure deterministic
/// by *pinning the writer lock open* while more than `capacity` submitters pile
/// onto admission simultaneously. With a correct gate, at most `capacity` of
/// them can be admitted (hold a permit) at once, so the observed peak is exactly
/// `capacity` and the surplus is rejected with explicit back-pressure — never a
/// transient breach of the bound.
#[test]
fn admit_lock_split_never_exceeds_capacity_deterministic() {
    const CAPACITY: usize = 4;
    const SURPLUS: usize = 6; // CAPACITY + SURPLUS submitters contend at once.
    let total = CAPACITY + SURPLUS;

    let serializer = Serializer::with_capacity(CAPACITY);

    // Pin the single writer lock open on a helper thread: it enters the critical
    // section (via `with_records`) and blocks there until released. While it
    // holds the lock, every admitted submitter is forced to wait *after* taking
    // its admission permit but *before* the append — exactly the admit/lock split
    // where a TOCTOU bound breach would show up as peak > capacity.
    let (lock_held_tx, lock_held_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder = {
        let s = serializer.clone();
        thread::spawn(move || {
            s.with_records(|_records| {
                // Announce the lock is held, then block until told to release.
                lock_held_tx.send(()).unwrap();
                release_rx.recv().unwrap();
            })
            .expect("holder takes the writer lock");
        })
    };
    // Wait until the writer lock is provably held before launching submitters.
    lock_held_rx.recv().unwrap();

    // Launch `total` submitters at once. CAPACITY will be admitted and block on
    // the (held) writer lock; the SURPLUS will be refused at the gate.
    let ready = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::with_capacity(total);
    for i in 0..total {
        let s = serializer.clone();
        let ready = Arc::clone(&ready);
        handles.push(thread::spawn(move || {
            ready.fetch_add(1, Ordering::AcqRel);
            while ready.load(Ordering::Acquire) < total {
                std::hint::spin_loop();
            }
            let op = Op::new(
                "ref.update",
                vec![format!("agent:probe-{i:03}")],
                format!(r#"{{"ref":"refs/heads/probe-{i:03}","target":"oid-{i:08x}"}}"#),
                1_717_000_000_000 + i as u64,
            );
            s.submit(op).is_ok()
        }));
    }

    // Spin until the admission gate is saturated: with the writer lock pinned,
    // exactly CAPACITY submitters can hold a permit at once. The bound says this
    // value must NEVER exceed CAPACITY at any instant.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while serializer.in_flight() < CAPACITY {
        assert!(
            serializer.in_flight() <= CAPACITY,
            "in-flight {} exceeded capacity {} while admission was pinned — TOCTOU bound breach",
            serializer.in_flight(),
            CAPACITY
        );
        assert!(
            std::time::Instant::now() < deadline,
            "admission never saturated to capacity within the deadline"
        );
        std::hint::spin_loop();
    }
    // Now saturated. The bound must hold and the peak must be exactly CAPACITY.
    assert!(
        serializer.in_flight() <= CAPACITY,
        "in-flight {} exceeded capacity {} at saturation",
        serializer.in_flight(),
        CAPACITY
    );
    assert_eq!(
        serializer.peak_in_flight(),
        CAPACITY,
        "peak in-flight must reach exactly capacity {CAPACITY} with the lock pinned, never more"
    );

    // Release the writer; the CAPACITY admitted ops drain and complete.
    release_tx.send(()).unwrap();
    holder.join().expect("holder thread completes");

    let accepted = handles
        .into_iter()
        .map(|h| h.join().expect("submitter completes"))
        .filter(|&ok| ok)
        .count();

    // The surplus was rejected by back-pressure, never silently dropped.
    assert_eq!(
        accepted, CAPACITY,
        "exactly capacity ops admitted; the {SURPLUS} surplus rejected by back-pressure"
    );
    assert!(
        serializer.peak_in_flight() <= CAPACITY,
        "peak in-flight {} must never exceed capacity {CAPACITY}",
        serializer.peak_in_flight()
    );
    assert_eq!(
        serializer.in_flight(),
        0,
        "all permits returned after drain — in-flight back to 0"
    );
    let chain_len = serializer.len().expect("writer not poisoned");
    assert_eq!(
        chain_len, CAPACITY,
        "exactly the admitted ops landed on the chain (zero loss under back-pressure)"
    );
}
