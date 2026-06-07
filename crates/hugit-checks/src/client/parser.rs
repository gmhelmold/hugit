//! CheckDef parsing + validation (client side).
//!
//! Checks are authored as JSON (checks-as-code). This module parses the authored
//! body into the frozen [`CheckDef`] contract type and validates it, including
//! recomputing the canonical `def_digest` so a check whose self-reported digest
//! disagrees with its body is rejected (a stale/forged digest must never reach
//! the memo-key path — that would be a false-hit vector).

use hugit_contracts::CheckDef;

use super::memo_key;

/// Why a CheckDef failed to parse or validate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// The input was not valid JSON for the CheckDef shape.
    Json(String),
    /// A required field was empty (e.g. an empty command can never execute).
    EmptyField(&'static str),
    /// The self-reported `def_digest` did not match the canonical digest of the
    /// body. The expected (canonical) digest is carried for diagnostics.
    DigestMismatch { expected: String, got: String },
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParseError::Json(e) => write!(f, "invalid CheckDef JSON: {e}"),
            ParseError::EmptyField(field) => {
                write!(f, "CheckDef field `{field}` must not be empty")
            }
            ParseError::DigestMismatch { expected, got } => write!(
                f,
                "CheckDef def_digest mismatch: body hashes to {expected}, def reports {got}"
            ),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse + validate an authored CheckDef from JSON, returning a normalized
/// [`CheckDef`] whose `def_digest` is the canonical digest of its body.
///
/// Validation, fail-closed:
///   - JSON must deserialize into the frozen shape (`deny_unknown_fields`).
///   - `command` and `toolchain_ref` must be non-empty (a check with neither
///     cannot be memoized honestly).
///   - if the authored `def_digest` is non-empty it MUST equal the canonical
///     digest — a mismatch is rejected, never silently corrected, so a forged
///     digest cannot smuggle a different body past the memo key.
pub fn parse_check_def(json: &str) -> Result<CheckDef, ParseError> {
    let def: CheckDef = serde_json::from_str(json).map_err(|e| ParseError::Json(e.to_string()))?;
    validate(def)
}

/// Validate an already-deserialized [`CheckDef`] (the in-memory path), applying
/// the same fail-closed rules as [`parse_check_def`].
pub fn validate(def: CheckDef) -> Result<CheckDef, ParseError> {
    if def.command.trim().is_empty() {
        return Err(ParseError::EmptyField("command"));
    }
    if def.toolchain_ref.trim().is_empty() {
        return Err(ParseError::EmptyField("toolchain_ref"));
    }

    let canonical = memo_key::compute_def_digest(&def);
    if !def.def_digest.is_empty() && def.def_digest != canonical {
        return Err(ParseError::DigestMismatch {
            expected: canonical,
            got: def.def_digest,
        });
    }

    Ok(memo_key::with_canonical_def_digest(def))
}
