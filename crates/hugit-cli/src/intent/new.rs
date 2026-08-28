//! `hugit intent new` — author an intent through the real refstore path
//! (WP-PC2).
//!
//! An intent is the unit of orchestrated work: a **charter**, an **acceptance**
//! list, optionally bound to a **campaign**, authored normally by a SUBAGENT.
//! `new` builds the frozen [`IntentSidecar`] (the intent record), lands it onto
//! the local event log through the real
//! [`import_sidecar`](hugit_refstore::intent::import_sidecar) path — the same
//! seam the dogfood wave drives — and prints the stable result object:
//!
//! ```json
//! {"intent_id":"…","already_exists":false}
//! ```
//!
//! ## Idempotency (decided)
//!
//! Agents retry safely. The `intent_id` is either explicit (`--id`) or a
//! deterministic content hash of the charter / acceptance / campaign / author —
//! so an identical re-run resolves to the SAME id. `import_sidecar` refuses a
//! `DuplicateIntentId`; we catch that and return the existing id with
//! `already_exists:true`, exit 0 (never a fake second landing, never an error).
//!
//! ## Authorship
//!
//! An intent is authored by a subagent in the normal flow. `--agent <type>`
//! records the authoring agent type on the landing event's principal chain;
//! the honest default is `"main"` (the orchestrator), never an invented agent.

use std::path::{Path, PathBuf};

use hugit_contracts::IntentSidecar;
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::EventLog;
use hugit_refstore::intent::{import_sidecar, intents_from_log};
use sha2::{Digest, Sha256};

use super::canonical_log;
use super::error::PorcelainError;
use super::store::{IntentStore, StoreError};

/// The honest default authoring agent type when `--agent` is not given.
pub const DEFAULT_AGENT: &str = "main";

/// The synthetic ref an authored (not-yet-pushed) intent lands onto in the
/// local hermetic store. Labelled as synthetic — fixture world until a real
/// push binds a git ref (no fake sha, same posture as the dogfood harness's
/// `dogfood:…` commit labels).
const AUTHORED_REF: &str = "refs/hugit/intents";

/// The inputs to `intent new` (transcribed from the binary's clap args so the
/// library owns the behavior, the binary owns only parsing).
pub struct NewIntent {
    /// Human-readable charter / description of what the intent intends to do.
    pub charter: String,
    /// The campaign key this intent is bound to.
    pub campaign: String,
    /// Acceptance criteria (the repeatable `--acceptance` items).
    pub acceptance: Vec<String>,
    /// Explicit intent id (idempotency key). `None` ⇒ a deterministic content
    /// hash is derived.
    pub id: Option<String>,
    /// The authoring agent type. `None` ⇒ [`DEFAULT_AGENT`].
    pub agent: Option<String>,
    /// Optional content-addressed ref to the full context blob (the envelope).
    pub context_ref: Option<String>,
    /// Optional **shared canonical event log** (`--log`). When given, the
    /// `intent.landed` record is ALSO appended to this `[EventRecord, …]` file —
    /// the same canonical seam `hugit pr` and `hugit campaign` read — so a later
    /// `pr open --intent <id>` can validate the intent exists. `--store`-only
    /// invocation (no `--log`) keeps working: the log is optional.
    pub log: Option<PathBuf>,
}

/// The stable result object printed by `intent new`.
///
/// **Key-set invariant (WB0):** the same fields are present on EVERY run,
/// whether this is a first landing (`already_exists:false`) or an idempotent
/// re-run (`already_exists:true`).  No field is ever silently dropped.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct NewResult {
    /// The id of the (now-landed) intent.
    pub intent_id: String,
    /// `true` when the intent already existed (idempotent re-run), else `false`.
    pub already_exists: bool,
    /// The campaign this intent is bound to (same value, first-run and re-run).
    pub campaign: String,
    /// The authoring agent token (same value, first-run and re-run).
    pub agent: String,
}

