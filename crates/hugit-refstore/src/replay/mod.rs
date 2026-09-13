//! Deterministic projection of the event log into a **ref state**.
//!
//! Refs are a *derived view*, never primary state. A [`RefState`] is the fold of
//! [`replay`] over the log's records, in chain order. Replay is pure and
//! deterministic: the same sequence of records always yields the same
//! `RefState`, byte-for-byte (this is what acceptance item ① asserts at 10k
//! events).
//!
//! Replay is **fail-closed**: it first re-verifies the hash chain
//! ([`crate::tamper::verify_chain`]) and refuses to project a ref state from a
//! broken chain. A derived view is never served off a tampered log.
//!
//! # Ref event grammar (the deterministic fold)
//!
//! The projection reads the **ref-mutating** event kinds; every other kind is
//! inert (it advances the chain but does not touch ref state — forward-
//! compatible by construction):
//!
//! - `ref.update` — a commit payload is canonical JSON
//!   `{"ref": <name>, "target": <oid>}` and sets `name -> oid` (insert or
//!   overwrite). Hook captures may instead carry `checkout:true` or
//!   `attempt:true`; those are valid inert observations and do not mutate refs.
//! - `ref.delete` — payload is canonical JSON `{"ref": <name>}`; removes `name`.
//! - `intent.landed` — payload is canonical JSON
//!   `{"intent_id": <id>, "ref": <name>, "target": <oid>, "charter": <text>}`; it
//!   *also* advances `name -> oid` (it is the kind D4's land/import path emits and
//!   the kind the machine altitude projects as a ref-advancing commit). A landed
//!   intent is a ref mutation, so it MUST fold like a `ref.update` — folding it as
//!   inert silently lost intent-set refs from the projection (the 2026-06-07
//!   review's undo-compensator defect). The extra payload fields are ignored here;
//!   the intent altitude ([`crate::intent`]) reads them.
//!
//! A malformed payload for any ref-mutating kind is a [`ReplayError::BadPayload`]
//! (fail-closed: a corrupt instruction is never silently dropped).

use crate::intent::model::INTENT_LANDED_KIND;
use crate::log::EventLog;
use crate::tamper::{TamperError, verify_chain};
use hugit_contracts::event_record::EventRecord;
use std::collections::BTreeMap;

/// The derived ref view: an ordered map of ref name → target object id.
///
/// Backed by a [`BTreeMap`] so the projection is canonical (key-sorted) and two
/// equal logs serialize byte-for-byte identically.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefState {
    refs: BTreeMap<String, String>,
}

impl RefState {
    /// An empty ref state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a ref to its target object id, if present.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.refs.get(name).map(String::as_str)
    }

    /// Number of live refs.
    pub fn len(&self) -> usize {
        self.refs.len()
    }

    /// Whether no refs are set.
    pub fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }

    /// Iterate refs in canonical (name-sorted) order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.refs.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    /// Canonical, deterministic serialization of the ref state.
    ///
    /// Newline-delimited `"<name> <target>"` lines in name-sorted order. Equal
    /// ref states produce identical bytes — the spine of the ① 10k-event
    /// replay-identity proof.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut out = String::new();
        for (name, target) in &self.refs {
            out.push_str(name);
            out.push(' ');
            out.push_str(target);
            out.push('\n');
        }
        out.into_bytes()
    }
}

/// Error from projecting the log into a ref state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplayError {
    /// The hash chain failed verification; the derived view is refused
    /// (fail-closed — see [`TamperError`]).
    Tamper(TamperError),
    /// A ref-mutating event carried an unparseable / malformed payload.
    BadPayload { seq: u64, kind: String },
}

impl std::fmt::Display for ReplayError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReplayError::Tamper(e) => write!(f, "replay refused: {e}"),
            ReplayError::BadPayload { seq, kind } => {
                write!(f, "malformed payload for {kind} event at seq {seq}")
            }
        }
    }
}

impl std::error::Error for ReplayError {}

impl From<TamperError> for ReplayError {
    fn from(e: TamperError) -> Self {
        ReplayError::Tamper(e)
    }
}

/// Replay a log into its derived [`RefState`].
///
/// Re-verifies the hash chain first (fail-closed), then folds the records in
/// chain order. Pure and deterministic: equal record sequences always yield
/// equal `RefState`s.
pub fn replay(log: &EventLog) -> Result<RefState, ReplayError> {
    verify_chain(log.records())?;
    replay_unchecked(log.records())
}

/// The pure fold, with the chain integrity check already discharged by the
/// caller. Exposed for the verifier and for replay-identity proofs that want to
/// fold the same records twice without re-verifying each time.
pub fn replay_unchecked(records: &[EventRecord]) -> Result<RefState, ReplayError> {
    let mut state = RefState::new();
    for record in records {
        match record.kind.as_str() {
            // A landed intent advances `ref -> target` exactly like a ref.update
            // (it carries extra intent fields the intent altitude reads, ignored
            // here). Unifying it with ref.update is the fix for the undo-
            // compensator defect: a ref last set by a landed intent must survive
            // into the prior-state projection.
            "ref.update" | INTENT_LANDED_KIND => {
                let v: serde_json::Value = parse_payload(record)?;
                // CHECKOUT and PUSH-ATTEMPT ref.updates are INERT observations:
                // they record worktree movement or a proposed push, but never
                // advance a ref without the commit capture's ref/target pair.
                // Treating either as a ref mutation would fail closed on a valid
                // captured Git activity log during replay/export.
                // Checkout schema evolved from `checkout:true` to a fact/truth
                // object with additive old/new aliases. Both forms stay inert.
                let inert_capture = v.get("checkout").is_some()
                    || v.get("attempt").and_then(serde_json::Value::as_bool) == Some(true)
                    || v.get("reference_transaction").is_some()
                    || v.get("reference_transaction_pairing").is_some();
                if inert_capture {
                    continue;
                }
                let name = field_str(&v, "ref").ok_or_else(|| bad(record))?;
                let target = field_str(&v, "target").ok_or_else(|| bad(record))?;
                state.refs.insert(name.to_string(), target.to_string());
            }
            "ref.delete" => {
                let v: serde_json::Value = parse_payload(record)?;
                let name = field_str(&v, "ref").ok_or_else(|| bad(record))?;
                state.refs.remove(name);
            }
            // Any other kind is inert: it advances the chain but does not touch
            // ref state (forward-compatible by construction).
            _ => {}
        }
    }
    Ok(state)
}

fn parse_payload(record: &EventRecord) -> Result<serde_json::Value, ReplayError> {
    serde_json::from_str(&record.payload).map_err(|_| bad(record))
}

fn field_str<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(serde_json::Value::as_str)
}

fn bad(record: &EventRecord) -> ReplayError {
    ReplayError::BadPayload {
        seq: record.seq,
        kind: record.kind.clone(),
    }
}
