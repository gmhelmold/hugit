//! §13.1 runner per-job metrics DTO ↔ frozen `IntentMetrics` mapping (WP-#2-PR2).
//!
//! PR1 ([`crate::runner::lease_client`]) returned the result-envelope META as a
//! raw, un-interpreted [`RawMeta`](crate::runner::lease_client::RawMeta). This
//! module is PR2: the TYPED §13.1 metrics DTO and its lossless mapping into the
//! frozen [`IntentMetrics`] type, plus a typed poll on the lease client.
//!
//! ## What is transcribed (NOT invented)
//!
//! The DTO mirrors the **§13.1 per-job metrics table** of the frozen
//! `corelink-runners` integration contract (v1.2.0,
//! `docs/spec/hugit-integration-contract.md` §13.1): the mandatory token
//! cache-split (`input` · `output` · `cache_read` · `cache_write` · `total`)
//! plus `wall_ms` · `active_ms` · `tool_calls` · `tool_breakdown` ·
//! `model_turns` · `cost_usd_micros`. Field names, JSON types, and order match
//! the §13.4 drift tripwire vector `conformance/IntentMetrics.json`
//! (sha256 `2d8d2215…`), committed byte-identical in both repos.
//!
//! ## Two deliberate serde decisions (per the contract)
//!
//! 1. **`deny_unknown_fields` is OFF.** §13.1: "the runner MAY carry additional
//!    runner-specific fields alongside them (e.g. `cpu_ms`, `mem_peak_mb`);
//!    hugit's forge will ignore fields outside the `IntentMetrics` vocabulary."
//!    So an extra field must be silently dropped, NOT an error.
//! 2. **The token cache-split is preserved in FULL** — never flattened to
//!    `total`. §13.1: "The cache split is mandatory … without the
//!    `cache_read`/`cache_write` split the memoization economics are not
//!    computable." The DTO and the mapping carry all five token fields across.

use hugit_contracts::IntentMetrics;
use hugit_contracts::context_envelope::{TokenCounts, ToolCount};
use serde::{Deserialize, Serialize};

use crate::runner::lease_client::{LeaseClient, RawMeta, RunnerError, RunnerTransport};

// ─────────────────────────────────────────────────────────────────────────────
// The §13.1 DTO — a strict superset-compatible projection of `IntentMetrics`.
// `deny_unknown_fields` is intentionally OFF (the runner MAY add cpu_ms etc.).
// ─────────────────────────────────────────────────────────────────────────────

/// §13.1 token spend with the MANDATORY cache split (mirrors
/// [`hugit_contracts::context_envelope::TokenCounts`]). Every field is `u64` per
/// the §13.1 table; the split is never flattened to `total`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerTokenCounts {
    /// `tokens.input` — non-cached input tokens consumed by the job's model calls.
    pub input: u64,
    /// `tokens.output` — output tokens.
    pub output: u64,
    /// `tokens.cache_read` — tokens read from prompt cache.
    pub cache_read: u64,
    /// `tokens.cache_write` — tokens written to prompt cache.
    pub cache_write: u64,
    /// `tokens.total` — derived total.
    pub total: u64,
}

/// §13.1 per-tool breakdown entry (same shape as
/// [`hugit_contracts::context_envelope::ToolCount`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerToolCount {
    /// Tool name (e.g. `"Bash"`, `"Edit"`).
    pub tool: String,
    /// Number of calls to this tool.
    pub count: u64,
}

