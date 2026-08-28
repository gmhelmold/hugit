//! The shared **canonical event-log** seam for `hugit intent new --log` (PC4).
//!
//! The `--log` file is the ONE canonical on-disk shape every porcelain verb
//! reads and writes: a JSON `[EventRecord, …]` array — the engine's
//! [`hugit_refstore::EventLog`] shape (the same the `hugit pr` and `hugit
//! campaign` verbs use). `intent new --log` lands its `intent.landed` record
//! onto this file through the REAL [`import_sidecar`] append path, so a later
//! `pr open --intent <id>` can validate the intent exists. The append is
//! idempotent on the log: an intent already landed there is left untouched.
//!
//! ## D14 routing (WA2b / WB-INT)
//!
//! The `intent.landed` append routes through
//! [`EventLog::append_authorized`](hugit_refstore::EventLog::append_authorized)
//! with:
//!
//! - **endpoint** = [`Endpoint::Push`] — the universal git verb; every
//!   principal class is permitted, so intent authorship by workers/subagents is
//!   LEGAL at this altitude (the matrix allows it — whitepaper §7 "Everyone …
//!   every git command").
//! - **class** = extracted from the `principal_chain` tail (falls back to
//!   [`PrincipalClass::Worker`] when the tail is unclassifiable — honest
//!   disclosure of the authn limit; see [`authz`](hugit_refstore::authz) module
//!   doc).
//!
//! The guard ALLOWS the append and still audits the path — the denial arm is
//! unreachable for `Push`, but the guard is wired so the mutation path is never
//! un-gated.

use std::path::Path;

use hugit_contracts::IntentSidecar;
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::EventLog;
use hugit_refstore::authz::{Endpoint, PrincipalClass};
use hugit_refstore::intent::{INTENT_LANDED_KIND, intents_from_log};

use super::error::PorcelainError;
use crate::pr::filelock::{self, FileLock, LockError};

/// The synthetic ref an authored-not-pushed intent lands onto (mirrors the
/// store's `AUTHORED_REF` — fixture world until a real push binds a git ref).
const AUTHORED_REF: &str = "refs/hugit/intents";

/// Map a [`LockError`] into the structured porcelain error.
///
/// A live holder is `log_busy` (retry-able — another verb owns the seam, so the
/// read-modify-write is serialized, never a clobber: the TOCTOU is dead); an
/// I/O fault is the existing `io` kind.
fn lock_porcelain_error(e: LockError) -> PorcelainError {
    match e {
        LockError::Busy { path } => PorcelainError::new(
            "log_busy",
            format!(
                "the --log file {} is locked by another hugit verb",
                path.display()
            ),
            "another `hugit` process holds the log lock; retry once it releases \
             (a stale lock is auto-reclaimed after a short window)",
        ),
        LockError::Io { .. } => PorcelainError::new(
            "io",
            e.to_string(),
            "check the --log path is on a writable directory",
        ),
    }
}

/// Land `sidecar`'s `intent.landed` record on the canonical `[EventRecord, …]`
/// log at `path`, persisting it back. Idempotent: if the intent id is already on
/// the log, the file is left untouched (no duplicate, no error). A missing file
/// is a fresh empty log (so the first `intent new --log` bootstraps it).
///
/// The append is routed through
/// [`EventLog::append_authorized`] with `Endpoint::Push` (the universal verb,
/// allowed for every principal class) and the actor class extracted from the
/// tail of `principal_chain`.  The D14 guard is wired — it ALLOWS the `Push`
/// and still audits the path, fulfilling WA2b / WB-INT item ④.
pub fn land_intent(
    path: &Path,
    sidecar: &IntentSidecar,
    principal_chain: &[String],
    recorded_at: u64,
) -> Result<(), PorcelainError> {
    // Acquire the advisory exclusive lock BEFORE the load and hold it across the
    // whole load→mutate→persist (the `_lock` guard releases on Drop / on any
    // early return). Two concurrent `intent new --log` on one file now serialize
    // or fail structured (`log_busy`) — never silently clobber (the TOCTOU is
    // dead). See [`crate::pr::filelock`].
    let _lock = FileLock::acquire(path).map_err(lock_porcelain_error)?;

    let mut log = load(path)?;

    // ── Terminal-seal precondition (C5-F2) ────────────────────────────────────
    // A sealed campaign is terminal: `intent new --log` must NOT append an
    // `intent.landed` into a campaign that already has a `campaign.closed` on
    // this log (Round 8 F2 — post-seal `intent.landed` mutated `done` AFTER the
    // immutable seal). Route through the SAME shared chokepoint `verdict` and
    // `pr` use, so no campaign-scoped verb can drift on the seal rule. The
    // campaign key is the one this intent is being filed under — `intent new`
    // stamps `campaign:<key>` at the head of the principal chain. This guard
    // runs BEFORE both the idempotent-reconcile and the fresh-landing branches,
    // so a sealed campaign refuses every `intent new --log` into it.
    let campaign_key = principal_chain
        .iter()
        .find_map(|s| s.strip_prefix("campaign:"))
        .unwrap_or("");
    if !campaign_key.is_empty()
        && let Err(v) = crate::campaign::seal_guard::guard_not_sealed(&log, campaign_key)
    {
        let safe_campaign = crate::redaction::scrub(&v.campaign);
        return Err(PorcelainError::new(v.kind(), v.message(), v.fix())
            .with_context("campaign", serde_json::json!(safe_campaign)));
    }

    // Idempotent on the shared log: an intent already landed here is left as-is.
    let already = intents_from_log(&log)
        .map(|p| p.by_id(&sidecar.intent_id).is_some())
        .unwrap_or(false);
    if already {
        return Ok(());
    }

    import_sidecar_authorized(
        &mut log,
        sidecar,
        AUTHORED_REF,
        &authored_target(&sidecar.intent_id),
        principal_chain.to_vec(),
        recorded_at,
    )?;

    persist(path, &log)
}

