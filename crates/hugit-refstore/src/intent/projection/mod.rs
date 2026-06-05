//! The deterministic, one-directional **intent → git** projection.
//!
//! git is the *derived projection* of the intent log — never the other way
//! around. We never reverse-engineer commits back into intents. Per
//! `docs/whitepaper/hugit-v1.md` §4:
//!
//! > Same store, two zooms; they can never disagree because one is derived from
//! > the other.
//!
//! # The two altitudes
//!
//! Both altitudes are folds of the *same* event log, in the *same* chain order:
//!
//! - **Intent altitude** (`hugit log`): the [`crate::intent::model::IntentLog`]
//!   — one row per landed intent, raw pushes elided as provenance-free.
//! - **Machine altitude** (`git log`): a [`MachineHistory`] — one
//!   [`ProjectionRow`] per *ref-mutating* event in log order. A landed intent
//!   projects to a [`GitCommit`] whose generated message **embeds its
//!   `intent_id`** ([`ProjectionRow::Intent`]); a raw push projects to an
//!   [`ProjectionRow::ExternalChange`] — provenance-free, never a fabricated
//!   intent (④).
//!
//! Because every `Intent` row in one altitude is the same log event as the
//! corresponding `ProjectionRow::Intent` in the other, with the same `seq`, the
//! altitudes are **provably consistent** ([`MachineHistory::is_consistent_with`])
//! and cannot disagree — that is item ②, asserted on a 50-intent fixture.
//!
//! The projection is *deterministic*: equal logs project to byte-for-byte equal
//! histories, so the commit set is **reproducible from the log** (item ①).

use std::fmt::Write as _;

use hugit_contracts::event_record::EventRecord;

use crate::intent::model::{
    INTENT_LANDED_KIND, Intent, IntentLog, IntentModelError, intents_from_records, is_raw_push,
    parse_intent,
};
use crate::log::EventLog;

/// Which altitude a derived view represents (zoom level over the one store).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Altitude {
    /// `hugit log` — one row per landed intent.
    Intent,
    /// `git log` — one commit/row per ref-mutating event.
    Machine,
}

/// A generated git commit projected from one landed intent.
///
/// The commit is *generated* (one-directional): its `message` embeds the
/// `intent_id` so the commit can always be traced back to the intent, and the
/// whole commit is reproducible from the log event alone (①).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitCommit {
    /// Log sequence of the originating `intent.landed` event.
    pub seq: u64,
    /// The intent this commit was generated from.
    pub intent_id: String,
    /// The ref the commit advances.
    pub ref_name: String,
    /// The object id the ref now points at.
    pub target: String,
    /// Generated commit message; **embeds the `intent_id`** (①).
    pub message: String,
}

impl GitCommit {
    /// The deterministic generated message for a landed intent.
    ///
    /// FROZEN SHAPE — this is what makes the commit set reproducible from the
    /// log: a header line carrying the charter, a blank line, and a trailer
    /// embedding the `intent_id` (the spine of ①'s "reproducible from log"
    /// proof). Pure function of the intent, no clock / no randomness.
    pub fn generate_message(intent: &Intent) -> String {
        let mut msg = String::new();
        // header: the intent charter (human altitude reads this).
        msg.push_str(&intent.charter);
        msg.push_str("\n\n");
        // trailer: the machine-traceable provenance back to the intent.
        let _ = write!(msg, "Intent-Id: {}", intent.intent_id);
        msg
    }

    fn from_intent(intent: &Intent) -> Self {
        GitCommit {
            seq: intent.seq,
            intent_id: intent.intent_id.clone(),
            ref_name: intent.ref_name.clone(),
            target: intent.target.clone(),
            message: GitCommit::generate_message(intent),
        }
    }

    /// Recover the embedded `intent_id` from a generated message.
    ///
    /// Inverse of [`generate_message`](GitCommit::generate_message)'s trailer —
    /// proves the commit message *embeds* the id (① "commits embed intent_id"):
    /// the id is recoverable from the message bytes alone.
    pub fn intent_id_from_message(message: &str) -> Option<&str> {
        message
            .lines()
            .find_map(|l| l.strip_prefix("Intent-Id: "))
            .map(str::trim)
    }
}

/// One row of the machine altitude (`git log`): either a provenance-bearing
/// generated commit, or a provenance-free external change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionRow {
    /// A generated commit projected from a landed intent (carries provenance).
    Intent(GitCommit),
    /// A raw push: a ref-mutating event with **no** intent. It stays
    /// external-change forever — no synthetic intent is ever fabricated (④).
    ExternalChange {
        /// Log sequence of the originating raw-push event.
        seq: u64,
        /// The event kind (`ref.update` / `ref.delete`).
        kind: String,
        /// The ref the external change touched (if the payload named one).
        ref_name: Option<String>,
        /// The object id the external change set (absent for deletes / malformed).
        target: Option<String>,
    },
}

