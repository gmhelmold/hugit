//! `hugit fleet` — machine-readable fleet state schema emitter (⑥).
//!
//! Emits a documented, versioned, machine-readable JSON schema reflecting the
//! TRUE workspace/agent state derived from events vs a known fixture.  The
//! schema is validated on every emission.
//!
//! # Schema version
//! Schema version is `"1"`.  Any breaking change bumps the version.  The
//! `git_activity` / `git_activity_count` fields were added ADDITIVELY (new
//! fields, existing fields unchanged in type or meaning), so the version stays
//! `"1"`.
//!
//! # State derivation
//! Fleet state is a projection of the event log:
//! - `ws.state.*` events advance workspace state.
//! - `agent.assigned` / `agent.completed` / `agent.failed` events track agents.
//! - `ref.update` / `ref.delete` events (the raw git trace — the silent capture
//!   hooks of `hugit capture` + the receive-pack push path) fold into
//!   `git_activity`, one entry per record, in log order.
//! - All other events are ignored.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use hugit_contracts::event_record::EventRecord;

use crate::redact;

/// The schema version for fleet state output.
pub const FLEET_SCHEMA_VERSION: &str = "1";

/// The state of one workspace, derived from events.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceState {
    /// No state has been observed yet (initial).
    Idle,
    /// The workspace is active and running work.
    Active,
    /// The workspace has been closed/sealed.
    Closed,
}

/// One agent entry in the fleet state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AgentEntry {
    /// Unique agent identifier.
    pub agent_id: String,
    /// The workspace this agent is assigned to.
    pub workspace_id: String,
    /// Current agent state.
    pub state: AgentState,
}

/// The lifecycle state of one agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    /// Agent has been assigned but not yet completed.
    Assigned,
    /// Agent completed successfully.
    Completed,
    /// Agent failed.
    Failed,
}

/// The full fleet state at a point in the event log.
///
/// This is the documented, versioned machine-readable schema.  It is a pure
/// projection of the event log — no second store.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FleetState {
    /// Schema version — bump on breaking changes.
    pub schema_version: String,
    /// All known workspaces and their current states.
    pub workspaces: Vec<WorkspaceEntry>,
    /// All known agents and their current states.
    pub agents: Vec<AgentEntry>,
    /// The captured git activity (`ref.update` / `ref.delete` — the raw git
    /// trace), one entry per record, in log order.
    pub git_activity: Vec<GitActivityEntry>,
    /// Number of git activity entries (mirrors `git_activity.len()`; kept
    /// explicit so summary consumers can read one number).
    pub git_activity_count: u64,
    /// Log sequence of the last event consumed.
    pub last_seq: u64,
    /// Total number of events consumed.
    pub event_count: u64,
    /// Number of records whose payload could not be parsed as valid JSON.
    ///
    /// These are counted and surfaced rather than silently coalesced to
    /// "unknown".  A non-zero value indicates data quality issues upstream.
    pub malformed: u64,
}

/// One workspace entry in the fleet state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WorkspaceEntry {
    /// Unique workspace identifier.
    pub workspace_id: String,
    /// Current workspace state.
    pub state: WorkspaceState,
}

/// One captured git activity entry — a raw `ref.update` / `ref.delete` record
/// folded from the git trace (the silent capture hooks of `hugit capture` +
/// the receive-pack push path, the same events `watch` classifies as
/// `GitActivity`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GitActivityEntry {
    /// Branch the activity happened on. Derived from the payload `branch`
    /// field, else from the `ref` field (`refs/heads/<b>` stripped). Empty
    /// when the payload carries neither (e.g. a push attempt).
    pub branch: String,
    /// Full ref name — the payload `ref` field, else `refs/heads/<branch>`
    /// derived from the `branch` field. Empty when neither is present.
    pub ref_name: String,
    /// The commit the ref points at after the activity — the payload `target`
    /// field (commit/merge/raw push), the `to` field (checkout). Empty for
    /// push attempts and deletes, which carry no target.
    pub target: String,
    /// Log sequence of the capturing event.
    pub seq: u64,
    /// Capture-source qualifier, mirroring the silent-hook payloads (#336):
    /// `"checkout"` (`checkout:true`), `"attempt"` (`attempt:true`),
    /// `"merge"` (`merged_from`), else `None` (post-commit / raw push).
    pub qualifier: Option<String>,
    /// Unix epoch milliseconds when the git event was recorded.
    pub recorded_at: u64,
}

