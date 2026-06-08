//! hugit-diag — diagnostics + the experiment harness.
//!
//! Module layout (body lands per owning WP):
//!   - `experiment/` — the experiment harness + experiment gate (WP-D8):
//!     auto-collects claim-disjointness + regen-honesty datapoints from the
//!     fleet's real waves and BINDS the experiment gate (claims-as-oracle
//!     advisory/OFF + regen promotion blocked until the report shows PASS).
//!   - `flake/` — flake-stats collector + quarantine policy + false-positive
//!     guard (WP-C6): feeds per-test statistics from every CheckResult,
//!     detects flaky tests within <30 runs, emits annotation-only quarantine
//!     list (no auto-act), and guards against classifying deterministic
//!     failures as flaky.

pub mod bisect;
pub mod experiment;
pub mod flake;