impl ProjectionRow {
    /// The originating log sequence number of this row.
    pub fn seq(&self) -> u64 {
        match self {
            ProjectionRow::Intent(c) => c.seq,
            ProjectionRow::ExternalChange { seq, .. } => *seq,
        }
    }

    /// Whether this row carries intent provenance (vs. being external-change).
    pub fn is_intent(&self) -> bool {
        matches!(self, ProjectionRow::Intent(_))
    }
}

/// The machine altitude (`git log`): the ordered projection of every
/// ref-mutating event on the log.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MachineHistory {
    rows: Vec<ProjectionRow>,
}

impl MachineHistory {
    /// The rows in log order (read-only view).
    pub fn rows(&self) -> &[ProjectionRow] {
        &self.rows
    }

    /// Number of ref-mutating rows.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether there are no ref-mutating rows.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// The generated commits (intent rows only), in log order.
    pub fn commits(&self) -> impl Iterator<Item = &GitCommit> {
        self.rows.iter().filter_map(|r| match r {
            ProjectionRow::Intent(c) => Some(c),
            ProjectionRow::ExternalChange { .. } => None,
        })
    }

    /// The external-change rows, in log order.
    pub fn external_changes(&self) -> impl Iterator<Item = &ProjectionRow> {
        self.rows.iter().filter(|r| !r.is_intent())
    }

    /// **Two-altitude consistency proof (②).**
    ///
    /// The intent altitude and the machine altitude can never disagree because
    /// one is derived from the other. Concretely, the intent rows of *this*
    /// machine history must be exactly the intents of `intent_altitude`, in the
    /// same order and with the same `(seq, intent_id, ref, target)` — and every
    /// non-intent row must be an external change (never a fabricated intent).
    ///
    /// Returns `true` iff the two zooms are consistent.
    pub fn is_consistent_with(&self, intent_altitude: &IntentLog) -> bool {
        let mut commits = self.commits();
        for intent in intent_altitude.intents() {
            let Some(commit) = commits.next() else {
                return false; // intent altitude has more intents than the machine altitude
            };
            if commit.seq != intent.seq
                || commit.intent_id != intent.intent_id
                || commit.ref_name != intent.ref_name
                || commit.target != intent.target
            {
                return false;
            }
            // the commit must actually embed the intent_id (① feeds ②).
            if GitCommit::intent_id_from_message(&commit.message) != Some(intent.intent_id.as_str())
            {
                return false;
            }
        }
        // no extra commits the intent altitude doesn't account for.
        commits.next().is_none()
    }
}

/// Error projecting the log to git.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProjectionError {
    /// Reading the intent altitude failed (fail-closed; see [`IntentModelError`]).
    Model(IntentModelError),
}

impl std::fmt::Display for ProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProjectionError::Model(e) => write!(f, "projection refused: {e}"),
        }
    }
}

impl std::error::Error for ProjectionError {}

impl From<IntentModelError> for ProjectionError {
    fn from(e: IntentModelError) -> Self {
        ProjectionError::Model(e)
    }
}

/// Project an event log to its **intent altitude** (`hugit log`).
///
/// Thin re-export of the model fold, kept on the projection surface so callers
/// get both altitudes from one module. Pure and deterministic.
pub fn project(log: &EventLog) -> Result<IntentLog, ProjectionError> {
    Ok(intents_from_records(log.records())?)
}

/// Project an event log to its **machine altitude** (`git log`).
///
/// One row per ref-mutating event, in chain order: landed intents become
/// generated commits embedding their `intent_id` (①); raw pushes become
/// external-change rows — never fabricated intents (④). Pure and deterministic,
/// so the commit set is reproducible from the log.
pub fn project_machine(log: &EventLog) -> Result<MachineHistory, ProjectionError> {
    let mut rows = Vec::new();
    for record in log.records() {
        if record.kind == INTENT_LANDED_KIND {
            let intent = parse_intent(record)?;
            rows.push(ProjectionRow::Intent(GitCommit::from_intent(&intent)));
        } else if is_raw_push(&record.kind) {
            rows.push(external_change_row(record));
        }
        // any other kind is inert: advances the chain, no machine-altitude row.
    }
    Ok(MachineHistory { rows })
}

/// Build an external-change row from a raw-push event. Best-effort payload read
/// (a malformed raw push is still recorded as external-change — it is never
/// promoted to an intent, and never dropped).
fn external_change_row(record: &EventRecord) -> ProjectionRow {
    let parsed: Option<serde_json::Value> = serde_json::from_str(&record.payload).ok();
    let ref_name = parsed
        .as_ref()
        .and_then(|v| v.get("ref"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    let target = parsed
        .as_ref()
        .and_then(|v| v.get("target"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    ProjectionRow::ExternalChange {
        seq: record.seq,
        kind: record.kind.clone(),
        ref_name,
        target,
    }
}