/// Derive a deterministic intent id from the authored content, so an identical
/// re-run (no explicit `--id`) resolves to the same id and is idempotent.
///
/// `intent-<sha256-prefix>` over the length-prefixed authored fields (charter /
/// campaign / each acceptance item / agent) — order-stable, collision-safe by
/// framing. Not a security boundary; an authoring convenience.
fn derive_intent_id(charter: &str, campaign: &str, acceptance: &[String], agent: &str) -> String {
    /// Length-prefixed field push (4-byte BE len ‖ bytes) — framing makes the
    /// concatenation unambiguous, mirroring the refstore `LP(s)` primitive.
    fn field(hasher: &mut Sha256, s: &str) {
        hasher.update((s.len() as u64).to_be_bytes());
        hasher.update(s.as_bytes());
    }
    let mut hasher = Sha256::new();
    field(&mut hasher, charter);
    field(&mut hasher, campaign);
    field(&mut hasher, agent);
    hasher.update((acceptance.len() as u64).to_be_bytes());
    for item in acceptance {
        field(&mut hasher, item);
    }
    let digest = hex::encode(hasher.finalize());
    format!("intent-{}", &digest[..16])
}

/// Author an intent: build the sidecar, land it through the real refstore path,
/// persist the store, and return the stable result.
///
/// Idempotent: a duplicate id (explicit or content-derived) returns the
/// existing intent with `already_exists:true` and exit 0.
///
/// ## Two-phase commit atomicity (C5-F3 / M-3)
///
/// `intent new --log L --store S` is a two-phase write: it appends the
/// `intent.landed` record to the hash-chained, append-only `--log` (the
/// **source of truth**, via [`canonical_log::land_intent`]) AND saves the
/// intent into the `--store`. The two cannot be made a single atomic write
/// across two files; the design instead guarantees **atomic-or-recoverable**:
///
/// 1. **Each individual write is atomic** — the store save lands via
///    temp-file-then-rename ([`IntentStore::save_locked`]), and the log append
///    likewise; a crash mid-write never tears a file.
/// 2. **The writes are ORDERED log-first** (K-ERRLAW2, preserved): the log
///    append runs BEFORE the store save. So the only divergence a mid-commit
///    failure can produce is **log-ahead** (the intent is on the authoritative
///    `--log` but not yet in the `--store`) — NEVER **store-ahead** (a phantom
///    intent in the store that the log never saw). If the log append itself
///    fails, NOTHING is committed (no orphan store entry).
/// 3. **A log-ahead state is self-healing.** Before this run lands anything,
///    [`reconcile_store_from_log`] replays any `intent.landed` present on the
///    `--log` but missing from the `--store` back into the store and persists
///    it. So the NEXT `intent new`/`intent list` over the same pair reconciles
///    the store from the log automatically — the divergence is transient, never
///    permanent, and the log (source of truth) is the one that drives.
pub fn run(input: NewIntent, store_path: &Path) -> Result<NewResult, PorcelainError> {
    if input.charter.trim().is_empty() {
        return Err(PorcelainError::new(
            "invalid_argument",
            "charter is empty",
            "pass a non-empty --charter describing what the intent does",
        ));
    }
    // WH-IDENT: validate --campaign as an identifier (empty + secret-prefix check).
    // Keeps the existing empty-campaign guard but broadens it to the shared validator
    // that also catches credential-shaped values.
    crate::ident::validate_identifier(&input.campaign, "--campaign")
        .map_err(|e| PorcelainError::new(e.kind, e.message, e.fix))?;

    // WH-IDENT: validate the explicit --id, when given.  Auto-derived ids are
    // content-hashed by this module (not user input), so they need no check.
    if let Some(id) = &input.id {
        crate::ident::validate_identifier(id, "--id")
            .map_err(|e| PorcelainError::new(e.kind, e.message, e.fix))?;
    }

    // Redaction parity (Wave E, P-REDACT-SURFACE): scrub every user-supplied
    // FREE-TEXT field through the hardened engine BEFORE it reaches the sidecar,
    // the store, or the hash-chained `--log` payload. The log is append-only and
    // forever — redact-before-append is the only fix (a secret hashed into the
    // chain is unredactable later). Validation above ran on the raw input so an
    // all-whitespace charter still errors.
    let charter = crate::redaction::scrub(&input.charter);
    let campaign = crate::redaction::scrub(&input.campaign);
    let acceptance = crate::redaction::scrub_all(&input.acceptance);
    let agent_raw = input.agent.unwrap_or_else(|| DEFAULT_AGENT.to_string());
    let agent = crate::redaction::scrub(&agent_raw);

    // WJ-UNIFY: the explicit `--id` is an identifier ADDRESS, NOT free text — it
    // must survive VERBATIM so the intent stays addressable end-to-end (`verdict
    // --intent <id>` must resolve it; advancing it to proven depends on the id
    // matching). The OLD `crate::redaction::scrub(&id)` here routed it through the
    // FULL free-text engine, which redacted a bare ULID/40-hex to `[REDACTED]`:
    // distinct ULID intents collapsed to one (`already_exists` on the 2nd/3rd)
    // and `verdict --intent <real-ULID>` saw `intent_not_found` → proven stuck at
    // 0. Input is validated for secret SHAPES by `ident::validate_identifier`
    // above; the id is structurally scrubbed at the ONE central boundary when the
    // `intent.landed` payload is appended (`canonical_log::land_intent` routes it
    // through `scrub_to_canonical`, which redacts a prefixed secret in the
    // `intent_id` key while a ULID/hex address survives). So the id is taken RAW
    // here; an auto-derived id (no `--id`) is a content hash of the REDACTED
    // free-text fields, so idempotency stays stable.
    let intent_id = input
        .id
        .clone()
        .unwrap_or_else(|| derive_intent_id(&charter, &campaign, &acceptance, &agent));

    let context_ref = crate::redaction::scrub(&input.context_ref.unwrap_or_default());
    let sidecar = IntentSidecar {
        intent_id: intent_id.clone(),
        charter: charter.clone(),
        acceptance: acceptance.clone(),
        context_ref: context_ref.clone(),
        // Frozen invariant: the sidecar is NEVER authoritative (B6④).
        authoritative: false,
    };
    // The principal chain records authorship as given (subagent normally),
    // bound to the campaign — honest provenance, not invented.
    let principal_chain = vec![format!("campaign:{campaign}"), format!("agent:{agent}")];

    // WI-PR bootstrap: `intent new` is the first-run authoring verb, and the
    // default `--store` is `.hugit/intents.json`. On a fresh checkout `.hugit/`
    // does not yet exist, so the lock-file `create_new` (and the atomic store
    // write) would fault `no such file or directory` — a `store_error` crash
    // despite the help promising a bootstrap. Create the store's parent dir
    // (mkdir -p semantics) before locking so a first-run `intent new` works.
    ensure_store_parent(store_path)?;

    // Lock the `--store` BEFORE the load and hold it across the whole
    // load→mutate→save (WF-CLI2 bug 2: the store-seam load→lock inversion — two
    // concurrent `intent new --store` for distinct intents each loaded the same
    // chain and clobbered on save). The guard releases on Drop / any early
    // return. The `--log` reconciliation below locks its OWN, DISTINCT file
    // (`--log` ≠ `--store`), so there is no deadlock between the two seams.
    let (store_lock, mut store) =
        IntentStore::lock_and_load(store_path).map_err(PorcelainError::from_store)?;

    // ── Two-phase recovery (C5-F3 / M-3) ──────────────────────────────────────
    // Before this run lands anything, heal any LOG-AHEAD divergence left by a
    // previous run that crashed/failed AFTER the `--log` append committed but
    // BEFORE its store save landed. The `--log` is the append-only source of
    // truth; any `intent.landed` present there but missing from the `--store` is
    // replayed back into the store and persisted, so a prior failure window is
    // reconciled HERE (idempotently) rather than persisting as a permanent
    // store↔log divergence. No-op when `--log` is absent or already in sync.
    if let Some(log_path) = input.log.as_deref() {
        reconcile_store_from_log(&mut store, &store_lock, store_path, log_path)?;
    }

    // Idempotency: if this id already landed in the store, return it unchanged
    // (exit 0). When a shared `--log` is given, still reconcile it (so an intent
    // already in the store is also present on the shared canonical log).
    if store
        .intent_for(&intent_id)
        .map_err(PorcelainError::from_store)?
        .is_some()
    {
        if let Some(log_path) = input.log.as_deref() {
            canonical_log::land_intent(log_path, &sidecar, &principal_chain, now_ms())?;
        }
        // Return the SAME key-set as the first-run path (WB0 stable key-set
        // invariant): campaign and agent are always present, never dropped.
        return Ok(NewResult {
            intent_id,
            already_exists: true,
            campaign: campaign.clone(),
            agent: agent.clone(),
        });
    }

    let recorded_at = now_ms();

    // The REAL refstore path: land the sidecar onto the event log keyed by its
    // intent_id (emits `intent.landed`). The synthetic ref/target are labelled
    // as authored-not-pushed (fixture world until a push binds a git oid).
    import_sidecar(
        &mut store.log,
        &sidecar,
        AUTHORED_REF,
        &authored_target(&intent_id),
        principal_chain.clone(),
        recorded_at,
    )
    .map_err(|e| {
        // DuplicateIntentId is already handled by the pre-check above; any
        // remaining import error (e.g. empty id) is a structured argument fault.
        PorcelainError::new(
            "invalid_intent",
            e.to_string(),
            "ensure --charter is set and the intent id is non-empty",
        )
    })?;

    // K-ERRLAW2 / divergence safety: attempt the `--log` append FIRST (the
    // authoritative shared seam that `pr open` and `campaign` read).  If the log
    // append fails (disk full, permission denied, log_busy), we return the
    // structured error WITHOUT persisting the store — the caller's "not created"
    // belief is then correct, and a retry of the exact command converges:
    //   - the log's already-landed check makes a re-append a no-op,
    //   - the store save is idempotent on the same id.
    // Committing the store BEFORE the log was the bug: a log failure left an
    // orphaned `--store` entry the caller could never see via `--log`, so
    // `pr open --intent <id> --log <same>` returned `intent_not_found` despite
    // the store having the entry.
    if let Some(log_path) = input.log.as_deref() {
        canonical_log::land_intent(log_path, &sidecar, &principal_chain, recorded_at)?;
    }

    // Log append succeeded (or was absent).  Now durably commit the store via
    // the ATOMIC temp-then-rename write (no torn/partial store ever lands).
    // Record the non-authoritative sidecar corpus and the disclosed envelope
    // ref (when given) by intent_id — one lifecycle, one id.
    //
    // C5-F3 / M-3 recovery contract: should THIS atomic save fail after the log
    // append above committed, the result is a transient LOG-AHEAD state (the
    // intent is on the authoritative `--log`, absent from the `--store`). It is
    // NOT a permanent divergence: the next `intent new`/`intent list` over this
    // pair runs `reconcile_store_from_log` first and backfills the store from
    // the log. We surface the error (the caller's "not created in store yet"
    // belief is honest for this run), never a silent store-ahead phantom.
    store.sidecars.insert(intent_id.clone(), sidecar.clone());
    if !context_ref.is_empty() {
        store.envelopes.insert(intent_id.clone(), context_ref);
    }
    // PS-9: record the canonical log this intent was authored against. This is the
    // OWNING log `intent list`/`show` resolve its `landed` state against — so a
    // fleet's global (one-call) view reports each intent's TRUE landed state
    // instead of a misleading `landed:null` for every cross-log intent. Canonical
    // (absolute) so the key is cwd-stable; fall back to the given path if
    // canonicalize fails (the log was just appended above, so it normally exists).
    if let Some(log_path) = input.log.as_deref() {
        let canonical = std::fs::canonicalize(log_path).unwrap_or_else(|_| log_path.to_path_buf());
        store
            .source_logs
            .insert(intent_id.clone(), canonical.to_string_lossy().into_owned());
    }
    store
        .save_locked(&store_lock, store_path)
        .map_err(PorcelainError::from_store)?;

    Ok(NewResult {
        intent_id,
        already_exists: false,
        campaign: campaign.clone(),
        agent: agent.clone(),
    })
}

