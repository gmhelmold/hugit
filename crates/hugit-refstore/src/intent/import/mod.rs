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
//! # WF-AUTHZ: Protected-ref guard — import may not advance a branch ref
//!
//! The `Push` cell is allowed for every principal class, which is correct for
//! authoring intents. But `intent.landed` also advances ref state in replay
//! (exactly like `ref.update`), so a caller that passes `ref_name =
//! "refs/heads/main"` would produce a main-advancing `intent.landed` gated only
//! on the all-classes Push cell — bypassing the `Land` authority that is the
//! only legitimate path for advancing protected branch refs.
//!
//! **Rule (WF-AUTHZ option a):** `import_sidecar` may only target the
//! **intent-namespace** (`refs/hugit/**`). Passing a `ref_name` that starts
//! with `refs/heads/` (or any other branch-like namespace outside
//! `refs/hugit/`) is rejected with [`ImportError::ProtectedRef`] before any
//! guard or append is attempted. Advancing a real branch MUST go through the
//! `Land`-gated path. This is deliberately fail-closed: unknown namespaces
//! outside `refs/hugit/` are also rejected rather than silently promoted.
//!
//! The CLI passes `AUTHORED_REF = "refs/hugit/intents"`, which is in the
//! intent namespace and is always accepted — the subagent intent-authorship path
//! is unaffected.
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
    /// The `ref_name` targets a protected ref namespace (e.g. `refs/heads/**`).
    /// `import_sidecar` is gated on the all-class `Push` cell, which is correct
    /// for intent authorship but MUST NOT be used to advance protected branch
    /// refs. Advancing a real branch requires the `Land`-gated path. Only the
    /// intent namespace (`refs/hugit/**`) is accepted here (WF-AUTHZ).
    ProtectedRef { ref_name: String },
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
            ImportError::ProtectedRef { ref_name } => write!(
                f,
                "import_sidecar cannot advance a protected ref '{ref_name}': \
                 intent.landed via import is restricted to refs/hugit/**; \
                 advancing a branch ref requires the Land-gated path"
            ),
            ImportError::Denied(reason) => {
                write!(f, "intent import denied by D14 guard: {}", reason.code())
            }
        }
    }
}

impl std::error::Error for ImportError {}

