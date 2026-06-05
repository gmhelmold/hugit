//! Import of a B6 [`IntentSidecar`] corpus into the native intent model
//! **by `intent_id`** — one lifecycle, one id (③; cf. X9③).
//!
//! A [`IntentSidecar`] is the frozen, PR-attached, *non-authoritative* intent
//! metadata B6 produces and stores to CAS by `intent_id`. Importing it is the
//! act of landing that intent onto the event log: we emit one `intent.landed`
//! event keyed by the **same `intent_id`**, so the imported corpus and the
//! native intent share one identity and one lifecycle. The sidecar's own
//! `authoritative` flag stays `false` (it never gates landing); the event log
//! remains the single source of truth.
//!
//! Import is purely additive over D1a's frozen append API
//! ([`EventLog::append`]): it appends, it never rewrites.
//!
//! [`IntentSidecar`]: hugit_contracts::intent_sidecar::IntentSidecar
//! [`EventLog::append`]: crate::log::EventLog::append

use hugit_contracts::event_record::EventRecord;
use hugit_contracts::intent_sidecar::IntentSidecar;

use crate::intent::model::INTENT_LANDED_KIND;
use crate::log::EventLog;

/// Error importing a sidecar corpus.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    /// The sidecar's `intent_id` is empty — there is no identity to import by.
    EmptyIntentId,
    /// An intent with this `intent_id` is already on the log. One lifecycle,
    /// one id: re-importing the same id is refused (fail-closed, idempotent at
    /// the call site — the caller decides whether to skip).
    DuplicateIntentId { intent_id: String },
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::EmptyIntentId => write!(f, "sidecar has an empty intent_id"),
            ImportError::DuplicateIntentId { intent_id } => {
                write!(f, "intent_id {intent_id} already landed on the log")
            }
        }
    }
}

impl std::error::Error for ImportError {}

/// Import one [`IntentSidecar`] corpus into the native intent model by landing
/// it onto the event log **keyed by its `intent_id`** (③).
///
/// Appends a single `intent.landed` event whose payload carries the sidecar's
/// `intent_id` and charter alongside the `(ref, target)` the import lands onto,
/// then returns the appended [`EventRecord`]. After this call the imported
/// corpus is visible at *both* altitudes
/// ([`crate::intent::project`] / [`crate::intent::project_machine`]) under the
/// same `intent_id` — one lifecycle, one id.
///
/// Fails closed on an empty id, and refuses to import an `intent_id` that is
/// already on the log (no duplicate identities).
pub fn import_sidecar(
    log: &mut EventLog,
    sidecar: &IntentSidecar,
    ref_name: &str,
    target: &str,
    principal_chain: Vec<String>,
    recorded_at: u64,
) -> Result<EventRecord, ImportError> {
    if sidecar.intent_id.is_empty() {
        return Err(ImportError::EmptyIntentId);
    }
    if already_landed(log, &sidecar.intent_id) {
        return Err(ImportError::DuplicateIntentId {
            intent_id: sidecar.intent_id.clone(),
        });
    }

    let payload = serde_json::json!({
        "intent_id": sidecar.intent_id,
        "ref": ref_name,
        "target": target,
        "charter": sidecar.charter,
    })
    .to_string();

    Ok(log.append(INTENT_LANDED_KIND, principal_chain, payload, recorded_at))
}

/// Whether an `intent.landed` event with this `intent_id` already exists.
fn already_landed(log: &EventLog, intent_id: &str) -> bool {
    log.records().iter().any(|r| {
        r.kind == INTENT_LANDED_KIND
            && serde_json::from_str::<serde_json::Value>(&r.payload)
                .ok()
                .and_then(|v| {
                    v.get("intent_id")
                        .and_then(serde_json::Value::as_str)
                        .map(|id| id == intent_id)
                })
                .unwrap_or(false)
    })
}
