//! Art.17 provenance redaction pass (ADR-0004, follow-up legs 2/4/5) — the transform
//! the erasure executor drives to render a subject's already-written CLEARTEXT
//! identifiers UNRECOVERABLE while the tamper-evident chain still verifies.
//!
//! It pairs the shreddable pseudonym ([`crate::provenance_pii`]) with the
//! hash-preserving redaction primitive ([`hugit_refstore::redact_record`] +
//! [`hugit_refstore::verify_chain`]'s redaction-awareness):
//!
//! - every provenance field carrying the subject's cleartext identity — a
//!   `clerk:{account}:{user}` principal, the bare `account` slug, an `owner_tenant`
//!   that equals it — is rewritten to the subject's `subj:<hmac>` pseudonym, and
//! - an APPEND-ONLY `provenance.redaction` marker is added per redacted record so the
//!   chain re-verifies via the preserved `this_hash` (never a history rewrite).
//!
//! After the executor SHREDS the subject key, the pseudonyms are one-way and the
//! cleartext is gone. This module owns ONLY the pure log transform; the executor
//! ([`crate::writes::erasure`]) owns the load→redact→CAS-persist orchestration + the
//! shred ordering.

use crate::provenance_pii::{SUBJECT_PSEUDONYM_PREFIX, SubjectKey, SubjectPseudonym};
use hugit_refstore::{EventLog, RedactionCommitment};
use serde_json::Value;

/// Rewrite ONE field value to the pseudonym IFF it carries the subject's cleartext
/// identity, else leave it untouched. The subject's cleartext appears as either:
/// - the bare account slug (`"acme"`), or
/// - a `clerk:{account}:{user}` principal (org == account slug, audit B2).
///
/// A value that is already pseudonymous, an unrelated principal (`orchestrator:…`, a
/// DIFFERENT tenant's `clerk:other:…`), or any non-identity string is returned as-is —
/// so a co-authored record keeps every non-subject principal intact.
#[must_use]
fn redact_value(value: &str, account: &str, pseudonym: &SubjectPseudonym) -> String {
    if value.starts_with(SUBJECT_PSEUDONYM_PREFIX) {
        return value.to_string(); // already pseudonymous — idempotent
    }
    if value == account {
        return pseudonym.as_str().to_string();
    }
    // `clerk:{account}:…` — the subject's authored principal (only when the org segment
    // is EXACTLY this account; a different tenant's principal is untouched).
    if org_of_clerk(value) == Some(account) {
        return pseudonym.as_str().to_string();
    }
    value.to_string()
}

/// Recursively rewrite every string leaf of a JSON payload via [`redact_value`]. Object
/// keys and structure are preserved; only VALUES that are the subject's cleartext become
/// the pseudonym. Returns the re-canonicalised payload (so it re-chains deterministically).
#[must_use]
fn redact_payload(payload: &str, account: &str, pseudonym: &SubjectPseudonym) -> String {
    let Ok(v) = serde_json::from_str::<Value>(payload) else {
        // A non-JSON payload can still be the raw cleartext identity — redact the whole.
        return redact_value(payload, account, pseudonym);
    };
    let redacted = redact_value_tree(v, account, pseudonym);
    let s = redacted.to_string();
    hugit_refstore::canonical_json(&s).unwrap_or(s)
}

fn redact_value_tree(v: Value, account: &str, pseudonym: &SubjectPseudonym) -> Value {
    match v {
        Value::String(s) => Value::String(redact_value(&s, account, pseudonym)),
        Value::Array(a) => Value::Array(
            a.into_iter()
                .map(|e| redact_value_tree(e, account, pseudonym))
                .collect(),
        ),
        Value::Object(o) => Value::Object(
            o.into_iter()
                .map(|(k, e)| (k, redact_value_tree(e, account, pseudonym)))
                .collect(),
        ),
        scalar => scalar,
    }
}

