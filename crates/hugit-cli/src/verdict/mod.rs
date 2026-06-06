//! Adversarial verdict panels — `verdict request --lens` fan-out (WP-D7).
//!
//! A verdict panel fans a change out to INDEPENDENT reviewer lenses. Each lens
//! is a distinct reviewer that judges the change against **served ground truth
//! only** — build-graph impact, contracts, and `CheckResult` evidence. The
//! change *never defends itself*: no author-controlled text is ever placed in a
//! reviewer's input set, so there is no persuasion channel for the author to
//! exploit (the no-self-defense invariant, [`lens_audit`]).
//!
//! The panel is **diversity-enforced** ([`diversity`]): a homogeneous panel
//! (every lens sharing one prompt *and* one model) is rejected by construction;
//! a real panel dispatches distinct prompts AND ≥2 distinct models.
//!
//! Human review is grounded interrogation, not generative prose ([`qa`]): every
//! answer is a citation to a real evidence object, or an explicit refusal when
//! no grounding exists — it never fabricates.
//!
//! ## Acceptance lane note
//!
//! Everything here is **structural / fixture-driven**: lens isolation, the
//! planted-bug catch, diversity enforcement, Q&A grounding, and the
//! persuasion-channel negative are all asserted over local fixtures and pure
//! dispatch logic. There are **no live model API calls** in the verdict
//! acceptance lane — a [`Reviewer`] is an injected strategy, and the fixtures
//! ship deterministic reviewers.

pub mod diversity;
pub mod lens_audit;
pub mod panel_dispatch;
pub mod qa;

pub mod fixtures;

pub use diversity::{DiversityError, DiversityReport, enforce_diversity};
pub use lens_audit::{IsolationError, IsolationReport, audit_isolation};
pub use panel_dispatch::{
    Lens, Panel, PanelError, Reviewer, ReviewerInput, ServedGroundTruth, dispatch,
};
pub use qa::{Answer, EvidenceStore, QaError, answer_question};

pub use hugit_contracts::{IntentSidecar, Verdict, VerdictObject};
