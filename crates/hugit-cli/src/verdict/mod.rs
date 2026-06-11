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

use std::path::PathBuf;
use std::process::ExitCode;

/// Event kind a recorded adversarial verdict is appended under.
///
/// **Producer (frozen at W0): `hugit verdict` — the W-VERDICT EXECUTE path**
/// ([`run`]). Additive over the D1 log, sibling to `check.recorded`/`pr.landed`.
/// `hugit campaign show` already READS this kind to surface a PR's `proven`
/// state (the verdict-recorded count), so naming it here freezes the wire string
/// the recorder targets; the campaign read and the recorder agree by this const.
pub const VERDICT_RECORDED_KIND: &str = "verdict.recorded";

/// `hugit verdict` flags — the adversarial-verdict EXECUTE path (W-VERDICT).
///
/// Fans a change out to independent reviewer lenses (the structural machinery in
/// this module) and — with `--store` — records the [`VerdictObject`] onto the
/// canonical `--log` as a [`VERDICT_RECORDED_KIND`] event. W0 freezes the verb +
/// flag seam; the panel-dispatch + recorder body lands at W-VERDICT.
#[derive(clap::Args, Debug)]
pub struct VerdictArgs {
    /// The intent / change id to convene the verdict panel over.
    #[arg(long)]
    pub intent: String,
    /// Path to the canonical JSON event log — the shared `--log` seam the
    /// recorded verdict is appended to and `campaign show` projects `proven` from.
    #[arg(long)]
    pub log: PathBuf,
    /// Record the [`VerdictObject`] onto the log as a `verdict.recorded` event
    /// (the recorder seam `campaign show` reads). Omit to convene without
    /// persisting (a dry panel).
    #[arg(long)]
    pub store: bool,
}

/// `hugit verdict` — convene an adversarial verdict panel (W-VERDICT EXECUTE).
///
/// W0 freezes the verb + flag seam (`--intent --log [--store]`) and dispatches an
/// honest NOT-IMPLEMENTED stub: the panel dispatch (over the diversity-enforced,
/// no-self-defense machinery in this module) and the `--store` recorder
/// (appending [`VERDICT_RECORDED_KIND`] onto the log) land at W-VERDICT. The stub
/// never returns a fake success — it emits the canonical
/// `{"error":{"kind":"not_implemented","wp":"W-VERDICT"}}` envelope on stdout,
/// exit 2.
pub fn run(_args: VerdictArgs) -> ExitCode {
    crate::porcelain::not_implemented("W-VERDICT")
}