/// FORWARD-write pseudonymisation (ADR-0004 leg 2): map a live request's
/// `principal_chain` to its stored, pseudonymous form, so a NEW record carries `subj:<hmac>`
/// instead of the cleartext `clerk:{org}:{user}` at the point of append. For each
/// `clerk:{org}:{user}` entry it ensures the ORG's durable key and substitutes the pseudonym;
/// every non-`clerk:` principal (`orchestrator:…`, `agent:…`) is passed through unchanged.
/// Fail-closed: a durable key-store fault is an `Err` (the write aborts rather than storing
/// cleartext or a wrong pseudonym).
///
/// GRANULARITY (FULL-PRINCIPAL, per-ACCOUNT key). The HMAC INPUT is the WHOLE principal string
/// `clerk:{org}:{user}` — NOT the bare org — so two distinct users in one org
/// (`clerk:acme:user-1` vs `clerk:acme:user-2`) get DISTINCT pseudonyms. This preserves
/// per-user granularity in the principal-keyed idempotency ledger (two users sharing an
/// Idempotency-Key on the same verb+resource do NOT collide) AND per-user audit accountability
/// (Art.5(2)). The KEY is still fetched per-ORG (`subject_key_ensure(org)`), so an account erase
/// shreds exactly ONE key and renders ALL of that org's per-user pseudonyms unrecoverable at
/// once.
///
/// IMPORTANT — this is applied ONLY to what is STORED, AFTER the write-path authz decisions
/// (which run on the live cleartext chain): authz keys on the projected `owner_tenant` + the
/// live request principal, never on the stored `principal_chain` (verified — see
/// `crate::authz`), so pseudonymising the stored chain leaves the read/write authz gates
/// unaffected. A stable pseudonym per (org, user) also keeps the principal-keyed idempotency
/// match consistent (same principal ⇒ same pseudonym while the key lives).
pub fn pseudonymize_write_principal_chain(
    state: &AppState,
    chain: &[String],
) -> Result<Vec<String>, EngineErr> {
    let mut out = Vec::with_capacity(chain.len());
    for p in chain {
        out.push(match org_of_clerk(p) {
            Some(org) => {
                let key = state.subject_key_ensure(org)?;
                // HMAC the FULL principal (per-user distinct), under the per-ORG key.
                SubjectPseudonym::derive(&key, p).as_str().to_string()
            }
            None => p.clone(),
        });
    }
    Ok(out)
}

/// READ-ONLY, single-principal pseudonym for the idempotency LOOKUP arm (ADR-0004 leg 2). The
/// stored pseudonym form of `principal` WITHOUT minting a key:
/// - a non-`clerk:` principal → `None` (its stored form equals cleartext — the cleartext match
///   arm already covers it), and
/// - a `clerk:{org}:{user}` whose account key is ABSENT/shredded → `None` (no pseudonymised
///   ledger entry could exist for it — writing one requires a minted key).
///
/// Used FLAG-INDEPENDENTLY by `idem_lookup`: the lookup ALWAYS considers both the cleartext AND
/// this pseudonym form regardless of the `HUGIT_SERVE_PROV_PSEUDONYM` kill-switch, so a key
/// first EXECUTED while pseudonymisation was ON still dedups after the switch flips OFF (no
/// re-execute on the live forge). The kill-switch governs only whether NEW writes are
/// pseudonymised, never whether the lookup reconstructs the pseudonym. Fail-closed on a durable
/// key-store fault (`Err`), so a transient read fault cannot silently drop the pseudonym arm and
/// re-execute.
pub fn readonly_principal_pseudonym(
    state: &AppState,
    principal: &str,
) -> Result<Option<String>, EngineErr> {
    match org_of_clerk(principal) {
        Some(org) => Ok(state.subject_key_for(org)?.map(|key| {
            SubjectPseudonym::derive(&key, principal)
                .as_str()
                .to_string()
        })),
        None => Ok(None),
    }
}

/// The org segment of a `clerk:{org}:{user}` principal (non-empty), else `None`.
fn org_of_clerk(principal: &str) -> Option<&str> {
    let rest = principal.strip_prefix("clerk:")?;
    let org = rest.split(':').next().unwrap_or("");
    (!org.is_empty()).then_some(org)
}

