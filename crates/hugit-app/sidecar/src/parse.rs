//! Parse and validate raw JSON into an `IntentSidecar`.
//!
//! ① parsed/validated/rendered — parse the PR-attached sidecar into the
//! `IntentSidecar` type; validate against its schema.
//!
//! ② malformed → actionable comment — a malformed sidecar produces an
//! actionable PR comment naming the specific validation_failure (which
//! field, what's expected) — never a silent drop, never a hard error
//! that blocks the PR.

use hugit_contracts::IntentSidecar;
use thiserror::Error;

/// A named field validation failure for actionable error reporting (②).
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationFailure {
    /// The field that failed validation.
    pub field: String,
    /// Human-readable description of what was expected.
    pub expected: String,
    /// The actual value or description of what was found.
    pub found: String,
}

impl std::fmt::Display for ValidationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "field `{}`: expected {}, found {}",
            self.field, self.expected, self.found
        )
    }
}

/// Errors returned by the sidecar parser.
#[derive(Debug, Error)]
pub enum ParseError {
    /// The raw input is not valid JSON.
    #[error("invalid JSON: {0}")]
    InvalidJson(#[from] serde_json::Error),

    /// The sidecar JSON is valid but fails schema validation.
    /// Contains a list of actionable validation failures (② — never a
    /// silent drop; always names the field and expectation).
    #[error("invalid_sidecar: {failures:?}")]
    InvalidSidecar { failures: Vec<ValidationFailure> },
}

impl ParseError {
    /// Returns true if this is a malformed / invalid-sidecar error.
    pub fn is_malformed(&self) -> bool {
        matches!(self, ParseError::InvalidSidecar { .. })
    }

    /// Render an actionable PR comment describing the validation failure(s).
    /// This is the ② "malformed → actionable comment" output: never silent,
    /// never a hard error that blocks the PR.
    pub fn actionable_comment(&self) -> String {
        match self {
            ParseError::InvalidJson(e) => {
                format!(
                    "**hugit intent sidecar** — malformed input\n\n\
                     The sidecar attached to this PR is not valid JSON.\n\n\
                     ```\n{e}\n```\n\n\
                     Please attach a valid `hugit-intent` sidecar. \
                     This notice is informational only and does not block landing."
                )
            }
            ParseError::InvalidSidecar { failures } => {
                let mut lines = vec![
                    "**hugit intent sidecar** — validation_failure(s) detected\n".to_string(),
                    "The sidecar attached to this PR failed schema validation:".to_string(),
                    "".to_string(),
                ];
                for vf in failures {
                    lines.push(format!(
                        "- `{}`: expected `{}`, found `{}`",
                        vf.field, vf.expected, vf.found
                    ));
                }
                lines.push("".to_string());
                lines.push(
                    "This notice is informational only and does **not** block or gate landing."
                        .to_string(),
                );
                lines.join("\n")
            }
        }
    }
}

/// Parse and validate a raw JSON string into an `IntentSidecar`.
///
/// On success, returns the validated `IntentSidecar`.
/// On failure, returns a `ParseError` with actionable detail (② — the
/// caller should render `ParseError::actionable_comment()` as a PR comment,
/// never silently drop or hard-fail the PR).
pub fn parse_sidecar(raw_json: &str) -> Result<IntentSidecar, ParseError> {
    let sidecar: IntentSidecar = serde_json::from_str(raw_json)?;
    let failures = validate_sidecar(&sidecar);
    if failures.is_empty() {
        Ok(sidecar)
    } else {
        Err(ParseError::InvalidSidecar { failures })
    }
}

/// Validate a parsed `IntentSidecar` against its schema constraints.
///
/// Returns a (possibly empty) list of `ValidationFailure`s.
/// An empty list means the sidecar is valid.
pub fn validate_sidecar(sidecar: &IntentSidecar) -> Vec<ValidationFailure> {
    let mut failures = Vec::new();

    // intent_id must be non-empty.
    if sidecar.intent_id.trim().is_empty() {
        failures.push(ValidationFailure {
            field: "intent_id".to_string(),
            expected: "non-empty UUID or content hash".to_string(),
            found: "empty string".to_string(),
        });
    }

    // charter must be non-empty.
    if sidecar.charter.trim().is_empty() {
        failures.push(ValidationFailure {
            field: "charter".to_string(),
            expected: "non-empty human-readable description".to_string(),
            found: "empty string".to_string(),
        });
    }

    // acceptance must have at least one item.
    if sidecar.acceptance.is_empty() {
        failures.push(ValidationFailure {
            field: "acceptance".to_string(),
            expected: "at least one acceptance criterion".to_string(),
            found: "empty array".to_string(),
        });
    }

    // context_ref must be non-empty.
    if sidecar.context_ref.trim().is_empty() {
        failures.push(ValidationFailure {
            field: "context_ref".to_string(),
            expected: "non-empty content-addressed ref (e.g. cas/sha256/…)".to_string(),
            found: "empty string".to_string(),
        });
    }

    // authoritative MUST be false — the sidecar is non-authoritative by design
    // (B6④). A sidecar claiming authoritative=true is a validation failure.
    if sidecar.authoritative {
        failures.push(ValidationFailure {
            field: "authoritative".to_string(),
            expected: "false (sidecar is non-authoritative by design)".to_string(),
            found: "true".to_string(),
        });
    }

    failures
}
