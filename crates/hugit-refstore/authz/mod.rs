//! WP-D14 — forge authorization: the mutation-surface guard.
//!
//! Every **mutating** forge endpoint — `push` / `land` / `undo` / `policy` —
//! is guarded here. The guard answers exactly one question: *may this principal
//! perform this mutation?* It does **not** implement the mutations themselves —
//! D1 owns the event log / refs / undo mechanics, D4 owns intent, D6 owns
//! policy. D14 sits at the entry seam and AUTHORIZES; it modifies none of
//! their internals.
//!
//! Three properties are sealed (decomposition v2.0 D14①–③):
//!
//! 1. **Mutating endpoints reject unauthorized principals (①), fail-CLOSED.**
//!    An unrecognized or unauthorized principal is *denied*, never
//!    defaulted-allow. See [`authorize`].
//! 2. **The permission model is documented + golden-tested per principal class
//!    (②).** The matrix below is the single written-down source of truth; the
//!    golden fixture in `authz/tests/` asserts every (principal class ×
//!    endpoint) cell against it.
//! 3. **Denials are audited (③).** Every denial emits an [`EventRecord`] via the
//!    D1a append point — which principal, which endpoint, when — so a denial is
//!    never silent and is always attributable. See [`AuditedGuard::authorize`].
//!
//! # Authentication is a disclosed seam (not faked here)
//!
//! This guard enforces the *authorization matrix* — given a principal class and
//! an endpoint, may the mutation proceed? It does **not** authenticate the
//! principal: it trusts the class it is handed. Two routes feed it:
//!
//! - **Chain-classified** ([`authorize`]): the actor is the tail of the
//!   `principal_chain` and its class is read from the identity prefix
//!   (`user:` / `agent:` / …). This binds class to the chain only as far as the
//!   chain itself is trustworthy.
//! - **Caller-asserted** ([`EventLog::append_authorized`]): the porcelain (CLI)
//!   supplies the class directly (e.g. `pr open --author-kind`). Today that
//!   token is **caller-supplied** — a subagent process could pass
//!   `--author-kind orchestrator`. The guard still enforces the matrix on the
//!   asserted class (a self-declared `worker` is denied `land`/`pr.opened`), but
//!   binding the asserted class to an *authenticated* principal lands with
//!   identity rollout (P2 / ADR-0002: one HuGR account, Clerk principals, PATs).
//!   Until then this is the honestly-disclosed authentication seam — the matrix
//!   is real and unbypassable on the mutation path; the principal→class binding
//!   is the part P2 completes. We do **not** fabricate authentication here.
//!
//! # The permission model (FROZEN — decomposition v2.0 D14②, whitepaper §3 + §7)
//!
//! Every principal is a first-class identity (whitepaper §3): human,
//! orchestrator, worker agent, model. The per-class command surface
//! (whitepaper §7 "product surface") fixes who may drive each mutating verb:
//!
//! | Endpoint   | Human | Orchestrator | Worker | Model | Source (§7 verb owner) |
//! |------------|:-----:|:------------:|:------:|:-----:|------------------------|
//! | **push**   |   ✓   |      ✓       |   ✓    |   ✓   | "Everyone … every git command" |
//! | **land**   |   ✗   |      ✓       |   ✗    |   ✗   | Orchestrator: `land`   |
//! | **undo**   |   ✓   |      ✗       |   ✗    |   ✗   | Human: `undo`          |
//! | **policy** |   ✓   |      ✗       |   ✗    |   ✗   | Human: `policy`        |
//!
//! `push` is the universal git verb — every principal class may push (the
//! degradation invariant: worst case is healthy git). `land` is the
//! orchestrator's integration verb. `undo` and `policy` are human-reserved
//! stakeholder controls (§7: the human "decides · approves", and policy / undo
//! are stakeholder verbs). Any cell not marked authorized is **denied**, and
//! any unrecognized principal is **denied** (fail-closed).

use hugit_contracts::event_record::EventRecord;

use crate::log::EventLog;

/// A mutating forge endpoint — the complete set of mutation surfaces D14 guards.
///
/// These are the four verbs that change repository state; reads are not guarded
/// here. The set is closed: there is no "other" endpoint, so the match over it
/// is exhaustive and the matrix can never silently grow an unguarded surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Endpoint {
    /// `push` — advance refs (the universal git mutation; every class allowed).
    Push,
    /// `land` — integrate a batch through the landing queue (orchestrator).
    Land,
    /// `undo` — append a compensating event (human stakeholder control).
    Undo,
    /// `policy` — change a policy / regen gate (human stakeholder control).
    Policy,
}

