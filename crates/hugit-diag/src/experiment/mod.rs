//! The experiment harness + the experiment gate (WP-D8).
//!
//! This module auto-collects claim-disjointness + regen-honesty datapoints from
//! the fleet's real waves and BINDS the experiment gate: claims-as-oracle stays
//! advisory/OFF and regen promotion stays blocked until the report shows PASS.
//!
//! The gate is a **fail-CLOSED CONTROL, not a dashboard** (contract ⑥): a
//! degraded evaluator = "insufficient" = cannot promote. The corpus is
//! pre-sealed and focus-gate-eligible at ingestion, and the gate report is
//! itself an attested tamper-evident object.
//!
//! Module layout (one responsibility per file, all owned by WP-D8):
//!   - [`datapoint`] — the datapoint a wave contributes (①) + ingestion (⑦⑧).
//!   - [`dashboard`]  — disjointness %, regen agree/disagree, n (②).
//!   - [`corpus`]     — pre-registered, SEALED corpus + post-hoc detection (④⑤).
//!   - [`report`]     — generated-not-handwritten report + X2 attestation (③⑨).
//!   - [`gate`]       — the binding control surface (⑥⑧⑨).
//!   - [`audit`]      — the audited `EventRecord` emitter for every gate event.

pub mod audit;
pub mod corpus;
pub mod dashboard;
pub mod datapoint;
pub mod gate;
pub mod report;

pub use audit::{ExperimentEvent, emit_event};
pub use corpus::{Corpus, CorpusError, SealedCorpus};
pub use dashboard::Dashboard;
pub use datapoint::{Datapoint, IngestError, RegenOutcome, SourceOrigin, WaveContribution, ingest};
pub use gate::{
    EvaluatorHealth, ExperimentGate, FeatureState, GateError, Promotion, PromotionAttempt, Verdict,
};
pub use report::{GateReport, MIN_SAMPLE_N, ReportError, ReportVerdict};

// Re-export the consumed frozen contract type the public API surfaces, so
// callers (and the acceptance oracle) reference one canonical `EventRecord`.
pub use hugit_contracts::EventRecord;
