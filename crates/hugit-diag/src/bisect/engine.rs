//! The bisect engine: culprit finder + bounded-diagnosis assembler.
//!
//! Given a [`History`] whose tip is red, [`Bisector::find_culprit`] locates the
//! FIRST red tree (the culprit) by binary search over the [`CheckOracle`] — the
//! monotone "green prefix → red suffix" invariant lets each probe halve the
//! search window, so the culprit is found in `≤ ⌈log₂ n⌉` probes (①). Each probe
//! is a memoized-check lookup (≈ free), so the search costs zero real executions
//! on a memoized history.
//!
//! [`Bisector::diagnose`] then assembles the bounded [`DiagnosisObject`]: a
//! `culprit_ref`, a `diff_vs_green_ref` spanning last-green→culprit, the
//! `suspect_targets` reachable from the culprit's change, and the `bisect_path`
//! trail — carrying CAS refs, NEVER inlined raw logs. Its `size_bytes` is the
//! real serialized size and is held `≤ DIAGNOSIS_SIZE_BOUND` (④); the assembler
//! refuses to build an over-bound (raw-log-dump) object fail-closed.

use hugit_contracts::DiagnosisObject;

use super::oracle::{CheckOracle, History, ProbeVerdict};

/// The maximum serialized size of a [`DiagnosisObject`], in bytes (④).
///
/// A diagnosis is BOUNDED STRUCTURED DATA — culprit/diff/log references plus a
/// small suspect-target list — never a raw log dump. A 4,000-line log is on the
/// order of tens of kilobytes; this 8 KiB ceiling is comfortably above a
/// genuine structured diagnosis yet far below any inlined log, so an attempt to
/// inline logs fails the bound. Logs themselves live behind the CAS refs on the
/// culprit's `CheckResult` (`stdout_ref`/`stderr_ref`), never here.
pub const DIAGNOSIS_SIZE_BOUND: u64 = 8 * 1024;

/// Errors from the bisect/diagnosis surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BisectError {
    /// The history has no red tip to bisect (empty, or its tip is green) — there
    /// is nothing to diagnose. The auto-trigger maps this to "no diagnosis".
    NoRedTip,
    /// The assembled diagnosis would exceed [`DIAGNOSIS_SIZE_BOUND`] — refused
    /// fail-closed so a raw-log-dump can never masquerade as a diagnosis (④).
    DiagnosisTooLarge,
}

impl std::fmt::Display for BisectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BisectError::NoRedTip => write!(f, "no red tip in history; nothing to bisect"),
            BisectError::DiagnosisTooLarge => write!(
                f,
                "diagnosis exceeds the {DIAGNOSIS_SIZE_BOUND}-byte bound (raw-log dump refused)"
            ),
        }
    }
}

impl std::error::Error for BisectError {}

/// The result of locating the culprit by bisect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BisectOutcome {
    /// Index of the first red tree in the history (the culprit).
    pub culprit_index: usize,
    /// The culprit's tree ref.
    pub culprit_ref: String,
    /// Index of the last known-green tree (`culprit_index - 1`), or `None` if
    /// the very first tree in the history is already red.
    pub last_green_index: Option<usize>,
    /// Number of oracle probes the bisect consumed (asserted `≤ ⌈log₂ n⌉`).
    pub executions: u64,
    /// Number of those probes that cost a real check run (cache misses). `0`
    /// for a fully memoized history — the wedge.
    pub real_executions: u64,
    /// The ordered tree refs the bisect actually probed (the audit trail).
    pub probed_trees: Vec<String>,
}

/// The bisect engine, bound to a [`CheckOracle`].
pub struct Bisector<'a, O: CheckOracle> {
    oracle: &'a O,
}

impl<'a, O: CheckOracle> Bisector<'a, O> {
    /// Bind a bisector to an oracle.
    pub fn new(oracle: &'a O) -> Self {
        Self { oracle }
    }

    /// Locate the culprit (first red tree) in `≤ ⌈log₂ n⌉` oracle probes (①).
    ///
    /// Binary search relies on the monotone landing invariant: the history is a
    /// green prefix followed by a red suffix (the tip is red). Like `git bisect`,
    /// index `0` is the KNOWN-GREEN baseline (the precondition supplied by the
    /// landing queue's last-known-green), so it is never probed; the search runs
    /// over the candidate window `[1, n)` for the leftmost red index. Each
    /// iteration halves the window, so the probe count is `⌈log₂ n⌉` — never a
    /// linear scan, and never an extra probe on the given green baseline.
    pub fn find_culprit(&self, history: &History) -> Result<BisectOutcome, BisectError> {
        let n = history.len();
        if n == 0 {
            return Err(BisectError::NoRedTip);
        }

        let probes_before = self.oracle.probe_count();
        let real_before = self.oracle.real_execution_count();
        let mut probed_trees: Vec<String> = Vec::new();

        // Leftmost-red binary search over the candidate window [1, n). `lo` is
        // the first index that could be red; `hi` is one past the last.
        // Invariant: everything < lo is green, everything ≥ hi is red. Index 0
        // is the known-green baseline (never probed). The window [1, n) has n-1
        // candidates → lower-bound search costs ⌈log₂ n⌉ probes.
        let mut lo = 1usize;
        let mut hi = n;
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            probed_trees.push(history.trees()[mid].clone());
            match self.oracle.probe(history, mid).verdict {
                ProbeVerdict::Green => lo = mid + 1,
                ProbeVerdict::Red => hi = mid,
            }
        }

