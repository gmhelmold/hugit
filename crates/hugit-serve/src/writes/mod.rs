//! The write path — the shared foundation every `/v1` POST verb rides.
//!
//! ## The factorization (why this is the spine)
//!
//! A verb body is a PURE function `fn(&mut EventLog, …args, req) -> Result<Accepted,
//! EngineErr>` (see [`verbs`]). The cross-cutting law — idempotency, durable
//! persistence, the body-hash replay guard — lives HERE, in [`with_write`], so it
//! is implemented ONCE and the verbs cannot drift from it. This is the
//! close-the-product Wave-2 §1 forcing insight: the idempotency ledger is the one
//! component every verb shares, so it is built once and frozen.
//!
//! ## Idempotency (spec §3)
//!
//! `Idempotency-Key` is MANDATORY (absent ⇒ `400 IDEMPOTENCY_REQUIRED`). On a
//! SUCCESS, the door appends an `idem.recorded` event capturing `(principal, verb,
//! key, body_sha256, outcome)` and persists it atomically alongside the verb's own
//! record(s). A replay of the same key returns the stored outcome **byte-identical**
//! — so a lost-response retry of `land` never enqueues twice (one land, one
//! position). Same key + a DIFFERENT body ⇒ `409 IDEM_MISMATCH`, never re-executed.
//!
//! A verb REJECTION (4xx) is NOT recorded: the T1 verbs reject deterministically
//! from the log state (a 404/400 re-evaluates identically on replay), so
//! determinism gives byte-identical rejection replay for free. The non-deterministic
//! rejection-replay (policy/erasure STEP-UP) is a Tier-3 concern, gated on the §6
//! owner design checkpoint.
//!
//! ## Persistence ([`LogSink`])
//!
//! The read server is read-only by design; the write path's durable persistence is
//! abstracted behind [`LogSink`] so the WRITE credential / R2-write architecture is
//! isolated to one impl (the read path is untouched). A verb mutates an in-memory
//! [`EventLog`]; the door persists ONCE on success (atomic — both the verb record
//! and the `idem.recorded` land together, or neither does).

pub mod erasure;
pub mod verbs;

use hugit_http_contracts::actions::Accepted;
use hugit_refstore::{EventLog, PrincipalClass};
use sha2::{Digest, Sha256};

use crate::error::EngineErr;

/// Derive the D14 principal CLASS to assert from the engine-resolved
/// `principal_chain`, **fail-closed**.
///
/// THE hole this closes (authz audit 2026-06-20): the write verbs previously
/// handed `append_authorized` a HARDCODED class (undo→Human, land/policy/verdict
/// →Orchestrator) regardless of who actually called — so the D14 matrix always
/// "passed" and a worker/model/subagent could `land`/`undo`/`policy`/`verdict`
/// over the serve path. The asserted class MUST be derived from the authenticated
/// actor (the chain tail), so the matrix is decided against the REAL caller.
///
/// ## Mapping the SERVE-layer principals onto the D14 classes
///
/// The D14 classifier ([`PrincipalClass::classify`]) understands the event-log
/// identity prefixes (`user:`/`human:`/`orchestrator:`/`worker:`/`agent:`/
/// `model:`). The serve write path is driven by the two ENGINE-RESOLVED
/// authenticated principals (`two_tier_auth`), which use a different vocabulary:
///
/// - `orchestrator:hugit` — the platform/dev OPERATOR → [`Orchestrator`].
/// - `clerk:{org}:{user}` — an authenticated OWNING-TENANT forge driver. It has
///   already cleared the ownership gate ([`crate::authz::authorize_write`]) in
///   [`with_write`] before any verb runs, so on the serve write surface it acts
///   as the repo's INTEGRATION authority → [`Orchestrator`].
///
/// Everything else falls back to [`PrincipalClass::classify`], so a WORKER
/// (`agent:`/`worker:`) or MODEL (`model:`) is recognized and then DENIED by the
/// frozen matrix inside [`EventLog::append_authorized`] (land/verdict/policy are
/// `Orchestrator`-only; undo is `Human`-only) — that IS the hole being closed.
///
/// Fail-closed: an empty chain (anonymous) or an unclassifiable actor (unknown
/// prefix, bare string, empty id) is denied — mapped to `404` (the write
/// boundary's no-existence-oracle convention), NOT defaulted into a class.
pub(crate) fn asserted_class(principal_chain: &[String]) -> Result<PrincipalClass, EngineErr> {
    let actor = principal_chain.last().map(String::as_str).unwrap_or("");
    // The serve-layer authenticated forge drivers (operator + owning tenant) map
    // to the integration authority; everything else goes through the D14
    // classifier (so worker/model are recognized → matrix-denied, not silently
    // allowed) and an unclassifiable principal fails closed.
    if actor.starts_with("orchestrator:") || actor.starts_with("clerk:") {
        // A non-empty org/user segment is required (a bare `clerk:`/`orchestrator:`
        // is malformed → no authenticated driver → deny).
        if actor
            .split_once(':')
            .is_some_and(|(_, rest)| !rest.is_empty())
        {
            return Ok(PrincipalClass::Orchestrator);
        }
    }
    PrincipalClass::classify(actor).ok_or_else(EngineErr::not_found)
}

/// The `idem.recorded` event kind — the idempotency ledger's record.
pub const IDEM_RECORDED_KIND: &str = "idem.recorded";

/// Max raw request-body size the write-door accepts (audit P2 — DoS / log-bloat
/// guard). Generous enough for a large file edit, small enough to refuse a
/// pathological multi-MB comment/ask. Over-limit ⇒ `400 INVALID_REQUEST`.
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Max compare-and-swap attempts in [`with_write`] before failing with a transient
/// 503. Each lost race reloads the (now-advanced) head and re-runs; a single engine
/// instance + the occasional out-of-band writer (the snapshot uploader) never
/// approaches this, so exhaustion means sustained, pathological contention.
pub const MAX_CAS_ATTEMPTS: u32 = 5;

