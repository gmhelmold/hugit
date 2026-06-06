//! Supported horizon for `ctx resume`.
//!
//! The horizon is the maximum age of a journal (measured from its last
//! recorded event) within which `ctx resume` will attempt reconstruction.
//! Beyond the horizon the resume is REFUSED or DEGRADED as documented —
//! never a silent stale reconstruction.
//!
//! **Documented horizon:** minutes-to-days (command catalog v2, §D,
//! "Journals + short-horizon resume" row). Concrete default: 7 days.
//!
//! This module is purely a decision: given a journal's `last_recorded_at`
//! (Unix epoch milliseconds) and a `now_ms` value, is resume within horizon?
//! There is no I/O, no randomness — the function is pure and deterministic.

/// The default supported horizon for `ctx resume` (7 days in milliseconds).
///
/// Value: `7 * 24 * 60 * 60 * 1000` ms = 604_800_000 ms.
///
/// Source: command catalog v2 — "minutes-to-days"; 7 days is the documented
/// upper boundary. Adjustment requires an explicit doc change (see catalog row
/// "Journals + short-horizon resume").
pub const DEFAULT_HORIZON_MS: u64 = 7 * 24 * 60 * 60 * 1000; // 604_800_000

/// The result of a horizon check — did the resume fall within the supported
/// window?
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HorizonResult {
    /// The journal's last event is within the supported horizon: resume may
    /// attempt reconstruction.
    WithinHorizon,
    /// The journal's last event is beyond the supported horizon: resume is
    /// REFUSED. The age (ms) and the horizon (ms) are reported for the
    /// caller to surface in a user-facing error/degraded response.
    BeyondHorizon {
        /// Age of the journal in milliseconds (`now_ms - last_recorded_at`).
        age_ms: u64,
        /// The horizon that was exceeded (milliseconds).
        horizon_ms: u64,
    },
}

impl HorizonResult {
    /// `true` if resume is within the supported horizon.
    pub fn is_within(&self) -> bool {
        matches!(self, HorizonResult::WithinHorizon)
    }

    /// `true` if resume is beyond the supported horizon.
    pub fn is_beyond(&self) -> bool {
        matches!(self, HorizonResult::BeyondHorizon { .. })
    }
}

/// Check whether a journal's last event falls within the supported resume
/// horizon.
///
/// # Arguments
/// * `last_recorded_at` — Unix epoch milliseconds of the journal's most
///   recent entry.
/// * `now_ms` — Unix epoch milliseconds at the time of the resume attempt.
/// * `horizon_ms` — The configured horizon (use [`DEFAULT_HORIZON_MS`] for the
///   standard window).
///
/// # Returns
/// [`HorizonResult::WithinHorizon`] if `now_ms - last_recorded_at ≤
/// horizon_ms`; [`HorizonResult::BeyondHorizon`] otherwise.
///
/// Saturating arithmetic is used: if `now_ms < last_recorded_at` (clock
/// skew) the age is treated as 0 (always within horizon).
pub fn check_horizon(last_recorded_at: u64, now_ms: u64, horizon_ms: u64) -> HorizonResult {
    let age_ms = now_ms.saturating_sub(last_recorded_at);
    if age_ms <= horizon_ms {
        HorizonResult::WithinHorizon
    } else {
        HorizonResult::BeyondHorizon { age_ms, horizon_ms }
    }
}
