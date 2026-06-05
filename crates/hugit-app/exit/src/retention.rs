//! ① per-install activity → week-3 retention computable vs ≥40% threshold.
//!
//! # Privacy model
//!
//! All events are collected as anonymized per-install activity buckets. No
//! personally identifiable information (PII) is stored. The install ID is a
//! locally-generated UUID with no account correlation. Week buckets are
//! computed server-side over aggregated counts.
//!
//! See the crate-level documentation for the full privacy model.

use serde::{Deserialize, Serialize};

/// Minimum week-3 retention rate required for the exit gate to pass.
/// 40% expressed as a ratio (0.0–1.0).
pub const RETENTION_THRESHOLD: f64 = 0.40;

/// Required minimum week index for retention measurement (week-3 = index 3).
pub const WEEK_3_INDEX: u32 = 3;

/// A single per-install anonymized activity event.
///
/// # Privacy
///
/// - `install_id`: locally-generated UUID; no PII, no account correlation.
/// - `week`: integer week bucket (0 = first week of use).
/// - `active`: whether the install had activity during this week.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActivityEvent {
    /// Anonymized install identifier (UUID, no PII).
    pub install_id: String,
    /// Week index since first install (0-based).
    pub week: u32,
    /// Whether the install was active during this week.
    pub active: bool,
}

/// Computed retention metrics for a cohort.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RetentionMetrics {
    /// Total installs observed at week-3.
    pub total_installs_at_week3: usize,
    /// Installs still active at week-3.
    pub active_at_week3: usize,
    /// Computed retention rate (active / total). `None` if total = 0.
    pub retention_rate: Option<f64>,
}

/// The outcome of a retention computation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RetentionResult {
    /// Retention rate meets or exceeds `RETENTION_THRESHOLD`.
    Pass { metrics: RetentionMetrics },
    /// Retention rate is below `RETENTION_THRESHOLD`.
    Fail { metrics: RetentionMetrics },
    /// Insufficient data to compute retention (no installs reached week-3).
    Insufficient,
}

/// Compute week-3 retention from a slice of activity events.
///
/// Only events at `week == WEEK_3_INDEX` are used for counting. An install
/// is counted as "present" at week-3 if any event with `week == 3` exists;
/// it counts as "active" if that event has `active == true`.
pub fn compute_retention(events: &[ActivityEvent]) -> RetentionResult {
    // Collect all install IDs that have a week-3 event.
    let mut total = std::collections::HashSet::new();
    let mut active = std::collections::HashSet::new();

    for ev in events {
        if ev.week == WEEK_3_INDEX {
            total.insert(ev.install_id.clone());
            if ev.active {
                active.insert(ev.install_id.clone());
            }
        }
    }

    let total_count = total.len();
    let active_count = active.len();

    if total_count == 0 {
        return RetentionResult::Insufficient;
    }

    let rate = active_count as f64 / total_count as f64;
    let metrics = RetentionMetrics {
        total_installs_at_week3: total_count,
        active_at_week3: active_count,
        retention_rate: Some(rate),
    };

    if rate >= RETENTION_THRESHOLD {
        RetentionResult::Pass { metrics }
    } else {
        RetentionResult::Fail { metrics }
    }
}