/// The §13.1 per-job metrics payload reported by the runner at job close.
///
/// A **strict superset-compatible projection** of [`IntentMetrics`]: every named
/// §13.1 field uses the same name, JSON type, and semantic. `deny_unknown_fields`
/// is OFF on purpose so runner-specific extras (`cpu_ms`, `mem_peak_mb`, …) are
/// ignored — present-but-unmodelled is NOT a decode error (§13.1). Missing or
/// wrong-typed fields in the modelled vocabulary remain a contract violation
/// (serde rejects them).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunnerJobMetrics {
    /// Token spend with the cache split.
    pub tokens: RunnerTokenCounts,
    /// Job born → job dead wall-clock, ms.
    pub wall_ms: u64,
    /// Model + tool busy time, ms (excludes idle / queue wait).
    pub active_ms: u64,
    /// Total tool-call count.
    pub tool_calls: u64,
    /// Per-tool breakdown.
    pub tool_breakdown: Vec<RunnerToolCount>,
    /// Number of model turns.
    pub model_turns: u64,
    /// Derived COGS in integer micro-USD (`1 USD = 1_000_000`); NEVER a billable
    /// meter — for trust/audit, exact-integer, no f64 epsilon.
    pub cost_usd_micros: u64,
}

// ─────────────────────────────────────────────────────────────────────────────
// The mapping into the frozen `IntentMetrics`. Every field crosses; the cache
// split is preserved in full.
// ─────────────────────────────────────────────────────────────────────────────

impl From<RunnerTokenCounts> for TokenCounts {
    fn from(t: RunnerTokenCounts) -> Self {
        TokenCounts {
            input: t.input,
            output: t.output,
            cache_read: t.cache_read,
            cache_write: t.cache_write,
            total: t.total,
        }
    }
}

impl From<RunnerToolCount> for ToolCount {
    fn from(t: RunnerToolCount) -> Self {
        ToolCount {
            tool: t.tool,
            count: t.count,
        }
    }
}

