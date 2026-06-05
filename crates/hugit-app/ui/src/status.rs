//! Live status page — item ① of WP-B7.
//!
//! Reflects current install/PR check state in real time. Serialisable so it
//! can be rendered as JSON or HTML by the GitHub App dashboard handler.

use serde::{Deserialize, Serialize};

/// State of a single PR's checks as reflected on the status page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PrCheckState {
    /// GitHub PR number.
    pub pr_number: u64,
    /// Git commit SHA this check state applies to.
    pub head_sha: String,
    /// Current check status ("queued", "in_progress", "completed").
    pub status: String,
    /// Conclusion when status = "completed" ("success", "failure", "neutral").
    pub conclusion: Option<String>,
    /// Saved minutes for this PR (memoisation hits × average duration).
    pub saved_minutes: u64,
    /// Audit link: content-addressed ref to the CheckResult set that produced
    /// `saved_minutes`. Every saved-minutes figure links to its CheckResult set.
    pub saved_minutes_audit_ref: String,
}

/// State of a GitHub App installation reflected on the status page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InstallState {
    /// GitHub App installation ID.
    pub installation_id: String,
    /// Repository slug (owner/repo).
    pub repo: String,
    /// Whether the installation is currently active.
    pub active: bool,
    /// Unix epoch ms of the last received event.
    pub last_event_at: Option<u64>,
}

/// The live status page — item ① of WP-B7.
///
/// Rendered by the App dashboard handler; never served via a new CLI binary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusPage {
    /// All currently tracked installations.
    pub installations: Vec<InstallState>,
    /// Per-PR check states for the current wave.
    pub pr_states: Vec<PrCheckState>,
    /// Unix epoch ms when this snapshot was produced.
    pub snapshot_at: u64,
}

impl StatusPage {
    /// Construct a new (empty) status page snapshot.
    pub fn new(snapshot_at: u64) -> Self {
        Self {
            installations: Vec::new(),
            pr_states: Vec::new(),
            snapshot_at,
        }
    }

    /// Add an installation to the status page.
    pub fn add_installation(&mut self, state: InstallState) {
        self.installations.push(state);
    }

    /// Add (or update) a PR check state on the status page.
    pub fn upsert_pr_state(&mut self, state: PrCheckState) {
        if let Some(existing) = self
            .pr_states
            .iter_mut()
            .find(|s| s.pr_number == state.pr_number && s.head_sha == state.head_sha)
        {
            *existing = state;
        } else {
            self.pr_states.push(state);
        }
    }

    /// Render the status page as a JSON string (used by the App dashboard).
    pub fn render_json(&self) -> serde_json::Result<String> {
        serde_json::to_string_pretty(self)
    }
}
