//! Admin control-plane wire VMs (operator area).
//!
//! These back the hugit-authored, githugr-hosted **admin area** — an operator
//! view over the engine's own state. All three are pure projections over the
//! already-chain-verified event log (no P2 infra), so they are honest the same
//! way every other read is: real data or a documented empty shape, never faked.
//!
//! - [`AuditVm`] — `GET /v1/repos/{repo}/audit` — the paginated, all-kinds event
//!   timeline (who did what, when), the thing the live SSE feed can't be (it
//!   filters internal kinds + isn't historical/paginated).
//! - [`ErasureHistoryVm`] — `GET /v1/repos/{repo}/erasure` — every erasure
//!   decision (approved + denied + by-id), not just the latest-approved one
//!   `security` surfaces.
//! - [`AdminOverviewVm`] — `GET /v1/repos/{repo}/admin/overview` — the
//!   one-call operational snapshot (queue depth, campaigns, attention, policy
//!   posture, last activity).

use serde::{Deserialize, Serialize};

/// One row in the audit timeline — a single event-log record projected to the
/// SAFE fields only. The raw payload is NEVER echoed (free-text → leak risk);
/// `summary` is a kind-aware, scrubbed one-liner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntryVm {
    /// The chain sequence (0-based, monotonic, gap-free) — the stable cursor.
    pub seq: u64,
    /// The event kind (fixed vocabulary, e.g. `policy.set` — safe to surface raw).
    pub kind: String,
    /// The acting principal (the tail of the principal chain), scrubbed; `"—"`
    /// when absent.
    pub principal: String,
    /// A kind-aware, scrubbed one-line summary (never a raw payload echo).
    pub summary: String,
    /// Humanized age of the record (e.g. "2h", "3d").
    pub age: String,
    /// Raw Unix-ms timestamp (for client-side sorting / exact display).
    pub recorded_at: u64,
    /// First 12 chars of the record's `this_hash` — the integrity reference an
    /// operator can cross-check against the chain.
    pub hash_short: String,
}

/// The paginated audit timeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditVm {
    /// Rows in ascending `seq` order, within `[since, since+limit)` and matching
    /// any `kind`/`principal` filter.
    pub entries: Vec<AuditEntryVm>,
    /// How many rows this page returned.
    pub returned: usize,
    /// The cursor to pass as the next `?since=` to continue forward, or `None`
    /// when the page reached the head (no more records).
    pub next_since: Option<u64>,
    /// The highest `seq` currently on the log (the head) — lets the UI show
    /// "showing X of N".
    pub head_seq: u64,
}

/// One erasure decision in the governance history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasureRowVm {
    /// The erasure request id (scrubbed if secret-shaped).
    pub erasure_id: String,
    /// The decision state: `approved` | `denied` (the latest decision per id).
    pub state: String,
    /// Execution stage — ALWAYS `pending` locally: recording a decision never
    /// executes the CAS scrub (X12 execution is the P2 seam).
    pub execution: String,
    /// The deciding principal (scrubbed), `"—"` when absent.
    pub decided_by: String,
    /// Humanized age of the decision.
    pub age: String,
    /// The chain seq of the decision record.
    pub seq: u64,
}

/// Every erasure decision (approved + denied), latest-per-id, newest first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErasureHistoryVm {
    pub entries: Vec<ErasureRowVm>,
    /// Count of distinct ids whose latest decision is `approved`.
    pub approved_count: usize,
    /// Count of distinct ids whose latest decision is `denied`.
    pub denied_count: usize,
    /// Honest note: execution of an approved erasure is the P2 CAS-scrub seam.
    pub note: String,
}

/// The one-call operational snapshot for the admin overview.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdminOverviewVm {
    /// Active PRs in the landing queue (not yet landed).
    pub queue_depth: usize,
    /// Distinct campaigns with at least one open PR.
    pub active_campaigns: usize,
    /// PRs needing operator attention (approved-not-landed + rejected/blocked).
    pub attention_count: usize,
    /// Total PRs ever opened on the log.
    pub total_prs: usize,
    /// Policy rules currently enabled (house + operator overrides).
    pub policy_rules_active: usize,
    /// Erasure decisions recorded (any state).
    pub erasure_decisions: usize,
    /// Total records on the chain (the audit depth).
    pub log_depth: u64,
    /// Humanized age of the most recent record, `"—"` on an empty log.
    pub last_activity_age: String,
}
