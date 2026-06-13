//! Shared wire atoms reused across ≥2 `/v1` view-models (landing · checks ·
//! pr_detail). Frozen ONE place so the per-screen modules never define them twice
//! (the shared-type drift trap). Transcribed BYTE-FOR-FIELD from the canonical
//! `../githugr/crates/githugr-vm/src/provider.rs`; derives are copied VERBATIM
//! (note: f64-bearing atoms like `CostVm` derive `PartialEq` only, never `Eq` —
//! match exactly).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiffLineKind {
    Context,
    Add,
    Del,
    KeyAdd,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffLineVm {
    pub kind: DiffLineKind,
    pub text: String,
    #[serde(default)]
    pub ln: String,
}

/// One hunk of a rendered diff (`header` is the `@@ …` line).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HunkVm {
    pub file: String,
    pub header: String,
    pub lines: Vec<DiffLineVm>,
}

/// One file row in a diff / file list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileRowVm {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffVm {
    pub files: Vec<FileRowVm>,
    pub hunks: Vec<HunkVm>,
}

/// A review verdict as displayed (APPROVE / FIX-FIRST / REJECT shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictVm {
    pub verdict: String,
    pub reviewer: String,
    pub summary: String,
    /// True when this verdict came from an adversarial / independent panel.
    pub adversarial: bool,
    #[serde(default)]
    pub lens: String,
    #[serde(default)]
    pub evidence_mono_terms: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnionVm {
    /// PR numbers tested together in this union batch (display order).
    pub batch: Vec<String>,
    pub verdict: String,
    pub green: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MirrorVm {
    pub synced: bool,
    pub detail: String,
}

/// A campaign chip: stable id + human label + the kit color class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CampaignChipVm {
    pub id: String,
    pub label: String,
    pub color_class: String,
    #[serde(default)]
    pub display_label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRowVm {
    pub name: String,
    pub ok: bool,
    pub duration_ms: u64,
    /// True = served from the AC (zero execution); false = executed.
    pub cache_hit: bool,
    pub log: String,
    pub memo_key: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub cost: String,
}

/// The per-intent envelope refs surfaced in the landing/PR drawer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentEnvelopeRefVm {
    pub context_cas: String,
    #[serde(default)]
    pub context_size: String,
    pub compact_context_ref: String,
    pub bundle_ref: String,
    pub compact_transcript_ref: String,
}

/// One intent inside the PR drawer (collapsible body). `PartialEq` only —
/// matches the canonical source (no `Eq`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentSummaryVm {
    pub id: String,
    pub title: String,
    pub status: String,
    pub charter: String,
    /// Pretty-printed `context.json` snapshot for the collapsible block.
    pub context_json: String,
    pub diff: DiffVm,
    pub verdicts: Vec<VerdictVm>,
    pub model: Option<String>,
    pub envelope: Option<IntentEnvelopeRefVm>,
    #[serde(default)]
    pub blame_quote: Option<String>,
    #[serde(default)]
    pub blame_proof: Option<String>,
}

/// The session envelope (ADR-0001) — shared by the PR and the campaign.
/// `Default` is an honest all-empty envelope (no faked refs) for the no-envelope
/// read path. All fields are `String`/`bool`/`Vec<String>` so the derive is total.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct EnvelopeVm {
    pub session: String,
    pub model: String,
    pub window: String,
    pub headline: String,
    pub transcript_complete: bool,
    pub session_summary: String,
    pub compact_transcript_ref: String,
    pub raw_transcript_ref: String,
    pub snapshot_files: Vec<String>,
    pub context_json: String,
    pub context_cas: String,
    pub compact_context_ref: String,
    pub compact_json_note: String,
    pub bundle_note: String,
}

/// Decomposed cost. `PartialEq` only (contains `f64`) — matches the source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CostVm {
    pub tokens_total: u64,
    pub usd: f64,
    /// (model, tokens) pairs, largest first.
    pub model_breakdown: Vec<(String, u64)>,
    #[serde(default)]
    pub cache_savings: String,
}