/// Verbs that require fresh re-auth (STEP-UP, spec §3). The door refuses them
/// with `403 STEP_UP_REQUIRED` unless the route presents a fresh-auth proof.
/// (The real fresh-auth is the P2 Clerk seam; the door enforces the GATE today.)
pub const STEP_UP_VERBS: &[&str] = &["policy", "erasure"];

/// An opaque concurrency token captured by [`LogSink::load`] and presented back to
/// [`LogSink::persist`] for the compare-and-swap. It pins the exact head the verb
/// ran against, so persist can reject a write whose head moved underneath it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CasToken {
    /// The log did not exist at load — persist must CREATE-only (R2 `If-None-Match: *`):
    /// it fails the CAS if another writer created it in the gap.
    Absent,
    /// The log existed at this version (the R2 object ETag, or a local content hash).
    /// Persist swaps only if the head still equals this version (R2 `If-Match: <v>`).
    Version(String),
    /// The sink does not implement CAS (the in-memory test sink). Persist is
    /// unconditional — used ONLY where there is provably no concurrent writer.
    Unsupported,
}

/// Durable persistence for a repo's event log. The read path stays read-only; the
/// WRITE credential / R2-write architecture lives ENTIRELY behind this trait.
pub trait LogSink: Send + Sync {
    /// Load + chain-verify `<repo>`'s event log (same verification as the read
    /// path) AND capture its [`CasToken`] (the head version, for the swap). A
    /// missing log is an empty world ONLY if the verb can create it; for writes
    /// against an existing repo, absence is the verb's own `404`.
    fn load(&self, repo: &str) -> Result<(EventLog, CasToken), EngineErr>;

    /// Durably persist `log` as the new source of truth for `<repo>`, as a
    /// COMPARE-AND-SWAP against `expected` (the token the matching [`load`]
    /// returned). Fail-honest: a non-durable write is an `Err` (never a silent
    /// partial / fake success).
    ///
    /// **CAS OBLIGATION (hard contract — audit P2→P1).** `with_write` does
    /// load → mutate → persist with NO lock held across the gap. On a concurrent
    /// head mismatch the impl MUST return [`EngineErr::cas_conflict`] (so the
    /// caller reloads + retries), NEVER last-writer-wins (which would silently drop
    /// the other request's records AND its idempotency-ledger entry). A non-CAS
    /// impl breaks cross-request atomicity — a contract violation, not an
    /// optimization.
    fn persist(&self, repo: &str, log: &EventLog, expected: &CasToken) -> Result<(), EngineErr>;
}

/// Durable persistence for a per-ACCOUNT event log (GDPR1) — the account-scoped twin
/// of [`LogSink`], keyed under the reserved `_accounts/{slug}` store (structurally
/// outside the repo namespace, never served/clone-able as a repo). Unlike [`LogSink`],
/// [`load_account`](AccountLogSink::load_account) is LOAD-OR-CREATE: an absent log is
/// the empty world (`CasToken::Absent`) so the first erasure request seeds the genesis
/// — there is no "repo 404" for an account that has never requested erasure.
pub trait AccountLogSink: Send + Sync {
    /// Load-or-create + chain-verify the `account`'s event log AND capture its head
    /// [`CasToken`]. Absent → `(EventLog::new(), CasToken::Absent)` (create-genesis).
    fn load_account(&self, account: &str) -> Result<(EventLog, CasToken), EngineErr>;

    /// Durably persist `log` as the account's new head, a COMPARE-AND-SWAP against
    /// `expected`. A concurrent head move → [`EngineErr::cas_conflict`] (reload+retry),
    /// NEVER last-writer-wins.
    fn persist_account(
        &self,
        account: &str,
        log: &EventLog,
        expected: &CasToken,
    ) -> Result<(), EngineErr>;
}

/// Hex SHA-256 of the raw request body — the idempotency body fingerprint.
fn body_sha256(body: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(body);
    hex::encode(h.finalize())
}

/// A prior idempotent outcome recovered from the ledger.
struct PriorOutcome {
    body_sha256: String,
    accepted: Accepted,
}

/// Scan the log for a prior `idem.recorded` matching `(principal, verb, resource,
/// key)`. Returns the stored body-hash + the replayable `Accepted`.
///
/// `resource` (the URL path tail, e.g. `prs/1/land`) is part of the key so an
/// `Idempotency-Key` reused across DIFFERENT resources (PR 1 vs PR 2) does NOT
/// replay the first resource's outcome (audit P0: URL-resourced verbs carry the id
/// only in the URL, not the body — without `resource` keyed, the 2nd mutation is
/// silently dropped).
///
/// Fail-CLOSED on corruption (audit hardening): a record that MATCHES the key tuple
/// but whose stored `body_sha256`/`outcome` will not deserialize is a corrupt ledger
/// entry — it returns `Err` (→ 503), NOT `Ok(None)`. The earlier `.ok()?` folded an
/// unparseable matched outcome into "no prior found", which would silently
/// RE-EXECUTE the verb (a double effect) — fail-open on the idempotency guard. A
/// missing match is still `Ok(None)` (run the verb for the first time).
fn idem_lookup(
    log: &EventLog,
    principal: &str,
    verb: &str,
    resource: &str,
    key: &str,
) -> Result<Option<PriorOutcome>, EngineErr> {
    let matched = log
        .records()
        .iter()
        .filter(|r| r.kind == IDEM_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<serde_json::Value>(&r.payload).ok())
        .find(|v| {
            v.get("principal").and_then(|x| x.as_str()) == Some(principal)
                && v.get("verb").and_then(|x| x.as_str()) == Some(verb)
                && v.get("resource").and_then(|x| x.as_str()) == Some(resource)
                && v.get("key").and_then(|x| x.as_str()) == Some(key)
        });
    let Some(v) = matched else {
        return Ok(None); // no prior for this key — first execution
    };
    // The key matched: its outcome MUST parse, or the ledger entry is corrupt.
    let corrupt = || EngineErr::unavailable("idempotency ledger entry is corrupt");
    let body_sha256 = v
        .get("body_sha256")
        .and_then(|x| x.as_str())
        .ok_or_else(corrupt)?
        .to_string();
    let outcome = v.get("outcome").ok_or_else(corrupt)?.clone();
    let accepted: Accepted = serde_json::from_value(outcome).map_err(|_| corrupt())?;
    Ok(Some(PriorOutcome {
        body_sha256,
        accepted,
    }))
}

