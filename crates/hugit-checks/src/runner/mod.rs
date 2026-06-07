//! The checks-as-code RUNNER path (WP-B2b).
//!
//! B2a built the CLIENT/local half of checks-as-code (parser, three-axis memo
//! key, AC, local memoized executor). B2b proves the *execution* side of the
//! SAME pure check function (whitepaper §6.2): forge/runner execution that is
//! **byte-identical** to the local executor, plus the two honesty surfaces the
//! wedge depends on.
//!
//! Three owned items, all provable hermetically NOW against the real surface:
//!
//!   - [`lease_exec`] — runner-side execution under a [`RunnerLease`]. The
//!     [`RunnerExecutor`] trait is the runner analogue of B2a's `CheckRunner`:
//!     it executes a [`CheckDef`] and produces a fully-formed [`CheckResult`]
//!     (artifacts + digests). An in-process reference executor proves the logic;
//!     the live runner box (`HUGIT_RUNNER_HOST`) is a documented P2 seam BEHIND
//!     the same trait.
//!   - [`byte_identity`] (③) — the local↔runner comparator. It compares the two
//!     `CheckResult`s by ARTIFACT CONTENT DIGEST (and the three memo axes), NOT
//!     by exit/result equality. Any artifact digest mismatch on a deterministic
//!     fixture is a divergence FAIL — "byte-identical by construction" is
//!     enforced, not assumed.
//!   - [`nondeterminism`] (④) — the per-`memo_key` divergence tracker. It
//!     records the produced-artifact fingerprint across runs and, after **3
//!     divergent runs** for the same key, flags the check `NonDeterministic` and
//!     surfaces it HONESTLY (never a silent re-memo, never a claimed clean hit).
//!   - [`hit_rate`] (⑤) — the honest hit-rate meter. It measures the ACTUAL AC
//!     hit-rate over a run sequence (e.g. an npm fixture with partially-cacheable
//!     work) and reports it AS-IS. PARTIAL-over-fake is law: a non-hermetic
//!     ecosystem (npm) yields a partial rate, and the meter NEVER claims full
//!     memoization for it.
//!
//! B2b consumes B2a's frozen `client/` surface (the `CheckDef` + the three-axis
//! `memo_key` derivation) and never modifies it.

pub mod byte_identity;
pub mod hit_rate;
pub mod lease_exec;
pub mod nondeterminism;

pub use byte_identity::{ArtifactDiff, ByteIdentityReport, compare_byte_identity};
pub use hit_rate::{HitRateMeter, HitRateReport};
pub use lease_exec::{
    InProcessRunnerExecutor, LiveBoxRunnerExecutor, RunnerExecError, RunnerExecutor,
    execute_on_lease,
};
pub use nondeterminism::{DeterminismState, NonDeterminismTracker, RunObservation};