/// The synthetic target oid for an authored-not-pushed intent — a content tag,
/// honestly labelled `authored:<id>` (never a fake 40-hex git sha).
fn authored_target(intent_id: &str) -> String {
    format!("authored:{intent_id}")
}

/// Reconcile the `--store` from the authoritative `--log` (C5-F3 / M-3).
///
/// The `--log` is the append-only source of truth. A previous `intent new` that
/// failed AFTER its log append committed but BEFORE its store save landed leaves
/// a **log-ahead** state: an `intent.landed` is on the `--log` but absent from
/// the `--store`. This replays every such missing intent back into the store's
/// own event log (via the real [`import_sidecar`] append, preserving the
/// hash-chained spine) plus a best-effort sidecar (the log carries the intent's
/// `charter`/provenance; the non-authoritative `acceptance`/`context_ref`
/// corpus is honestly empty when only the log survived), then persists the store
/// **atomically**.
///
/// Idempotent and direction-safe:
/// - It only ADDS to the store intents the LOG already has, so it can never
///   create a store-ahead phantom (an intent the log never saw).
/// - `import_sidecar` is itself idempotency-guarded (a `DuplicateIntentId` for an
///   intent already in the store's log is skipped), so re-running is a no-op once
///   the store is in sync.
/// - When nothing is missing, the store is left untouched (no needless write).
fn reconcile_store_from_log(
    store: &mut IntentStore,
    store_lock: &crate::pr::filelock::FileLock,
    store_path: &Path,
    log_path: &Path,
) -> Result<(), PorcelainError> {
    // Load the canonical `--log` and rehydrate + VERIFY its hash chain, so a
    // tampered/corrupt log fails closed here rather than seeding the store with
    // forged intents. A missing log is an empty log (nothing to reconcile). This
    // mirrors `canonical_log`'s own loader (which is private to that module); we
    // only READ the log here (the reconcile never writes the `--log`), so there
    // is no orchestration change to the append seam.
    let log = load_log_verified(log_path)?;
    let log_intents = intents_from_log(&log).map_err(|e| {
        PorcelainError::new(
            "chain_broken",
            format!("project intents from --log: {e}"),
            "the --log file's intent records must be well-formed",
        )
    })?;

    // O(n) — N-2 (was O(I·n) ≈ O(n²)). The OLD loop called `store.intent_for(id)`
    // once per `--log` intent, and EACH such call re-ran `intents_from_log` over
    // the WHOLE store log (a full re-parse of every payload). With I log-intents
    // over an n-event store that is O(I·n): measured 866 ms @ 1k events, 3.64 s @
    // 2k, 18.2 s @ 5k — paid on EVERY `intent new --log`, even when already in
    // sync. The fix: project the store's altitude ONCE and build an O(1)-lookup
    // id index, then a single pass over the log-intents.
    let store_intents = intents_from_log(&store.log)
        .map_err(|e| PorcelainError::from_store(StoreError::ChainBroken(e.to_string())))?;
    // Owned so we can also fold in each id we heal this pass: a `--log` that
    // carries the SAME intent_id twice must still be a no-op on the second
    // occurrence (the first `import_sidecar` already landed it into `store.log`,
    // and a re-import would be a `DuplicateIntentId` hard error). The OLD per-id
    // re-projection saw the just-landed id and skipped it; the owned-set insert
    // below preserves that idempotency without the re-projection.
    let mut store_ids: std::collections::HashSet<String> = store_intents
        .intents()
        .iter()
        .map(|i| i.intent_id.clone())
        .collect();

    // In-sync FAST-PATH (the common case): if every `--log` intent is already in
    // the store's altitude, reconcile is a no-op — a cheap membership sweep, no
    // re-projection per id, no write. A divergence is the rare post-crash case;
    // the steady state must not pay for it.
    if log_intents
        .intents()
        .iter()
        .all(|i| store_ids.contains(i.intent_id.as_str()))
    {
        return Ok(());
    }

    // PS-9: a healed intent was authored against THIS `--log`, so record it as the
    // owning log too (canonical, cwd-stable) — keeping the reconcile path's
    // source-log bookkeeping consistent with the normal `new --log` write above.
    let canonical_log_key = std::fs::canonicalize(log_path)
        .unwrap_or_else(|_| log_path.to_path_buf())
        .to_string_lossy()
        .into_owned();

    let mut healed = false;
    for intent in log_intents.intents() {
        // Already present in the store's own log (or already healed earlier this
        // pass)? Then there is no divergence to heal for this id. O(1) against the
        // once-built index (was a per-id re-projection of the whole store log —
        // the N-2 O(n²) source).
        if store_ids.contains(intent.intent_id.as_str()) {
            continue;
        }

        // Log-ahead: this intent exists on the authoritative --log but not in the
        // store. Replay it into the store's event log (the real append path) and
        // record a best-effort sidecar so the store's projection agrees with the
        // log's source of truth.
        let sidecar = IntentSidecar {
            intent_id: intent.intent_id.clone(),
            charter: intent.charter.clone(),
            // Not recoverable from the log alone (the log carries existence +
            // charter + provenance, not the full B6 corpus). Honestly empty —
            // the original `intent new` that crashed pre-store-save is the only
            // place that knew them; a re-run with the same inputs re-supplies
            // them via normal landing (idempotent on id).
            acceptance: Vec::new(),
            context_ref: String::new(),
            authoritative: false,
        };
        import_sidecar(
            &mut store.log,
            &sidecar,
            AUTHORED_REF,
            &authored_target(&intent.intent_id),
            intent.principal_chain.clone(),
            intent.recorded_at,
        )
        .map_err(|e| {
            PorcelainError::new(
                "store_error",
                format!("reconcile store from --log: {e}"),
                "the --log and --store could not be reconciled; inspect both files",
            )
        })?;
        store.sidecars.insert(intent.intent_id.clone(), sidecar);
        store
            .source_logs
            .insert(intent.intent_id.clone(), canonical_log_key.clone());
        // Fold the just-healed id in so a later DUPLICATE occurrence of the same
        // intent_id on the `--log` is skipped (not re-imported into a
        // `DuplicateIntentId` error) — the idempotency the old per-id
        // re-projection provided implicitly.
        store_ids.insert(intent.intent_id.clone());
        healed = true;
    }

    if healed {
        store
            .save_locked(store_lock, store_path)
            .map_err(PorcelainError::from_store)?;
    }
    Ok(())
}