/// Whether `record`'s `principal_chain`/`payload` carry ANY of the subject's cleartext —
/// i.e. redacting it would change bytes. Used to skip records that need no redaction (so
/// no spurious marker is emitted) and to prove, post-pass, that none remain.
#[must_use]
pub fn record_has_subject_cleartext(
    principal_chain: &[String],
    payload: &str,
    account: &str,
    pseudonym: &SubjectPseudonym,
) -> bool {
    principal_chain
        .iter()
        .any(|p| redact_value(p, account, pseudonym) != *p)
        || redact_payload(payload, account, pseudonym) != payload
}

/// Produce a redaction-rewritten copy of `log` for `account` under `key`, plus the
/// per-record [`RedactionCommitment`]s the append-only markers must carry. `None` when the
/// log carries NO cleartext for this subject (nothing to redact — the caller persists
/// nothing). The returned log has each subject-bearing record's `principal_chain`/`payload`
/// pseudonymised WITH ITS `this_hash` PRESERVED; the caller appends one
/// `provenance.redaction` marker per `(seq, commitment)` before persisting.
///
/// Pure + deterministic; performs no I/O.
#[must_use]
pub fn redact_log_for_subject(
    log: &EventLog,
    account: &str,
    key: &SubjectKey,
) -> Option<(EventLog, Vec<(u64, RedactionCommitment)>)> {
    let pseudonym = SubjectPseudonym::derive(key, account);
    let mut rebuilt = EventLog::new();
    let mut markers: Vec<(u64, RedactionCommitment)> = Vec::new();

    for r in log.records() {
        let new_chain: Vec<String> = r
            .principal_chain
            .iter()
            .map(|p| redact_value(p, account, &pseudonym))
            .collect();
        let new_payload = redact_payload(&r.payload, account, &pseudonym);

        if new_chain == r.principal_chain && new_payload == r.payload {
            // Unchanged — no subject cleartext here; keep it verbatim.
            rebuilt
                .push_record(r.clone())
                .expect("re-pushing an existing record preserves seq order");
        } else {
            let (redacted, commitment) = hugit_refstore::redact_record(r, new_chain, new_payload);
            rebuilt
                .push_record(redacted)
                .expect("a redacted record preserves seq/prev/this hashes");
            markers.push((r.seq, commitment));
        }
    }

    if markers.is_empty() {
        return None; // nothing carried the subject's cleartext
    }
    Some((rebuilt, markers))
}

// ── the executor orchestration (ADR-0004 leg 5): shred + redact on a COMPLETED erase ──

use crate::error::EngineErr;
use crate::provenance_pii::{ERASURE_PII_SHREDDED_KIND, PseudonymousErasureRecord};
use crate::state::AppState;
use crate::writes::{AccountLogSink, LogSink, MAX_CAS_ATTEMPTS};
use hugit_refstore::{Endpoint, PrincipalClass};

/// Whether the account log durably carries a record of `kind` (fail-closed on a load fault
/// so an indeterminate read never masquerades as "absent" and skips the PII step).
fn account_has_kind(state: &AppState, account: &str, kind: &str) -> Result<bool, EngineErr> {
    let sink: &dyn AccountLogSink = state;
    let (log, _) = sink.load_account(account)?;
    Ok(log.records().iter().any(|r| r.kind == kind))
}

/// The `requested_at` of the standing erasure request (for the retained accountability
/// record), read best-effort from the account log; falls back to `default_at`.
fn requested_at_of(log: &EventLog, default_at: u64) -> u64 {
    log.records()
        .iter()
        .find(|r| r.kind == crate::writes::verbs::write_account_erase::ERASURE_REQUESTED_KIND)
        .map(|r| r.recorded_at)
        .unwrap_or(default_at)
}

/// Append one `provenance.redaction` marker per redacted record onto `rebuilt`, under the
/// pseudonym (never cleartext) — system bookkeeping, routed through the guarded door like
/// the idempotency ledger.
fn append_markers(
    rebuilt: &mut EventLog,
    markers: &[(u64, RedactionCommitment)],
    pseudonym: &SubjectPseudonym,
    at: u64,
) -> Result<(), EngineErr> {
    for (seq, commitment) in markers {
        rebuilt
            .append_authorized(
                PrincipalClass::Orchestrator,
                Endpoint::Land,
                hugit_refstore::PROVENANCE_REDACTION_KIND,
                vec![pseudonym.as_str().to_string()],
                commitment.to_payload(*seq),
                at,
            )
            .map_err(|d| {
                EngineErr::unavailable(format!(
                    "redaction marker append denied: {}",
                    d.reason.code()
                ))
            })?;
    }
    Ok(())
}

