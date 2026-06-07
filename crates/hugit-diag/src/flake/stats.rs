//! Per-test running statistics (contract ①).
//!
//! Every `CheckResult` from the B2 executor feeds one `TestStats` entry.
//! Statistics are keyed by `CheckResult.memo_key` (the stable (tree, def,
//! toolchain) identity across repeated executions of the same test).

/// Running statistics for one test, keyed by `memo_key`.
///
/// Accumulated by [`crate::flake::collector::FlakeCollector`] as executions
/// arrive.  All fields are monotonically increasing (never decremented).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TestStats {
    /// Total number of executions recorded for this test.
    pub run_count: u64,
    /// Number of executions that exited 0 (pass).
    pub pass_count: u64,
    /// Number of executions that exited non-zero (fail).
    pub fail_count: u64,
}

impl TestStats {
    /// Record one execution with the given exit code.
    pub(crate) fn record(&mut self, exit: i32) {
        self.run_count += 1;
        if exit == 0 {
            self.pass_count += 1;
        } else {
            self.fail_count += 1;
        }
    }

    /// Failure rate in [0.0, 1.0]; 0.0 when no runs recorded.
    pub fn fail_rate(&self) -> f64 {
        if self.run_count == 0 {
            0.0
        } else {
            self.fail_count as f64 / self.run_count as f64
        }
    }
}