/// Append the `idem.recorded` ledger entry for a successful write. Canonical JSON
/// (sorted keys) so the chain pre-image is deterministic.
#[allow(clippy::too_many_arguments)]
fn idem_record(
    log: &mut EventLog,
    principal: &str,
    verb: &str,
    resource: &str,
    key: &str,
    body_sha256: &str,
    accepted: &Accepted,
    at: u64,
) -> Result<(), EngineErr> {
    let outcome = serde_json::to_value(accepted)
        .map_err(|e| EngineErr::unavailable(format!("idem outcome serialize: {e}")))?;
    let value = serde_json::json!({
        "body_sha256": body_sha256,
        "key": key,
        "outcome": outcome,
        "principal": principal,
        "resource": resource,
        "verb": verb,
    });
    let payload =
        hugit_refstore::canonical_json(&value.to_string()).unwrap_or_else(|| value.to_string());
    // The ledger record is system-emitted bookkeeping, not a user mutation; route
    // it through the same D14-guarded door under the acting principal.
    log.append_authorized(
        hugit_refstore::PrincipalClass::Orchestrator,
        hugit_refstore::Endpoint::Land,
        IDEM_RECORDED_KIND,
        vec![principal.to_string()],
        payload,
        at,
    )
    .map_err(|d| {
        EngineErr::unavailable(format!("idem ledger append denied: {}", d.reason.code()))
    })?;
    Ok(())
}

/// The write-door: the one chokepoint every POST verb rides.
///
/// - `verb` — the stable verb name (the STEP-UP discriminator, e.g. `"land"`).
/// - `resource` — the URL path tail (e.g. `prs/1/land`); part of the idempotency
///   key so a key reused across different resources does NOT replay (audit P0).
/// - `idem_key` — the `Idempotency-Key` header (empty ⇒ `400`).
/// - `body` — the raw request body bytes (the replay fingerprint; size-capped).
/// - `step_up_presented` — did the route present fresh re-auth? Required for the
///   [`STEP_UP_VERBS`] (policy/erasure), else `403 STEP_UP_REQUIRED`.
/// - `run` — the pure verb body; it mutates the loaded log and returns `Accepted`.
///
/// On success the verb's record(s) + the `idem.recorded` entry persist atomically.
///
/// **Concurrency (CAS):** the load→mutate→persist cycle holds no lock across the
/// gap, so a concurrent writer can move the head between this request's `load` and
/// `persist`. The cycle is therefore wrapped in a bounded retry loop: `persist` is
/// a compare-and-swap against the loaded head; on a [`EngineErr::is_cas_conflict`]
/// the loop reloads and re-runs from scratch. The reload re-checks idempotency, so
/// two concurrent requests with the SAME key resolve to one execution + one replay
/// (never a double append), and with DIFFERENT keys both survive (never a silent
/// drop). After [`MAX_CAS_ATTEMPTS`] lost races the door fails honestly with a
/// transient-contention 503 — the write did NOT happen; the client may retry.
#[allow(clippy::too_many_arguments)]
pub fn with_write<F>(
    sink: &dyn LogSink,
    repo: &str,
    verb: &str,
    resource: &str,
    idem_key: &str,
    body: &[u8],
    step_up_presented: bool,
    principal_chain: Vec<String>,
    at: u64,
    run: F,
) -> Result<Accepted, EngineErr>
where
    F: Fn(&mut EventLog, Vec<String>, u64) -> Result<Accepted, EngineErr>,
{
    // STEP-UP gate (audit P1): a step-up verb without fresh re-auth is refused at
    // the door, BEFORE any load/effect — the verb itself is step-up-agnostic.
    if STEP_UP_VERBS.contains(&verb) && !step_up_presented {
        return Err(EngineErr::step_up_required());
    }
    // Body-size cap (audit P2): refuse a pathological body before scanning/hashing it.
    if body.len() > MAX_BODY_BYTES {
        return Err(EngineErr::invalid_request(
            "corpo da requisição excede o limite",
        ));
    }
    if idem_key.trim().is_empty() {
        return Err(EngineErr::idempotency_required());
    }
    let principal = principal_chain.last().cloned().unwrap_or_default();
    let body_hash = body_sha256(body);

    for _attempt in 0..MAX_CAS_ATTEMPTS {
        // Re-load the head on every attempt: the CAS token pins THIS read so the
        // matching persist can detect a concurrent move (and a reload after a lost
        // race sees the winner's records — including its idempotency ledger entry).
        let (mut log, token) = sink.load(repo)?;

        // WRITE-SIDE PER-TENANT GATE (the engine re-decides on EVERY
        // verb, not just reads): the caller must OWN the repo (or be the operator)
        // to mutate it. Uses `authorize_WRITE`, NOT the read gate — write permission
        // is OWNERSHIP, never read-visibility: a `public` repo is readable by all
        // but writable ONLY by its owner/operator (else any signed-up tenant could
        // land/verdict/policy/… into the public launch repo). A non-owner is denied
        // as 404 (no existence leak), BEFORE the idempotency lookup or any effect.
        if !crate::authz::authorize_write(&principal_chain, &crate::authz::project_repo_meta(&log))
        {
            return Err(EngineErr::not_found());
        }

        // Replay guard: a seen key returns the stored outcome (or 409 on a body
        // change) BEFORE the verb runs — so a lost-response retry (or a same-key
        // race lost above) never re-executes the effect.
        if let Some(prior) = idem_lookup(&log, &principal, verb, resource, idem_key)? {
            if prior.body_sha256 != body_hash {
                return Err(EngineErr::idem_mismatch());
            }
            return Ok(prior.accepted);
        }

        // First time for this key on this head: run the verb (mutates the in-memory
        // log), record the idempotent outcome, then compare-and-swap-persist ONCE
        // (atomic: both records or neither).
        let accepted = run(&mut log, principal_chain.clone(), at)?;
        idem_record(
            &mut log, &principal, verb, resource, idem_key, &body_hash, &accepted, at,
        )?;
        match sink.persist(repo, &log, &token) {
            Ok(()) => return Ok(accepted),
            // The head moved under us — discard this attempt's in-memory work and
            // retry from a fresh load (which re-checks idempotency).
            Err(e) if e.is_cas_conflict() => continue,
            Err(e) => return Err(e),
        }
    }

    // Exhausted the retry budget: sustained contention. Fail honest + transient —
    // nothing was persisted on the losing attempts (each was a rejected CAS).
    Err(EngineErr::unavailable(
        "escrita sob contenção concorrente — tente novamente",
    ))
}

