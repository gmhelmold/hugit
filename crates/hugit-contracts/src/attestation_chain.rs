//! AttestationChain — frozen by decomposition §1, item 14 (+).
//!
//! Full provenance chain (whitepaper §9 provenance).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Full provenance attestation chain (decomposition §1, item 14 (+);
/// whitepaper §9).
///
/// Each field is a content-addressed ref or signature blob representing one
/// link in the provenance chain.
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttestationChain {
    /// Content-addressed ref to the workspace tree snapshot.
    pub tree: String,

    /// Content-addressed ref / digest of the CheckDef used.
    pub def: String,

    /// Content-addressed ref identifying the runner that executed the check.
    pub runner: String,

    /// Model identifier used in any AI-driven step of the pipeline.
    pub model: String,

    /// Ordered chain of principals who triggered or approved this chain.
    pub principal: Vec<String>,

    /// Detached signature over the canonical pre-image below (base64-encoded
    /// Ed25519 signature).
    ///
    /// # FROZEN signature pre-image (BYTE-EXACT, single-sourced)
    ///
    /// The exact bytes signed/verified are built only by
    /// `hugit_refstore::attestation_sig_preimage` — import and call it, never
    /// re-transcribe.
    ///
    /// ```text
    /// preimage = LP(tree) ‖ LP(def) ‖ LP(runner) ‖ LP(model) ‖ VEC(principal)
    /// ```
    ///
    /// where `LP(s)` = `u32_be(byte_len(s)) ‖ utf8_bytes(s)` and
    /// `VEC(v)` = `u32_be(elem_count(v)) ‖ LP(v[0]) ‖ LP(v[1]) ‖ …` (the same
    /// vector framing as `EventRecord::principal_chain`). Fields appear in struct
    /// order: `tree`, `def`, `runner`, `model`, `principal`. The result is the
    /// raw ed25519 message — signed/verified directly, with no extra hashing.
    pub sig: String,
}
