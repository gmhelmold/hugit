//! Prompt-isolation auditor (WP-D7 ① + ⑥).
//!
//! The auditor takes a built [`ReviewerInput`] and the [`IntentSidecar`] of the
//! change under review, and **proves** that no author-controlled field of the
//! sidecar appears anywhere in the reviewer's input. This is a belt-and-braces
//! check on top of the structural guarantee in `panel_dispatch`: ground truth
//! has no sidecar-prose constructor, and this auditor confirms the contract
//! held for a concrete input.
//!
//! It also audits **lens isolation**: distinct lenses must carry distinct
//! prompts (no two reviewers are secretly the same reviewer).

use crate::verdict::panel_dispatch::{Panel, ReviewerInput};
use hugit_contracts::IntentSidecar;
use std::collections::BTreeSet;

/// The author-controlled fields of an [`IntentSidecar`] — the persuasion
/// surface that must never reach a reviewer.
///
/// `intent_id` and `authoritative` are intentionally NOT here: the bare id is a
/// served identifier (attribution, not prose), and `authoritative` is a fixed
/// `false` flag, not author free-text.
pub fn author_controlled_fields(sidecar: &IntentSidecar) -> Vec<&str> {
    let mut fields = vec![sidecar.charter.as_str(), sidecar.context_ref.as_str()];
    fields.extend(sidecar.acceptance.iter().map(|s| s.as_str()));
    fields
}

/// Why an isolation audit failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IsolationError {
    /// An author-controlled string leaked into the reviewer input.
    AuthorTextReachedReviewer {
        /// The leaked field's content (truncated for the message).
        leaked: String,
    },
    /// Two lenses in the panel share an identical prompt (not isolated).
    LensesNotIsolated,
}

impl std::fmt::Display for IsolationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IsolationError::AuthorTextReachedReviewer { leaked } => {
                let preview: String = leaked.chars().take(48).collect();
                write!(f, "author-controlled text reached reviewer: {preview:?}")
            }
            IsolationError::LensesNotIsolated => {
                write!(f, "lenses are not isolated: a prompt is shared")
            }
        }
    }
}

impl std::error::Error for IsolationError {}

/// Proof that a reviewer input is isolated from author persuasion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolationReport {
    /// Number of author-controlled fields checked for leakage.
    pub author_fields_checked: usize,
    /// Number of distinct lens prompts audited (when auditing a panel).
    pub distinct_prompts: usize,
}

/// Audit that a single reviewer input contains none of the change author's
/// controlled text.
///
/// The check scans the FULL serialized reviewer input (prompt + every ground
/// truth field) for any non-trivial author-controlled substring.
pub fn audit_isolation(
    input: &ReviewerInput,
    sidecar: &IntentSidecar,
) -> Result<IsolationReport, IsolationError> {
    let haystack = serialize_input(input);
    let author_fields = author_controlled_fields(sidecar);

    for field in &author_fields {
        let needle = field.trim();
        // Empty / whitespace-only fields carry no persuasion; skip them so the
        // audit is meaningful (an empty substring matches everything).
        if needle.is_empty() {
            continue;
        }
        if haystack.contains(needle) {
            return Err(IsolationError::AuthorTextReachedReviewer {
                leaked: (*field).to_string(),
            });
        }
    }

    Ok(IsolationReport {
        author_fields_checked: author_fields.len(),
        distinct_prompts: 1,
    })
}

/// Audit lens isolation across a whole panel: every lens prompt must be
/// distinct (independent reviewers), and (when a sidecar is supplied) each
/// lens's served input must be free of author text.
pub fn audit_panel_isolation(
    panel: &Panel,
    ground_truth: &crate::verdict::panel_dispatch::ServedGroundTruth,
    sidecar: &IntentSidecar,
) -> Result<IsolationReport, IsolationError> {
    let prompts: BTreeSet<String> = panel.lenses.iter().map(|l| l.prompt_digest()).collect();
    if prompts.len() != panel.lenses.len() {
        return Err(IsolationError::LensesNotIsolated);
    }

    let mut fields_checked = 0;
    for lens in &panel.lenses {
        let input = ReviewerInput::new(lens.prompt.clone(), ground_truth.clone());
        let report = audit_isolation(&input, sidecar)?;
        fields_checked = report.author_fields_checked;
    }

    Ok(IsolationReport {
        author_fields_checked: fields_checked,
        distinct_prompts: prompts.len(),
    })
}

/// Flatten a reviewer input into one searchable string covering every field a
/// reviewer can observe.
fn serialize_input(input: &ReviewerInput) -> String {
    let gt = &input.ground_truth;
    let mut parts: Vec<String> = vec![
        input.prompt.clone(),
        gt.intent.clone(),
        gt.tree_hash.clone(),
    ];
    parts.extend(gt.impact.iter().cloned());
    parts.extend(gt.contract_digests.iter().cloned());
    parts.extend(gt.evidence_refs.iter().cloned());
    for cr in &gt.check_results {
        parts.push(cr.tree_hash.clone());
        parts.push(cr.stdout_ref.clone());
        parts.push(cr.stderr_ref.clone());
        for a in &cr.artifacts {
            parts.push(a.path.clone());
            parts.push(a.digest.clone());
        }
    }
    parts.join("\n")
}
