//! WP-C6 — flake-stats collector + quarantine policy + false-positive guard.
//!
//! ## Module layout
//!
//! - [`stats`]       — per-test running statistics, fed by every `CheckResult` (①).
//! - [`detector`]    — statistical flake detector: planted 20% flake detected <30 runs (②).
//! - [`quarantine`]  — policy-artifact quarantine list, annotation-only, no auto-act (③).
//! - [`collector`]   — `FlakeCollector` facade: wires stats → detector → quarantine (①–④).
//!
//! ## Key invariants (all provable hermetically)
//!
//! - **①** Every `CheckResult` fed via [`feed_result`] advances the per-test
//!   running statistics in the in-process collector immediately.
//! - **②** A planted 20%-flake test is classified [`Classification::Flaky`]
//!   within <30 executions of that test.
//! - **③** The quarantine list is a POLICY ARTIFACT only: it carries non-gating
//!   [`QuarantineAnnotation`]s.  No reorder / skip / block / gate action exists —
//!   the [`AutoActMechanism`] type is an uninhabited (zero-variant) enum.
//! - **④** A deterministically-failing (100% fail) test is classified
//!   [`Classification::Real`] and never appears in the quarantine list.
//!
//! C6 is PURE stats/policy logic.  It consumes [`CheckResult`] (the frozen B2
//! contract output) and produces a [`QuarantineList`] of annotations.  It does
//! NOT wire itself into any execution gate.

mod collector;
mod detector;
mod quarantine;
mod stats;

pub use collector::{FlakeCollector, feed_result};
pub use detector::Classification;
pub use quarantine::{AutoActMechanism, QuarantineAnnotation, QuarantineList};
pub use stats::TestStats;
