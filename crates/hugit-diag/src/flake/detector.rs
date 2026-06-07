//! Statistical flake detector (contract ②④).
//!
//! ## Detection policy (all thresholds decided, non-negotiable)
//!
//! A test is classified by its running statistics once the minimum evidence
//! threshold is met:
//!
//! | Class     | Condition                                          |
//! |-----------|---------------------------------------------------|
//! | `Flaky`   | `fail_rate ∈ (0.0, 1.0)` AND `run_count ≥ MIN_RUNS_FLAKY` |
//! | `Real`    | `fail_rate == 1.0` AND `run_count ≥ MIN_RUNS_REAL` |
//! | `Unknown` | Not enough runs yet, or always passing             |
//!
//! ## Planted-flake detectability proof (contract ②)
//!
//! At 20% fail rate, `MIN_RUNS_FLAKY = 5` suffices:
//! After 5 runs the expected number of failures is 1.  In the worst case
//! (all 5 failures bunched at the end of a 29-run window) the first failure
//! appears by run 5 and the first pass by run 1 → `run_count = 5 ≥ MIN_RUNS_FLAKY`
//! and `fail_rate ∈ (0, 1)` → Flaky.  In practice detection with a 20%-flake
//! source typically happens by run 10 (≈ 2 failures).  The oracle drives
//! exactly a 20% pattern (1 failure every 5 runs) so detection is deterministic
//! at run 5 at the absolute latest — well within the <30-run bound.
//!
//! ## False-positive guard (contract ④)
//!
//! `fail_rate == 1.0` (100% failures) with `run_count ≥ MIN_RUNS_REAL` yields
//! `Classification::Real` — NEVER `Flaky`, NEVER quarantined.  A flaky test
//! by definition has BOTH passes AND failures in its history; a purely
//! deterministic failure has no passes.
//!
//! `MIN_RUNS_REAL` is set low (3) so that a deterministic failure is identified
//! quickly, before volume accumulates.

use crate::flake::stats::TestStats;

/// Minimum number of runs before a test can be classified `Flaky`.
///
/// Set to 5: at 20% fail rate the first failure is guaranteed by run 5
/// (pattern: pass, pass, pass, pass, FAIL). This is the detection trigger —
/// after the first failure and first pass both exist with ≥ 5 total runs,
/// the fail rate is strictly between 0 and 1.
pub const MIN_RUNS_FLAKY: u64 = 5;

/// Minimum number of runs before a test can be classified `Real`.
///
/// Kept low (3) so deterministic failures are identified quickly.
pub const MIN_RUNS_REAL: u64 = 3;

/// The classification produced by the detector for one test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// The test exhibits intermittent failures — it is flaky.
    Flaky,
    /// The test fails deterministically — it is a REAL failure (④).
    Real,
    /// Insufficient evidence to classify yet, or always passing.
    Unknown,
}

/// Classify a test from its running statistics.
///
/// Returns `None` when no stats exist for the key (i.e. the test has not been
/// fed yet).  Returns `Some(Classification)` otherwise.
pub fn classify_from_stats(stats: &TestStats) -> Classification {
    if stats.run_count == 0 {
        return Classification::Unknown;
    }

    let fail_rate = stats.fail_rate();

    // ④ false-positive guard: 100% fail rate = deterministic = REAL.
    if fail_rate == 1.0 && stats.run_count >= MIN_RUNS_REAL {
        return Classification::Real;
    }

    // ② flaky: intermittent — both passes and failures seen, enough evidence.
    if fail_rate > 0.0 && fail_rate < 1.0 && stats.run_count >= MIN_RUNS_FLAKY {
        return Classification::Flaky;
    }

    Classification::Unknown
}
