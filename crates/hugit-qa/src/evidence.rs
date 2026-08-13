//! # Evidence — the immutable record of what a user actually experienced
//!
//! The journey executor never asserts. It ACTS (through the real surfaces), then
//! records for every step: what was sent, what came back, how long it took, and
//! the observable delta in the world (the chained event log). This module owns the
//! **types** and the **append-only collector**. Analysis happens later, over this
//! record, in [`crate::analyzer`].
//!
//! Everything here is `Serialize`-able: the evidence trail is a JSON Lines log,
//! reproducible across runs.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The observable state of ONE repository's canonical world: its chained event log
/// (what a real client, or the operator `audit` view, can read).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RepoWorld {
    /// Record count in the canonical log (`<repo>.json`).
    pub records: u64,
    /// `seq` of the last (head) record, if any — the chain's high-water mark.
    pub head_seq: Option<u64>,
    /// Event-kind histogram over the whole log (e.g. `pr.opened: 3`).
    pub kinds: BTreeMap<String, u64>,
    /// Whether the log could be read at all (absent repo = the honest no-oracle).
    pub present: bool,
}

/// A point-in-time view of the entire world the journey touches: `repo → world`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldSnapshot {
    pub repos: BTreeMap<String, RepoWorld>,
}

impl WorldSnapshot {
    /// The delta between two snapshots over the SAME repo set — the `world_delta`
    /// an evidence record carries (`log_records +N, kinds {…}`). Absent repos are
    /// skipped so a repo *created during the step* shows as its full new world.
    pub fn diff(&self, before: &WorldSnapshot) -> WorldDelta {
        let mut delta = WorldDelta::default();
        for (repo, after) in &self.repos {
            match before.repos.get(repo) {
                None => {
                    delta.repos.insert(
                        repo.clone(),
                        RepoDelta {
                            records_delta: after.records as i64,
                            kinds_added: after.kinds.clone(),
                        },
                    );
                }
                Some(b) => {
                    let mut kinds_added = BTreeMap::new();
                    for (k, c) in &after.kinds {
                        let b = b.kinds.get(k).copied().unwrap_or(0);
                        if c > &b {
                            kinds_added.insert(k.clone(), c - b);
                        }
                    }
                    delta.repos.insert(
                        repo.clone(),
                        RepoDelta {
                            records_delta: after.records as i64 - b.records as i64,
                            kinds_added,
                        },
                    );
                }
            }
        }
        delta
    }
}

/// The `world_delta` between two snapshots for one repo.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RepoDelta {
    /// Net log-record delta (`+N` appended, `0` unchanged, negative impossible on
    /// an append-only log — but the arithmetic type says what happened).
    pub records_delta: i64,
    /// Kinds that grew, with how many records of that kind were added.
    pub kinds_added: BTreeMap<String, u64>,
}

/// The aggregate world delta across repos (the `world_delta` of an evidence record).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorldDelta {
    pub repos: BTreeMap<String, RepoDelta>,
}

/// Which product surface a step exercised — the analyst groups by this.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// The `/v1` HTTP API over a real TCP socket.
    Api,
    /// The REAL `hugit` CLI binary.
    Cli,
    /// The real `git` client over the smart-HTTP wire.
    Git,
    /// Identity issuance (`TokenStore::mint` — the credential a user holds).
    Identity,
    /// World building / observation (setup, snapshots, waits).
    Setup,
}

/// ONE step of the journey, fully recorded. Immutable after the collector writes
/// it: the analyzer may only read.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepEvidence {
    /// Monotonic step index within the journey (its display order).
    pub index: u32,
    /// The step's name (from the journey DSL).
    pub name: String,
    /// The product surface exercised.
    pub surface: Surface,
    /// Human/analyst note — what the step was *for* (the goal framing).
    pub goal: String,
    /// The concrete invocation (e.g. `POST /v1/repos/acme/prs`, `check run`, `git push`).
    pub detail: String,
    /// The process command, when the step ran a binary (`cli`/`git`).
    pub command: Option<String>,
    /// Captured stdout of the binary (truncated to a bounded frame).
    pub stdout: Option<String>,
    /// Captured stderr of the binary (truncated to a bounded frame).
    pub stderr: Option<String>,
    /// HTTP status for `Api`/git-wire steps.
    pub status: Option<u16>,
    /// Process exit code for `Cli`/`Git` process steps (0 ⇒ success).
    pub exit_code: Option<i32>,
    /// The HTTP response body / CLI envelope body, if JSON (bounded).
    pub body: Option<serde_json::Value>,
    /// Wall-clock duration of the step.
    pub duration_ms: u64,
    /// World before the step (the executor's view, taken just before acting).
    pub world_before: Option<WorldSnapshot>,
    /// World after the step (taken just after acting).
    pub world_after: Option<WorldSnapshot>,
    /// Wall-clock timestamp (unix millis).
    pub ts_ms: u64,
}

/// The evidence trail: autonomous + append-only + serializable. The analyzer
/// consumes `steps()` and the final world.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Evidence {
    steps: Vec<StepEvidence>,
    /// Final world snapshot (the journey's end state).
    pub final_world: Option<WorldSnapshot>,
}

/// Bounded frame for captured stdout/stderr/body so a runaway producer can never
/// balloon the evidence into an OOM (the analyzer needs shape, not megabytes).
pub const CAPTURE_BUDGET_BYTES: usize = 32 * 1024;

/// Truncate a captured byte-stream to the evidence budget, preserving the head.
pub fn bounded(text: &str) -> String {
    let mut bounded = String::with_capacity(text.len().min(CAPTURE_BUDGET_BYTES));
    for ch in text.chars() {
        if bounded.len() + ch.len_utf8() > CAPTURE_BUDGET_BYTES {
            bounded.push('…');
            break;
        }
        bounded.push(ch);
    }
    bounded
}

impl Evidence {
    /// A fresh collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append one step record (the ONLY mutation; `steps()` never mutates).
    pub fn push(&mut self, step: StepEvidence) {
        self.steps.push(step);
    }

    /// Read-only view of the full trail, in journey order.
    pub fn steps(&self) -> &[StepEvidence] {
        &self.steps
    }

    /// Rendered as bounded JSON Lines (one line per step) — the repeatable audit
    /// artefact a report references.
    pub fn to_jsonl(&self) -> String {
        self.steps
            .iter()
            .filter_map(|s| serde_json::to_string(s).ok())
            .collect::<Vec<_>>()
            .join("\n")
    }
}
