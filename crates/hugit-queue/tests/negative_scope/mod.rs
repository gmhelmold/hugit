//! Negative-scope fixtures for WP-B10.
//!
//! This module holds the absence-assertion helpers used by the
//! `acceptance_wp-b10` test binary.  Everything here is read-only with
//! respect to the crate source: no `fs::write`, no `File::create`.

/// Source-path constants for grep-based absence assertions.
/// Used by `acceptance_wp-b10` tests to anchor the structural proofs to
/// specific source directories (read-only; never passed to fs::write).
///
/// Read by the acceptance test to verify mechanism absence at the source
/// level (static, structural proof).
#[allow(dead_code)]
pub mod paths {
    /// Phase-B dispatch path (pure core engine).
    pub const QUEUE_CORE: &str = "crates/hugit-queue/src/core";
    /// Phase-B github-layer dispatch path (absent in Phase B; dir may not exist).
    pub const QUEUE_GITHUB: &str = "crates/hugit-queue/src/github";
    /// hugit-checks derived-file regen drivers (C4-sanctioned; scoped grep).
    pub const CHECKS_REGEN: &str = "crates/hugit-checks/src/regen";
    /// Full hugit-queue src tree (for broad rebase-symbol scan).
    pub const QUEUE_SRC: &str = "crates/hugit-queue/src";
}

/// Symbol sets whose presence in the Phase-B dispatch path would indicate a
/// demoted feature leaked back into the build.
/// Used by `acceptance_wp-b10` tests to document and anchor the grep patterns.
#[allow(dead_code)]
pub mod forbidden_symbols {
    /// Symbols that would indicate a dispatch-time claim/lease acquisition
    /// mechanism (demoted; conflict discovery must happen at landing/union only).
    pub const DISPATCH_CLAIM: &[&str] = &[
        "claim_acquire",
        "acquire_claim",
        "RunnerLease::dispatch",
        "dispatch_lease",
    ];

    /// Symbols that would indicate the regenerative-rebase path
    /// (⛔ CUT from Phase B; textual fast-path is the only rebase in Phase B).
    pub const REGEN_REBASE: &[&str] = &[
        "regen_rebase",
        "regenerative_rebase",
        "RegenRebase",
        "reexec_rebase",
        "rebase_regenerat",
    ];
}