impl Endpoint {
    /// Stable, lowercase wire name for this endpoint (used in audit payloads).
    pub fn as_str(self) -> &'static str {
        match self {
            Endpoint::Push => "push",
            Endpoint::Land => "land",
            Endpoint::Undo => "undo",
            Endpoint::Policy => "policy",
        }
    }
}

/// A first-class principal class (whitepaper §3).
///
/// Every principal acting on the forge is one of these four classes. An actor
/// whose class cannot be recognized never reaches this enum — it is rejected by
/// [`PrincipalClass::classify`] and denied fail-closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PrincipalClass {
    /// The human stakeholder — decides, approves, interrogates.
    Human,
    /// The orchestrator (e.g. the tech-lead seat) — plans, dispatches, lands.
    Orchestrator,
    /// A worker agent — executes one intent within a campaign.
    Worker,
    /// A model — a first-class identity with permissions, budgets, attribution.
    Model,
}

impl PrincipalClass {
    /// Stable, lowercase wire name for this principal class.
    pub fn as_str(self) -> &'static str {
        match self {
            PrincipalClass::Human => "human",
            PrincipalClass::Orchestrator => "orchestrator",
            PrincipalClass::Worker => "worker",
            PrincipalClass::Model => "model",
        }
    }

    /// Classify a principal-chain identity string into its class, **fail-closed**.
    ///
    /// The convention follows the `principal_chain` strings used across the
    /// event log (e.g. `"user:gustavo"`, `"agent:runner-03"`): a `<class>:<id>`
    /// shape where the prefix names the class. We map the documented identity
    /// prefixes to classes:
    ///
    /// - `human:` / `user:`          → [`Human`](PrincipalClass::Human)
    /// - `orchestrator:` / `orch:`   → [`Orchestrator`](PrincipalClass::Orchestrator)
    /// - `worker:` / `agent:`        → [`Worker`](PrincipalClass::Worker)
    /// - `model:`                    → [`Model`](PrincipalClass::Model)
    ///
    /// Anything else — an unknown prefix, a bare string, an empty identity —
    /// returns [`None`]: the actor is **unrecognized** and will be denied. This
    /// is the fail-closed entry point: an unclassifiable principal can never be
    /// defaulted into an allowed class.
    pub fn classify(identity: &str) -> Option<PrincipalClass> {
        let (prefix, rest) = identity.split_once(':')?;
        if rest.is_empty() {
            return None;
        }
        match prefix {
            "human" | "user" => Some(PrincipalClass::Human),
            "orchestrator" | "orch" => Some(PrincipalClass::Orchestrator),
            "worker" | "agent" => Some(PrincipalClass::Worker),
            "model" => Some(PrincipalClass::Model),
            _ => None,
        }
    }
}

/// The outcome of an authorization decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The principal is authorized for the endpoint.
    Allow,
    /// The principal is denied; the reason is attributable in the audit event.
    Deny(DenyReason),
}

impl Decision {
    /// Whether this decision authorizes the mutation.
    pub fn is_allowed(&self) -> bool {
        matches!(self, Decision::Allow)
    }
}

/// Why an authorization was denied (carried into the audit event payload).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DenyReason {
    /// The acting identity could not be classified into a known principal class.
    UnrecognizedPrincipal,
    /// The principal class is known but not permitted for this endpoint.
    NotPermitted {
        /// The recognized class of the denied actor.
        class: PrincipalClass,
    },
}

impl DenyReason {
    /// Stable, lowercase reason code for the audit payload.
    pub fn code(&self) -> &'static str {
        match self {
            DenyReason::UnrecognizedPrincipal => "unrecognized_principal",
            DenyReason::NotPermitted { .. } => "not_permitted",
        }
    }
}

