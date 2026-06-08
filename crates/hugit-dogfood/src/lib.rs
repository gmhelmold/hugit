//! hugit-dogfood — the B8 dogfood harness.
//!
//! Drives a real 5-PR agent-fleet wave through the actual Phase-B stack
//! (hugit-queue + hugit-checks + hugit-refstore), measures against a defined
//! memoization-OFF baseline in a versioned report with formulas, and proves a
//! soak harness with 0 wrong-merge / 0 lost-PR (event-audited).
//!
//! # Claims (B8 contract)
//! Only `crates/hugit-dogfood/**`, root `Cargo.toml` (member addition),
//! `CHANGELOG.md` (append), `Cargo.lock`.  NO production-crate modifications.
//!
//! # P2 seam (documented)
//! The real 48-hour wall-clock soak against a live GitHub App installation is
//! gated behind `HUGIT_DOGFOOD_LIVE`. Every in-process item (①②③ harness) is
//! proven hermetically now.

pub mod baseline;
pub mod focus_gate;
pub mod soak;
pub mod wave;

/// A target repository known to the dogfood harness.
/// Only targets in [`focus_gate::DOGFOOD_TARGET_ALLOWLIST`] may be enrolled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DogfoodTarget {
    /// Short name of the target (e.g. "hugit", "corelink-workspaces").
    pub name: String,
}

impl DogfoodTarget {
    /// Construct a target after passing the focus gate.
    pub fn new(name: impl Into<String>) -> Result<Self, focus_gate::FocusGateError> {
        let name = name.into();
        focus_gate::assert_excluded(&name)?;
        Ok(DogfoodTarget { name })
    }
}

/// Configuration for the compressed/accelerated deterministic soak (item ③).
#[derive(Debug, Clone)]
pub struct SoakConfig {
    /// Number of waves in the compressed soak.
    pub wave_count: usize,
    /// PR count per wave (default: 5 to match item ①).
    pub prs_per_wave: usize,
}

impl SoakConfig {
    /// A compressed deterministic soak: `wave_count` waves, 5 PRs each.
    pub fn compressed_deterministic(wave_count: usize) -> Self {
        SoakConfig {
            wave_count,
            prs_per_wave: 5,
        }
    }
}
