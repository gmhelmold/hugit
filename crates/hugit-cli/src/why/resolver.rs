//! `why` provenance resolver (WP-D10 ①④).
//!
//! Resolves a file/line or symbol reference to the originating [`Intent`] +
//! the full attestation provenance (charter / author / model / cost) by
//! walking the event log. The answer is asserted to MATCH the event log —
//! there is no divergent second source.
//!
//! ## Derived-bytes honesty (item ④ / R6)
//!
//! When the queried file is regenerated/derived, `why` resolves to the
//! REGEN/DERIVATION event, and [`AuthorKind`] is set to
//! [`AuthorKind::Derived`]. It NEVER fabricates a human author for machine-
//! produced bytes.

use hugit_contracts::{AttestationChain, EventRecord, IntentSidecar};
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Query shape
// ---------------------------------------------------------------------------

/// A query to `hugit why`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhyQuery {
    /// The file path being queried.
    pub path: String,
    /// Optional 1-based line number within the file.
    pub line: Option<u64>,
    /// Optional symbol name (function / type / const).
    pub symbol: Option<String>,
}

// ---------------------------------------------------------------------------
// Answer shape
// ---------------------------------------------------------------------------

/// Whether the originating author was a human intent or a machine derivation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthorKind {
    /// A human-authored landed intent.  `principal_chain` names the humans /
    /// agents that produced it.
    Intent,
    /// Regenerated / derived bytes — produced by a regen/derivation event, not
    /// by a human. The event `kind` is stored in `event_kind`.
    Derived { event_kind: String },
}

/// The full provenance answer returned by `resolve_why`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProvenanceAnswer {
    // ── originating intent (item ①) ─────────────────────────────────────
    /// The stable intent_id of the originating intent, if this is an
    /// `AuthorKind::Intent`.  `None` for derived/regen events.
    pub intent_id: Option<String>,

    /// Human-readable charter describing what the intent/regen did.
    pub charter: String,

    /// Ordered chain of principals (humans / runners / models) that produced
    /// this event — the "author" field.
    pub author: Vec<String>,

    /// Model identifier that participated in the pipeline (`""` if none).
    pub model: String,

    /// Cost annotation as recorded on the attestation chain (`""` if none).
    ///
    /// Whitepaper §9: cost is carried in the attestation, not re-derived.
    pub cost: String,

    // ── event-log anchor ─────────────────────────────────────────────────
    /// Log sequence number of the originating event.
    pub event_seq: u64,

    /// Event kind string (`"intent.landed"`, `"regen.derived"`, etc.).
    pub event_kind: String,

    /// SHA-256 hex digest of the originating event (from the log).
    pub event_hash: String,

    // ── author kind (item ④ / R6) ────────────────────────────────────────
    /// Whether this is a human intent or a machine derivation.
    pub author_kind: AuthorKind,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from `resolve_why`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhyError {
    /// No event in the log matches the queried path.
    NotFound { path: String },
    /// An event payload could not be parsed.
    BadPayload { seq: u64 },
}

impl std::fmt::Display for WhyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WhyError::NotFound { path } => write!(f, "no provenance found for `{path}`"),
            WhyError::BadPayload { seq } => {
                write!(f, "malformed event payload at seq {seq}")
            }
        }
    }
}

impl std::error::Error for WhyError {}

// ---------------------------------------------------------------------------
// Event-log entry used by the resolver
// ---------------------------------------------------------------------------

/// A self-contained event-log entry presented to the resolver.
///
/// Combines the frozen [`EventRecord`] with the optional attestation chain
/// and sidecar that a real implementation would load from the refstore.
#[derive(Debug, Clone)]
pub struct LogEntry {
    pub record: EventRecord,
    pub attestation: Option<AttestationChain>,
    pub sidecar: Option<IntentSidecar>,
}

// ---------------------------------------------------------------------------
// Event-kind constants
// ---------------------------------------------------------------------------

/// The event kind for a landed intent (mirrors D4's `INTENT_LANDED_KIND`).
pub const INTENT_LANDED_KIND: &str = "intent.landed";