/// The documented permission matrix (②) — the single source of truth in code.
///
/// `true` ⇔ the cell is authorized in the table in this module's docs. This is
/// the only place the matrix is encoded; the golden fixture asserts the table
/// and this function agree, cell-by-cell.
///
/// Fail-closed by construction: only the explicitly-`true` cells are allowed;
/// everything else (including future endpoints, were the match non-exhaustive)
/// would be denied.
pub fn matrix(class: PrincipalClass, endpoint: Endpoint) -> bool {
    use Endpoint::*;
    use PrincipalClass::*;
    match (class, endpoint) {
        // push — the universal git verb: every principal class may push.
        (_, Push) => true,
        // land — the orchestrator's integration verb.
        (Orchestrator, Land) => true,
        (_, Land) => false,
        // undo — human stakeholder control.
        (Human, Undo) => true,
        (_, Undo) => false,
        // policy — human stakeholder control.
        (Human, Policy) => true,
        (_, Policy) => false,
    }
}

/// Authorize a single mutation request, **fail-closed** (①, ②).
///
/// The acting identity is the *last* link of the `principal_chain` — the
/// principal that directly drives the mutation (the chain records the full
/// delegation path; the actor at the head of the request is its tail). An
/// empty chain has no actor and is denied.
///
/// The actor is classified; an unclassifiable actor is denied with
/// [`DenyReason::UnrecognizedPrincipal`]. A recognized actor is checked against
/// [`matrix`]; a disallowed cell is denied with [`DenyReason::NotPermitted`].
/// Only an authorized cell returns [`Decision::Allow`].
///
/// This function does not audit — pair it with [`AuditedGuard`] to emit the
/// denial event (③). It is exposed standalone so the decision is unit-testable
/// without a log.
pub fn authorize(principal_chain: &[String], endpoint: Endpoint) -> Decision {
    let Some(actor) = principal_chain.last() else {
        return Decision::Deny(DenyReason::UnrecognizedPrincipal);
    };
    let Some(class) = PrincipalClass::classify(actor) else {
        return Decision::Deny(DenyReason::UnrecognizedPrincipal);
    };
    authorize_class(class, endpoint)
}

/// Authorize an already-classified principal against [`matrix`], **fail-closed**.
///
/// This is the decision over a *caller-asserted* class — used by
/// [`EventLog::append_authorized`](crate::log::EventLog::append_authorized) when
/// the porcelain hands the class directly (e.g. `pr open --author-kind`) rather
/// than carrying it in the `principal_chain`. The class→principal *binding* is
/// the disclosed authentication seam (see the module doc); the matrix decision
/// itself is real. A disallowed cell is [`DenyReason::NotPermitted`].
pub fn authorize_class(class: PrincipalClass, endpoint: Endpoint) -> Decision {
    if matrix(class, endpoint) {
        Decision::Allow
    } else {
        Decision::Deny(DenyReason::NotPermitted { class })
    }
}

/// The audit event kind appended for every authorization denial (③).
pub const AUTHZ_DENIED_KIND: &str = "authz.denied";

/// A guard that authorizes mutations **and audits denials** to the event log (③).
///
/// It wraps an [`EventLog`] (the D1a append point — the *only* mutating
/// primitive in the refstore) and routes every denial through it: a denied
/// request appends an [`EventRecord`] of kind [`AUTHZ_DENIED_KIND`] carrying the
/// denied principal chain, the endpoint, and the reason, so the denial is
/// attributable and never silent. Authorized requests are *not* audited here —
/// the endpoint that performs the mutation records its own event.
///
/// D14 owns only this guard; it consumes [`EventLog::append`] additively and
/// never touches the log's internals (D1's territory).
pub struct AuditedGuard<'a> {
    log: &'a mut EventLog,
}

impl<'a> AuditedGuard<'a> {
    /// Wrap an event log to audit denials into it.
    pub fn new(log: &'a mut EventLog) -> Self {
        Self { log }
    }

    /// Authorize a mutation and, on denial, append an audit event (③).
    ///
    /// Returns the [`Decision`]. On [`Decision::Deny`] an [`EventRecord`] of kind
    /// [`AUTHZ_DENIED_KIND`] is appended via the log's frozen append path and the
    /// record is returned alongside the decision; on [`Decision::Allow`] no event
    /// is appended and the record is [`None`].
    pub fn authorize(
        &mut self,
        principal_chain: &[String],
        endpoint: Endpoint,
        recorded_at: u64,
    ) -> (Decision, Option<EventRecord>) {
        let decision = authorize(principal_chain, endpoint);
        match &decision {
            Decision::Allow => (decision, None),
            Decision::Deny(reason) => {
                let payload = denial_payload(endpoint, reason);
                let record = self.log.append(
                    AUTHZ_DENIED_KIND,
                    principal_chain.to_vec(),
                    payload,
                    recorded_at,
                );
                (decision, Some(record))
            }
        }
    }
}

