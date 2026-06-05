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

    /// Detached signature over the concatenated refs (format: base64-encoded
    /// Ed25519 or ECDSA-P256 signature).
    pub sig: String,
}
