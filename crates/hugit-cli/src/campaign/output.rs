//! Structured output + errors for the campaign porcelain (WP-PC1).
//!
//! One shape for every error: `{"error":{"kind":...,"message":...,"fix":...}}`
//! on **stdout** (so an agent parsing stdout always gets a machine signal,
//! never a fake success), exit code [`PORCELAIN_ERROR_EXIT`]. The `fix` field
//! is the suggested remediation the agent can act on without a human.

use std::process::ExitCode;

use serde_json::{Value, json};

use crate::porcelain::PORCELAIN_ERROR_EXIT;

/// A structured campaign error — emitted as `{"error":{...}}` JSON on stdout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignError {
    /// Stable machine kind (e.g. `"in_flight_prs"`, `"io"`, `"parse"`).
    kind: &'static str,
    /// Human/agent-readable message.
    message: String,
    /// The suggested fix the caller can act on.
    fix: String,
    /// Optional structured detail (e.g. the list of in-flight PRs).
    detail: Option<Value>,
}

impl CampaignError {
    /// A `kind` + `message` + suggested `fix` error, no extra detail.
    pub fn new(kind: &'static str, message: impl Into<String>, fix: impl Into<String>) -> Self {
        CampaignError {
            kind,
            message: message.into(),
            fix: fix.into(),
            detail: None,
        }
    }

    /// Attach structured detail (folded into the `error` object).
    pub fn with_detail(mut self, detail: Value) -> Self {
        self.detail = Some(detail);
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
        .with_detail(json!({ "path": path.display().to_string() }))
    }

    /// A parse error on the event log file.
    pub fn parse(e: &serde_json::Error) -> Self {
        CampaignError::new(
            "parse",
            format!("event log is not valid JSON: {e}"),
            "the --log file must be a canonical JSON [EventRecord, …] array \
             (the engine's EventLog shape, shared by every porcelain verb)",
        )
    }

    /// Render the canonical `{"error":{...}}` envelope (stable wire shape).
    pub fn to_json(&self) -> String {
        let mut error = json!({
            "kind": self.kind,
            "message": self.message,
            "fix": self.fix,
        });
        if let (Some(detail), Some(map)) = (&self.detail, error.as_object_mut()) {
            map.insert("detail".to_string(), detail.clone());
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
    fn error_json_is_the_stable_shape() {
        let e = CampaignError::new("k", "m", "f");
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "k");
        assert_eq!(v["error"]["message"], "m");
        assert_eq!(v["error"]["fix"], "f");
        assert!(v["error"].get("detail").is_none());
    }

    #[test]
    fn detail_is_folded_in() {
        let e = CampaignError::new("k", "m", "f").with_detail(json!({"prs": ["PR-1"]}));
        let v: Value = serde_json::from_str(&e.to_json()).unwrap();
        assert_eq!(v["error"]["detail"]["prs"][0], "PR-1");
    }
}
