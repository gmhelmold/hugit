//! Versioned, auditable cost model — item ④(R3) of WP-B7.
//!
//! "$ saved" = `minutes × rate` via a VERSIONED cost model whose rates are
//! stated in the artifact. The figure is reconcilable against the minutes count
//! (same minutes × stated rate = stated $). The model version is recorded so a
//! past figure is reproducible. No unversioned or hidden-rate dollar claims.

use serde::{Deserialize, Serialize};

/// The version identifier for a specific cost model rate table.
///
/// Versioned so past figures remain reproducible from the stated rates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostModelVersion(pub &'static str);

/// v1 of the cost model — USD rate per CI minute for GitHub-hosted runners.
///
/// Rate source: GitHub Actions pricing (2024-H2 public list):
///   - Linux 2-core: $0.008/min
///
/// Version string: "v1-2024-H2".
pub const COST_MODEL_VERSION_V1: CostModelVersion = CostModelVersion("v1-2024-H2");

/// A cost model: a versioned table of per-minute rates (USD).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostModel {
    /// The version of this model (recorded in every saved-cost artifact).
    pub model_version: &'static str,
    /// USD per minute for the default Linux 2-core runner.
    pub usd_per_minute_linux_2core: f64,
}

impl CostModel {
    /// The v1 cost model (2024-H2 GitHub-hosted runner pricing).
    pub const V1: CostModel = CostModel {
        model_version: "v1-2024-H2",
        usd_per_minute_linux_2core: 0.008,
    };

    /// Compute the dollar saving for a given number of saved minutes.
    ///
    /// Returns a [`SavedCost`] with the stated rate and reconcilable figure:
    /// `dollars = minutes × rate` (both values recorded for audit).
    pub fn compute(&self, saved_minutes: u64) -> SavedCost {
        let dollars = saved_minutes as f64 * self.usd_per_minute_linux_2core;
        SavedCost {
            model_version: self.model_version.to_string(),
            saved_minutes,
            rate_usd_per_minute: self.usd_per_minute_linux_2core,
            dollars_saved: dollars,
        }
    }

    /// Format the dollar saving as a display string (e.g. "$0.08").
    pub fn format_dollars(dollars: f64) -> String {
        format!("${:.2}", dollars)
    }
}

/// A computed savings figure with full reconciliation provenance.
///
/// Invariant: `dollars_saved == saved_minutes as f64 × rate_usd_per_minute`.
/// Any past figure can be re-derived from `(saved_minutes, rate_usd_per_minute, model_version)`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedCost {
    /// The cost model version that produced this figure (owned for serialisation).
    pub model_version: String,
    /// Number of CI minutes saved.
    pub saved_minutes: u64,
    /// Rate applied (USD per minute), as stated in the model.
    pub rate_usd_per_minute: f64,
    /// Derived dollar saving: `saved_minutes × rate_usd_per_minute`.
    pub dollars_saved: f64,
}

impl SavedCost {
    /// Verify the reconciliation invariant: stated minutes × stated rate = stated $.
    ///
    /// Returns `true` if the stored `dollars_saved` equals
    /// `saved_minutes * rate_usd_per_minute` within floating-point epsilon.
    pub fn is_reconcilable(&self) -> bool {
        let expected = self.saved_minutes as f64 * self.rate_usd_per_minute;
        (self.dollars_saved - expected).abs() < 1e-9
    }

    /// Format the dollar saving as a display string (e.g. "$0.08").
    pub fn display(&self) -> String {
        CostModel::format_dollars(self.dollars_saved)
    }
}