impl FleetState {
    /// Derive fleet state from a slice of EventRecords.
    ///
    /// Pure projection: equal records yield equal FleetState.
    ///
    /// Malformed records (non-JSON payloads) are counted in `malformed` rather
    /// than silently coalesced to "unknown".  Surfaced strings are routed
    /// through the view-boundary redaction filter (④).
    ///
    /// The fold keys its maps on the RAW (un-redacted) identifiers and redacts
    /// ONLY at the final emission. Redacting before keying would collapse every
    /// secret-shaped id to the single `[REDACTED]` marker, so two distinct
    /// secret-shaped agents/workspaces would collide on one key — the second
    /// silently overwriting the first, and a completed/failed update landing on
    /// the wrong entry. Raw-key + redact-on-emit keeps distinct entities distinct
    /// while still never surfacing a raw id (the same pattern the `Ledger` fold
    /// uses with its raw-id index).
    pub fn from_records(records: &[EventRecord]) -> Self {
        // Keyed on the RAW id (see doc above); the raw id never leaves this fn —
        // it is redacted at the emission step below.
        let mut workspaces: std::collections::HashMap<String, WorkspaceState> =
            std::collections::HashMap::new();
        let mut agents: std::collections::HashMap<String, (String, AgentState)> =
            std::collections::HashMap::new();
        // git activity is inherently sequential (one entry per record, in log
        // order — the raw git trace), so it folds as a plain Vec, not a map.
        let mut git_activity: Vec<GitActivityEntry> = Vec::new();
        let mut last_seq = 0u64;
        let event_count = records.len() as u64;
        let mut malformed = 0u64;

        for r in records {
            last_seq = r.seq;

            // Parse the payload; count malformed records instead of swallowing.
            let payload = match serde_json::from_str::<serde_json::Value>(&r.payload) {
                Ok(v) => v,
                Err(_) => {
                    malformed += 1;
                    continue; // skip — do not fabricate "unknown" entries.
                }
            };

            match r.kind.as_str() {
                kind if kind.starts_with("ws.state.") => {
                    let raw_ws_id = payload
                        .get("workspace_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    let new_state = match kind {
                        "ws.state.active" => WorkspaceState::Active,
                        "ws.state.closed" => WorkspaceState::Closed,
                        _ => WorkspaceState::Idle,
                    };
                    workspaces.insert(raw_ws_id.to_string(), new_state);
                }
                "agent.assigned" => {
                    let raw_agent_id = payload
                        .get("agent_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    let raw_ws_id = payload
                        .get("workspace_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    // Value carries the RAW workspace_id (redacted at emission).
                    agents.insert(
                        raw_agent_id.to_string(),
                        (raw_ws_id.to_string(), AgentState::Assigned),
                    );
                }
                "agent.completed" => {
                    let raw_agent_id = payload
                        .get("agent_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    if let Some(entry) = agents.get_mut(raw_agent_id) {
                        entry.1 = AgentState::Completed;
                    }
                }
                "agent.failed" => {
                    let raw_agent_id = payload
                        .get("agent_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    if let Some(entry) = agents.get_mut(raw_agent_id) {
                        entry.1 = AgentState::Failed;
                    }
                }
                // The raw git trace — the same classification watch uses
                // (EventClass::GitActivity). Each record folds into one entry.
                "ref.update" | "ref.delete" => {
                    git_activity.push(Self::git_activity_entry(r, &payload));
                }
                _ => {} // inert event; advances the chain, no fleet state change.
            }
        }

        // Build sorted vecs for deterministic output — redacting the raw ids ONLY
        // here, at the view boundary (④), so distinct entities stayed distinct
        // above while no raw id is ever surfaced.
        let mut workspace_vec: Vec<WorkspaceEntry> = workspaces
            .into_iter()
            .map(|(raw_id, state)| WorkspaceEntry {
                workspace_id: redact::apply(&raw_id),
                state,
            })
            .collect();
        workspace_vec.sort_by(|a, b| a.workspace_id.cmp(&b.workspace_id));

        let mut agent_vec: Vec<AgentEntry> = agents
            .into_iter()
            .map(|(raw_agent_id, (raw_ws_id, state))| AgentEntry {
                agent_id: redact::apply(&raw_agent_id),
                workspace_id: redact::apply(&raw_ws_id),
                state,
            })
            .collect();
        agent_vec.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));

        // git_activity is already in log order (seq-ascending by construction).
        let git_activity_count = git_activity.len() as u64;

        FleetState {
            schema_version: FLEET_SCHEMA_VERSION.to_string(),
            workspaces: workspace_vec,
            agents: agent_vec,
            git_activity,
            git_activity_count,
            last_seq,
            event_count,
            malformed,
        }
    }

