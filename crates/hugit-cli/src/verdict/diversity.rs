//! Diversity enforcement (WP-D7 ⑤).
//!
//! A panel's value is its independence. A homogeneous panel — every lens
//! sharing one prompt *and* one model — is no panel at all: it is one reviewer
//! polled N times. [`enforce_diversity`] rejects such a panel by construction
//! and requires a real panel to dispatch **distinct prompts AND ≥2 distinct
//! models**.

use crate::verdict::panel_dispatch::Panel;
use std::collections::BTreeSet;

/// The minimum number of distinct models a real panel must dispatch to.
pub const MIN_DISTINCT_MODELS: usize = 2;

/// Why a panel failed diversity enforcement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiversityError {
    /// The panel had fewer than two lenses — diversity is undefined.
    TooFewLenses {
        /// Number of lenses present.
        found: usize,
    },
    /// Fewer than [`MIN_DISTINCT_MODELS`] distinct models were dispatched.
    InsufficientModelDiversity {
        /// Number of distinct models found.
        distinct_models: usize,
        /// The required minimum.
        required: usize,
    },
    /// Two or more lenses share an identical prompt.
    DuplicatePrompt,
    /// Fully homogeneous: one prompt AND one model across every lens.
    HomogeneousPanel,
}

impl std::fmt::Display for DiversityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiversityError::TooFewLenses { found } => {
                write!(f, "panel needs ≥2 lenses for diversity, found {found}")
            }
            DiversityError::InsufficientModelDiversity {
                distinct_models,
                required,
            } => write!(
                f,
                "panel dispatched {distinct_models} distinct model(s), requires {required}"
            ),
            DiversityError::DuplicatePrompt => {
                write!(f, "panel has lenses sharing an identical prompt")
            }
            DiversityError::HomogeneousPanel => {
                write!(
                    f,
                    "homogeneous panel: one prompt and one model across all lenses"
                )
            }
        }
    }
}

impl std::error::Error for DiversityError {}

/// Proof that a panel is diverse.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiversityReport {
    /// Number of lenses dispatched.
    pub lens_count: usize,
    /// Number of distinct models dispatched.
    pub distinct_models: usize,
    /// Number of distinct prompt digests dispatched.
    pub distinct_prompts: usize,
}

/// Enforce diversity. Returns a [`DiversityReport`] for a real panel, or a
/// [`DiversityError`] for a homogeneous / under-diverse one.
///
/// Required for acceptance:
/// - ≥2 lenses,
/// - ≥[`MIN_DISTINCT_MODELS`] distinct models,
/// - every prompt distinct (no two lenses share one).
///
/// The fully-homogeneous case (one prompt + one model) is reported with the
/// dedicated [`DiversityError::HomogeneousPanel`] for an unambiguous signal.
pub fn enforce_diversity(panel: &Panel) -> Result<DiversityReport, DiversityError> {
    let lens_count = panel.lenses.len();
    if lens_count < 2 {
        return Err(DiversityError::TooFewLenses { found: lens_count });
    }

    let distinct_models: BTreeSet<&str> = panel.lenses.iter().map(|l| l.model.as_str()).collect();
    let distinct_prompt_digests: BTreeSet<String> =
        panel.lenses.iter().map(|l| l.prompt_digest()).collect();

    // Fully homogeneous: a single prompt AND a single model.
    if distinct_models.len() == 1 && distinct_prompt_digests.len() == 1 {
        return Err(DiversityError::HomogeneousPanel);
    }

    if distinct_prompt_digests.len() != lens_count {
        return Err(DiversityError::DuplicatePrompt);
    }

    if distinct_models.len() < MIN_DISTINCT_MODELS {
        return Err(DiversityError::InsufficientModelDiversity {
            distinct_models: distinct_models.len(),
            required: MIN_DISTINCT_MODELS,
        });
    }

    Ok(DiversityReport {
        lens_count,
        distinct_models: distinct_models.len(),
        distinct_prompts: distinct_prompt_digests.len(),
    })
}
