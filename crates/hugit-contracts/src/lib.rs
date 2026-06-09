//! hugit-contracts — frozen shared types for the hugit workspace.
//!
//! Every type in this crate is the single source of truth for a shared
//! contract surface. This crate has ZERO runtime logic beyond derive-generated
//! (de)serialization and schema generation.
//!
//! All 15 types were frozen by decomposition §1 (WP-00).

/// The canonical redaction sentinel — the single source of truth for the
/// `"[REDACTED]"` marker used by both the ledger/verdict redaction
/// (`hugit-ledger`) and the CLI export redaction (`hugit-cli`), so the two
/// cannot silently drift (audit remediation). Additive shared constant: it does
/// not touch any frozen type or its serialization.
pub const REDACTED_MARKER: &str = "[REDACTED]";

pub mod app_webhooks;
pub mod attention_rank;
pub mod attestation_chain;
pub mod check_def;
pub mod check_result;
pub mod diagnosis_object;
pub mod event_record;
pub mod export_schema;
pub mod fence_manifest;
pub mod intent_sidecar;
pub mod queue_api;
pub mod regen_gate;
pub mod runner_lease;
pub mod shadow_policy;
pub mod verdict_object;

pub use app_webhooks::{
    AckReceipt, AppWebhooks, ChecksWriteRequest, ChecksWriteResponse, SignedEventEnvelope,
};
pub use attention_rank::AttentionRank;
pub use attestation_chain::AttestationChain;
pub use check_def::CheckDef;
pub use check_result::CheckResult;
pub use diagnosis_object::DiagnosisObject;
pub use event_record::EventRecord;
pub use export_schema::ExportSchema;
pub use fence_manifest::FenceManifest;
pub use intent_sidecar::IntentSidecar;
pub use queue_api::{BatchSeal, LandableEntry, MinimalFailingPair, QueueApi, UnionResult};
pub use regen_gate::RegenGate;
pub use runner_lease::{RunnerLease, RunnerState};
pub use shadow_policy::ShadowPolicy;
pub use verdict_object::{Verdict, VerdictObject};