/// Append one `intent.landed` event through the D14 authorization guard
/// (endpoint = `Push`, the universal verb — allowed for every principal class
/// including `Worker`/`agent:`).
///
/// This is the guarded path that makes the mutation surface unbypassable:
/// even though `Push` is always ALLOWED, the guard is wired so the path is
/// never un-gated and denials are auditable.
///
/// The principal class is inferred from the tail of `principal_chain`; when
/// unclassifiable the call falls back to [`PrincipalClass::Worker`] — the
/// honest default for a subagent whose identity hasn't been bound by P2 yet.
fn import_sidecar_authorized(
    log: &mut EventLog,
    sidecar: &IntentSidecar,
    ref_name: &str,
    target: &str,
    principal_chain: Vec<String>,
    recorded_at: u64,
) -> Result<(), PorcelainError> {
    if sidecar.intent_id.is_empty() {
        return Err(PorcelainError::new(
            "invalid_intent",
            "sidecar has an empty intent_id",
            "ensure --charter is set and the intent id is non-empty",
        ));
    }

    // Classify the actor (the tail of the principal chain) for the D14 guard.
    // Falls back to Worker — the intent-class actor — when unclassifiable
    // (honest P2-seam disclosure: classification is caller-supplied today).
    let actor_class = principal_chain
        .last()
        .and_then(|id| PrincipalClass::classify(id))
        .unwrap_or(PrincipalClass::Worker);

    // Extract the campaign key from the principal chain (WG-COHERENCE/B3):
    // `intent new` stamps `campaign:<key>` first. The ledger keys `intent.landed`
    // by this field; an absent campaign falls back to "default", making
    // `by_campaign(key)` find nothing and `proven` stick at 0. Carrying it here
    // ensures the ledger files the entry under the correct campaign.
    let campaign = principal_chain
        .iter()
        .find(|s| s.starts_with("campaign:"))
        .and_then(|s| s.strip_prefix("campaign:"))
        .unwrap_or("")
        .to_string();

    // Build the intent.landed payload (carries `campaign` when present, B3) and
    // SCRUB-ON-APPEND (WG-SCRUB): route every user string value through the
    // central scrubbing helper BEFORE the bytes reach the hash chain — the
    // structural backstop so this append can never leak a secret.
    let payload_value = if campaign.is_empty() {
        serde_json::json!({
            "intent_id": sidecar.intent_id,
            "ref": ref_name,
            "target": target,
            "charter": sidecar.charter,
        })
    } else {
        serde_json::json!({
            "intent_id": sidecar.intent_id,
            "ref": ref_name,
            "target": target,
            "charter": sidecar.charter,
            "campaign": campaign,
        })
    };
    let payload = crate::porcelain::scrub_to_canonical(payload_value);

    // Route through append_authorized (Endpoint::Push — always Allow; guard
    // is wired so no mutation is ever un-gated).
    log.append_authorized(
        actor_class,
        Endpoint::Push,
        INTENT_LANDED_KIND,
        principal_chain,
        payload,
        recorded_at,
    )
    .map_err(|e| {
        // This arm is unreachable for Push (all classes are allowed), but we
        // map it honestly for completeness — fail-closed if the matrix ever
        // changes.
        PorcelainError::new(
            "authz_denied",
            e.to_string(),
            "the acting principal is not permitted to author intents at this altitude",
        )
    })?;

    Ok(())
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
    // PS-13: rehydrate + verify through the SINGLE chokepoint
    // (`checks::rehydrate_and_verify`); a tampered chain fails closed as
    // `chain_broken`/exit-2, never re-implementing the verify loop here.
    crate::checks::rehydrate_and_verify(records).map_err(|fault| match fault {
        crate::checks::ChainLoadFault::Rehydrate(e) => PorcelainError::new(
            "rehydrate",
            format!("rehydrate log {}: {e}", path.display()),
            "the --log file's records must form a gap-free, monotonic chain",
        ),
        crate::checks::ChainLoadFault::ChainBroken(e) => PorcelainError::new(
            "chain_broken",
            format!("log {} failed integrity verification: {e}", path.display()),
            "the --log file's hash chain is tampered or corrupt",
        ),
    })
}

/// Persist the log back to `path` as a pretty `[EventRecord, …]` array, via the
/// **atomic** temp-file-then-rename write (held under the caller's lock). A
/// reader or a crash sees either the whole old file or the whole new one — never
/// a half-written, truncated log.
fn persist(path: &Path, log: &EventLog) -> Result<(), PorcelainError> {
    let bytes = serde_json::to_vec_pretty(log.records()).map_err(|e| {
        // PR-5: this is a bug-class internal fault — exit 1 via the internal flag.
        PorcelainError::internal(format!("serialise log: {e}"))
    })?;
    filelock::atomic_write(path, &bytes).map_err(lock_porcelain_error)
}

/// The synthetic target oid for an authored-not-pushed intent (mirrors the
/// store's helper — honest `authored:<id>` tag, never a fake 40-hex git sha).
fn authored_target(intent_id: &str) -> String {
    format!("authored:{intent_id}")
}
