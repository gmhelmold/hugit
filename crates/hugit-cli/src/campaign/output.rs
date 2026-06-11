//! Structured output + errors for the campaign porcelain (WP-PC1).
//!
//! **One canonical shape for every porcelain verb** (the one error law,
//! [`crate::porcelain`]): `{"error":{"kind":...,"message":...,"fix":..., …ctx}}`
//! on **stdout** (so an agent parsing stdout always gets a machine signal,
//! never a fake success), exit code [`PORCELAIN_ERROR_EXIT`]. The `fix` field
//! is the suggested remediation the agent can act on without a human.
//!
//! Extra structured context (e.g. the in-flight PR list, the offending path)
//! is folded **FLAT** into the `error` object — never nested under a `detail`
//! sub-object (WF — error-shape true uniformity: `campaign` used to nest `path`
//! under `detail` while every sibling put it top-level; now ONE parser
//! (`error.<key>`) works across every verb).

use std::process::ExitCode;

use serde_json::{Value, json};

use crate::porcelain::PORCELAIN_ERROR_EXIT;

/// A structured campaign error — emitted as `{"error":{...}}` JSON on stdout,
/// the SAME canonical shape as [`crate::porcelain::PorcelainError`] (context
/// folded flat, never under `detail`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignError {
    /// Stable machine kind (e.g. `"in_flight_prs"`, `"io"`, `"parse"`).
    kind: &'static str,
    /// Human/agent-readable message.
    message: String,
    /// The suggested fix the caller can act on.
    fix: String,
    /// Extra structured context, folded FLAT into the `error` object on render
    /// (e.g. `("path", …)`, `("in_flight", …)`) — matching the canonical
    /// [`crate::porcelain::PorcelainError`]. Insertion order preserved.
    context: Vec<(&'static str, Value)>,
}

impl CampaignError {
    /// A `kind` + `message` + suggested `fix` error, no extra context.
    pub fn new(kind: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        CampaignError {
            kind,
            message: message.into(),
            fix: fix.into(),
            context: Vec::new(),
        }
    }

    /// Fold a flat structured context key into the `error` object (e.g.
    /// `("in_flight", json!([…]))`). Repeatable; insertion order preserved.
    /// Matches [`crate::porcelain::PorcelainError::with_context`] — the ONE
    /// canonical placement (flat, never under `detail`).
    pub fn with_context(mut self, key: &'static str, value: Value) -> Self {
        self.context.push((key, value));
        self
    }

    /// An I/O error reading/writing the world file.
    pub fn io(action: &str, path: &std::path::Path, e: &std::io::Error) -> Self {
        CampaignError::new(
            "io",
            format!("{action} {}: {e}", path.display()),
            "check the --log path exists and is writable",
        )
    }

    /// A `--log` FILE that does not exist (the path is absent on disk). Explicit
    /// — NEVER silently an empty world (P-CAMPAIGN-EMPTY). Exit `2`. Carries the
    /// `path` as flat context, matching the sibling porcelain
    /// [`crate::porcelain::PorcelainError::log_not_found`].
    pub fn log_not_found(path: &std::path::Path) -> Self {
        CampaignError::new(
            "log_not_found",
            format!("--log file does not exist: {}", path.display()),
            "open the campaign first (`hugit campaign open --log <path> …` \
             bootstraps it) or point --log at an existing canonical \
             [EventRecord, …] file",
        )
        .with_context("path", json!(path.display().to_string()))
    }

    /// A parse error on the event log file.
    pub fn parse(e: &serde_json::Error) -> Self {
        CampaignError::new(
            "parse",
            format!("event log is not valid JSON: {e}"),
            "the --log file must be a canonical JSON [EventRecord, …] array \
             (the engine's EventLog shape, shared by every flow porcelain verb)",
        )
    }

    /// Render the canonical `{"error":{...}}` envelope (stable wire shape) —
    /// context folded FLAT, identical placement to every sibling verb.
    pub fn to_json(&self) -> String {
        let mut error = json!({
            "kind": self.kind,
            "message": self.message,
            "fix": self.fix,
        });
        if let Some(map) = error.as_object_mut() {
            for (k, v) in &self.context {
                map.insert((*k).to_string(), v.clone());
            }
        }
        json!({ "error": error }).to_string()
    }

    /// The process exit code for a structured error.
    pub fn exit_code(&self) -> ExitCode {
        ExitCode::from(PORCELAIN_ERROR_EXIT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_json_is_the_one_canonical_shape() {
        let e = CampaignError::new("k", "m", "f");
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "k");
        assert_eq!(v["error"]["message"], "m");
        assert_eq!(v["error"]["fix"], "f");
        // Uniformity: context is folded FLAT, never under a `detail` sub-object.
        assert!(v["error"].get("detail").is_none());
        assert!(v.get("kind").is_none(), "must be nested, never flat");
    }

    #[test]
    fn context_folds_flat_into_the_error_object() {
        // The same flat placement the canonical PorcelainError uses — one parser
        // (`error.<key>`) works across campaign + every sibling verb.
        let e = CampaignError::new("k", "m", "f")
            .with_context("in_flight", json!(["PR-1"]))
            .with_context("path", json!("/x/log.json"));
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["in_flight"][0], "PR-1");
        assert_eq!(v["error"]["path"], "/x/log.json");
        assert!(
            v["error"].get("detail").is_none(),
            "context is flat, never nested under detail"
        );
    }
}
