//! Panel dispatch — lens fan-out over **served ground truth only**.
//!
//! The dispatch path is the structural heart of the no-self-defense invariant
//! (WP-D7 ⑥). A reviewer's entire input is a [`ReviewerInput`], and a
//! `ReviewerInput` can only be constructed from [`ServedGroundTruth`]. Ground
//! truth is built from served evidence — `CheckResult`s, build-graph impact,
//! and frozen contract digests — and there is **no field, method, or path** by
//! which author-controlled `IntentSidecar` text enters it. The author's
//! persuasion fields are therefore not merely ignored; they are structurally
//! unreachable.

use hugit_contracts::{CheckResult, IntentSidecar, Verdict, VerdictObject};
use sha2::{Digest, Sha256};

/// Served ground truth — the *only* material a reviewer ever sees.
///
/// Constructed exclusively from served evidence. There is deliberately **no
/// constructor and no field** that accepts author-controlled `IntentSidecar`
/// prose: the intent id and tree hash carried here are bare identifiers used
/// for attribution, not persuasion surface.
#[derive(Debug, Clone, PartialEq)]
pub struct ServedGroundTruth {
    /// The intent id under review (a bare identifier — never the charter).
    pub intent: String,
    /// The reviewed workspace snapshot's Merkle tree hash.
    pub tree_hash: String,
    /// Build-graph impact: the set of affected target identifiers.
    pub impact: Vec<String>,
    /// Frozen contract surfaces touched by the change (digest-addressed).
    pub contract_digests: Vec<String>,
    /// Served check results (memoised evidence) the reviewer judges against.
    pub check_results: Vec<CheckResult>,
    /// Content-addressed refs to the served evidence blobs.
    pub evidence_refs: Vec<String>,
}

impl ServedGroundTruth {
    /// Build ground truth from served evidence. Note the absence of any
    /// `IntentSidecar` parameter: author prose has no entry point.
    pub fn from_served(
        intent: impl Into<String>,
        tree_hash: impl Into<String>,
        impact: Vec<String>,
        contract_digests: Vec<String>,
        check_results: Vec<CheckResult>,
        evidence_refs: Vec<String>,
    ) -> Self {
        Self {
            intent: intent.into(),
            tree_hash: tree_hash.into(),
            impact,
            contract_digests,
            check_results,
            evidence_refs,
        }
    }

    /// Project ONLY the served, non-author-controlled fields of an
    /// [`IntentSidecar`] (its bare `intent_id`) into ground truth. The
    /// author-controlled fields — `charter`, `acceptance`, `context_ref` — are
    /// not read. This is the single permitted touch-point with the sidecar, and
    /// it copies an identifier, never prose.
    pub fn intent_id_only(sidecar: &IntentSidecar) -> String {
        sidecar.intent_id.clone()
    }
}

/// A single review lens: an independent reviewer with its own prompt + model.
#[derive(Clone)]
pub struct Lens {
    /// Stable lens identifier (e.g. `"security"`, `"contracts"`).
    pub id: String,
    /// The full reviewer prompt text for this lens.
    pub prompt: String,
    /// The model identifier this lens is dispatched to.
    pub model: String,
    /// The reviewer strategy. Injected so the acceptance lane never makes a
    /// live model call — fixtures supply deterministic reviewers.
    pub reviewer: std::sync::Arc<dyn Reviewer>,
}

impl Lens {
    /// Construct a lens.
    pub fn new(
        id: impl Into<String>,
        prompt: impl Into<String>,
        model: impl Into<String>,
        reviewer: std::sync::Arc<dyn Reviewer>,
    ) -> Self {
        Self {
            id: id.into(),
            prompt: prompt.into(),
            model: model.into(),
            reviewer,
        }
    }

    /// SHA-256 hex digest of this lens's prompt.
    pub fn prompt_digest(&self) -> String {
        prompt_digest(&self.prompt)
    }
}

impl std::fmt::Debug for Lens {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Lens")
            .field("id", &self.id)
            .field("model", &self.model)
            .field("prompt_digest", &self.prompt_digest())
            .finish_non_exhaustive()
    }
}

/// The exact, complete input a reviewer receives. It can ONLY be built from
/// served ground truth (plus the lens's own prompt) — there is no author
/// channel into this struct.
#[derive(Debug, Clone, PartialEq)]
pub struct ReviewerInput {
    /// The lens prompt (reviewer-authored, not change-author-authored).
    pub prompt: String,
    /// The served ground truth — the only change-derived material.
    pub ground_truth: ServedGroundTruth,
}

impl ReviewerInput {
    /// The only constructor: served ground truth + a reviewer prompt.
    pub fn new(prompt: impl Into<String>, ground_truth: ServedGroundTruth) -> Self {
        Self {
            prompt: prompt.into(),
            ground_truth,
        }
    }
}

/// A reviewer strategy. Injected per lens; the acceptance lane uses fixture
/// reviewers, so no live model call ever happens in tests.
pub trait Reviewer: Send + Sync {
    /// Judge the served input and return `(verdict, claims_checked)`.
    fn review(&self, input: &ReviewerInput) -> (Verdict, Vec<String>);
}

/// A panel of independent lenses.
#[derive(Clone, Debug)]
pub struct Panel {
    /// The lenses dispatched in this panel.
    pub lenses: Vec<Lens>,
}

impl Panel {
    /// Construct a panel from lenses.
    pub fn new(lenses: Vec<Lens>) -> Self {
        Self { lenses }
    }
}

/// Dispatch errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelError {
    /// The panel had no lenses.
    EmptyPanel,
}

impl std::fmt::Display for PanelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PanelError::EmptyPanel => write!(f, "panel has no lenses"),
        }
    }
}

impl std::error::Error for PanelError {}

/// SHA-256 hex digest of a prompt string.
pub fn prompt_digest(prompt: &str) -> String {
    let mut h = Sha256::new();
    h.update(prompt.as_bytes());
    hex::encode(h.finalize())
}

/// Fan a change out to every lens against the SAME served ground truth and
/// collect a `VerdictObject` per lens.
///
/// Each reviewer sees ONLY a [`ReviewerInput`] built from `ground_truth` and
/// its own lens prompt. The reviewer never receives the `IntentSidecar`, so
/// author-controlled text is structurally outside the reviewer input set.
pub fn dispatch(
    panel: &Panel,
    ground_truth: &ServedGroundTruth,
) -> Result<Vec<VerdictObject>, PanelError> {
    if panel.lenses.is_empty() {
        return Err(PanelError::EmptyPanel);
    }

    let mut verdicts = Vec::with_capacity(panel.lenses.len());
    for lens in &panel.lenses {
        let input = ReviewerInput::new(lens.prompt.clone(), ground_truth.clone());
        let (verdict, claims_checked) = lens.reviewer.review(&input);
        verdicts.push(VerdictObject {
            intent: ground_truth.intent.clone(),
            tree_hash: ground_truth.tree_hash.clone(),
            lens: lens.id.clone(),
            model: lens.model.clone(),
            prompt_digest: lens.prompt_digest(),
            verdict,
            claims_checked,
            evidence_refs: ground_truth.evidence_refs.clone(),
        });
    }
    Ok(verdicts)
}
