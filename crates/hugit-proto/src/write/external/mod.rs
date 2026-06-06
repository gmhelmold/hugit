//! Raw push → **opaque external-change event with attribution** (WP-D3b item ③).
//!
//! A raw `git push` from a compatibility user is recorded on the D1 event log as
//! an *external change*: a ref mutation **without provenance**. It carries its
//! attribution — *who* (the principal chain), *when* (the recorded timestamp),
//! and *which ref* (name + target) — but it is **opaque**: it is never
//! interpreted as, projected into, or reverse-engineered into an Intent. Externals
//! stay external (command-catalog: "Raw `git push` from compat users = recorded
//! as opaque change-events, **never reverse-engineered into fake intents**").
//!
//! The emitted event uses exactly the
//! [`RAW_PUSH_KINDS`](hugit_refstore::intent::RAW_PUSH_KINDS) discriminators
//! (`ref.update` / `ref.delete`) that D1a's replay folds and D4 reads as
//! external-change. It is **never** the
//! [`INTENT_LANDED_KIND`](hugit_refstore::intent::INTENT_LANDED_KIND) — that is
//! the structural guarantee behind item ⑤ (no synthetic intent).

use hugit_contracts::event_record::EventRecord;
use hugit_refstore::EventLog;
use hugit_refstore::intent::{INTENT_LANDED_KIND, RAW_PUSH_KINDS};

/// The ref-mutating event kind emitted for a raw push that sets/updates a tip.
pub const REF_UPDATE_KIND: &str = "ref.update";
/// The ref-mutating event kind emitted for a raw push that deletes a tip.
pub const REF_DELETE_KIND: &str = "ref.delete";

/// Whether `kind` is an external-change (raw-push) kind, never an intent kind.
///
/// True iff `kind` is one of the consumed
/// [`RAW_PUSH_KINDS`](hugit_refstore::intent::RAW_PUSH_KINDS) and not the
/// [`INTENT_LANDED_KIND`]. The emitted kinds can never drift from D1/D4's surface.
pub fn is_external_change_kind(kind: &str) -> bool {
    RAW_PUSH_KINDS.contains(&kind) && kind != INTENT_LANDED_KIND
}

/// The mutation a raw push asks for: set a ref to a target, or delete it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawPush {
    /// Set / update `ref_name` to point at `target` (a git object id).
    Update {
        /// The ref the push moves (e.g. `refs/heads/main`).
        ref_name: String,
        /// The object id the ref is moved to.
        target: String,
    },
    /// Delete `ref_name`.
    Delete {
        /// The ref the push removes.
        ref_name: String,
    },
}

impl RawPush {
    /// The ref this push touches (present for both update and delete).
    pub fn ref_name(&self) -> &str {
        match self {
            RawPush::Update { ref_name, .. } | RawPush::Delete { ref_name } => ref_name,
        }
    }

    /// The external-change event *kind* for this push (a [`RAW_PUSH_KINDS`] value,
    /// never [`INTENT_LANDED_KIND`]).
    pub fn kind(&self) -> &'static str {
        match self {
            RawPush::Update { .. } => REF_UPDATE_KIND,
            RawPush::Delete { .. } => REF_DELETE_KIND,
        }
    }

    /// The canonical JSON payload of the external-change event. The `ref`/`target`
    /// shape is exactly what D1a's replay folds, so a raw push projects correctly
    /// into the derived ref view.
    fn payload(&self) -> String {
        match self {
            RawPush::Update { ref_name, target } => {
                format!(
                    r#"{{"ref":{},"target":{}}}"#,
                    json_str(ref_name),
                    json_str(target)
                )
            }
            RawPush::Delete { ref_name } => {
                format!(r#"{{"ref":{}}}"#, json_str(ref_name))
            }
        }
    }
}

/// The attribution carried by a recorded external change: who, when, which ref.
///
/// Read back off the appended [`EventRecord`] so callers can prove a raw push is
/// attributed (item ③) without re-deriving anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attribution {
    /// *Who*: the ordered principal chain that produced the push.
    pub principal_chain: Vec<String>,
    /// *When*: unix epoch milliseconds the change was recorded.
    pub recorded_at: u64,
    /// *Which ref* the change touched.
    pub ref_name: String,
    /// The opaque event kind (a raw-push kind, never an intent kind).
    pub kind: String,
}

impl Attribution {
    /// Whether this attribution names at least one principal (who pushed).
    pub fn is_attributed(&self) -> bool {
        !self.principal_chain.is_empty()
    }
}

/// Why a raw push could not be recorded as an external change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalChangeError {
    /// The push carried no principal — an external change must be attributable.
    /// Fail-closed: an unattributed mutation is refused, never recorded blind.
    MissingAttribution,
}

impl std::fmt::Display for ExternalChangeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExternalChangeError::MissingAttribution => {
                write!(
                    f,
                    "raw push refused: external change must carry attribution"
                )
            }
        }
    }
}

impl std::error::Error for ExternalChangeError {}

/// Record a raw push as an opaque external-change event **with attribution**.
///
/// Appends one [`RAW_PUSH_KINDS`] event to `log` carrying the pusher's
/// `principal_chain` (who), `recorded_at` (when), and the ref/target (which). The
/// event is opaque: no intent is created, no intent kind is emitted. Returns the
/// appended record and its read-back [`Attribution`].
///
/// Fail-closed: a push with an empty principal chain is refused
/// ([`ExternalChangeError::MissingAttribution`]) — an external change must be
/// attributable.
pub fn record_external_change(
    log: &mut EventLog,
    push: &RawPush,
    principal_chain: Vec<String>,
    recorded_at: u64,
) -> Result<(EventRecord, Attribution), ExternalChangeError> {
    if principal_chain.is_empty() {
        return Err(ExternalChangeError::MissingAttribution);
    }

    let record = log.append(push.kind(), principal_chain, push.payload(), recorded_at);

    // Structural guarantee for ⑤: the recorded kind is an external-change kind,
    // never an intent. An attempt to record an intent kind here is impossible —
    // `push.kind()` only ever returns a RAW_PUSH_KINDS value.
    debug_assert_ne!(record.kind, INTENT_LANDED_KIND);

    let attribution = Attribution {
        principal_chain: record.principal_chain.clone(),
        recorded_at: record.recorded_at,
        ref_name: push.ref_name().to_string(),
        kind: record.kind.clone(),
    };
    Ok((record, attribution))
}

/// Minimal JSON string escaping (quotes + backslash + control chars) so the
/// canonical payload is valid JSON for the same parser D1a's replay uses.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