impl From<RunnerJobMetrics> for IntentMetrics {
    fn from(m: RunnerJobMetrics) -> Self {
        IntentMetrics {
            tokens: m.tokens.into(),
            wall_ms: m.wall_ms,
            active_ms: m.active_ms,
            tool_calls: m.tool_calls,
            tool_breakdown: m.tool_breakdown.into_iter().map(ToolCount::from).collect(),
            model_turns: m.model_turns,
            cost_usd_micros: m.cost_usd_micros,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The REVERSE mapping (`IntentMetrics` → `RunnerJobMetrics`). Field-identical +
// lossless (same names/types/order), so the round-trip preserves EVERY byte the
// fabric's `intent_metrics_sig` pre-image covers. Needed so the off-box
// cost-attestation verdict can reconstruct the exact `RunnerJobMetrics` the
// fabric signed from a [`crate::runner::dispatch::DispatchOutcome`] that carries
// the mapped [`IntentMetrics`]. Never lossy: no field is dropped or defaulted.
// ─────────────────────────────────────────────────────────────────────────────

impl From<TokenCounts> for RunnerTokenCounts {
    fn from(t: TokenCounts) -> Self {
        RunnerTokenCounts {
            input: t.input,
            output: t.output,
            cache_read: t.cache_read,
            cache_write: t.cache_write,
            total: t.total,
        }
    }
}

impl From<ToolCount> for RunnerToolCount {
    fn from(t: ToolCount) -> Self {
        RunnerToolCount {
            tool: t.tool,
            count: t.count,
        }
    }
}

impl From<IntentMetrics> for RunnerJobMetrics {
    fn from(m: IntentMetrics) -> Self {
        RunnerJobMetrics {
            tokens: m.tokens.into(),
            wall_ms: m.wall_ms,
            active_ms: m.active_ms,
            tool_calls: m.tool_calls,
            tool_breakdown: m
                .tool_breakdown
                .into_iter()
                .map(RunnerToolCount::from)
                .collect(),
            model_turns: m.model_turns,
            cost_usd_micros: m.cost_usd_micros,
        }
    }
}

impl RunnerJobMetrics {
    /// Parse a raw result-envelope META (as returned by
    /// [`LeaseClient::poll_meta`]) into the typed §13.1 DTO. Unknown
    /// runner-specific fields are ignored; missing/wrong-typed modelled fields
    /// are a [`RunnerError::Decode`].
    pub fn from_raw_meta(raw: &RawMeta) -> Result<Self, RunnerError> {
        serde_json::from_value(raw.0.clone()).map_err(|e| RunnerError::Decode(e.to_string()))
    }

    /// Map straight to the frozen [`IntentMetrics`].
    pub fn into_intent_metrics(self) -> IntentMetrics {
        self.into()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Typed poll on the lease client — parse RawMeta → §13.1 DTO → IntentMetrics.
// ─────────────────────────────────────────────────────────────────────────────

impl<T: RunnerTransport> LeaseClient<T> {
    /// Typed counterpart of [`LeaseClient::poll_meta`]: poll the result-envelope
    /// META and return the frozen [`IntentMetrics`]. Internally this is
    /// `poll_meta` → [`RunnerJobMetrics::from_raw_meta`] → [`IntentMetrics`], so
    /// the runner's full token cache-split and `cost_usd_micros` survive intact.
    /// A modelled field that is missing or wrong-typed surfaces as
    /// [`RunnerError::Decode`]; runner-specific extras are ignored.
    pub fn poll_metrics(&self, lease_id: &str) -> Result<IntentMetrics, RunnerError> {
        let raw = self.poll_meta(lease_id)?;
        Ok(RunnerJobMetrics::from_raw_meta(&raw)?.into_intent_metrics())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Hermetic tests — the §13.4 drift vector, ZERO network.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// The pinned §13.4 conformance vector (sha256 `2d8d2215…`), committed
    /// byte-identical in both repos. Read at compile time so the test cannot
    /// drift from the file on disk; we MUST NOT modify it.
    const VECTOR: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../conformance/IntentMetrics.json"
    ));

    /// The §13.1 DTO deserializes the pinned vector and maps → `IntentMetrics`
    /// with EVERY field surviving — especially the full token cache-split and
    /// `cost_usd_micros`.
    #[test]
    fn vector_maps_into_intent_metrics_field_for_field() {
        let dto: RunnerJobMetrics = serde_json::from_str(VECTOR).unwrap();

        // The DTO captured the literal vector values (transcribe-check).
        assert_eq!(dto.tokens.input, 48211);
        assert_eq!(dto.tokens.output, 9143);
        assert_eq!(dto.tokens.cache_read, 120557);
        assert_eq!(dto.tokens.cache_write, 3361);
        assert_eq!(dto.tokens.total, 181272);
        assert_eq!(dto.wall_ms, 754000);
        assert_eq!(dto.active_ms, 612450);
        assert_eq!(dto.tool_calls, 41);
        assert_eq!(dto.model_turns, 58);
        assert_eq!(dto.cost_usd_micros, 1834290);
        assert_eq!(dto.tool_breakdown.len(), 3);
        assert_eq!(dto.tool_breakdown[0].tool, "Bash");
        assert_eq!(dto.tool_breakdown[0].count, 17);
        assert_eq!(dto.tool_breakdown[1].tool, "Edit");
        assert_eq!(dto.tool_breakdown[1].count, 13);
        assert_eq!(dto.tool_breakdown[2].tool, "Read");
        assert_eq!(dto.tool_breakdown[2].count, 11);

        // The mapping crosses every field into the frozen type — the cache split
        // (memo-economics signal) and cost_usd_micros must be bit-exact.
        let im: IntentMetrics = dto.into();
        assert_eq!(im.tokens.input, 48211);
        assert_eq!(im.tokens.output, 9143);
        assert_eq!(im.tokens.cache_read, 120557);
        assert_eq!(im.tokens.cache_write, 3361);
        assert_eq!(im.tokens.total, 181272);
        assert_eq!(im.wall_ms, 754000);
        assert_eq!(im.active_ms, 612450);
        assert_eq!(im.tool_calls, 41);
        assert_eq!(im.model_turns, 58);
        assert_eq!(im.cost_usd_micros, 1834290);
        assert_eq!(im.tool_breakdown.len(), 3);
        assert_eq!(im.tool_breakdown[0].tool, "Bash");
        assert_eq!(im.tool_breakdown[0].count, 17);
        assert_eq!(im.tool_breakdown[1].tool, "Edit");
        assert_eq!(im.tool_breakdown[1].count, 13);
        assert_eq!(im.tool_breakdown[2].tool, "Read");
        assert_eq!(im.tool_breakdown[2].count, 11);
    }

    /// The mapped `IntentMetrics` equals the SAME vector decoded directly into the
    /// frozen type — proves the projection is lossless against the frozen schema.
    #[test]
    fn mapped_equals_direct_intent_metrics_decode() {
        let via_dto: IntentMetrics = serde_json::from_str::<RunnerJobMetrics>(VECTOR)
            .unwrap()
            .into();
        let direct: IntentMetrics = serde_json::from_str(VECTOR).unwrap();
        assert_eq!(via_dto, direct);
    }

    /// §13.1: an extra runner-specific field (`cpu_ms`, `mem_peak_mb`) MUST be
    /// ignored, never a decode error.
    #[test]
    fn unknown_runner_fields_are_ignored_not_an_error() {
        let with_extra = r#"{
            "tokens": {"input": 1, "output": 2, "cache_read": 3, "cache_write": 4, "total": 10},
            "wall_ms": 100,
            "active_ms": 90,
            "tool_calls": 2,
            "tool_breakdown": [{"tool": "Bash", "count": 2}],
            "model_turns": 3,
            "cost_usd_micros": 500,
            "cpu_ms": 4242,
            "mem_peak_mb": 1536
        }"#;
        let dto: RunnerJobMetrics =
            serde_json::from_str(with_extra).expect("extra fields must be ignored, not an error");
        let im: IntentMetrics = dto.into();
        assert_eq!(im.cost_usd_micros, 500);
        assert_eq!(im.tokens.total, 10);
        assert_eq!(im.tokens.cache_read, 3);
    }

    /// A missing MODELLED field IS a contract violation — serde rejects it (the
    /// other side of "unknown extras are fine").
    #[test]
    fn missing_modelled_field_is_a_decode_error() {
        // `cost_usd_micros` omitted.
        let missing = r#"{
            "tokens": {"input": 1, "output": 2, "cache_read": 3, "cache_write": 4, "total": 10},
            "wall_ms": 100,
            "active_ms": 90,
            "tool_calls": 2,
            "tool_breakdown": [],
            "model_turns": 3
        }"#;
        assert!(serde_json::from_str::<RunnerJobMetrics>(missing).is_err());
    }

    /// The DTO is also `Serialize`: it round-trips the pinned vector BYTE-EXACTLY
    /// (pretty-printed, 2-space, trailing newline — the on-disk format). This is
    /// the §13.4 tripwire: any field rename/reorder/type drift breaks the bytes.
    #[test]
    fn dto_round_trips_vector_byte_exactly() {
        let dto: RunnerJobMetrics = serde_json::from_str(VECTOR).unwrap();
        let mut rendered = serde_json::to_string_pretty(&dto).unwrap();
        rendered.push('\n'); // the file ends with a trailing newline
        assert_eq!(
            rendered, VECTOR,
            "DTO re-serialization drifted from the pinned vector"
        );
    }

    /// `from_raw_meta` (the RawMeta→DTO bridge poll_metrics uses) parses the
    /// vector and yields the same mapped metrics.
    #[test]
    fn from_raw_meta_parses_vector() {
        let value: serde_json::Value = serde_json::from_str(VECTOR).unwrap();
        let raw = RawMeta(value);
        let im = RunnerJobMetrics::from_raw_meta(&raw)
            .unwrap()
            .into_intent_metrics();
        assert_eq!(im.cost_usd_micros, 1834290);
        assert_eq!(im.tokens.cache_write, 3361);
    }
}
