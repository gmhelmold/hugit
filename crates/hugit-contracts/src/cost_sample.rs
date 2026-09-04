//! cost_sample — the FROZEN wire contract for per-dock cost metering (WP-DOCK-4).
//!
//! # Why this type exists
//!
//! The cost killer needs **real, per-unit, non-derived** cost: the gateway
//! (Omnirouter, the irmão) measures each model call at the point of spend and
//! emits a [`CostSampleV1`]; hugit spools it locally (offline-safe) and lands it
//! into the attestation chain, where `hugit why` already reads cost.
//!
//! # Freeze discipline (B5, mirroring the #220 runner-lease freeze)
//!
//! This DTO is frozen at version 1 and MUST be **byte-identical in both repos**
//! (hugit + Omnirouter). The conformance vector (`conformance/cost_sample_v1.
//! json` + the tripwire) re-verifies on CI that the two sides still agree — the
//! 3-wire-drift history ends here. A field addition is a NEW major version
//! (`CostSampleV2`), never a silent field on V1.
//!
//! # Honesty rule (verbatim from CLAUDE.md)
//!
//! Cost is MEASURED at the model call by the gateway — never re-derived from
//! internal metrics, never the derived `IntentMetrics` COGS. The gateway passes
//! the provider's billed figure; absent a gateway sample, cost is `None`/zero
//! (honest-zero), never an estimate.
//!
//! # Field semantics
//!
//! - `dock_id` — the dock (gitdir+branch hash) the call happened under; may be
//!   empty ⇒ the sample is `unlabeled` (visible residual bucket, never dropped).
//!   Resolution honors the cwd-wins rule (R5): the gateway stamps what it saw;
//!   hugit reconciles to the physical cwd dock on ingest.
//! - `model` — the provider model id (the exact token-name from the call).
//! - `input_tokens` / `output_tokens` — the raw token counts the gateway saw.
//! - `cost_usd_micros` — the provider's billed cost in integer micro-USD
//!   (1_000_000 = $1.00; the WA4 contract unit — no floats on the wire).
//! - `ts_ms` — the wall-clock of the model call (ms).
//! - `run_id` — the gateway run/request id (dedupe key: a repeated ingest of the
//!   same run_id is a no-op — exact-once M2).
//!
//! Computed fields (like a price-card-derived cost) are NEVER on the wire — the
//! gateway measures, it does not model.

use serde::{Deserialize, Serialize};

/// The frozen cost-sample wire type, version 1.
///
/// Byte-identical across hugit + Omnirouter (conformance-pinned). Derived as
/// verbose JSON for the vector, with a compact alternate for internal use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostSampleV1 {
    /// The dock (gitdir+branch hash) this model call ran under. Empty string
    /// ⇒ `unlabeled` (visible residual, never dropped).
    pub dock_id: String,
    /// The provider model id (the exact token-name from the call).
    pub model: String,
    /// Raw input tokens the gateway counted.
    pub input_tokens: u64,
    /// Raw output tokens the gateway counted.
    pub output_tokens: u64,
    /// The provider's BILLED cost, integer micro-USD (1_000_000 = $1.00).
    pub cost_usd_micros: u64,
    /// Wall-clock of the model call (ms since epoch).
    pub ts_ms: u64,
    /// The gateway's run/request id — the dedupe key (exact-once M2).
    pub run_id: String,
}

impl CostSampleV1 {
    /// A sample with no dock is the honest `unlabeled` signal (never dropped).
    #[must_use]
    pub fn is_unlabeled(&self) -> bool {
        self.dock_id.is_empty()
    }

    /// The dedupe key: a repeated ingest of the same run_id is a no-op.
    #[must_use]
    pub fn dedupe_key(&self) -> String {
        format!("{}:{}", self.run_id, self.ts_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_sample_serde_roundtrip_preserves_all_fields() {
        let s = CostSampleV1 {
            dock_id: "abc123def456".to_string(),
            model: "claude-opus-4-8".to_string(),
            input_tokens: 1_000,
            output_tokens: 500,
            cost_usd_micros: 4_200_000,
            ts_ms: 1_700_000_000_000,
            run_id: "run-xyz".to_string(),
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: CostSampleV1 = serde_json::from_str(&json).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn empty_dock_id_is_unlabeled() {
        let s = CostSampleV1 {
            dock_id: String::new(),
            model: "m".to_string(),
            input_tokens: 1,
            output_tokens: 1,
            cost_usd_micros: 1,
            ts_ms: 1,
            run_id: "r".to_string(),
        };
        assert!(s.is_unlabeled());
    }

    #[test]
    fn dedupe_key_is_run_plus_ts() {
        let s = CostSampleV1 {
            dock_id: "d".to_string(),
            model: "m".to_string(),
            input_tokens: 1,
            output_tokens: 1,
            cost_usd_micros: 1,
            ts_ms: 5,
            run_id: "run-7".to_string(),
        };
        assert_eq!(s.dedupe_key(), "run-7:5");
    }
}