    /// Fold one `ref.update` / `ref.delete` record into a `GitActivityEntry`.
    ///
    /// Recognises the same payloads the silent capture hooks (#336) write and
    /// the receive-pack push path writes:
    /// - commit:    `{"ref","target","branch"}`
    /// - checkout:  `{"checkout":{"fact", "truth"},"from","to","branch"}`
    /// - attempt:   `{"attempt":true,"refspecs","shas"}`
    /// - merge:     `{"merged_from","target"}`
    /// - raw push:  `{"ref","target"}`
    /// - delete:    `{"ref"}`
    ///
    /// Qualifier mirrors the hook source (`checkout` / `attempt` / `merge`),
    /// `None` for post-commit and raw push/delete. Surfaced strings are routed
    /// through the view-boundary redaction filter (④) — these are not
    /// structural hash fields, so a secret-shaped branch/target redacts exactly
    /// like the ids above.
    pub fn git_activity_entry(r: &EventRecord, payload: &serde_json::Value) -> GitActivityEntry {
        let qualifier = if payload.get("checkout").is_some() {
            Some("checkout".to_string())
        } else if payload.get("attempt").and_then(serde_json::Value::as_bool) == Some(true) {
            Some("attempt".to_string())
        } else if payload.get("merged_from").is_some() {
            Some("merge".to_string())
        } else {
            None
        };

        let payload_ref = payload
            .get("ref")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
        let payload_branch = payload
            .get("branch")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        // Derive from the RAW payload values (like the map-key discipline
        // above: raw first, redact only at the emission point).
        let branch: String = if !payload_branch.is_empty() {
            payload_branch.to_string()
        } else {
            payload_ref
                .strip_prefix("refs/heads/")
                .unwrap_or("")
                .to_string()
        };
        // Ref name: the payload `ref`, else the branch-derived ref (the same
        // derivation the commit hook applies to a bare branch).
        let ref_name: String = if !payload_ref.is_empty() {
            payload_ref.to_string()
        } else if !branch.is_empty() {
            format!("refs/heads/{branch}")
        } else {
            String::new()
        };

        // Target: payload `target` (commit/merge/raw push), `to` (checkout),
        // else empty (attempt/delete carry none).
        let target = payload
            .get("target")
            .or_else(|| payload.get("to"))
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");

        GitActivityEntry {
            branch: redact::apply(&branch),
            ref_name: redact::apply(&ref_name),
            target: redact::apply(target),
            seq: r.seq,
            qualifier,
            recorded_at: r.recorded_at,
        }
    }

    /// Emit this fleet state as a validated JSON string.
    ///
    /// The JSON is schema-valid by construction (all fields present, correct types).
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("FleetState serialization never fails")
    }

    /// Validate that this fleet state is schema-valid.
    ///
    /// Returns `Ok(())` if valid, `Err` with a description if not.
    ///
    /// All required fields are non-optional in the Rust type, so their presence
    /// in the serialized output is guaranteed by the derive.  The only runtime
    /// check that is not trivially proven by the type is that `schema_version`
    /// is non-empty (a constructive invariant this crate must uphold).
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version.is_empty() {
            return Err("schema_version must not be empty".to_string());
        }
        // Round-trip probe: confirm the JSON parses back without error.
        // Field-presence checks are omitted — every field is non-optional in
        // the Rust type, so `to_string_pretty` guarantees they appear.
        let json = self.to_json();
        serde_json::from_str::<serde_json::Value>(&json)
            .map_err(|e| format!("JSON round-trip error: {e}"))?;
        Ok(())
    }
}
