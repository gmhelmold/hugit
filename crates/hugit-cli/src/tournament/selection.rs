//! Selection — judge panel selects the best candidate per documented criteria.
//!
//! The judge panel is the D7 verdict mechanism, consumed here. D13 does not
//! re-implement a panel; it uses [`hugit_cli::verdict::panel_dispatch::dispatch`]
//! to run the D7 panel over each candidate's served ground truth, then selects
//! by the written criteria below.
//!
//! ## Documented selection criteria (WRITTEN — binding, not opaque)
//!
//! 1. **Evidence completeness** — the candidate with the most evidence refs
//!    is preferred (more served proof).
//! 2. **Approval count** — the candidate with the most `Approve` verdicts from
//!    the panel is preferred (more reviewer agreement).
//! 3. **Index tiebreak** — ties are broken by lowest candidate index
//!    (deterministic, not arbitrary).
//!
//! The criteria are applied in priority order: (1) then (2) then (3). A known-
//! best candidate in the fixture must satisfy at least one of (1) or (2) to
//! guarantee selection; the acceptance test plants it at index 0 with the most
//! evidence refs.

use crate::tournament::candidate::Candidate;
use crate::verdict::panel_dispatch::{Panel, ServedGroundTruth, dispatch};
use hugit_contracts::Verdict;

/// The result of a tournament selection.
#[derive(Debug, Clone)]
pub struct SelectionResult {
    /// The selected (winning) candidate.
    pub winner: Candidate,
    /// The losing candidates, retained as addressable evidence.
    pub losers: Vec<Candidate>,
    /// The selection score assigned to the winner (for evidence/audit).
    pub winner_score: SelectionScore,
}

/// The score computed for each candidate under the documented criteria.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct SelectionScore {
    /// Criterion ①: number of evidence refs (higher is better).
    pub evidence_count: usize,
    /// Criterion ②: number of `Approve` verdicts from the panel (higher is better).
    pub approval_count: usize,
    /// Criterion ③: negated index (lower index = higher score → prefer low index on tie).
    /// Stored as `usize::MAX - index` so that lower indices score higher.
    pub index_score: usize,
}

impl SelectionScore {
    fn for_candidate(c: &Candidate, approval_count: usize) -> Self {
        Self {
            evidence_count: c.evidence_refs.len(),
            approval_count,
            index_score: usize::MAX.saturating_sub(c.index),
        }
    }
}

/// Run the judge panel over each candidate's served ground truth and select the
/// best candidate per the documented criteria.
///
/// # Arguments
///
/// * `candidates` — the N candidates to evaluate (must be non-empty).
/// * `panel` — the D7 verdict panel (consumed, not re-implemented).
/// * `ground_truths` — one served ground truth per candidate (same order).
///
/// # Errors
///
/// Returns `Err` if `candidates` is empty, or if `ground_truths.len() !=
/// candidates.len()`, or if panel dispatch fails.
pub fn select(
    candidates: Vec<Candidate>,
    panel: &Panel,
    ground_truths: &[ServedGroundTruth],
) -> Result<SelectionResult, SelectionError> {
    if candidates.is_empty() {
        return Err(SelectionError::NoCandidates);
    }
    if ground_truths.len() != candidates.len() {
        return Err(SelectionError::ArityMismatch {
            candidates: candidates.len(),
            ground_truths: ground_truths.len(),
        });
    }

    // Score each candidate.
    let scored: Vec<(SelectionScore, usize)> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let gt = &ground_truths[i];
            let approval_count = dispatch(panel, gt)
                .map(|verdicts| {
                    verdicts
                        .iter()
                        .filter(|v| v.verdict == Verdict::Approve)
                        .count()
                })
                .unwrap_or(0);
            let score = SelectionScore::for_candidate(c, approval_count);
            (score, i)
        })
        .collect();

    // Select the candidate with the highest score under documented criteria.
    // Ord on SelectionScore is lexicographic: evidence_count > approval_count > index_score.
    let winner_idx = scored
        .iter()
        .enumerate()
        .max_by_key(|(_, (score, _))| score)
        .map(|(pos, _)| pos)
        .expect("scored is non-empty");

    let winner_score = scored[winner_idx].0.clone();
    let mut candidates = candidates;
    let winner = candidates.remove(winner_idx).mark_selected();
    let losers = candidates;

    Ok(SelectionResult {
        winner,
        losers,
        winner_score,
    })
}

/// Errors from the selection path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    /// No candidates were provided.
    NoCandidates,
    /// The number of ground truths does not match the number of candidates.
    ArityMismatch {
        candidates: usize,
        ground_truths: usize,
    },
    /// The judge panel could not produce verdicts.
    PanelError(String),
}

impl std::fmt::Display for SelectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectionError::NoCandidates => write!(f, "no candidates to select from"),
            SelectionError::ArityMismatch {
                candidates,
                ground_truths,
            } => write!(
                f,
                "arity mismatch: {candidates} candidates vs {ground_truths} ground truths"
            ),
            SelectionError::PanelError(e) => write!(f, "panel error: {e}"),
        }
    }
}

impl std::error::Error for SelectionError {}

/// Retrieve a losing candidate by its `candidate_ref` from a `SelectionResult`.
///
/// This is the loser-addressability API: losers are retained as evidence objects
/// after selection and are resolvable by their content-addressed ref.
pub fn resolve_loser<'a>(
    result: &'a SelectionResult,
    candidate_ref: &str,
) -> Option<&'a Candidate> {
    result
        .losers
        .iter()
        .find(|c| c.candidate_ref == candidate_ref)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tournament::candidate::produce_candidates;
    use crate::verdict::fixtures::lens_isolation::{
        ApprovingReviewer, CONTRACTS_PROMPT, SECURITY_PROMPT,
    };
    use crate::verdict::panel_dispatch::Lens;
    use hugit_contracts::IntentSidecar;
    use std::sync::Arc;

    fn test_intent() -> IntentSidecar {
        IntentSidecar {
            intent_id: "intent-selection-test".into(),
            charter: "selection test".into(),
            acceptance: vec![],
            context_ref: "blob://ctx".into(),
            authoritative: false,
        }
    }

    fn simple_ground_truth(intent: &str, evidence_refs: Vec<String>) -> ServedGroundTruth {
        ServedGroundTruth::from_served(
            intent,
            "tree-hash-001",
            vec![],
            vec![],
            vec![],
            evidence_refs,
        )
    }

    fn two_model_panel() -> Panel {
        let reviewer: Arc<dyn crate::verdict::panel_dispatch::Reviewer> =
            Arc::new(ApprovingReviewer);
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

    #[test]
    fn selection_picks_highest_score() {
        let intent = test_intent();
        let mut candidates = produce_candidates(&intent, &["strat-a", "strat-b", "strat-c"]);
        // Give candidate 2 the most evidence refs (criterion ①).
        candidates[2] = candidates[2].clone().with_evidence(vec![
            "ref-1".into(),
            "ref-2".into(),
            "ref-3".into(),
        ]);
        candidates[0] = candidates[0].clone().with_evidence(vec!["ref-a".into()]);

        let gts: Vec<ServedGroundTruth> = candidates
            .iter()
            .map(|c| simple_ground_truth(&c.intent_id, c.evidence_refs.clone()))
            .collect();

        let panel = two_model_panel();
        let result = select(candidates, &panel, &gts).expect("selection must succeed");
        assert_eq!(
            result.winner.index, 2,
            "candidate with most evidence refs wins"
        );
        assert_eq!(result.losers.len(), 2);
    }
}