/// Load + chain-verify the canonical `--log` for the C5-F3 reconcile (READ-only).
///
/// A non-existent file is a fresh empty log (nothing to reconcile). Any
/// read/parse/rehydrate fault, or a broken hash chain, fails closed with a
/// structured error — so a reconcile can never seed the store from a tampered
/// log. This is the read half of `canonical_log`'s own private loader; the
/// reconcile path NEVER writes the `--log`, so the append seam is untouched.
fn load_log_verified(path: &Path) -> Result<EventLog, PorcelainError> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(EventLog::new()),
        Err(e) => {
            return Err(PorcelainError::new(
                "io",
                format!("read --log {}: {e}", path.display()),
                "check the --log path exists and is readable",
            ));
        }
    };
    let records: Vec<EventRecord> = serde_json::from_slice(&bytes).map_err(|e| {
        PorcelainError::new(
            "parse",
            format!("parse --log {}: {e}", path.display()),
            "the --log file must be a canonical JSON [EventRecord, …] array",
        )
    })?;
    // PS-13 chokepoint (Wave M integration): the reconcile's rehydrate+verify
    // routes through the SINGLE verified loader `checks::rehydrate_and_verify`
    // (the sole production site of `push_record` + `verify_chain`) rather than
    // hand-rolling it — so this read path cannot drift from the chain-integrity
    // law and a future edit here that forgot to verify would break the build.
    crate::checks::rehydrate_and_verify(records).map_err(|e| match e {
        crate::checks::ChainLoadFault::Rehydrate(m) => PorcelainError::new(
            "rehydrate",
            format!("rehydrate --log {}: {m}", path.display()),
            "the --log file's records must form a gap-free, monotonic chain",
        ),
        crate::checks::ChainLoadFault::ChainBroken(m) => PorcelainError::new(
            "chain_broken",
            format!(
                "--log {} failed integrity verification: {m}",
                path.display()
            ),
            "the --log file's hash chain is tampered or corrupt",
        ),
    })
}