/// Load → redact → CAS-persist the ACCOUNT log for `account` (bounded retry). A no-op when
/// the account log carries no subject cleartext. Fail-closed on a durable fault.
fn redact_account_log(
    state: &AppState,
    account: &str,
    key: &SubjectKey,
    pseudonym: &SubjectPseudonym,
    at: u64,
) -> Result<(), EngineErr> {
    let sink: &dyn AccountLogSink = state;
    for _ in 0..MAX_CAS_ATTEMPTS {
        let (log, token) = sink.load_account(account)?;
        let Some((mut rebuilt, markers)) = redact_log_for_subject(&log, account, key) else {
            return Ok(()); // nothing to redact
        };
        append_markers(&mut rebuilt, &markers, pseudonym, at)?;
        match sink.persist_account(account, &rebuilt, &token) {
            Ok(()) => return Ok(()),
            Err(e) if e.is_cas_conflict() => continue,
            Err(e) => return Err(e),
        }
    }
    Err(EngineErr::unavailable(
        "redação de PII do log de conta sob contenção — tente novamente",
    ))
}

/// Load → redact → CAS-persist ONE repo log (bounded retry). A no-op when the repo log
/// carries no subject cleartext. Fail-closed on a durable fault.
fn redact_repo_log(
    state: &AppState,
    repo: &str,
    account: &str,
    key: &SubjectKey,
    pseudonym: &SubjectPseudonym,
    at: u64,
) -> Result<(), EngineErr> {
    let sink: &dyn LogSink = state;
    for _ in 0..MAX_CAS_ATTEMPTS {
        let (log, token) = sink.load(repo)?;
        let Some((mut rebuilt, markers)) = redact_log_for_subject(&log, account, key) else {
            return Ok(());
        };
        append_markers(&mut rebuilt, &markers, pseudonym, at)?;
        match sink.persist(repo, &rebuilt, &token) {
            Ok(()) => return Ok(()),
            Err(e) if e.is_cas_conflict() => continue,
            Err(e) => return Err(e),
        }
    }
    Err(EngineErr::unavailable(
        "redação de PII do log de repositório sob contenção — tente novamente",
    ))
}

/// Load → redact → CAS-persist the per-tenant REPO REGISTRY (`_tenants/{account}.json`) — the
/// durable cap denominator ([`crate::tenant_registry`]). A no-op when it carries no subject
/// cleartext (the forward-write door already pseudonymised the principal, or the tenant has no
/// registry). Fail-closed on a durable fault.
///
/// This closes the second half of the registry hole: the forward-write door
/// ([`AppState::register_repo_in_tenant`]) now stores the pseudonym, but a record written while
/// the `HUGIT_SERVE_PROV_PSEUDONYM` kill-switch was OFF — or any legacy record predating that
/// wiring — still holds a cleartext `clerk:{account}:{user}` principal, which the erase pass must
/// render unrecoverable BEFORE the key shred (after the shred the pseudonym is no longer
/// derivable, so a surviving cleartext principal could NEVER be redacted). Hash-preserving
/// (append-only `provenance.redaction` markers), so `verify_chain` still passes and the fold —
/// which keys on the `repo` PAYLOAD field, never the principal — is untouched (the count/cap does
/// not regress).
fn redact_tenant_registry(
    state: &AppState,
    account: &str,
    key: &SubjectKey,
    pseudonym: &SubjectPseudonym,
    at: u64,
) -> Result<(), EngineErr> {
    for _ in 0..MAX_CAS_ATTEMPTS {
        let (log, token) = state.load_tenant_registry(account)?;
        let Some((mut rebuilt, markers)) = redact_log_for_subject(&log, account, key) else {
            return Ok(()); // nothing to redact — already pseudonymous / no registry
        };
        append_markers(&mut rebuilt, &markers, pseudonym, at)?;
        match state.persist_tenant_registry(account, &rebuilt, &token) {
            Ok(()) => return Ok(()),
            Err(e) if e.is_cas_conflict() => continue,
            Err(e) => return Err(e),
        }
    }
    Err(EngineErr::unavailable(
        "redação do registro de tenant sob contenção — tente novamente",
    ))
}

