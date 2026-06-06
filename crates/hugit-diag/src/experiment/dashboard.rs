//! The experiment dashboard (contract ②).
//!
//! Computes, from a SEALED corpus only, the three reported figures:
//!   - claim-disjointness percentage,
//!   - regen agree / disagree counts,
//!   - `n` (the evaluated sample size).
//!
//! The dashboard is a pure projection of the sealed corpus — it never reads the
//! mutable pre-seal buffer, so its figures always correspond to the audited
//! sample (④).

use crate::experiment::corpus::SealedCorpus;
use crate::experiment::datapoint::RegenOutcome;

/// The dashboard figures derived from a sealed corpus (②).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dashboard {
    /// Evaluated sample size (degraded waves excluded — ⑦).
    pub n: usize,
    /// Number of datapoints whose claims stayed disjoint.
    pub disjoint_count: usize,
    /// Number of regen-honesty AGREE outcomes.
    pub regen_agree: usize,
    /// Number of regen-honesty DISAGREE outcomes.
    pub regen_disagree: usize,
}

impl Dashboard {
    /// Project a sealed corpus into the dashboard figures.
    pub fn from_sealed(corpus: &SealedCorpus) -> Self {
        let evaluated = corpus.evaluated();
        let n = evaluated.len();
        let disjoint_count = evaluated.iter().filter(|d| d.claims_disjoint).count();
        let regen_agree = evaluated
            .iter()
            .filter(|d| matches!(d.regen, RegenOutcome::Agree))
            .count();
        let regen_disagree = n - regen_agree;
        Dashboard {
            n,
            disjoint_count,
            regen_agree,
            regen_disagree,
        }
    }

    /// Claim-disjointness as a percentage in `[0.0, 100.0]`. Returns `0.0` for
    /// an empty sample (there is nothing disjoint to report).
    pub fn disjointness_pct(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            (self.disjoint_count as f64) * 100.0 / (self.n as f64)
        }
    }
}
