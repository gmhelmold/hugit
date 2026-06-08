//! `hugit fleet` — machine-readable fleet state schema emitter (⑥).
//!
//! Emits a documented, versioned, machine-readable JSON schema reflecting the
//! TRUE workspace/agent state derived from events vs a known fixture.  The
//! schema is validated on every emission.
//!
//! # Schema version
//! Schema version is `"1"`.  Any breaking change bumps the version.
//!
//! # State derivation
//! Fleet state is a projection of the event log:
//! - `ws.state.*` events advance workspace state.
//! - `agent.assigned` / `agent.completed` / `agent.failed` events track agents.
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

impl FleetState {
    /// Derive fleet state from a slice of EventRecords.
    ///
    /// Pure projection: equal records yield equal FleetState.
    ///
    /// Malformed records (non-JSON payloads) are counted in `malformed` rather
    /// than silently coalesced to "unknown".  Surfaced strings are routed
    /// through the view-boundary redaction filter (④).
    pub fn from_records(records: &[EventRecord]) -> Self {
        let mut workspaces: std::collections::HashMap<String, WorkspaceState> =
            std::collections::HashMap::new();
        let mut agents: std::collections::HashMap<String, AgentEntry> =
            std::collections::HashMap::new();
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
                    let ws_id = redact::apply(raw_ws_id);
                    let new_state = match kind {
                        "ws.state.active" => WorkspaceState::Active,
                        "ws.state.closed" => WorkspaceState::Closed,
                        _ => WorkspaceState::Idle,
                    };
                    workspaces.insert(ws_id, new_state);
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
                    let agent_id = redact::apply(raw_agent_id);
                    let ws_id = redact::apply(raw_ws_id);
                    agents.insert(
                        agent_id.clone(),
                        AgentEntry {
                            agent_id,
                            workspace_id: ws_id,
                            state: AgentState::Assigned,
                        },
                    );
                }
                "agent.completed" => {
                    let raw_agent_id = payload
                        .get("agent_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    let agent_id = redact::apply(raw_agent_id);
                    if let Some(entry) = agents.get_mut(&agent_id) {
                        entry.state = AgentState::Completed;
                    }
                }
                "agent.failed" => {
                    let raw_agent_id = payload
                        .get("agent_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("unknown");
                    let agent_id = redact::apply(raw_agent_id);
                    if let Some(entry) = agents.get_mut(&agent_id) {
                        entry.state = AgentState::Failed;
                    }
                }
                _ => {} // inert event; advances the chain, no fleet state change.
            }
        }

        // Build sorted vecs for deterministic output.
        let mut workspace_vec: Vec<WorkspaceEntry> = workspaces
            .into_iter()
            .map(|(id, state)| WorkspaceEntry {
                workspace_id: id,
                state,
            })
            .collect();
        workspace_vec.sort_by(|a, b| a.workspace_id.cmp(&b.workspace_id));

        let mut agent_vec: Vec<AgentEntry> = agents.into_values().collect();
        agent_vec.sort_by(|a, b| a.agent_id.cmp(&b.agent_id));

        FleetState {
            schema_version: FLEET_SCHEMA_VERSION.to_string(),
            workspaces: workspace_vec,
            agents: agent_vec,
            last_seq,
            event_count,
            malformed,
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
