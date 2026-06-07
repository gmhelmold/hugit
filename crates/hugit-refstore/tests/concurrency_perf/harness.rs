//! WP-D1c load/perf harness — the 100-concurrent fixture and p99 calculator
//! that the owned acceptance item drives.
//!
//! Kept in the contract-owned `tests/concurrency_perf/` directory and included
//! by `tests/acceptance_d1c.rs` (the oracle the run.sh invokes). The harness
//! holds the mechanics — spawn N submitters, time each op, compute the p99 —
//! so the oracle item reads as the assertions of the contract, not plumbing.

use hugit_refstore::concurrency::{Op, Serializer, SubmitError};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// Outcome of a single concurrent load run.
pub struct LoadRun {
    /// The serializer after the run (its chain is the proof surface).
    pub serializer: Serializer,
    /// Per-accepted-op end-to-end latencies (submit call duration), one entry
    /// per record that landed on the chain.
    pub latencies: Vec<Duration>,
    /// Operations explicitly rejected by back-pressure (never silently dropped).
    pub rejected: usize,
    /// Operations launched (one per submitter thread).
    pub launched: usize,
    /// Peak simultaneously-admitted in-flight operations observed during the run
    /// (the serializer's high-water mark). The back-pressure bound requires this
    /// to never exceed `serializer.capacity()`.
    pub peak_in_flight: usize,
}

impl LoadRun {
    /// Number of accepted (chained) operations = number of measured latencies.
    pub fn accepted(&self) -> usize {
        self.latencies.len()
    }

    /// The p99 latency (nearest-rank, 99th percentile) over accepted ops.
    ///
    /// Nearest-rank: index `ceil(0.99 * n) - 1` of the sorted samples. With a
    /// 100-sample run this is the 99th-slowest op — the canonical p99.
    pub fn p99(&self) -> Duration {
        percentile(&self.latencies, 99)
    }

    /// The maximum observed latency (p100) — useful context in the report.
    pub fn max(&self) -> Duration {
        self.latencies.iter().copied().max().unwrap_or_default()
    }
}

/// Nearest-rank percentile over a latency sample set (`pct` in `1..=100`).
pub fn percentile(samples: &[Duration], pct: usize) -> Duration {
    if samples.is_empty() {
        return Duration::ZERO;
    }
    let mut sorted: Vec<Duration> = samples.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    // ceil(pct/100 * n), 1-based rank, clamped into [1, n].
    let rank = (pct * n).div_ceil(100);
    let idx = rank.clamp(1, n) - 1;
    sorted[idx]
}

/// Drive `concurrency` submitters at one shared [`Serializer`] simultaneously,
/// each issuing one operation, and collect per-op latencies plus loss accounting.
///
/// All threads rendezvous on a spin barrier before submitting so the load is
/// genuinely concurrent (maximal contention on the single writer), which is what
/// the p99 bound is asserted against.
pub fn run_concurrent(concurrency: usize, serializer: Serializer) -> LoadRun {
    let ready = Arc::new(AtomicUsize::new(0));
    let mut handles = Vec::with_capacity(concurrency);

    for i in 0..concurrency {
        let s = serializer.clone();
        let ready = Arc::clone(&ready);
        handles.push(thread::spawn(move || {
            // Distinct, deterministic op per thread so every accepted op is a
            // unique ref mutation — zero-loss is then countable by ref name too.
            let op = Op::new(
                "ref.update",
                vec![format!("agent:submitter-{i:03}"), "user:gustavo".into()],
                format!(r#"{{"ref":"refs/heads/op-{i:03}","target":"oid-{i:064x}"}}"#),
                1_717_000_000_000 + i as u64,
            );

            // Rendezvous: announce ready, then spin until all are ready.
            ready.fetch_add(1, Ordering::AcqRel);
            while ready.load(Ordering::Acquire) < concurrency {
                std::hint::spin_loop();
            }

            // Time the full submit (admission + serialization + append).
            let started = Instant::now();
            let result = s.submit(op);
            let elapsed = started.elapsed();
            (result, elapsed)
        }));
    }

    let mut latencies = Vec::with_capacity(concurrency);
    let mut rejected = 0usize;
    for h in handles {
        match h.join().expect("submitter thread must not panic") {
            (Ok(_record), elapsed) => latencies.push(elapsed),
            (Err(SubmitError::Backpressure { .. }), _) => rejected += 1,
            (Err(e), _) => panic!("unexpected submit error: {e}"),
        }
    }

    let peak_in_flight = serializer.peak_in_flight();
    LoadRun {
        serializer,
        latencies,
        rejected,
        launched: concurrency,
        peak_in_flight,
    }
}
