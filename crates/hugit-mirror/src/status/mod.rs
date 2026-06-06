//! Status emitter module (WP-E3).
//!
//! Emits `CheckResult` outcomes as GitHub commit statuses, serves a status
//! badge with an explicit staleness bound, and handles API 429/5xx via
//! bounded backoff with observable failures — never silent-wrong, never
//! stuck-pending.
//!
//! Sub-modules:
//! - [`emitter`] — maps each `CheckResult` to a GitHub commit status (item ①)
//! - [`badge`] — badge renderer with staleness-bound + last-known/API-down
//!   semantics (item ②)
//! - [`backoff`] — bounded retry/backoff for 429/5xx; surfaces terminal
//!   failures as observable, never stuck-pending (item ③)

pub mod backoff;
pub mod badge;
pub mod emitter;

pub use backoff::{BackoffConfig, BackoffError, MockHttpResponse, RetryOutcome, StatusBackoff};
pub use badge::{BadgeState, BadgeStatus, STALENESS_BOUND_MS};
pub use emitter::{EmitError, StatusEmitter, StatusPayload};
