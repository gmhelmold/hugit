//! The `FlakeCollector` facade — wires stats → detector → quarantine (①–④).
//!
//! ## Responsibilities
//!
//! - **①** [`feed_result`] ingests every `CheckResult` from the B2 executor
//!   into the per-test running statistics immediately.
//! - **②** [`FlakeCollector::classify`] runs the statistical detector against
//!   accumulated stats and returns the current [`Classification`].
//! - **③** [`FlakeCollector::quarantine_list`] builds the policy-artifact
//!   quarantine list (annotation-only) from the current detected-flake set.
//! - **④** The false-positive guard is enforced by the detector:
//!   [`Classification::Real`] is never quarantined.
//!
//! The collector holds no mutable state between calls to `quarantine_list` —
//! the list is derived on demand from the accumulated stats.

use std::collections::HashMap;

use hugit_contracts::CheckResult;

use crate::flake::detector::{Classification, classify_from_stats};
use crate::flake::quarantine::QuarantineList;
use crate::flake::stats::TestStats;

/// The in-process flake-stats collector.
///
/// Keyed by `CheckResult.memo_key` (the stable (tree, def, toolchain) identity
/// that uniquely identifies one test configuration across repeated executions).
#[derive(Debug, Default)]
pub struct FlakeCollector {
    stats: HashMap<String, TestStats>,
}

impl FlakeCollector {
    /// Create an empty collector.
    pub fn new() -> Self {
        FlakeCollector {
            stats: HashMap::new(),
        }
    }

    /// Feed one `CheckResult` into the running statistics (①).
    ///
    /// This is the single ingestion point for execution stats.  Called once per
    /// `CheckResult` produced by the B2 executor — there is no batching.
    pub fn record(&mut self, result: &CheckResult) {
        self.stats
            .entry(result.memo_key.clone())
            .or_default()
            .record(result.exit);
    }

    /// Return the accumulated statistics for one test, or `None` if never fed.
    pub fn stats_for(&self, test_id: &str) -> Option<&TestStats> {
        self.stats.get(test_id)
    }

    /// Classify one test using the current accumulated stats.
    ///
    /// Returns `None` if the test has never been fed.  Returns
    /// `Some(Classification)` otherwise (may be `Unknown` if insufficient
    /// evidence).
    pub fn classify(&self, test_id: &str) -> Option<Classification> {
        self.stats.get(test_id).map(classify_from_stats)
    }

    /// Build the policy-artifact quarantine list (③).
    ///
    /// Scans all accumulated stats, classifies each test, and adds a
    /// non-gating [`crate::flake::QuarantineAnnotation`] for every test
    /// currently classified [`Classification::Flaky`].
    ///
    /// The false-positive guard (④) is enforced here: tests classified
    /// [`Classification::Real`] are NEVER added to the list, even if they
    /// appear in the stats.
    ///
    /// The list is computed fresh on each call — it reflects the current
    /// accumulated stats at the moment of the call.
    pub fn quarantine_list(&self) -> QuarantineList {
        let mut list = QuarantineList::default();
        for (test_id, stats) in &self.stats {
            if classify_from_stats(stats) == Classification::Flaky {
                let note = format!(
                    "detected flaky: {}/{} executions failed ({:.0}% fail rate)",
                    stats.fail_count,
                    stats.run_count,
                    stats.fail_rate() * 100.0
                );
                list.add(test_id.clone(), note);
            }
        }
        list
    }
}

/// Feed one `CheckResult` into a collector (free-function convenience wrapper).
///
/// Equivalent to `collector.record(result)`.  Provided so the acceptance
/// oracle can call `feed_result(&mut collector, &result)` without importing
/// the method name separately.
pub fn feed_result(collector: &mut FlakeCollector, result: &CheckResult) {
    collector.record(result);
}
