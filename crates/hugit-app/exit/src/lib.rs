//! hugit-app/exit — exit telemetry + the money gate (WP-B9).
//!
//! Implements:
//!   ① per-install activity → week-3 retention computable vs ≥40% threshold
//!   ② feedback capture distinguishes UNPROMPTED from PROMPTED ("I'd pay")
//!   ③ exit-metric report generated from data, auditable
//!   ④ cohort/window guards: n=10 external teams, ≥3 weeks, anchored to
//!      first-10-paying-customers event, inside 90 days — else "insufficient/
//!      out-of-window", never a pass
//!   ⑤(R4) ≥3 gate pass/fail: 2 correctly-counted unprompted → FAIL even
//!      with ≥40% retention; exactly 3 → PASS (both gates must hold)
//!   ⑥ THE MONEY GATE BINDS: billing structurally blocked until report=PASS;
//!      FAIL/insufficient/out-of-window CANNOT enable billing; DEGRADED
//!      evaluator = "insufficient" → fails closed; enable-billing event audited
//!
//! # Privacy model (①)
//!
//! Activity events collected per-install are:
//! - A monotonically-increasing install ID (UUID, no PII)
//! - Week-of-activity bucket (integer 0..N), not a timestamp
//! - Event kind: `active` | `inactive`
//!
//! No usernames, email addresses, IP addresses, or repository contents are
//! collected. The install-ID is generated locally and is not correlated with
//! any account identity outside the install. All retention computation is
//! performed server-side over anonymized event buckets.

pub mod classifier;
pub mod cohort;
pub mod gate;
pub mod report;
pub mod retention;

pub use classifier::{FeedbackKind, FeedbackSignal};
pub use cohort::{CohortGuardResult, CohortState};
pub use gate::{EnableBillingError, GateEvaluatorState, MoneyGate, MoneyGateDecision};
pub use report::{ExitReport, ExitReportStatus};
pub use retention::{ActivityEvent, RETENTION_THRESHOLD, RetentionMetrics, RetentionResult};
