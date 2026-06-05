//! ExportSchema — frozen by decomposition §1, item 13 (+).
//!
//! Versioned, machine-validatable export envelope (E5③/⑥).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Versioned, machine-validatable export envelope
/// (decomposition §1, item 13 (+); E5③/⑥).
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExportSchema {
    /// Schema / export format version string (semver).
    pub version: String,

    /// List of object class names included in this export.
    pub object_classes: Vec<String>,

    /// Content-addressed ref to the redaction manifest (describes which
    /// fields were redacted and under what policy).
    pub redaction_manifest: String,
}