/// Whether `ref_name` is within the intent namespace permitted for import.
///
/// Only `refs/hugit/**` is allowed via the all-class `Push` cell (WF-AUTHZ).
/// Any other namespace — including `refs/heads/**` (branches), `refs/tags/**`,
/// or bare/unknown ref paths — is a protected ref that must be advanced through
/// the `Land`-gated path, not through `import_sidecar`.
fn is_intent_namespace(ref_name: &str) -> bool {
    ref_name.starts_with("refs/hugit/")
}

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
/// **WF-AUTHZ protected-ref guard:** `ref_name` MUST be within the intent
/// namespace (`refs/hugit/**`). Passing a branch ref (e.g. `refs/heads/main`)
/// or any other non-intent namespace is rejected with
/// [`ImportError::ProtectedRef`] before any append is attempted. Advancing a
/// real branch ref requires the `Land`-gated path; this function is never the
/// right path for that.
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
    // WF-AUTHZ: guard first — before the empty-id or duplicate checks — so a
    // caller cannot probe "does this intent exist?" by passing a protected ref.
    if !is_intent_namespace(ref_name) {
        return Err(ImportError::ProtectedRef {
            ref_name: ref_name.to_string(),
        });
    }
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
    //! orchestrator / worker subagent / model) is ALLOWED for the intent namespace
    //! (`refs/hugit/**`) — the legitimate landed-intent path is never broken —
    //! while the mutation surface is gated through `append_authorized` (no
    //! raw-append bypass). No `authz.denied` audit is emitted on the allowed path;
    //! exactly one `intent.landed` lands.
    //!
    //! WF-AUTHZ tests additionally cover the protected-ref guard: a worker-class
    //! import targeting `refs/heads/main` is refused BEFORE the D14 guard and
    //! before any log mutation; only `refs/hugit/**` is accepted.
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

    /// Helper that imports into the INTENT NAMESPACE (refs/hugit/intents) —
    /// the correct path for intent authorship under the all-class Push cell.
    fn import(log: &mut EventLog, id: &str, actor: &str) -> Result<EventRecord, ImportError> {
        import_sidecar(
            log,
            &sidecar(id),
            "refs/hugit/intents",
            &format!("authored:{id}"),
            vec![actor.into()],
            1_717_000_000_000,
        )
    }

    #[test]
    fn every_class_may_author_intent_into_intent_namespace() {
        // human / orchestrator / worker (subagent) / model — all allowed for the
        // intent namespace (refs/hugit/**) via the Push cell.
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
        // A bare/unclassifiable identity falls back to Worker — which Push allows
        // for the intent namespace.
        // (The honest P2 authn seam: classification is caller-supplied today.)
        let mut log = EventLog::new();
        let rec = import(&mut log, "intent-y", "queue")
            .expect("unclassifiable → Worker fallback, allowed for Push on intent namespace");
        assert_eq!(rec.kind, INTENT_LANDED_KIND);
        assert_eq!(log.len(), 1);
        assert!(!log.records().iter().any(|r| r.kind == "authz.denied"));
    }

    #[test]
    fn empty_and_duplicate_ids_still_refused_before_the_d14_guard() {
        // Pre-guard refusals are unchanged: empty id and duplicate id never reach
        // the D14 append guard, so the log stays empty / single-record.
        // NOTE: the protected-ref check runs BEFORE these — the helper already
        // uses the intent namespace, so these are the only two remaining early
        // refusals before the D14 guard.
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

    // ── WF-AUTHZ: protected-ref guard tests ──────────────────────────────────
    // Rule (option a): import_sidecar may only target refs/hugit/**. Any attempt
    // to pass refs/heads/** (or other non-intent namespaces) is refused BEFORE
    // any log mutation — the log is left untouched.

    #[test]
    fn worker_import_into_intent_namespace_succeeds() {
        // Worker-class actor importing into refs/hugit/intents: ALLOWED (the
        // CLI `intent new` path — Push cell, intent namespace).
        let mut log = EventLog::new();
        let rec = import_sidecar(
            &mut log,
            &sidecar("intent-ok"),
            "refs/hugit/intents",
            "authored:intent-ok",
            vec!["agent:runner-07".into()],
            1_717_000_001_000,
        )
        .expect("worker into refs/hugit/** is always allowed");
        assert_eq!(rec.kind, INTENT_LANDED_KIND);
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn worker_import_targeting_refs_heads_main_is_refused() {
        // WF-AUTHZ: a worker-class actor MUST NOT be able to advance
        // refs/heads/main via import_sidecar (which is gated only on the
        // all-class Push cell). The protected-ref guard REFUSES before any
        // append is attempted — the log stays completely empty.
        let mut log = EventLog::new();
        let err = import_sidecar(
            &mut log,
            &sidecar("intent-bad"),
            "refs/heads/main",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            vec!["agent:runner-07".into()],
            1_717_000_002_000,
        )
        .expect_err("targeting refs/heads/main must be refused");
        assert!(
            matches!(err, ImportError::ProtectedRef { ref ref_name } if ref_name == "refs/heads/main"),
            "expected ProtectedRef for refs/heads/main, got {err:?}"
        );
        // The log MUST NOT have been mutated — no append at all.
        assert_eq!(
            log.len(),
            0,
            "protected-ref refusal must not append anything"
        );
        assert!(
            !log.records().iter().any(|r| r.kind == "authz.denied"),
            "protected-ref guard fires before the D14 guard — no authz.denied record"
        );
    }

    #[test]
    fn protected_ref_guard_covers_all_non_intent_namespaces() {
        // The guard rejects anything not in refs/hugit/**, not just refs/heads/**.
        // A caller cannot find a hole by using a different namespace.
        let cases = [
            "refs/heads/main",
            "refs/heads/feature-x",
            "refs/tags/v1.0",
            "refs/remotes/origin/main",
            "HEAD",
            "refs/",
            "",
        ];
        for ref_name in cases {
            let mut log = EventLog::new();
            let err = import_sidecar(
                &mut log,
                &sidecar("intent-probe"),
                ref_name,
                "target",
                vec!["agent:runner-07".into()],
                1_717_000_003_000,
            )
            .expect_err(&format!(
                "'{ref_name}' must be refused as a non-intent namespace"
            ));
            assert!(
                matches!(err, ImportError::ProtectedRef { .. }),
                "expected ProtectedRef for '{ref_name}', got {err:?}"
            );
            assert_eq!(log.len(), 0, "no append for '{ref_name}'");
        }
    }

    #[test]
    fn orchestrator_import_targeting_refs_heads_is_also_refused() {
        // Even an orchestrator-class actor cannot bypass the protected-ref guard
        // via import_sidecar: the Land-gated path is the right one for branch refs.
        let mut log = EventLog::new();
        let err = import_sidecar(
            &mut log,
            &sidecar("intent-orch"),
            "refs/heads/main",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            vec!["orchestrator:lead".into()],
            1_717_000_004_000,
        )
        .expect_err("orchestrator targeting refs/heads/main via import must still be refused");
        assert!(matches!(err, ImportError::ProtectedRef { .. }));
        assert_eq!(log.len(), 0);
    }
}
