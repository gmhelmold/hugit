//! The **bounded one-way mirror** — the safe rung of the compat ladder.
//!
//! This is the *first bounded leg* of the "bounded bidirectional mirror" rung:
//! a one-directional reflect of forge branch tips OUT to GitHub that is
//!
//! - **idempotent** — a ref already reflected at its current tip re-emits
//!   nothing (keys off the engine's last-emitted bookkeeping, not raw
//!   tip-inequality, so a re-run is a no-op);
//! - **bounded** — at most `bound` pushes per pass; the remainder is `Deferred`
//!   to the next pass (a reflect pass can never fan out unboundedly);
//! - **conflict-detecting + fail-closed** — if the GitHub side has moved a ref
//!   we mirror to a tip that is neither our last-emitted tip nor the new forge
//!   tip, someone wrote to GitHub **under us**: that is a divergence, and the
//!   pass **HALTS** (emits nothing further) rather than force-pushing over it.
//!   *A broken bridge kills trust; a divergence halts, never clobbers.*
//!
//! The divergence is surfaced ([`ReflectHalt`]) for the engine's
//! forge-authoritative arbitration ([`super::engine::BidirSync::arbitrate_branch_divergence`],
//! which preserves the GitHub tip as a recoverable incident) — never dropped,
//! never overwritten.
//!
//! ## What is NOT in this leg (the next WP)
//!
//! This is one-directional (forge → GitHub) only. The **inbound** leg (ingesting
//! a detected GitHub-side change back into the forge) already has its engine
//! primitives ([`super::engine::BidirSync::converge_inbound`] /
//! `ingest_github_branch`); wiring it to a live change *feed* needs the P2
//! detect seam ([`super::detect`], gated on `HUGIT_GH_TEST_REPO` + a live App).
//! Building the batched inbound reflect + the live detect transport is the next
//! WP, STOP-reported rather than half-built here.

/// One desired reflection: push `forge_tip` out to `ref_name`, given the tip
/// currently observed on the GitHub side (`None` = unknown / first sight).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectRef {
    /// The branch ref to reflect (e.g. `refs/heads/feature`).
    pub ref_name: String,
    /// The forge-side tip we want GitHub to hold.
    pub forge_tip: String,
    /// The tip currently observed on the GitHub side, if known. Used for the
    /// divergence (never-clobber) check.
    pub observed_github_tip: Option<String>,
}

impl ReflectRef {
    /// A reflection with a known GitHub-side tip.
    pub fn new(
        ref_name: impl Into<String>,
        forge_tip: impl Into<String>,
        observed_github_tip: Option<String>,
    ) -> Self {
        Self {
            ref_name: ref_name.into(),
            forge_tip: forge_tip.into(),
            observed_github_tip,
        }
    }
}

/// The per-ref result of a reflect pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefReflect {
    /// The forge tip was newly emitted outbound for this ref.
    Emitted {
        /// The ref reflected.
        ref_name: String,
        /// The tip emitted.
        tip: String,
    },
    /// Already at this tip on both sides — idempotent no-op (no push).
    Skipped {
        /// The ref left untouched.
        ref_name: String,
    },
    /// Beyond this pass's `bound` (or after a halt) — deferred to the next pass.
    Deferred {
        /// The ref not processed this pass.
        ref_name: String,
    },
}

/// A detected divergence that HALTED the reflect pass (never clobbered).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectHalt {
    /// The ref whose GitHub side diverged.
    pub ref_name: String,
    /// The forge tip we were about to reflect.
    pub forge_tip: String,
    /// The divergent tip observed on the GitHub side (preserved, not overwritten).
    pub github_tip: String,
}

/// The outcome of one bounded reflect pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReflectReport {
    /// Per-ref results, in the order they were considered.
    pub results: Vec<RefReflect>,
    /// The divergence that halted the pass, if any. When `Some`, every ref after
    /// the halted one is `Deferred` and NOTHING was clobbered.
    pub halted: Option<ReflectHalt>,
}

impl ReflectReport {
    /// The number of refs actually pushed (emitted) this pass.
    pub fn emitted_count(&self) -> usize {
        self.results
            .iter()
            .filter(|r| matches!(r, RefReflect::Emitted { .. }))
            .count()
    }

    /// Whether the pass halted on a divergence (fail-closed).
    pub fn is_halted(&self) -> bool {
        self.halted.is_some()
    }
}
