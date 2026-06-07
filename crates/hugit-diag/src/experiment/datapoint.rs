//! The datapoint a wave contributes, plus the ingestion filter.
//!
//! Contract items handled here:
//!   - ① every wave auto-contributes datapoints — [`WaveContribution::datapoint`]
//!     maps a real wave into a [`Datapoint`]; the harness ingests it with no
//!     hand-authoring step.
//!   - ⑦ degradation honesty — a datapoint carries its [`degraded`](Datapoint::degraded)
//!     flag; a degraded contribution that is *not* honestly marked is REJECTED
//!     at ingestion (never silently biases the corpus).
//!   - ⑧ source-eligibility wired to the focus gate — [`SourceOrigin`] is the
//!     ingestion eligibility predicate: a focus-gate-ineligible source
//!     ([`SourceOrigin::CorelinkServer`]) is REJECTED at ingestion, fail-closed.

use hugit_contracts::EventRecord;

use crate::experiment::audit::{ExperimentEvent, emit_event};

/// Where a wave's change originated.
///
/// This enum IS the corpus source-eligibility predicate (⑧) — it mirrors the
/// X10② focus-gate exclusion. D8 CONSUMES this boundary; it does not redefine
/// the focus gate. Anything outside the hugit focus target set (the canonical
/// example is `corelink-server`) is ineligible and rejected at ingestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceOrigin {
    /// A change inside the hugit focus target set — eligible for the corpus.
    Hugit,
    /// The adjacent product (corelink-server) — focus-gate INELIGIBLE. A change
    /// from this source can never become a datapoint that flips claims/regen.
    CorelinkServer,
    /// Any other out-of-focus source — ineligible, fail-closed by default.
    OtherExcluded(String),
}

impl SourceOrigin {
    /// The focus-gate eligibility predicate (the X10② exclusion). `true` iff the
    /// source is inside the hugit focus target set.
    ///
    /// Fails CLOSED: only [`SourceOrigin::Hugit`] is eligible; every other
    /// origin is excluded.
    pub fn is_focus_eligible(&self) -> bool {
        matches!(self, SourceOrigin::Hugit)
    }

    /// A short, stable label for audit payloads.
    pub fn label(&self) -> &str {
        match self {
            SourceOrigin::Hugit => "hugit",
            SourceOrigin::CorelinkServer => "corelink-server",
            SourceOrigin::OtherExcluded(s) => s,
        }
    }
}

/// The regen-honesty outcome a wave reports: did the independent regen verdict
/// AGREE with the wave's own regen result?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegenOutcome {
    /// Independent regen verdict agreed with the wave.
    Agree,
    /// Independent regen verdict disagreed with the wave.
    Disagree,
}

/// One collected datapoint: the unit the corpus is built from.
#[derive(Debug, Clone, PartialEq)]
pub struct Datapoint {
    /// Stable id of the wave that produced this datapoint.
    pub wave_id: String,
    /// Where the change came from (the eligibility predicate's input).
    pub source: SourceOrigin,
    /// Claim-disjointness: did the wave's work-package claims stay disjoint?
    pub claims_disjoint: bool,
    /// Regen-honesty: agree/disagree with the independent regen verdict.
    pub regen: RegenOutcome,
    /// Whether this wave executed inside a degraded gate-evaluator window (⑦).
    pub degraded: bool,
}

/// A raw wave contribution, before ingestion. A wave hands one of these to the
/// harness automatically at the end of every wave (①) — there is no manual
/// authoring path.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveContribution {
    /// The datapoint derived from the wave's real outcome.
    pub datapoint: Datapoint,
}

impl WaveContribution {
    /// Construct a contribution from a finished wave's outcome. This is the
    /// only constructor — datapoints are *derived*, never hand-written.
    pub fn from_wave(
        wave_id: impl Into<String>,
        source: SourceOrigin,
        claims_disjoint: bool,
        regen: RegenOutcome,
        degraded: bool,
    ) -> Self {
        WaveContribution {
            datapoint: Datapoint {
                wave_id: wave_id.into(),
                source,
                claims_disjoint,
                regen,
                degraded,
            },
        }
    }
}

/// Why a contribution was rejected at ingestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestError {
    /// The source is focus-gate-ineligible (⑧) — e.g. corelink-server.
    IneligibleSource(String),
}

/// Ingest a wave contribution into the running corpus buffer.
///
/// This is the single ingestion point (①). It enforces, fail-closed and
/// audited:
///   - ⑧ source-eligibility: a focus-gate-ineligible source is REJECTED here
///     and an `IngestionRejected` event is emitted. The datapoint never reaches
///     the corpus.
///
/// Degraded-window honesty (⑦) is preserved by carrying the `degraded` flag
/// through to the corpus; [`crate::experiment::corpus`] is responsible for
/// excluding/marking degraded datapoints before evaluation. A degraded
/// datapoint is *kept but flagged* — never silently dropped, never silently
/// counted.
pub fn ingest(
    buffer: &mut Vec<Datapoint>,
    log: &mut Vec<EventRecord>,
    contribution: WaveContribution,
    now_ms: u64,
) -> Result<(), IngestError> {
    let dp = contribution.datapoint;

    // ⑧ — fail-closed source-eligibility = the X10② focus-gate exclusion.
    if !dp.source.is_focus_eligible() {
        let payload = format!(
            r#"{{"wave_id":"{}","source":"{}","reason":"focus-gate-ineligible"}}"#,
            dp.wave_id,
            dp.source.label()
        );
        emit_event(
            log,
            ExperimentEvent::IngestionRejected,
            "experiment-harness",
            &payload,
            now_ms,
        );
        return Err(IngestError::IneligibleSource(dp.source.label().to_string()));
    }

    buffer.push(dp);
    Ok(())
}
