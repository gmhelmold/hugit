//! Shared wire atoms reused across ≥2 `/v1` view-models (landing · checks ·
//! pr_detail). Frozen ONE place so the per-screen modules never define them twice
//! (the shared-type drift trap). Transcribed BYTE-FOR-FIELD from the canonical
//! companion web frontend's view-model definitions; derives are copied VERBATIM
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

/// The structured outcome class of a [`VerdictVm`], beside the localized
/// `verdict` prose — githugr styles from this discriminant instead of
/// substring-matching the prose. Three-state, mirroring the engine's
/// `Verdict::{Approve, FixFirst, Reject}` (`verdict_object.rs`):
/// - `Pass` — approve-shaped (APPROVE / PROVEN).
/// - `Warn` — fix-first-shaped (FIX-FIRST / changes-requested).
/// - `Fail` — reject-shaped (REJECT).
///
/// CONSERVATIVE DEFAULT: an unrecognized outcome maps to the safest NON-green
/// `Warn` (never `Pass` — an unknown verdict is never silently "green"). The
/// `#[default]` variant is `Pass` only so an ABSENT field on an old/forward-compat
/// payload round-trips to the value those payloads implied (no verdict carried =
/// the contract's neutral baseline); the live mapping fn [`decision_of`] never
/// returns `Pass` for an unrecognized string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum VerdictDecision {
    #[default]
    Pass,
    Warn,
    Fail,
}

/// Map an engine verdict outcome string to a structured [`VerdictDecision`].
///
/// The SINGLE shared mapping used by every VerdictVm build site so all surfaces
/// agree. Accepts every canonical spelling the engine emits across the 4 sites:
/// the uppercase porcelain form (`APPROVE` / `FIX-FIRST` / `REJECT`,
/// `review.rs`/`attention.rs`/`intent_detail.rs`) AND the `Debug`-of-enum form
/// the redacted `VerdictView.outcome` carries (`Approve` / `FixFirst` / `Reject`,
/// `landing.rs`/the ledger fallback). Also recognizes the `PROVEN` / `changes
/// requested` aliases that appear on the intent status surfaces. Matching is
/// case-insensitive and tolerant of `-`/`_`/space separators.
///
/// CONSERVATIVE: an UNRECOGNIZED outcome → `Warn` (the safest non-green), never
/// `Pass` — an unknown verdict is never silently treated as approved.
pub fn decision_of(outcome: &str) -> VerdictDecision {
    let norm: String = outcome
        .chars()
        .filter(|c| !matches!(c, '-' | '_' | ' '))
        .flat_map(char::to_lowercase)
        .collect();
    match norm.as_str() {
        "approve" | "approved" | "pass" | "passed" | "proven" => VerdictDecision::Pass,
        "fixfirst" | "warn" | "changesrequested" => VerdictDecision::Warn,
        "reject" | "rejected" | "fail" | "failed" => VerdictDecision::Fail,
        // Conservative: an unknown outcome is never "green".
        _ => VerdictDecision::Warn,
    }
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
    /// Structured outcome class beside the `verdict` prose (githugr styles from
    /// this discriminant). Derived from the SAME outcome via [`decision_of`].
    /// Additive + forward-compat: an old payload without it defaults to
    /// [`VerdictDecision::Pass`] (the neutral baseline). The prose `verdict` stays.
    #[serde(default)]
    pub decision: VerdictDecision,
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
    /// DEPRECATED (kept one release for back-compat) — the pre-divided dollar cost
    /// as an f64. Prefer [`cost_usd_micros`](Self::cost_usd_micros): the canonical
    /// integer-micro-USD money unit (ADR-0001), no float rounding. While both
    /// coexist, `usd == cost_usd_micros as f64 / 1_000_000.0`.
    pub usd: f64,
    /// The canonical money unit: integer micro-USD (ADR-0001) — the same
    /// `cost_usd_micros: u64` the rest of the contract uses. Additive + serde-default
    /// (forward-compat: an older payload without it deserializes to `0`).
    #[serde(default)]
    pub cost_usd_micros: u64,
    /// (model, tokens) pairs, largest first.
    pub model_breakdown: Vec<(String, u64)>,
    #[serde(default)]
    pub cache_savings: String,
}