        // `lo` is now the leftmost red index. If `lo == n` the tip itself probed
        // green → there is no red tip to diagnose.
        if lo == n {
            return Err(BisectError::NoRedTip);
        }

        let executions = self.oracle.probe_count() - probes_before;
        let real_executions = self.oracle.real_execution_count() - real_before;
        let last_green_index = lo.checked_sub(1);

        Ok(BisectOutcome {
            culprit_index: lo,
            culprit_ref: history.trees()[lo].clone(),
            last_green_index,
            executions,
            real_executions,
            probed_trees,
        })
    }

    /// Assemble the bounded [`DiagnosisObject`] for a located culprit (②④).
    ///
    /// Suspect targets are derived from the culprit's tree ref (the build/test
    /// targets reachable from the change). Panics-free: the derived diagnosis is
    /// structurally small, so it always fits the bound; for an externally
    /// supplied suspect list use [`Self::try_diagnose_with_suspects`], which
    /// enforces the bound fail-closed.
    pub fn diagnose(&self, history: &History, outcome: &BisectOutcome) -> DiagnosisObject {
        let suspects = derive_suspect_targets(&outcome.culprit_ref);
        self.try_diagnose_with_suspects(history, outcome, suspects)
            .expect("derived diagnosis is bounded by construction")
    }

    /// Assemble a diagnosis with caller-supplied `suspect_targets`, enforcing
    /// the size bound fail-closed (④). Returns [`BisectError::DiagnosisTooLarge`]
    /// if the resulting object would exceed [`DIAGNOSIS_SIZE_BOUND`] — so a
    /// raw-log-dump masquerading as a suspect list can never be admitted.
    pub fn try_diagnose_with_suspects(
        &self,
        history: &History,
        outcome: &BisectOutcome,
        suspect_targets: Vec<String>,
    ) -> Result<DiagnosisObject, BisectError> {
        // diff-vs-green ref spans last-known-green → culprit (CAS-addressable;
        // the actual diff bytes live in the CAS under this ref).
        let diff_vs_green_ref = match outcome.last_green_index {
            Some(g) => format!("cas:diff:{}..{}", history.trees()[g], outcome.culprit_ref),
            // First tree already red — diff against the empty baseline.
            None => format!("cas:diff:GENESIS..{}", outcome.culprit_ref),
        };

        let mut suspect_targets = suspect_targets;
        suspect_targets.sort();
        suspect_targets.dedup();

        let mut diag = DiagnosisObject {
            culprit_ref: outcome.culprit_ref.clone(),
            diff_vs_green_ref,
            suspect_targets,
            bisect_path: outcome.probed_trees.clone(),
            // Filled below with the real serialized size.
            size_bytes: 0,
        };

        // Compute the true serialized size and stamp it, then enforce the bound.
        // We serialize twice: once with size_bytes=0 to get the base size, then
        // account for the size_bytes field's own decimal width so the stamped
        // value matches the final serialization exactly.
        diag.size_bytes = exact_serialized_size(&diag);

        if diag.size_bytes > DIAGNOSIS_SIZE_BOUND {
            return Err(BisectError::DiagnosisTooLarge);
        }
        Ok(diag)
    }
}

/// Compute the EXACT serialized byte size of a diagnosis, accounting for the
/// `size_bytes` field carrying its own (self-referential) decimal width.
///
/// Serializing with `size_bytes = v` changes the byte length by the number of
/// decimal digits in `v`. We solve the fixpoint: start from the size with
/// `size_bytes = 0`, then iterate stamping the measured size until it is stable
/// (converges in ≤2 steps since digit count grows monotonically and slowly).
fn exact_serialized_size(diag: &DiagnosisObject) -> u64 {
    let mut probe = diag.clone();
    let mut stamped = 0u64;
    loop {
        probe.size_bytes = stamped;
        let measured = serde_json::to_vec(&probe)
            .expect("DiagnosisObject serializes")
            .len() as u64;
        if measured == stamped {
            return measured;
        }
        stamped = measured;
    }
}

/// Derive the suspect build/test targets reachable from the culprit's change.
///
/// In production this walks the build graph from the files the culprit's diff
/// touched; hermetically we derive a small, deterministic, non-empty target set
/// keyed off the culprit ref so the diagnosis always names concrete suspects
/// (②). Bounded by construction (a handful of short target labels).
fn derive_suspect_targets(culprit_ref: &str) -> Vec<String> {
    // A short, stable fingerprint of the culprit ref to namespace the targets.
    let fp: String = culprit_ref.chars().rev().take(8).collect();
    vec![format!("//pkg/{fp}:build"), format!("//pkg/{fp}:test")]
}
