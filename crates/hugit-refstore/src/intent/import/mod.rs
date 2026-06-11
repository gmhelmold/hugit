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
//! Import appends over D1a's frozen append API, **through the D14 authorization
//! guard** ([`EventLog::append_authorized`]): it appends, it never rewrites.
//!
//! # D14 routing — intent authorship is the `Push` verb (every class allowed)
//!
//! `intent.landed` is the act of authoring/landing an intent onto the log. Per
//! the whitepaper §7 product surface, intent authorship at this altitude is the
//! universal git verb — **every** principal class (human / orchestrator /
//! worker-subagent / model) may do it ("Everyone … every git command"). The
//! matrix cell that encodes that is [`Endpoint::Push`] (allowed for all
//! classes), **not** [`Endpoint::Land`] (the orchestrator-only batch-integration
//! verb). So this emitter routes through [`EventLog::append_authorized`] under
//! [`Endpoint::Push`], with the actor class read from the `principal_chain`
//! tail (falling back to [`PrincipalClass::Worker`] — the honest intent-class
//! default — when the tail is unclassifiable, the disclosed P2 authn seam).
//!
//! This ALLOWS the legitimate subagent-authored landed-intent path (the matrix
//! permits it) while making the mutation surface unbypassable: the path is
//! always gated, and the denial arm (fail-closed) is wired should the matrix
//! ever change. This matches the CLI `intent new --log` routing established in
//! `hugit-cli/src/intent/canonical_log.rs` (one altitude, one matrix cell).
//!
//! [`IntentSidecar`]: hugit_contracts::intent_sidecar::IntentSidecar
//! [`EventLog::append_authorized`]: crate::log::EventLog::append_authorized

use hugit_contracts::event_record::EventRecord;
use hugit_contracts::intent_sidecar::IntentSidecar;

use crate::authz::{DenyReason, Endpoint, PrincipalClass};
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
    /// The D14 guard denied the intent-landing append. Intent authorship is the
    /// `Push` verb (every class allowed), so this arm is **unreachable today** —
    /// it exists for fail-closed completeness: if the matrix ever restricts the
    /// `Push` cell, the `intent.landed` event is **not** appended (an
    /// `authz.denied` audit record (③) was written instead) and this carries the
    /// [`DenyReason`]. Wired so the mutation path is never silently un-gated.
    Denied(DenyReason),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::EmptyIntentId => write!(f, "sidecar has an empty intent_id"),
            ImportError::DuplicateIntentId { intent_id } => {
                write!(f, "intent_id {intent_id} already landed on the log")
            }
            ImportError::Denied(reason) => {
                write!(f, "intent import denied by D14 guard: {}", reason.code())
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
///
/// The `intent.landed` append routes through the D14 [`EventLog::append_authorized`]
/// guard under [`Endpoint::Push`] (intent authorship is the universal git verb —
/// every principal class is permitted, including worker subagents; see the
/// module doc). The actor class is read from the `principal_chain` tail, falling
/// back to [`PrincipalClass::Worker`] when unclassifiable. The guard ALLOWS the
/// legitimate landed-intent path and audits the mutation surface; the denial
/// arm ([`ImportError::Denied`]) is fail-closed and unreachable while `Push`
/// stays universally allowed.
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

    // Classify the actor (tail of the chain) for the D14 guard; fall back to
    // Worker — the intent-class actor — when unclassifiable (the disclosed P2
    // authn seam: classification is caller-supplied today). Route through
    // append_authorized under Endpoint::Push: the matrix allows every class, so
    // the legitimate path (including subagent authorship) is ALLOWED, the
    // mutation is gated, and a denial (impossible for Push today) is audited.
    let actor_class = principal_chain
        .last()
        .and_then(|id| PrincipalClass::classify(id))
        .unwrap_or(PrincipalClass::Worker);

    log.append_authorized(
        actor_class,
        Endpoint::Push,
        INTENT_LANDED_KIND,
        principal_chain,
        payload,
        recorded_at,
    )
    .map_err(|denied| ImportError::Denied(denied.reason))
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

#[cfg(test)]
mod guard_tests {
    //! D14 guard on the `import_sidecar` intent-landing path (P-GUARD2). Intent
    //! authorship is the universal `Push` verb, so EVERY principal class (human /
    //! orchestrator / worker subagent / model) is ALLOWED — the legitimate
    //! landed-intent path is never broken — while the mutation surface is gated
    //! through `append_authorized` (no raw-append bypass). No `authz.denied`
    //! audit is emitted on the allowed path; exactly one `intent.landed` lands.
    use super::*;

    fn sidecar(id: &str) -> IntentSidecar {
        IntentSidecar {
            intent_id: id.into(),
            charter: format!("charter for {id}"),
            acceptance: vec!["does the thing".into()],
            context_ref: format!("cas://ctx/{id}"),
            authoritative: false,
        }
    }

    fn import(log: &mut EventLog, id: &str, actor: &str) -> Result<EventRecord, ImportError> {
        import_sidecar(
            log,
            &sidecar(id),
            "refs/heads/main",
            "oid-1",
            vec![actor.into()],
            1_717_000_000_000,
        )
    }

    #[test]
    fn every_class_may_author_intent_via_push_cell() {
        // human / orchestrator / worker (subagent) / model — all allowed.
        for actor in [
            "user:gustavo",
            "orchestrator:lead",
            "agent:runner-03",
            "model:claude",
        ] {
            let mut log = EventLog::new();
            let rec = import(&mut log, "intent-x", actor)
                .expect("intent authorship is the Push verb — every class allowed");
            assert_eq!(rec.kind, INTENT_LANDED_KIND);
            assert_eq!(rec.principal_chain, vec![actor.to_string()]);
            // Exactly the landed-intent event; no denial audit on the allowed path.
            assert_eq!(log.len(), 1, "exactly one intent.landed appended");
            assert!(
                !log.records().iter().any(|r| r.kind == "authz.denied"),
                "the allowed Push path emits no denial audit"
            );
        }
    }

    #[test]
    fn unclassifiable_principal_falls_back_to_worker_and_is_allowed() {
        // A bare/unclassifiable identity falls back to Worker — which Push allows.
        // (The honest P2 authn seam: classification is caller-supplied today.)
        let mut log = EventLog::new();
        let rec = import(&mut log, "intent-y", "queue")
            .expect("unclassifiable → Worker fallback, allowed for Push");
        assert_eq!(rec.kind, INTENT_LANDED_KIND);
        assert_eq!(log.len(), 1);
        assert!(!log.records().iter().any(|r| r.kind == "authz.denied"));
    }

    #[test]
    fn empty_and_duplicate_ids_still_refused_before_the_guard() {
        // Pre-guard refusals are unchanged: empty id and duplicate id never reach
        // the append, so the log stays empty / single-record.
        let mut log = EventLog::new();
        assert!(matches!(
            import(&mut log, "", "user:g"),
            Err(ImportError::EmptyIntentId)
        ));
        assert_eq!(log.len(), 0, "empty-id refusal appends nothing");

        import(&mut log, "dup", "user:g").expect("first import lands");
        assert!(matches!(
            import(&mut log, "dup", "user:g"),
            Err(ImportError::DuplicateIntentId { .. })
        ));
        assert_eq!(log.len(), 1, "duplicate refusal appends nothing");
    }
}