/// The ACCOUNT write-door (GDPR1) — the account-scoped analogue of [`with_write`]
/// over an [`AccountLogSink`]. It shares the SAME idempotency ledger + body-hash
/// replay guard + bounded CAS retry loop, so the account path can never drift from the
/// repo path on the cross-cutting law.
///
/// TWO deliberate differences from [`with_write`], each a security decision:
/// 1. **No `authorize_write` repo gate.** An account-erase is NOT a repo mutation;
///    there is no `repo.meta` to own. The SUBJECT is the caller's OWN account, derived
///    by the verb from the verified principal (`derive_owner_tenant` refuses
///    operator/anon) — a caller can only ever erase itself. Ownership is therefore
///    intrinsic to the verb, not a separate gate here.
/// 2. **Load-or-create, never 404.** [`AccountLogSink::load_account`] returns the empty
///    world (`Absent`) for an account with no prior erasure log, so the first request
///    seeds the genesis via a create-only persist.
///
/// STEP-UP is still enforced HERE (identical gate): `"erasure"` ∈ [`STEP_UP_VERBS`], so
/// a request without a fresh-auth proof is refused `403` before any load/effect. The
/// top-level route skips the repo door, so this is the ONLY step-up chokepoint — it
/// must not be bypassed.
#[allow(clippy::too_many_arguments)]
pub fn with_account_write<F>(
    sink: &dyn AccountLogSink,
    account: &str,
    verb: &str,
    resource: &str,
    idem_key: &str,
    body: &[u8],
    step_up_presented: bool,
    principal_chain: Vec<String>,
    at: u64,
    run: F,
) -> Result<Accepted, EngineErr>
where
    F: Fn(&mut EventLog, Vec<String>, u64) -> Result<Accepted, EngineErr>,
{
    // STEP-UP gate (the only chokepoint on the top-level route): refused BEFORE any
    // load/effect. `"erasure"` is a step-up verb.
    if STEP_UP_VERBS.contains(&verb) && !step_up_presented {
        return Err(EngineErr::step_up_required());
    }
    if body.len() > MAX_BODY_BYTES {
        return Err(EngineErr::invalid_request(
            "corpo da requisição excede o limite",
        ));
    }
    if idem_key.trim().is_empty() {
        return Err(EngineErr::idempotency_required());
    }
    let principal = principal_chain.last().cloned().unwrap_or_default();
    let body_hash = body_sha256(body);

    for _attempt in 0..MAX_CAS_ATTEMPTS {
        // Load-or-create the account head + its CAS token (absent → empty world).
        let (mut log, token) = sink.load_account(account)?;

        // Replay guard (identical to the repo door): a seen key returns the stored
        // outcome (or 409 on a body change) BEFORE the verb runs.
        if let Some(prior) = idem_lookup(&log, &principal, verb, resource, idem_key)? {
            if prior.body_sha256 != body_hash {
                return Err(EngineErr::idem_mismatch());
            }
            return Ok(prior.accepted);
        }

        // First time for this key on this head: run the verb (which itself derives +
        // authorizes the subject), record the idempotent outcome, then CAS-persist ONCE.
        let accepted = run(&mut log, principal_chain.clone(), at)?;
        idem_record(
            &mut log, &principal, verb, resource, idem_key, &body_hash, &accepted, at,
        )?;
        match sink.persist_account(account, &log, &token) {
            Ok(()) => return Ok(accepted),
            Err(e) if e.is_cas_conflict() => continue,
            Err(e) => return Err(e),
        }
    }

    Err(EngineErr::unavailable(
        "escrita sob contenção concorrente — tente novamente",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn asserted_class_is_chain_derived_fail_closed() {
        // The serve-layer authenticated forge drivers (operator + owning tenant)
        // map to the integration authority (Orchestrator).
        assert_eq!(
            asserted_class(&["orchestrator:hugit".into()]).unwrap(),
            PrincipalClass::Orchestrator
        );
        assert_eq!(
            asserted_class(&["clerk:org-a:user-1".into()]).unwrap(),
            PrincipalClass::Orchestrator,
            "an authenticated owning tenant drives the forge as the integration authority"
        );
        // D14 identity prefixes classify normally (the chain TAIL is the actor).
        assert_eq!(
            asserted_class(&["user:alice".into()]).unwrap(),
            PrincipalClass::Human
        );
        assert_eq!(
            asserted_class(&["agent:runner".into()]).unwrap(),
            PrincipalClass::Worker
        );
        assert_eq!(
            asserted_class(&["model:claude".into()]).unwrap(),
            PrincipalClass::Model
        );
        // The actor is the LAST link (a human-delegated worker IS a worker).
        assert_eq!(
            asserted_class(&["user:g".into(), "agent:r".into()]).unwrap(),
            PrincipalClass::Worker
        );
        // Unclassifiable / empty / malformed ⇒ denied 404 (no oracle), never
        // defaulted into a class.
        assert_eq!(asserted_class(&["weird:x".into()]).unwrap_err().status, 404);
        assert_eq!(asserted_class(&["nobody".into()]).unwrap_err().status, 404);
        assert_eq!(asserted_class(&["clerk:".into()]).unwrap_err().status, 404);
        assert_eq!(asserted_class(&[]).unwrap_err().status, 404);
    }

    /// An in-memory sink over a single repo's log — the testable `LogSink`.
    struct MemSink {
        log: RefCell<EventLog>,
    }
    impl LogSink for MemSink {
        fn load(&self, _repo: &str) -> Result<(EventLog, CasToken), EngineErr> {
            // `Unsupported`: this sink opts out of CAS (no concurrent writer in the
            // single-threaded happy-path tests). The CAS retry loop is exercised by
            // `CasSink` below, which DOES enforce + simulates a concurrent bump.
            Ok((self.log.borrow().clone(), CasToken::Unsupported))
        }
        fn persist(
            &self,
            _repo: &str,
            log: &EventLog,
            _expected: &CasToken,
        ) -> Result<(), EngineErr> {
            *self.log.borrow_mut() = log.clone();
            Ok(())
        }
    }
    // RefCell isn't Sync; for single-threaded tests we only need the trait shape.
    // SAFETY: tests are single-threaded; this unblocks `&dyn LogSink` use.
    unsafe impl Sync for MemSink {}

    /// A CAS-enforcing in-memory sink: it versions the head and rejects a persist
    /// whose `expected` version no longer matches — the in-memory analogue of R2
    /// `If-Match`. A "concurrent writer" is simulated by injecting records (and
    /// bumping the version) just before a persist, so a test can force a lost CAS
    /// race and assert `with_write` reloads + recovers:
    /// - `pending_bump` — records injected ONCE, on the next persist (then drained).
    /// - `always_bump` — inject an unrelated record on EVERY persist, so no attempt
    ///   ever wins (drives the retry budget to exhaustion).
    struct CasSink {
        log: RefCell<EventLog>,
        version: RefCell<u64>,
        pending_bump: RefCell<Vec<(String, String)>>,
        always_bump: bool,
    }
    impl CasSink {
        fn inject(&self, recs: &[(String, String)]) {
            let mut cur = self.log.borrow_mut();
            for (kind, payload) in recs {
                let at = cur.records().len() as u64 + 1;
                cur.append_for_test(kind, vec!["orchestrator:other".into()], payload.clone(), at);
            }
            *self.version.borrow_mut() += 1;
        }
    }
    impl LogSink for CasSink {
        fn load(&self, _repo: &str) -> Result<(EventLog, CasToken), EngineErr> {
            Ok((
                self.log.borrow().clone(),
                CasToken::Version(self.version.borrow().to_string()),
            ))
        }
        fn persist(
            &self,
            _repo: &str,
            log: &EventLog,
            expected: &CasToken,
        ) -> Result<(), EngineErr> {
            // A concurrent writer lands first, moving the head — once (pending_bump)
            // or on every attempt (always_bump).
            if self.always_bump {
                self.inject(&[("other.churn".into(), "{}".into())]);
            } else {
                let pending: Vec<_> = self.pending_bump.borrow_mut().drain(..).collect();
                if !pending.is_empty() {
                    self.inject(&pending);
                }
            }
            let head = CasToken::Version(self.version.borrow().to_string());
            if *expected != head {
                return Err(EngineErr::cas_conflict());
            }
            *self.log.borrow_mut() = log.clone();
            *self.version.borrow_mut() += 1;
            Ok(())
        }
    }
    // SAFETY: single-threaded tests only (same as MemSink).
    unsafe impl Sync for CasSink {}

    fn cas_sink_with_pr(pr_id: &str) -> CasSink {
        let seeded = sink_with_pr(pr_id).log.into_inner();
        CasSink {
            log: RefCell::new(seeded),
            version: RefCell::new(0),
            pending_bump: RefCell::new(Vec::new()),
            always_bump: false,
        }
    }

    /// The (kind, payload) records a completed `with_write(key)` adds beyond the
    /// `sink_with_pr` seed — the faithful "winner's records" (its effect + its
    /// `idem.recorded`) to inject as a same-key concurrent winner.
    fn winner_records(pr_id: &str, key: &str, body: &[u8]) -> Vec<(String, String)> {
        let w = sink_with_pr(pr_id);
        with_write(
            &w,
            "r",
            "land",
            "res",
            key,
            body,
            true,
            vec!["orchestrator:o".into()],
            2,
            dummy_verb,
        )
        .expect("winner write ok");
        let log = w.log.borrow();
        log.records()[1..]
            .iter()
            .map(|r| (r.kind.clone(), r.payload.clone()))
            .collect()
    }

    fn sink_with_pr(pr_id: &str) -> MemSink {
        let mut log = EventLog::new();
        let payload = hugit_refstore::canonical_json(
            &serde_json::json!({
                "author_kind":"orchestrator","campaign":"c","intent_ids":["i"],
                "pr_id":pr_id,"principal":null,"run_id":"r"
            })
            .to_string(),
        )
        .unwrap();
        log.append_for_test("pr.opened", vec!["orchestrator:t".into()], payload, 1);
        MemSink {
            log: RefCell::new(log),
        }
    }

    /// A trivial verb that appends one record and returns a fixed Accepted.
    fn dummy_verb(
        log: &mut EventLog,
        principals: Vec<String>,
        at: u64,
    ) -> Result<Accepted, EngineErr> {
        let rec = log
            .append_authorized(
                hugit_refstore::PrincipalClass::Orchestrator,
                hugit_refstore::Endpoint::Land,
                "pr.queued",
                principals,
                r#"{"pr_id":"1"}"#,
                at,
            )
            .map_err(|d| EngineErr::unavailable(d.reason.code().to_string()))?;
        Ok(Accepted {
            seq: rec.seq,
            note: "ok".into(),
            extra: None,
            queue_pos: Some(1),
            pr_number: Some(1),
            branch: None,
            state: None,
            charter_preview: None,
        })
    }

    #[test]
    fn absent_idem_key_is_400() {
        let sink = sink_with_pr("1");
        let err = with_write(
            &sink,
            "r",
            "land",
            "res",
            "",
            b"{}",
            true,
            vec!["orchestrator:o".into()],
            2,
            dummy_verb,
        )
        .expect_err("empty key must 400");
        assert_eq!(err.status, 400);
        assert_eq!(err.code, "IDEMPOTENCY_REQUIRED");
    }

    #[test]
    fn replay_same_key_same_body_is_byte_identical_no_double_effect() {
        let sink = sink_with_pr("1");
        let body = b"{\"mode\":\"union\"}";
        let first = with_write(
            &sink,
            "r",
            "land",
            "res",
            "K1",
            body,
            true,
            vec!["orchestrator:o".into()],
            2,
            dummy_verb,
        )
        .expect("first ok");
        // A replay (lost-response retry) must return the SAME outcome and NOT append
        // a second pr.queued — the land one-position invariant.
        let replay = with_write(
            &sink,
            "r",
            "land",
            "res",
            "K1",
            body,
            true,
            vec!["orchestrator:o".into()],
            3,
            dummy_verb,
        )
        .expect("replay ok");
        assert_eq!(first.seq, replay.seq, "replay must return the original seq");
        let queued = sink
            .log
            .borrow()
            .records()
            .iter()
            .filter(|r| r.kind == "pr.queued")
            .count();
        assert_eq!(queued, 1, "exactly ONE pr.queued despite the replay");
    }

    #[test]
    fn same_key_different_body_is_409() {
        let sink = sink_with_pr("1");
        with_write(
            &sink,
            "r",
            "land",
            "res",
            "K1",
            b"{\"mode\":\"union\"}",
            true,
            vec!["orchestrator:o".into()],
            2,
            dummy_verb,
        )
        .expect("first ok");
        let err = with_write(
            &sink,
            "r",
            "land",
            "res",
            "K1",
            b"{\"mode\":\"serial\"}",
            true,
            vec!["orchestrator:o".into()],
            3,
            dummy_verb,
        )
        .expect_err("different body must 409");
        assert_eq!(err.status, 409);
        assert_eq!(err.code, "IDEM_MISMATCH");
    }

    #[test]
    fn step_up_verb_without_fresh_auth_is_403() {
        let sink = sink_with_pr("1");
        // `policy` is a STEP_UP verb; step_up_presented=false ⇒ 403 before any effect.
        let err = with_write(
            &sink,
            "r",
            "policy",
            "res",
            "K1",
            b"{}",
            false,
            vec!["orchestrator:o".into()],
            2,
            dummy_verb,
        )
        .expect_err("step-up verb without fresh auth must 403");
        assert_eq!(err.status, 403);
        assert_eq!(err.code, "STEP_UP_REQUIRED");
        // A non-step-up verb is unaffected by step_up_presented=false.
        assert!(
            with_write(
                &sink,
                "r",
                "land",
                "res",
                "K2",
                b"{}",
                false,
                vec!["orchestrator:o".into()],
                3,
                dummy_verb
            )
            .is_ok()
        );
    }

    #[test]
    fn oversize_body_is_400() {
        let sink = sink_with_pr("1");
        let big = vec![b'x'; MAX_BODY_BYTES + 1];
        let err = with_write(
            &sink,
            "r",
            "land",
            "res",
            "K1",
            &big,
            true,
            vec!["orchestrator:o".into()],
            2,
            dummy_verb,
        )
        .expect_err("oversize body must 400");
        assert_eq!(err.status, 400);
        assert_eq!(err.code, "INVALID_REQUEST");
    }

    #[test]
    fn same_key_different_resource_does_not_replay() {
        // Audit P0: an Idempotency-Key reused across DIFFERENT resources (PR 1 vs
        // PR 2) must NOT replay the first resource's outcome — the verb runs again.
        let sink = sink_with_pr("1");
        let body = b"{\"mode\":\"union\"}";
        with_write(
            &sink,
            "r",
            "land",
            "prs/1/land",
            "K",
            body,
            true,
            vec!["orchestrator:o".into()],
            2,
            dummy_verb,
        )
        .expect("first resource ok");
        // Same verb + key + body, DIFFERENT resource → must execute (not replay).
        with_write(
            &sink,
            "r",
            "land",
            "prs/2/land",
            "K",
            body,
            true,
            vec!["orchestrator:o".into()],
            3,
            dummy_verb,
        )
        .expect("second resource ok");
        let queued = sink
            .log
            .borrow()
            .records()
            .iter()
            .filter(|r| r.kind == "pr.queued")
            .count();
        assert_eq!(
            queued, 2,
            "a key reused across resources must NOT collapse to one effect"
        );
    }

    #[test]
    fn corrupt_idem_entry_with_matching_key_fails_closed_not_re_executes() {
        // Audit H12: a ledger entry that MATCHES the key tuple but whose `outcome`
        // will not deserialize must FAIL CLOSED (503), never fold to "no prior" and
        // silently re-execute the verb (fail-open on the idempotency guard).
        let sink = sink_with_pr("1");
        // Seed a corrupt idem.recorded for (principal=orchestrator:o, verb=land,
        // resource=res, key=K1): matching key tuple, but `outcome` is a bare string
        // (not an Accepted object) → from_value fails.
        let corrupt = serde_json::json!({
            "body_sha256": "deadbeef",
            "key": "K1",
            "outcome": "not-an-accepted-object",
            "principal": "orchestrator:o",
            "resource": "res",
            "verb": "land",
        });
        let payload = hugit_refstore::canonical_json(&corrupt.to_string()).unwrap();
        sink.log.borrow_mut().append_for_test(
            IDEM_RECORDED_KIND,
            vec!["orchestrator:o".into()],
            payload,
            2,
        );
        let queued_before = sink
            .log
            .borrow()
            .records()
            .iter()
            .filter(|r| r.kind == "pr.queued")
            .count();
        let err = with_write(
            &sink,
            "r",
            "land",
            "res",
            "K1",
            b"{\"mode\":\"union\"}",
            true,
            vec!["orchestrator:o".into()],
            3,
            dummy_verb,
        )
        .expect_err("a corrupt matched ledger entry must fail closed, not re-execute");
        assert_eq!(err.status, 503);
        assert_eq!(err.code, "ENGINE_UNAVAILABLE");
        let queued_after = sink
            .log
            .borrow()
            .records()
            .iter()
            .filter(|r| r.kind == "pr.queued")
            .count();
        assert_eq!(
            queued_before, queued_after,
            "the verb must NOT have re-executed (no new pr.queued)"
        );
    }

    #[test]
    fn cas_conflict_with_different_writer_reloads_and_both_survive() {
        // A concurrent DIFFERENT-key writer lands an unrelated record between our
        // load and persist. The first persist loses the CAS (head moved); the door
        // reloads and re-runs against the advanced head. Neither write is dropped.
        let sink = cas_sink_with_pr("1");
        *sink.pending_bump.borrow_mut() = vec![("pr.landed".into(), r#"{"verdict":"ok"}"#.into())];
        let accepted = with_write(
            &sink,
            "r",
            "land",
            "res",
            "MINE",
            b"{\"mode\":\"union\"}",
            true,
            vec!["orchestrator:o".into()],
            2,
            dummy_verb,
        )
        .expect("must recover from the lost race, not fail");
        let log = sink.log.borrow();
        let kinds: Vec<&str> = log.records().iter().map(|r| r.kind.as_str()).collect();
        // The other writer's record survives AND ours landed — no last-writer-wins.
        assert!(
            kinds.contains(&"pr.landed"),
            "the concurrent write must survive"
        );
        assert_eq!(
            kinds.iter().filter(|k| **k == "pr.queued").count(),
            1,
            "our effect landed exactly once after the reload"
        );
        assert!(
            kinds.contains(&IDEM_RECORDED_KIND),
            "our idempotency ledger entry persisted (not dropped with the lost attempt)"
        );
        // The seq we returned is the one actually on the persisted head.
        assert!(log.records().iter().any(|r| r.seq == accepted.seq));
    }

    #[test]
    fn cas_conflict_same_key_winner_collapses_to_replay_no_double_effect() {
        // The classic same-key race: another request with OUR key commits first
        // (its effect + idem.recorded). Our persist loses the CAS; on reload we find
        // the winner's ledger entry and REPLAY it — exactly one effect, no double.
        let body = b"{\"mode\":\"union\"}";
        let sink = cas_sink_with_pr("1");
        *sink.pending_bump.borrow_mut() = winner_records("1", "SAME", body);
        let accepted = with_write(
            &sink,
            "r",
            "land",
            "res",
            "SAME",
            body,
            true,
            vec!["orchestrator:o".into()],
            2,
            dummy_verb,
        )
        .expect("same-key race must resolve to a replay, not an error");
        let log = sink.log.borrow();
        assert_eq!(
            log.records()
                .iter()
                .filter(|r| r.kind == "pr.queued")
                .count(),
            1,
            "the same-key race must collapse to ONE effect (the winner's)"
        );
        assert_eq!(
            log.records()
                .iter()
                .filter(|r| r.kind == IDEM_RECORDED_KIND)
                .count(),
            1,
            "exactly one idempotency ledger entry for the key"
        );
        // We returned the winner's recorded outcome (replay), not a fresh one.
        assert!(log.records().iter().any(|r| r.seq == accepted.seq));
    }

    #[test]
    fn cas_exhaustion_fails_transient_503_with_no_partial_write() {
        // Sustained contention: every persist loses the race. After the bounded
        // budget the door fails honest + transient (503), and NONE of our records
        // (verb effect or idempotency entry) leaked onto the head.
        let sink = CasSink {
            always_bump: true,
            ..cas_sink_with_pr("1")
        };
        let err = with_write(
            &sink,
            "r",
            "land",
            "res",
            "MINE",
            b"{\"mode\":\"union\"}",
            true,
            vec!["orchestrator:o".into()],
            2,
            dummy_verb,
        )
        .expect_err("sustained contention must fail, not silently drop");
        assert_eq!(err.status, 503, "exhaustion is a transient-retry failure");
        let log = sink.log.borrow();
        assert_eq!(
            log.records()
                .iter()
                .filter(|r| r.kind == "pr.queued")
                .count(),
            0,
            "no verb effect leaked from a losing attempt"
        );
        assert_eq!(
            log.records()
                .iter()
                .filter(|r| r.kind == IDEM_RECORDED_KIND)
                .count(),
            0,
            "no idempotency entry leaked from a losing attempt"
        );
        // It did try the full budget (each churns the head once).
        assert_eq!(*sink.version.borrow(), u64::from(MAX_CAS_ATTEMPTS));
    }

    // ── the ACCOUNT write-door (GDPR1) ───────────────────────────────────────

    use hugit_http_contracts::write_requests::AccountEraseReq;

    /// A CAS-enforcing in-memory account sink (load-or-create): an absent log is the
    /// empty world (`Absent`); a present one carries a monotonic `Version`. Mirrors
    /// `CasSink` but with account load-or-create semantics + optional perpetual churn.
    struct AcctSink {
        log: RefCell<Option<EventLog>>,
        version: RefCell<u64>,
        winner: RefCell<Option<EventLog>>, // a same-key concurrent winner injected once
        always_bump: bool,
    }
    impl AcctSink {
        fn empty() -> Self {
            AcctSink {
                log: RefCell::new(None),
                version: RefCell::new(0),
                winner: RefCell::new(None),
                always_bump: false,
            }
        }
    }
    impl AccountLogSink for AcctSink {
        fn load_account(&self, _account: &str) -> Result<(EventLog, CasToken), EngineErr> {
            match &*self.log.borrow() {
                None => Ok((EventLog::new(), CasToken::Absent)),
                Some(l) => Ok((
                    l.clone(),
                    CasToken::Version(self.version.borrow().to_string()),
                )),
            }
        }
        fn persist_account(
            &self,
            _account: &str,
            log: &EventLog,
            expected: &CasToken,
        ) -> Result<(), EngineErr> {
            // A same-key concurrent winner commits first (once) → head moves.
            if let Some(w) = self.winner.borrow_mut().take() {
                *self.log.borrow_mut() = Some(w);
                *self.version.borrow_mut() += 1;
            }
            if self.always_bump {
                let mut cur = self.log.borrow_mut().take().unwrap_or_default();
                let at = cur.records().len() as u64 + 1;
                cur.append_for_test("other.churn", vec!["orchestrator:x".into()], "{}", at);
                *self.log.borrow_mut() = Some(cur);
                *self.version.borrow_mut() += 1;
            }
            let head = match &*self.log.borrow() {
                None => CasToken::Absent,
                Some(_) => CasToken::Version(self.version.borrow().to_string()),
            };
            if *expected != head {
                return Err(EngineErr::cas_conflict());
            }
            *self.log.borrow_mut() = Some(log.clone());
            *self.version.borrow_mut() += 1;
            Ok(())
        }
    }
    // SAFETY: single-threaded tests only (same as MemSink/CasSink).
    unsafe impl Sync for AcctSink {}

    fn erase(account: &str) -> AccountEraseReq {
        AccountEraseReq {
            confirm: account.into(),
        }
    }
    fn tenant(org: &str) -> Vec<String> {
        vec![format!("clerk:{org}:user-1")]
    }
    /// Run the account door with the REAL `write_account_erase` verb (end-to-end).
    fn run_erase(
        sink: &AcctSink,
        account: &str,
        req: &AccountEraseReq,
        key: &str,
        step_up: bool,
        principal: Vec<String>,
        at: u64,
    ) -> Result<Accepted, EngineErr> {
        let body = serde_json::to_vec(req).unwrap();
        with_account_write(
            sink,
            account,
            "erasure",
            "account/erase",
            key,
            &body,
            step_up,
            principal,
            at,
            |log, p, at| verbs::write_account_erase::write_account_erase(log, req, p, at),
        )
    }

    #[test]
    fn account_erase_first_request_seeds_genesis() {
        let sink = AcctSink::empty();
        let acc = run_erase(
            &sink,
            "org-a",
            &erase("org-a"),
            "K1",
            true,
            tenant("org-a"),
            1,
        )
        .expect("first erase stages a request");
        assert_eq!(acc.state.as_deref(), Some("requested"));
        let log = sink.log.borrow();
        let recs = log.as_ref().expect("genesis persisted").records();
        // Exactly the erasure.requested + its idem.recorded — nothing else.
        assert_eq!(
            recs.iter()
                .filter(|r| r.kind == verbs::write_account_erase::ERASURE_REQUESTED_KIND)
                .count(),
            1
        );
        assert_eq!(
            recs.iter().filter(|r| r.kind == IDEM_RECORDED_KIND).count(),
            1
        );
    }

    #[test]
    fn account_erase_step_up_required() {
        let sink = AcctSink::empty();
        let e = run_erase(
            &sink,
            "org-a",
            &erase("org-a"),
            "K1",
            false,
            tenant("org-a"),
            1,
        )
        .expect_err("erasure without step-up must 403");
        assert_eq!(e.status, 403);
        assert_eq!(e.code, "STEP_UP_REQUIRED");
        assert!(sink.log.borrow().is_none(), "no state written on a 403");
    }

    #[test]
    fn account_erase_replay_is_idempotent_no_double_request() {
        let sink = AcctSink::empty();
        let first = run_erase(
            &sink,
            "org-a",
            &erase("org-a"),
            "K1",
            true,
            tenant("org-a"),
            1,
        )
        .unwrap();
        let replay = run_erase(
            &sink,
            "org-a",
            &erase("org-a"),
            "K1",
            true,
            tenant("org-a"),
            2,
        )
        .unwrap();
        assert_eq!(first.seq, replay.seq, "replay returns the original outcome");
        let log = sink.log.borrow();
        assert_eq!(
            log.as_ref()
                .unwrap()
                .records()
                .iter()
                .filter(|r| r.kind == verbs::write_account_erase::ERASURE_REQUESTED_KIND)
                .count(),
            1,
            "exactly ONE erasure.requested despite the replay"
        );
    }

    #[test]
    fn account_erase_missing_idem_key_is_400() {
        let sink = AcctSink::empty();
        let e = run_erase(
            &sink,
            "org-a",
            &erase("org-a"),
            "",
            true,
            tenant("org-a"),
            1,
        )
        .expect_err("empty idem key must 400");
        assert_eq!(e.status, 400);
        assert_eq!(e.code, "IDEMPOTENCY_REQUIRED");
    }

    #[test]
    fn account_erase_wrong_confirm_is_400_no_state() {
        let sink = AcctSink::empty();
        let e = run_erase(
            &sink,
            "org-a",
            &erase("WRONG"),
            "K1",
            true,
            tenant("org-a"),
            1,
        )
        .expect_err("mismatched confirm must 400");
        assert_eq!(e.status, 400);
        assert!(
            sink.log.borrow().is_none(),
            "a rejected verb writes nothing"
        );
    }

    #[test]
    fn account_erase_same_key_race_collapses_to_replay() {
        // A same-key concurrent winner commits its (erasure.requested + idem.recorded)
        // first; our persist loses the CAS; on reload we replay — exactly one request.
        let sink = AcctSink::empty();
        // Build the winner's log by running the door against a throwaway sink.
        let winner_sink = AcctSink::empty();
        run_erase(
            &winner_sink,
            "org-a",
            &erase("org-a"),
            "SAME",
            true,
            tenant("org-a"),
            1,
        )
        .unwrap();
        *sink.winner.borrow_mut() = winner_sink.log.borrow().clone();
        let acc = run_erase(
            &sink,
            "org-a",
            &erase("org-a"),
            "SAME",
            true,
            tenant("org-a"),
            2,
        )
        .expect("same-key race resolves to a replay");
        let log = sink.log.borrow();
        assert_eq!(
            log.as_ref()
                .unwrap()
                .records()
                .iter()
                .filter(|r| r.kind == verbs::write_account_erase::ERASURE_REQUESTED_KIND)
                .count(),
            1,
            "the race collapses to ONE erasure.requested (the winner's)"
        );
        assert!(
            log.as_ref()
                .unwrap()
                .records()
                .iter()
                .any(|r| r.seq == acc.seq),
            "we returned the winner's recorded outcome (replay)"
        );
    }
}