/// Bootstrap the store's parent directory (mkdir -p) so a first-run
/// `intent new` against the default `.hugit/intents.json` store works even when
/// `.hugit/` does not yet exist.
///
/// The lock-file `create_new` and the atomic temp-then-rename store write both
/// require the target's directory to exist; without this a fresh checkout
/// faults `store_error` ("no such file or directory") despite the help
/// promising a bootstrap. A store path with no parent (a bare filename in the
/// cwd) needs nothing; [`std::fs::create_dir_all`] is a no-op on an
/// already-present dir, so this is idempotent and safe to call every run.
fn ensure_store_parent(store_path: &Path) -> Result<(), PorcelainError> {
    match store_path.parent() {
        // No parent component, or the parent is the (always-present) cwd root.
        Some(parent) if !parent.as_os_str().is_empty() => {
            std::fs::create_dir_all(parent).map_err(|e| {
                PorcelainError::new(
                    "store_error",
                    format!("create store directory {}: {e}", parent.display()),
                    "ensure the --store path's parent directory is creatable",
                )
            })
        }
        _ => Ok(()),
    }
}

/// Current unix time in milliseconds (the landing event's observability
/// annotation; excluded from the hash pre-image by design).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// K-ERRLAW2 divergence safety: if the `--log` append fails, the `--store`
    /// must NOT contain the orphaned intent.
    ///
    /// Setup: writable `--store` dir, non-existent/unwritable `--log` dir → the
    /// log append fails, the store save must be skipped, and the exit is a
    /// structured error (not a silent success).  A retry once the log dir is
    /// writable must succeed and leave store + log in agreement.
    #[test]
    fn log_fail_leaves_no_orphaned_store_entry_and_retry_converges() {
        use std::path::PathBuf;

        let base = std::env::temp_dir().join(format!(
            "hugit-errlaw2-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos())
                .unwrap_or(0)
        ));
        std::fs::create_dir_all(&base).expect("create base dir");

        // Store lives in a writable subdir.
        let store_dir = base.join("store");
        std::fs::create_dir_all(&store_dir).expect("create store dir");
        let store_path: PathBuf = store_dir.join("intents.json");

        // Log is inside a path whose PARENT is a FILE (not a dir) — the
        // lock+create inside `canonical_log::land_intent` cannot create a
        // child of a non-directory (PR-4 auto-creates missing dirs, so the
        // previous "non-existent dir" assertion no longer triggers an I/O
        // error; we use parent-as-file which is unambiguously unwritable).
        let blocking_file = base.join("blocking_file");
        std::fs::write(&blocking_file, b"i am a file").expect("write blocking file");
        let log_path: PathBuf = blocking_file.join("events.json");

        let input = NewIntent {
            charter: "divergence safety test".to_string(),
            campaign: "test-campaign".to_string(),
            acceptance: vec!["store is clean on log failure".to_string()],
            id: Some("intent-divergence-test-0001".to_string()),
            agent: None,
            context_ref: None,
            log: Some(log_path.clone()),
        };

        // --- first run: log dir is missing → should error, store must be empty ---
        let result = run(input, &store_path);
        assert!(
            result.is_err(),
            "K-ERRLAW2: a missing log dir must produce a structured error, not Ok"
        );
        // Confirm the store does NOT contain the orphaned intent.
        if store_path.exists() {
            let (_, store) =
                IntentStore::lock_and_load(&store_path).expect("load store for inspection");
            let found = store
                .intent_for("intent-divergence-test-0001")
                .expect("query store");
            assert!(
                found.is_none(),
                "K-ERRLAW2: no orphaned store entry must exist after a log-append failure"
            );
        }
        // Store file should not even exist yet (no successful save).
        // (If the store file does exist but is empty/no entry, that's also fine —
        // the assertion above already covers the absence of the specific entry.)

        // --- create the log dir so the retry can succeed ---
        std::fs::create_dir_all(blocking_file.parent().unwrap())
            .expect("create log dir for retry");
        // Remove the blocking file so the dir is empty.
        std::fs::remove_file(&blocking_file).ok();

        let input2 = NewIntent {
            charter: "divergence safety test".to_string(),
            campaign: "test-campaign".to_string(),
            acceptance: vec!["store is clean on log failure".to_string()],
            id: Some("intent-divergence-test-0001".to_string()),
            agent: None,
            context_ref: None,
            log: Some(log_path.clone()),
        };

        let result2 = run(input2, &store_path);
        assert!(
            result2.is_ok(),
            "K-ERRLAW2: retry with writable log must succeed; got: {result2:?}"
        );
        let out = result2.unwrap();
        assert_eq!(out.intent_id, "intent-divergence-test-0001");
        assert!(!out.already_exists);

        // Store and log must now agree: the intent is in both.
        let (_, store) = IntentStore::lock_and_load(&store_path).expect("load store after retry");
        let in_store = store
            .intent_for("intent-divergence-test-0001")
            .expect("query store")
            .is_some();
        assert!(
            in_store,
            "K-ERRLAW2: intent must be in store after successful retry"
        );

        assert!(
            log_path.exists(),
            "K-ERRLAW2: log file must exist after retry"
        );
        let log_bytes = std::fs::read(&log_path).expect("read log");
        let log_str = String::from_utf8(log_bytes).expect("log is utf-8");
        assert!(
            log_str.contains("intent-divergence-test-0001"),
            "K-ERRLAW2: intent id must be in log after retry"
        );

        // cleanup
        let _ = std::fs::remove_dir_all(&base);
    }
}
