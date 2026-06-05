//! AppWebhooks — frozen by decomposition §1, item 10.
//!
//! GitHub App ingest/ack surface: signed-event envelope, ack-receipt, and
//! the Checks-API write-back request/response types. Compound root.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Signed inbound webhook event envelope from GitHub App.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SignedEventEnvelope {
    /// GitHub delivery GUID.
    pub delivery_id: String,
    /// GitHub webhook event type (e.g. "check_suite", "pull_request").
    pub event_type: String,
    /// HMAC-SHA256 hex signature of the raw payload (from X-Hub-Signature-256
    /// header).
    pub signature: String,
    /// Raw JSON payload as a string (kept opaque for signature verification).
    pub payload: String,
    /// Unix epoch milliseconds when this envelope was received.
    pub received_at: u64,
}

/// Acknowledgement receipt returned after processing an inbound webhook.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AckReceipt {
    /// Delivery GUID echoed back.
    pub delivery_id: String,
    /// Opaque internal processing ID assigned to this event.
    pub processing_id: String,
    /// Unix epoch milliseconds when the ack was issued.
    pub acked_at: u64,
}

/// Request to write back to GitHub Checks API.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChecksWriteRequest {
    /// GitHub repository (owner/repo).
    pub repo: String,
    /// Git commit SHA this check run is attached to.
    pub head_sha: String,
    /// GitHub App check run name.
    pub check_name: String,
    /// Check run status ("queued", "in_progress", "completed").
    pub status: String,
    /// Conclusion ("success", "failure", "neutral", etc.); present when
    /// status = "completed".
    pub conclusion: Option<String>,
    /// Short summary for the check run output.
    pub summary: String,
    /// Content-addressed ref to the full check output blob.
    pub output_ref: String,
}

/// Response from the GitHub Checks API write-back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChecksWriteResponse {
    /// GitHub check run ID assigned by the API.
    pub check_run_id: u64,
    /// GitHub API URL for the created/updated check run.
    pub html_url: String,
}

/// GitHub App ingest/ack surface — compound root type aggregating all
/// webhook-related sub-types (decomposition §1, item 10).
///
/// The committed JSON Schema is the schema of this root type.
///
/// Frozen by WP-00.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AppWebhooks {
    /// Inbound signed event envelope.
    pub inbound: SignedEventEnvelope,
    /// Acknowledgement receipt.
    pub ack: AckReceipt,
    /// Checks API write-back request.
    pub write_request: ChecksWriteRequest,
    /// Checks API write-back response.
    pub write_response: ChecksWriteResponse,
}
