//! The shared **canonical event-log** seam for `hugit intent new --log` (PC4).
//!
//! The `--log` file is the ONE canonical on-disk shape every porcelain verb
//! reads and writes: a JSON `[EventRecord, …]` array — the engine's
//! [`hugit_refstore::EventLog`] shape (the same the `hugit pr` and `hugit
//! campaign` verbs use). `intent new --log` lands its `intent.landed` record
//! onto this file through the REAL [`import_sidecar`] append path, so a later
//! `pr open --intent <id>` can validate the intent exists. The append is
//! idempotent on the log: an intent already landed there is left untouched.

use std::path::Path;

use hugit_contracts::IntentSidecar;
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::intent::{import_sidecar, intents_from_log};
use hugit_refstore::{EventLog, verify_chain};

use super::error::PorcelainError;

/// The synthetic ref an authored-not-pushed intent lands onto (mirrors the
/// store's `AUTHORED_REF` — fixture world until a real push binds a git ref).
const AUTHORED_REF: &str = "refs/hugit/intents";

/// Land `sidecar`'s `intent.landed` record on the canonical `[EventRecord, …]`
/// log at `path`, persisting it back. Idempotent: if the intent id is already on
/// the log, the file is left untouched (no duplicate, no error). A missing file
/// is a fresh empty log (so the first `intent new --log` bootstraps it).
pub fn land_intent(
    path: &Path,
    sidecar: &IntentSidecar,
    principal_chain: &[String],
    recorded_at: u64,
) -> Result<(), PorcelainError> {
    let mut log = load(path)?;

    // Idempotent on the shared log: an intent already landed here is left as-is.
    let already = intents_from_log(&log)
        .map(|p| p.by_id(&sidecar.intent_id).is_some())
        .unwrap_or(false);
    if already {
        return Ok(());
    }

    import_sidecar(
        &mut log,
        sidecar,
        AUTHORED_REF,
        &authored_target(&sidecar.intent_id),
        principal_chain.to_vec(),
        recorded_at,
    )
    .map_err(|e| {
        PorcelainError::new(
            "invalid_intent",
            e.to_string(),
            "ensure --charter is set and the intent id is non-empty",
        )
    })?;

    persist(path, &log)
}

/// Load the canonical `[EventRecord, …]` log at `path`, rehydrating + verifying
/// the hash chain. A non-existent path is an EMPTY log; any other
/// read/parse/chain fault fails closed with a structured error.
fn load(path: &Path) -> Result<EventLog, PorcelainError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(EventLog::new()),
        Err(e) => {
            return Err(PorcelainError::new(
                "io",
                format!("read log {}: {e}", path.display()),
                "check the --log path exists and is readable",
            ));
        }
    };
    let records: Vec<EventRecord> = serde_json::from_slice(&bytes).map_err(|e| {
        PorcelainError::new(
            "parse",
            format!("parse log {}: {e}", path.display()),
            "the --log file must be a canonical JSON [EventRecord, …] array \
             (the engine's EventLog shape, shared by every porcelain verb)",
        )
    })?;
    let mut log = EventLog::new();
    for record in records {
        log.push_record(record).map_err(|e| {
            PorcelainError::new(
                "rehydrate",
                format!("rehydrate log {}: {e}", path.display()),
                "the --log file's records must form a gap-free, monotonic chain",
            )
        })?;
    }
    verify_chain(log.records()).map_err(|e| {
        PorcelainError::new(
            "chain_broken",
            format!("log {} failed integrity verification: {e}", path.display()),
            "the --log file's hash chain is tampered or corrupt",
        )
    })?;
    Ok(log)
}

/// Persist the log back to `path` as a pretty `[EventRecord, …]` array.
fn persist(path: &Path, log: &EventLog) -> Result<(), PorcelainError> {
    let bytes = serde_json::to_vec_pretty(log.records()).map_err(|e| {
        PorcelainError::new(
            "internal",
            format!("serialise log: {e}"),
            "this is an internal bug; report it",
        )
    })?;
    std::fs::write(path, bytes).map_err(|e| {
        PorcelainError::new(
            "io",
            format!("write log {}: {e}", path.display()),
            "check the --log path is writable",
        )
    })
}

/// The synthetic target oid for an authored-not-pushed intent (mirrors the
/// store's helper — honest `authored:<id>` tag, never a fake 40-hex git sha).
fn authored_target(intent_id: &str) -> String {
    format!("authored:{intent_id}")
}
