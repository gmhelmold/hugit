//! Native **Intent** objects read out of the append-only event log.
//!
//! An Intent is *not* a second store. It is one event kind on the D1 log:
//!
//! - [`INTENT_LANDED_KIND`] (`"intent.landed"`) — a landed intent. Its payload
//!   is canonical JSON
//!   `{"intent_id": <id>, "ref": <name>, "target": <oid>, "charter": <text>}`.
//!   The event simultaneously *is* the intent (provenance) and carries the ref
//!   mutation the intent landed (`ref` → `target`).
//!
//! Everything else on the log is **not** an intent. In particular the
//! [`RAW_PUSH_KINDS`] (`ref.update` / `ref.delete`, the kinds D1a's replay folds
//! and the kinds D3b's raw-push write path emits) are *external changes*: they
//! mutate refs without provenance. WP-D4 reads them as external-change and
//! **never** synthesises an intent for them — the D4 leg of the on-record
//! no-fake-intents adjudication (D3⑤+D4④+E2⑤).

use hugit_contracts::event_record::EventRecord;

use crate::log::EventLog;

/// The event kind that carries a native landed intent.
pub const INTENT_LANDED_KIND: &str = "intent.landed";

/// The ref-mutating event kinds that are **external changes**, never intents.
///
/// These are exactly the kinds D1a's [`crate::replay`] folds into ref state and
/// the kinds D3b's raw-push write path emits. Seeing one of these on the log is
/// an *external change*; WP-D4 never fabricates an intent from it.
pub const RAW_PUSH_KINDS: [&str; 2] = ["ref.update", "ref.delete"];

/// The **closed** set of event kinds a cross-crate raw external-change emitter is
/// allowed to append (C4-F1).
///
/// This is the type-level companion to [`RAW_PUSH_KINDS`]: the
/// [`EventLog::append_external_change`](crate::EventLog::append_external_change)
/// shim takes this enum, NOT a free `impl Into<String>`, so a cross-crate
/// raw-push recorder (`hugit-proto`, `hugit-mirror`) is *structurally incapable*
/// of emitting `intent.landed` / `pr.*` / `verdict.*`. It replaces the prior
/// runtime, debug-only `debug_assert_ne!` guard with a compile-time one — the
/// only legitimate cross-crate raw-append need once
/// [`EventLog::append`](crate::EventLog::append) is `pub(crate)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalChangeKind {
    /// A ref *update* — `ref.update` (a ref now points at a new target oid).
    RefUpdate,
    /// A ref *delete* — `ref.delete` (a ref was removed).
    RefDelete,
}

impl ExternalChangeKind {
    /// The frozen wire string for this external-change kind — always a member of
    /// [`RAW_PUSH_KINDS`], never an intent/pr/verdict kind.
    pub fn as_kind(self) -> &'static str {
        match self {
            ExternalChangeKind::RefUpdate => "ref.update",
            ExternalChangeKind::RefDelete => "ref.delete",
        }
    }
}

/// A native Intent object, projected out of one `intent.landed` event.
///
/// The `seq` ties the intent back to its exact position on the event log, so the
/// intent altitude and the machine altitude (both folds of the same log) order
/// identically and can never disagree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Intent {
    /// Log sequence number of the `intent.landed` event this intent was read
    /// from. Total-orders intents identically to the underlying log.
    pub seq: u64,
    /// Stable unique identifier of the intent (mirrors
    /// [`hugit_contracts::intent_sidecar::IntentSidecar::intent_id`]).
    pub intent_id: String,
    /// The ref this intent landed onto.
    pub ref_name: String,
    /// The object id the ref now points at.
    pub target: String,
    /// Human-readable charter of what the intent did.
    pub charter: String,
    /// The ordered principal chain that produced the landing event.
    pub principal_chain: Vec<String>,
    /// Unix epoch milliseconds the landing event was recorded.
    pub recorded_at: u64,
}

/// The **intent altitude**: every native intent on the log, in log order.
///
/// This is `hugit log` — a derived view, exactly like [`crate::replay::RefState`]
/// is a derived view. It is a fold of the same event log the machine altitude
/// folds, which is why the two altitudes can never disagree.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IntentLog {
    intents: Vec<Intent>,
}

