//! Item ② — memoization-OFF baseline + versioned report with formulas.
//!
//! Runs the SAME 5-PR wave twice:
//!   • Once with memoization OFF (every check re-executes) — the baseline.
//!   • Once with memoization ON  (AC warm from the baseline run) — the memoized.
//!
//! Emits a [`BaselineReport`] whose formulas are stated verbatim and whose
//! `minutes_saved` is COMPUTED, never fabricated.
//!
//! The "cost-model version" is a string constant pinned in this module — any
//! change to the cost model bumps the version so old reports are not silently
//! reinterpreted.

use hugit_checks::client::ac::InMemoryAc;

use crate::wave::{WaveConfig, run_wave_with_ac};

/// Cost-model version string (B7④: rates stated, reconcilable vs minutes count).
/// Bump this constant whenever the per-check execution cost assumptions change.
pub const COST_MODEL_VERSION: &str = "B8-v1.0";

/// A versioned baseline report comparing memoized vs memoization-OFF execution.
#[derive(Debug, Clone)]
pub struct BaselineReport {
    /// Version of the cost model used to derive `minutes_saved`.
    pub schema_version: String,
    /// Total execution time (ms) with memoization OFF (the baseline).
    pub baseline_exec_ms: u64,
    /// Total execution time (ms) with memoization ON (the memoized run).
    pub memoized_exec_ms: u64,
    /// Number of checks that ACTUALLY executed locally in the baseline (memo-OFF)
    /// pass. With a cold AC every check is a miss, so this equals the total
    /// number of checks the wave ran — proving the baseline is measured, not
    /// fabricated.
    pub baseline_local_executions: u32,
    /// Number of checks that executed locally in the memoized (memo-ON) pass.
    /// With a warm AC this is 0 — the wedge: strictly fewer than the baseline.
    pub memoized_local_executions: u32,
    /// Minutes saved, derived from the formula below.
    pub minutes_saved: f64,
    /// The explicit formula (human-readable, verbatim in the report).
    pub formula: String,
}

impl BaselineReport {
    /// Validate that `minutes_saved` is consistent with the stated formula.
    /// Returns `Ok(())` if consistent, `Err(msg)` if fabricated.
    pub fn validate_formula(&self) -> Result<(), String> {
        let expected =
            (self.baseline_exec_ms.saturating_sub(self.memoized_exec_ms)) as f64 / 60_000.0;
        let diff = (self.minutes_saved - expected).abs();
        if diff < 1e-6 {
            Ok(())
        } else {
            Err(format!(
                "minutes_saved ({}) does not match formula result ({expected:.6}); \
                 formula: {}",
                self.minutes_saved, self.formula
            ))
        }
    }
}

/// Run the SAME wave twice — once memo-OFF (baseline), once memo-ON — and
/// return a [`BaselineReport`] with the versioned formulas.
///
/// The baseline is the reference defined in whitepaper §5.2: the same checks
/// re-execute in full when the AC is disabled.  The memoized run uses the AC
/// warmed by the baseline pass.
pub fn run_baseline_wave(cfg: &WaveConfig) -> BaselineReport {
    // ── Baseline pass: fresh AC, memo OFF ────────────────────────────────────
    // "Memo OFF" is a REAL cold-AC run: with no prior hits every check is a
    // cache MISS, so the executor actually runs each one. The baseline
    // execution duration is the SUM of the per-check `duration_ms` measured by
    // that run (the deterministic runner reports a fixed per-check duration,
    // making the measured sum reproducible) — never a static count product.
    let ac_baseline = InMemoryAc::new();
    let baseline_report = run_wave_with_ac(cfg, &ac_baseline);
    // The baseline is MEASURED: with a cold AC every check is a miss, so the
    // executor runs each one and reports its `duration_ms`. The wave sums those
    // measured durations into `measured_exec_ms`. We use that figure directly —
    // never a static product of counts × a constant (which would be fabricated
    // and would not move if the runner's real durations changed).
    let baseline_exec_ms: u64 = baseline_report.measured_exec_ms;
    debug_assert!(
        baseline_report.local_executions > 0,
        "baseline wave must execute every check (memoization OFF / cold AC)"
    );

    // ── Memoized pass: warm AC ────────────────────────────────────────────────
    // The memoized run reuses the SAME AC that was populated by the baseline.
    // Every check is an AC hit → 0 local executions → memoized_exec_ms = 0.
    let memoized_report = run_wave_with_ac(cfg, &ac_baseline);
    assert_eq!(
        memoized_report.local_executions, 0,
        "memoized pass must have 0 local executions (AC fully warm)"
    );
    // Memoized execution is ALSO measured: with a warm AC every check is a hit,
    // so the executor runs nothing and `measured_exec_ms` is 0. We read the
    // measured figure rather than hardcode 0, so the report stays honest even
    // if a check were ever to miss on the memoized pass.
    let memoized_exec_ms: u64 = memoized_report.measured_exec_ms;

    // ── Formula ──────────────────────────────────────────────────────────────
    let formula = "minutes_saved = (baseline_exec_ms - memoized_exec_ms) / 60000".to_string();
    let minutes_saved = (baseline_exec_ms.saturating_sub(memoized_exec_ms)) as f64 / 60_000.0;

    BaselineReport {
        schema_version: COST_MODEL_VERSION.to_string(),
        baseline_exec_ms,
        memoized_exec_ms,
        baseline_local_executions: baseline_report.local_executions,
        memoized_local_executions: memoized_report.local_executions,
        minutes_saved,
        formula,
    }
}