/// The event kind for a regen / derivation event.
pub const REGEN_DERIVED_KIND: &str = "regen.derived";

// ---------------------------------------------------------------------------
// resolve_why
// ---------------------------------------------------------------------------

/// Resolve a [`WhyQuery`] against a slice of log entries.
///
/// ## Algorithm
///
/// 1. Walk entries in reverse log order (most-recent first).
/// 2. For each entry, check whether its payload references `query.path`.
/// 3. The first matching entry is the originating event.
/// 4. Build a [`ProvenanceAnswer`] from the matching entry — never from a
///    second source.
/// 5. If the event kind is a regen/derivation kind, set
///    `author_kind = AuthorKind::Derived` — NEVER fabricate a human author
///    (item ④ / R6).
///
/// ## Fixture contract
///
/// `entries` is the complete log slice.  In production this would come from the
/// refstore; in tests it is a hand-crafted fixture that doubles as the oracle.
pub fn resolve_why(query: &WhyQuery, entries: &[LogEntry]) -> Result<ProvenanceAnswer, WhyError> {
    // Walk in reverse so we get the most-recent (=originating for the current
    // state) event.  For a file that was modified N times we return the latest
    // event that touches it — which is correct: `why` answers "what last
    // produced these bytes", not the full history.
    for entry in entries.iter().rev() {
        if payload_references_path(&entry.record.payload, &query.path) {
            return build_answer(entry).map_err(|_| WhyError::BadPayload {
                seq: entry.record.seq,
            });
        }
    }
    Err(WhyError::NotFound {
        path: query.path.clone(),
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn payload_references_path(payload: &str, path: &str) -> bool {
    // Parse the payload as JSON and look for a "path" or "files" key.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) {
        if let Some(p) = v.get("path").and_then(|x| x.as_str())
            && p == path
        {
            return true;
        }
        if let Some(files) = v.get("files").and_then(|x| x.as_array()) {
            for f in files {
                if f.as_str() == Some(path) {
                    return true;
                }
            }
        }
    }
    false
}

fn build_answer(entry: &LogEntry) -> Result<ProvenanceAnswer, ()> {
    let record = &entry.record;

    // Determine author kind (item ④ / R6).
    let author_kind = if is_derived_kind(&record.kind) {
        AuthorKind::Derived {
            event_kind: record.kind.clone(),
        }
    } else {
        AuthorKind::Intent
    };

    // Extract intent_id and charter from the payload (best-effort).
    let payload_val: serde_json::Value = serde_json::from_str(&record.payload).map_err(|_| ())?;
    let intent_id = payload_val
        .get("intent_id")
        .and_then(|v| v.as_str())
        .map(String::from);
    let charter = payload_val
        .get("charter")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    // Pull model + cost from the attestation chain (if provided).
    let (model, cost) = entry
        .attestation
        .as_ref()
        .map(|a| (a.model.clone(), a.def.clone()))
        .unwrap_or_default();

    Ok(ProvenanceAnswer {
        intent_id,
        charter,
        author: record.principal_chain.clone(),
        model,
        cost,
        event_seq: record.seq,
        event_kind: record.kind.clone(),
        event_hash: record.this_hash.clone(),
        author_kind,
    })
}

/// True when the event kind represents a regen / derivation (machine-produced)
/// event rather than a human-authored intent.
pub fn is_derived_kind(kind: &str) -> bool {
    kind == REGEN_DERIVED_KIND || kind.starts_with("regen.") || kind.starts_with("derived.")
}

// ---------------------------------------------------------------------------
// Fixture helpers (used by tests)
// ---------------------------------------------------------------------------

/// Build a minimal `EventRecord` for tests.
pub fn fixture_event(
    seq: u64,
    kind: &str,
    principal_chain: Vec<String>,
    payload: serde_json::Value,
) -> EventRecord {
    EventRecord {
        seq,
        prev_hash: format!("{:064x}", seq.saturating_sub(1)),
        this_hash: format!("{:064x}", seq),
        kind: kind.to_string(),
        principal_chain,
        payload: payload.to_string(),
        recorded_at: 1_700_000_000_000 + seq * 1000,
    }
}