/// Build the JSON audit payload for a denial (stable shape, attributable).
pub(crate) fn denial_payload(endpoint: Endpoint, reason: &DenyReason) -> String {
    match reason {
        DenyReason::UnrecognizedPrincipal => serde_json::json!({
            "endpoint": endpoint.as_str(),
            "reason": reason.code(),
        })
        .to_string(),
        DenyReason::NotPermitted { class } => serde_json::json!({
            "endpoint": endpoint.as_str(),
            "reason": reason.code(),
            "class": class.as_str(),
        })
        .to_string(),
    }
}

/// The complete set of guarded mutating endpoints (for golden enumeration).
pub const ALL_ENDPOINTS: [Endpoint; 4] = [
    Endpoint::Push,
    Endpoint::Land,
    Endpoint::Undo,
    Endpoint::Policy,
];

/// The complete set of principal classes (for golden enumeration).
pub const ALL_CLASSES: [PrincipalClass; 4] = [
    PrincipalClass::Human,
    PrincipalClass::Orchestrator,
    PrincipalClass::Worker,
    PrincipalClass::Model,
];

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(actor: &str) -> Vec<String> {
        vec![actor.to_string()]
    }

    #[test]
    fn unrecognized_principal_denied_fail_closed() {
        for endpoint in ALL_ENDPOINTS {
            // bare string, unknown prefix, empty id, empty chain — all denied.
            assert_eq!(
                authorize(&chain("nobody"), endpoint),
                Decision::Deny(DenyReason::UnrecognizedPrincipal)
            );
            assert_eq!(
                authorize(&chain("alien:x"), endpoint),
                Decision::Deny(DenyReason::UnrecognizedPrincipal)
            );
            assert_eq!(
                authorize(&chain("human:"), endpoint),
                Decision::Deny(DenyReason::UnrecognizedPrincipal)
            );
            assert_eq!(
                authorize(&[], endpoint),
                Decision::Deny(DenyReason::UnrecognizedPrincipal)
            );
        }
    }

    #[test]
    fn classify_known_prefixes() {
        assert_eq!(
            PrincipalClass::classify("user:gustavo"),
            Some(PrincipalClass::Human)
        );
        assert_eq!(
            PrincipalClass::classify("human:gustavo"),
            Some(PrincipalClass::Human)
        );
        assert_eq!(
            PrincipalClass::classify("orchestrator:opus"),
            Some(PrincipalClass::Orchestrator)
        );
        assert_eq!(
            PrincipalClass::classify("agent:runner-03"),
            Some(PrincipalClass::Worker)
        );
        assert_eq!(
            PrincipalClass::classify("model:claude"),
            Some(PrincipalClass::Model)
        );
        assert_eq!(PrincipalClass::classify("garbage"), None);
    }

    #[test]
    fn actor_is_chain_tail() {
        // delegation chain: human delegated to a worker; the worker is the actor.
        let delegated = vec!["user:gustavo".to_string(), "agent:runner".to_string()];
        // worker may push, may not land.
        assert!(authorize(&delegated, Endpoint::Push).is_allowed());
        assert!(!authorize(&delegated, Endpoint::Land).is_allowed());
    }

    #[test]
    fn denial_audited_to_log() {
        let mut log = EventLog::new();
        let mut guard = AuditedGuard::new(&mut log);
        let (decision, record) =
            guard.authorize(&chain("agent:runner"), Endpoint::Land, 1_717_000_000_000);
        assert!(!decision.is_allowed());
        let record = record.expect("denial must emit an audit event");
        assert_eq!(record.kind, AUTHZ_DENIED_KIND);
        assert_eq!(record.principal_chain, chain("agent:runner"));
        assert!(record.payload.contains("\"endpoint\":\"land\""));
        assert!(record.payload.contains("\"reason\":\"not_permitted\""));
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn allow_not_audited() {
        let mut log = EventLog::new();
        let mut guard = AuditedGuard::new(&mut log);
        let (decision, record) = guard.authorize(&chain("orchestrator:opus"), Endpoint::Land, 1);
        assert!(decision.is_allowed());
        assert!(record.is_none());
        assert_eq!(log.len(), 0);
    }
    // ── denial_payload JSON-shape pin tests (one per DenyReason variant) ────────
    // These pin the EXACT serialized key/value shape of the denial audit payload.
    // If the shape changes, the pin fails — intentional: audit consumers parse
    // these fields by key name; a silent rename would break attribution.

    /// UnrecognizedPrincipal denial payload: two fields, exact key/value shape.
    #[test]
    fn denial_payload_unrecognized_principal_shape() {
        use super::*;
        // All endpoints produce the same shape for UnrecognizedPrincipal.
        let payload = denial_payload(Endpoint::Push, &DenyReason::UnrecognizedPrincipal);
        let v: serde_json::Value =
            serde_json::from_str(&payload).expect("denial payload must be valid JSON");
        assert_eq!(
            v.get("endpoint").and_then(|x| x.as_str()),
            Some("push"),
            "endpoint key must be the endpoint wire name"
        );
        assert_eq!(
            v.get("reason").and_then(|x| x.as_str()),
            Some("unrecognized_principal"),
            "reason key must be the DenyReason code"
        );
        // No extra fields (no `class`).
        let obj = v.as_object().expect("payload is a JSON object");
        assert_eq!(
            obj.len(),
            2,
            "UnrecognizedPrincipal payload has exactly 2 keys"
        );
    }

    /// Verify UnrecognizedPrincipal payload shape for every endpoint.
    #[test]
    fn denial_payload_unrecognized_principal_all_endpoints() {
        use super::*;
        let expected_endpoints = ["push", "land", "undo", "policy"];
        for (endpoint, expected_ep_str) in ALL_ENDPOINTS.iter().zip(expected_endpoints.iter()) {
            let payload = denial_payload(*endpoint, &DenyReason::UnrecognizedPrincipal);
            let v: serde_json::Value =
                serde_json::from_str(&payload).expect("payload must be valid JSON");
            assert_eq!(
                v.get("endpoint").and_then(|x| x.as_str()),
                Some(*expected_ep_str),
                "endpoint field mismatch for {:?}",
                endpoint
            );
            assert_eq!(
                v.get("reason").and_then(|x| x.as_str()),
                Some("unrecognized_principal"),
            );
        }
    }

    /// NotPermitted denial payload: three fields — endpoint, reason, class.
    #[test]
    fn denial_payload_not_permitted_shape() {
        use super::*;
        // Worker trying to Land: denied with class=worker.
        let payload = denial_payload(
            Endpoint::Land,
            &DenyReason::NotPermitted {
                class: PrincipalClass::Worker,
            },
        );
        let v: serde_json::Value =
            serde_json::from_str(&payload).expect("denial payload must be valid JSON");
        assert_eq!(
            v.get("endpoint").and_then(|x| x.as_str()),
            Some("land"),
            "endpoint key"
        );
        assert_eq!(
            v.get("reason").and_then(|x| x.as_str()),
            Some("not_permitted"),
            "reason key"
        );
        assert_eq!(
            v.get("class").and_then(|x| x.as_str()),
            Some("worker"),
            "class key must be the principal class wire name"
        );
        let obj = v.as_object().expect("payload is a JSON object");
        assert_eq!(obj.len(), 3, "NotPermitted payload has exactly 3 keys");
    }

    /// NotPermitted: every (class, endpoint) denial carries the correct class wire name.
    #[test]
    fn denial_payload_not_permitted_all_classes() {
        use super::*;
        let cases: &[(PrincipalClass, &str)] = &[
            (PrincipalClass::Human, "human"),
            (PrincipalClass::Orchestrator, "orchestrator"),
            (PrincipalClass::Worker, "worker"),
            (PrincipalClass::Model, "model"),
        ];
        for (class, expected_class_str) in cases {
            let payload = denial_payload(
                Endpoint::Policy,
                &DenyReason::NotPermitted { class: *class },
            );
            let v: serde_json::Value =
                serde_json::from_str(&payload).expect("payload must be valid JSON");
            assert_eq!(
                v.get("class").and_then(|x| x.as_str()),
                Some(*expected_class_str),
                "class field mismatch for {:?}",
                class
            );
            assert_eq!(
                v.get("reason").and_then(|x| x.as_str()),
                Some("not_permitted"),
            );
        }
    }
}