/// The COMPLETED-erase provenance-PII step (ADR-0004 legs 4/5), driven ONCE from the
/// executor's `Executed` branch — NEVER on `partial`/`cancelled` (those must leave the key
/// intact so the subject's non-erased data stays resolvable):
///
/// 1. `ensure_key` the subject's durable key + derive its pseudonym.
/// 2. Append the retained pseudonymous accountability record (`erasure.pii_shredded`,
///    Art.5(2)) to the account log — it carries the pseudonym + dsr id + timestamps, NO
///    cleartext.
/// 3. Redact (hash-preserving) the account log, the per-tenant repo registry
///    (`_tenants/{account}.json`), AND every tombstoned repo log, so no cleartext account
///    slug / `clerk:` principal survives ANYWHERE while `verify_chain` still passes.
/// 4. SHRED the subject key LAST — after which the pseudonyms are one-way and the cleartext
///    is unrecoverable. This is the point the executor may truthfully claim the provenance
///    cleartext is erased.
///
/// Fail-closed: ANY durable fault returns `Err` (the caller must not then claim `executed`);
/// idempotent under retry (redaction skips already-pseudonymous records; the accountability
/// record de-dupes on kind; shred is idempotent).
pub fn shred_and_redact_on_execute(
    state: &AppState,
    account: &str,
    repo_slugs: &[String],
    dsr_id: &str,
    at: u64,
) -> Result<(), EngineErr> {
    // 1. Resolve the durable key. `None` is AMBIGUOUS — either "already shredded"
    //    (a completed prior run: shred is the LAST step, so `None` ⇒ the accountability
    //    record + redaction already landed) OR "never minted" (a subject that never wrote a
    //    pseudonym). Disambiguate on the accountability record:
    //    - `None` + accountability PRESENT ⇒ this erase already completed → idempotent no-op
    //      (this is the MF2 fix: we do NOT call `ensure_key`, which would create-conflict on
    //      the shred tombstone and 503 forever — a re-drive after a completed shred converges);
    //    - `None` + accountability ABSENT ⇒ never minted → mint now so we can derive the
    //      pseudonym and redact;
    //    - `Some(key)` ⇒ mid-flight (shred not yet reached) → drive to completion.
    let accountability_done = account_has_kind(state, account, ERASURE_PII_SHREDDED_KIND)?;
    let key = match state.subject_key_for(account)? {
        Some(k) => k,
        None if accountability_done => return Ok(()), // shredded + complete → no-op self-heal
        None => state.subject_key_ensure(account)?,   // never minted → mint to redact
    };
    let pseudonym = SubjectPseudonym::derive(&key, account);

    // 2. Retain the pseudonymous accountability record on the account log (de-dupes on kind).
    let acct_sink: &dyn AccountLogSink = state;
    for _ in 0..MAX_CAS_ATTEMPTS {
        let (mut log, token) = acct_sink.load_account(account)?;
        if log
            .records()
            .iter()
            .any(|r| r.kind == ERASURE_PII_SHREDDED_KIND)
        {
            break; // already retained — idempotent
        }
        let requested_at = requested_at_of(&log, at);
        let rec = PseudonymousErasureRecord {
            subject_pseudonym: pseudonym.clone(),
            dsr_id: (!dsr_id.is_empty()).then(|| dsr_id.to_string()),
            requested_at,
            executed_at: at,
        };
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            ERASURE_PII_SHREDDED_KIND,
            vec![pseudonym.as_str().to_string()],
            rec.to_canonical_payload(),
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!(
                "accountability record append denied: {}",
                d.reason.code()
            ))
        })?;
        match acct_sink.persist_account(account, &log, &token) {
            Ok(()) => break,
            Err(e) if e.is_cas_conflict() => continue,
            Err(e) => return Err(e),
        }
    }

    // 3. Redact the account log + the per-tenant repo registry + every tombstoned repo log
    //    (hash-preserving). The registry (`_tenants/{account}.json`) is enumerated HERE — it is
    //    a durable, cleartext-bearing artifact the account/repo logs do not cover — so no
    //    cleartext `clerk:` principal survives ANYWHERE for the account after the shred below.
    redact_account_log(state, account, &key, &pseudonym, at)?;
    redact_tenant_registry(state, account, &key, &pseudonym, at)?;
    for repo in repo_slugs {
        redact_repo_log(state, repo, account, &key, &pseudonym, at)?;
    }

    // 4. SHRED the key LAST — the cleartext is now unrecoverable.
    state.subject_key_shred(account)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance_pii::is_pseudonymous;
    use hugit_refstore::{Endpoint, PrincipalClass, verify_chain};

    fn key() -> SubjectKey {
        SubjectKey::from_bytes([9u8; 32])
    }

    fn subject_log(account: &str) -> EventLog {
        let mut log = EventLog::new();
        // A repo.meta owning the subject.
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            "repo.meta",
            vec!["orchestrator:hugit".into()],
            hugit_refstore::canonical_json(
                &serde_json::json!({"visibility":"public","owner_tenant":account}).to_string(),
            )
            .unwrap(),
            1,
        )
        .unwrap();
        // A subject-authored record: cleartext principal + payload.
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            "erasure.requested",
            vec![format!("clerk:{account}:user-1")],
            hugit_refstore::canonical_json(
                &serde_json::json!({"account":account,"subject":account,"state":"requested"})
                    .to_string(),
            )
            .unwrap(),
            2,
        )
        .unwrap();
        // An UNRELATED record (different tenant) that must be left intact.
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            "pr.opened",
            vec!["clerk:other-org:u".into()],
            hugit_refstore::canonical_json(&serde_json::json!({"pr":1}).to_string()).unwrap(),
            3,
        )
        .unwrap();
        log
    }

    #[test]
    fn redaction_removes_cleartext_and_the_chain_still_verifies() {
        let account = "acme";
        let log = subject_log(account);
        verify_chain(log.records()).expect("baseline verifies");

        let (mut rebuilt, markers) =
            redact_log_for_subject(&log, account, &key()).expect("there is cleartext to redact");
        // Append the markers (append-only), exactly as the executor will.
        for (i, (seq, commitment)) in markers.iter().enumerate() {
            rebuilt
                .append_authorized(
                    PrincipalClass::Orchestrator,
                    Endpoint::Land,
                    hugit_refstore::PROVENANCE_REDACTION_KIND,
                    vec![
                        SubjectPseudonym::derive(&key(), account)
                            .as_str()
                            .to_string(),
                    ],
                    commitment.to_payload(*seq),
                    100 + i as u64,
                )
                .unwrap();
        }

        // 1. The chain STILL verifies (tamper-evidence intact via the markers).
        verify_chain(rebuilt.records()).expect("redacted chain verifies");

        // 2. NO record carries the subject's cleartext account slug or clerk principal.
        for r in rebuilt.records() {
            assert!(
                !r.payload.contains(&format!("\"{account}\"")),
                "no payload carries the cleartext account slug"
            );
            assert!(
                r.principal_chain
                    .iter()
                    .all(|p| p != &format!("clerk:{account}:user-1")),
                "no principal_chain carries the cleartext clerk principal"
            );
        }

        // 3. The subject-authored record is now pseudonymous; the UNRELATED tenant's
        //    record is untouched.
        let subj_rec = &rebuilt.records()[1];
        assert!(subj_rec.principal_chain.iter().all(|p| is_pseudonymous(p)));
        let other = &rebuilt.records()[2];
        assert_eq!(other.principal_chain, vec!["clerk:other-org:u".to_string()]);
    }

    #[test]
    fn a_log_without_the_subject_needs_no_redaction() {
        let log = subject_log("acme");
        // A DIFFERENT subject's key/account → nothing matches → no redaction.
        assert!(redact_log_for_subject(&log, "nobody", &key()).is_none());
    }

    #[test]
    fn redaction_is_idempotent() {
        let account = "acme";
        let log = subject_log(account);
        let (rebuilt, _markers) = redact_log_for_subject(&log, account, &key()).unwrap();
        // Re-running over the already-pseudonymised records finds nothing new.
        assert!(redact_log_for_subject(&rebuilt, account, &key()).is_none());
    }
}
