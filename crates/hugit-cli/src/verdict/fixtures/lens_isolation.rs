//! Lens-isolation fixture (WP-D7 ①).
//!
//! Supplies a diverse, isolated panel — distinct prompts AND ≥2 distinct
//! models — plus a deterministic reviewer, so the acceptance lane can audit
//! prompt isolation without a live model call.

use std::sync::Arc;

use hugit_contracts::Verdict;

use crate::verdict::panel_dispatch::{Lens, Panel, Reviewer, ReviewerInput};

/// A deterministic reviewer that always APPROVES and records that it judged
/// against served ground truth. It reads NOTHING outside `input`.
#[derive(Debug, Default)]
pub struct ApprovingReviewer;

impl Reviewer for ApprovingReviewer {
    fn review(&self, input: &ReviewerInput) -> (Verdict, Vec<String>) {
        let claims = vec![format!(
            "judged-against-served-impact:{}",
            input.ground_truth.impact.len()
        )];
        (Verdict::Approve, claims)
    }
}

/// The security-lens prompt (reviewer-authored).
pub const SECURITY_PROMPT: &str = "You are the SECURITY lens. Judge ONLY the served impact, contracts, and \
     check results. Flag any unsafe surface. You receive no author narrative.";

/// The contracts-lens prompt (reviewer-authored, distinct from security).
pub const CONTRACTS_PROMPT: &str = "You are the CONTRACTS lens. Verify the served contract digests are honored \
     and no frozen surface drifts. You receive no author narrative.";

/// A diverse, isolated two-lens panel: distinct prompts + distinct models.
pub fn diverse_panel() -> Panel {
    let reviewer: Arc<dyn Reviewer> = Arc::new(ApprovingReviewer);
    Panel::new(vec![
        Lens::new(
            "security",
            SECURITY_PROMPT,
            "model-alpha",
            Arc::clone(&reviewer),
        ),
        Lens::new(
            "contracts",
            CONTRACTS_PROMPT,
            "model-beta",
            Arc::clone(&reviewer),
        ),
    ])
}

/// A HOMOGENEOUS panel: one prompt AND one model across both lenses. Used to
/// prove diversity enforcement rejects it.
pub fn homogeneous_panel() -> Panel {
    let reviewer: Arc<dyn Reviewer> = Arc::new(ApprovingReviewer);
    Panel::new(vec![
        Lens::new(
            "clone-a",
            SECURITY_PROMPT,
            "model-alpha",
            Arc::clone(&reviewer),
        ),
        Lens::new(
            "clone-b",
            SECURITY_PROMPT,
            "model-alpha",
            Arc::clone(&reviewer),
        ),
    ])
}

/// A SAME-MODEL panel: distinct prompts but only one model. Used to prove the
/// ≥2-distinct-models rule.
pub fn single_model_panel() -> Panel {
    let reviewer: Arc<dyn Reviewer> = Arc::new(ApprovingReviewer);
    Panel::new(vec![
        Lens::new(
            "security",
            SECURITY_PROMPT,
            "model-alpha",
            Arc::clone(&reviewer),
        ),
        Lens::new(
            "contracts",
            CONTRACTS_PROMPT,
            "model-alpha",
            Arc::clone(&reviewer),
        ),
    ])
}