impl IntentLog {
    /// An empty intent altitude.
    pub fn new() -> Self {
        Self::default()
    }

    /// The intents in log order (read-only view).
    pub fn intents(&self) -> &[Intent] {
        &self.intents
    }

    /// Number of landed intents.
    pub fn len(&self) -> usize {
        self.intents.len()
    }

    /// Whether no intents have landed.
    pub fn is_empty(&self) -> bool {
        self.intents.is_empty()
    }

    /// Look an intent up by its stable `intent_id`, if present.
    pub fn by_id(&self, intent_id: &str) -> Option<&Intent> {
        self.intents.iter().find(|i| i.intent_id == intent_id)
    }

    /// The set of all landed `intent_id`s on this altitude, materialised ONCE.
    ///
    /// O(n) to build, O(1) per membership query — the index a reconcile/sync
    /// pass uses to test "is this log-intent already present here?" without the
    /// O(n²) of a fresh [`by_id`](Self::by_id) linear scan (itself over a fresh
    /// [`intents_from_log`] re-parse) per candidate. Borrows the ids; the
    /// returned set lives no longer than `&self`.
    pub fn id_set(&self) -> std::collections::HashSet<&str> {
        self.intents.iter().map(|i| i.intent_id.as_str()).collect()
    }
}

/// Error reading native intents out of the event log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IntentModelError {
    /// An `intent.landed` event carried an unparseable / incomplete payload.
    /// Fail-closed: a corrupt intent instruction is never silently dropped.
    BadIntentPayload { seq: u64 },
}

impl std::fmt::Display for IntentModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IntentModelError::BadIntentPayload { seq } => {
                write!(f, "malformed intent.landed payload at seq {seq}")
            }
        }
    }
}

impl std::error::Error for IntentModelError {}

/// Read the **intent altitude** out of an event log.
///
/// Folds the log in chain order, lifting each `intent.landed` event into a
/// native [`Intent`]. Raw-push kinds ([`RAW_PUSH_KINDS`]) and inert events are
/// skipped — they are not intents and no intent is fabricated for them (④).
///
/// Pure and deterministic: equal logs yield equal [`IntentLog`]s.
pub fn intents_from_log(log: &EventLog) -> Result<IntentLog, IntentModelError> {
    intents_from_records(log.records())
}

/// The pure fold over raw records (shared by [`intents_from_log`] and the
/// projection, so the two altitudes read the *same* records the same way).
pub(crate) fn intents_from_records(records: &[EventRecord]) -> Result<IntentLog, IntentModelError> {
    let mut intents = Vec::new();
    for record in records {
        if record.kind == INTENT_LANDED_KIND {
            intents.push(parse_intent(record)?);
        }
    }
    Ok(IntentLog { intents })
}

/// Parse one `intent.landed` event into a native [`Intent`]. Fail-closed.
pub(crate) fn parse_intent(record: &EventRecord) -> Result<Intent, IntentModelError> {
    let bad = || IntentModelError::BadIntentPayload { seq: record.seq };
    let v: serde_json::Value = serde_json::from_str(&record.payload).map_err(|_| bad())?;
    let intent_id = field_str(&v, "intent_id").ok_or_else(bad)?;
    let ref_name = field_str(&v, "ref").ok_or_else(bad)?;
    let target = field_str(&v, "target").ok_or_else(bad)?;
    let charter = field_str(&v, "charter").ok_or_else(bad)?;
    Ok(Intent {
        seq: record.seq,
        intent_id: intent_id.to_string(),
        ref_name: ref_name.to_string(),
        target: target.to_string(),
        charter: charter.to_string(),
        principal_chain: record.principal_chain.clone(),
        recorded_at: record.recorded_at,
    })
}

/// Whether an event kind is a raw push (external change), never an intent.
pub(crate) fn is_raw_push(kind: &str) -> bool {
    RAW_PUSH_KINDS.contains(&kind)
}

fn field_str<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(serde_json::Value::as_str)
}
