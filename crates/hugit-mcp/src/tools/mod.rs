//! The four hugit MCP tools.
//!
//! Each tool is a pure-ish function `(serde_json::Value args) -> ToolOutcome`.
//! The MCP `tools/call` handler ([`crate::server`]) wraps the outcome into the
//! MCP content envelope. The four tools, honest scope each:
//!
//! - [`claim_disjointness`] — wraps the REAL union-engine disjointness primitive
//!   (`hugit_queue::core::AffectedSet::is_disjoint`). HONEST CAVEAT: the affected
//!   set is a path-approximation in v1 (file-path overlap, not the true memoized
//!   check-key blast radius B3 will supply) — stated in every result.
//! - [`land_status`] — shells `hugit queue show --log <path>` and parses the
//!   stdout JSON. No second source of truth.
//! - [`cost_attest`] — `GET /v1/repos/{repo}/insights` with a real Bearer token.
//!   STRUCTURALLY omits any hand-stamp override; per-PR cost is honest-null until
//!   the runner fabric supplies a provider-billed figure.
//! - [`liveness_probe`] — `/readyz` + a bounded Bearer probe with a git UA;
//!   disambiguates 404-denied / 404-missing / 401-gate / 403-bot. REFUSES heavy
//!   reads against the single-thread engine.

pub mod claim_disjointness;
pub mod cost_attest;
pub mod land_status;
pub mod liveness_probe;

use serde_json::Value;

/// The outcome of a tool invocation.
///
/// `Ok` carries a structured JSON value rendered into the MCP content envelope
/// as pretty JSON text. `Err` carries an `is_error: true` MCP tool error with a
/// human-readable message — distinct from a JSON-RPC protocol error (a tool
/// error is a normal, successful `tools/call` whose CONTENT reports a failure,
/// per the MCP spec, so the model can see and react to it).
pub enum ToolOutcome {
    Ok(Value),
    Err(String),
}

impl ToolOutcome {
    /// Convenience: a tool error from any displayable cause.
    pub fn err(msg: impl std::fmt::Display) -> Self {
        ToolOutcome::Err(msg.to_string())
    }
}

/// Read a required string field from the tool arguments object.
pub(crate) fn req_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, String> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("missing or empty required string argument `{key}`"))
}

/// Read an optional string field.
pub(crate) fn opt_str<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}