// ── Phase-2 shared atoms (used by ≥2 of the 26 read VMs) ─────────────────────

/// Which sub-line variant a [`KpiVm`] card renders. Shared by insights + security.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum KpiSubKind {
    #[default]
    None,
    DeltaUp,
    DeltaDn,
    Plain,
    Streak,
}

/// A KPI card. Used by `InsightsVm.kpis` and `SecurityVm.posture`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct KpiVm {
    pub label: String,
    pub value: String,
    pub delta: Option<String>,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub sub_kind: KpiSubKind,
    #[serde(default)]
    pub sub_text: String,
    #[serde(default)]
    pub label_has_period: bool,
}

/// GitHub-App savings strip. Used by `DashboardVm` and `GithubAppVm`.
/// Contains an f64 (`saved_usd`) → derives `PartialEq` only, never `Eq`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GithubAppStripVm {
    pub saved_usd: f64,
    pub saved_ci_minutes: u32,
}

/// A repo row. Used by `DashboardVm.repos` and `OrgVm.repos`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardRepoVm {
    pub org: String,
    pub name: String,
    pub main_green: bool,
    pub status: String,
    pub stack: String,
    pub visibility: String,
    pub open_prs: u32,
    pub last_activity: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `decision_of` maps every canonical engine outcome spelling (both the
    /// uppercase porcelain form and the `Debug`-of-enum form) to the right class,
    /// and falls CONSERVATIVE (`Warn`, never `Pass`) for an unrecognized outcome.
    #[test]
    fn decision_of_maps_canonical_outcomes() {
        // approve-shaped → Pass
        for s in ["APPROVE", "Approve", "approved", "PROVEN", "proven", "pass"] {
            assert_eq!(decision_of(s), VerdictDecision::Pass, "{s} → Pass");
        }
        // fix-first / warn-shaped → Warn
        for s in [
            "FIX-FIRST",
            "FixFirst",
            "fix_first",
            "warn",
            "changes requested",
        ] {
            assert_eq!(decision_of(s), VerdictDecision::Warn, "{s} → Warn");
        }
        // reject / fail-shaped → Fail
        for s in ["REJECT", "Reject", "rejected", "fail"] {
            assert_eq!(decision_of(s), VerdictDecision::Fail, "{s} → Fail");
        }
        // Unrecognized → conservative Warn (NEVER Pass).
        assert_eq!(decision_of("???"), VerdictDecision::Warn);
        assert_eq!(decision_of(""), VerdictDecision::Warn);
    }

    /// `VerdictVm.decision` round-trips beside the prose AND forward-compat-
    /// defaults to `Pass` when absent from an old payload (the prose stays).
    #[test]
    fn verdict_vm_decision_round_trips_and_defaults() {
        let vm = VerdictVm {
            verdict: "REJECT".to_string(),
            reviewer: "opus-4.8".to_string(),
            summary: "blocked".to_string(),
            adversarial: true,
            lens: "correctness".to_string(),
            evidence_mono_terms: vec!["iat".to_string()],
            decision: decision_of("REJECT"),
        };
        assert_eq!(vm.decision, VerdictDecision::Fail);
        let reparsed: VerdictVm =
            serde_json::from_str(&serde_json::to_string(&vm).unwrap()).unwrap();
        assert_eq!(vm, reparsed, "VerdictVm round-trip is lossless");

        // Forward-compat: an old payload WITHOUT `decision` defaults to Pass.
        let legacy = r#"{
            "verdict": "APPROVE", "reviewer": "r", "summary": "s",
            "adversarial": true
        }"#;
        let parsed: VerdictVm = serde_json::from_str(legacy).unwrap();
        assert_eq!(
            parsed.decision,
            VerdictDecision::Pass,
            "absent decision defaults to Pass (forward-compat)"
        );
    }
}
