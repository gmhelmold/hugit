//! `GET /v1/repos/{repo}/viewer-can` → [`ViewerCanVm`].
//!
//! The per-repo capability set of the signed-in viewer (spec §4) — a RENDER
//! GATE only. It tells the window which write affordances to show; the engine
//! re-decides every verb fail-closed at its write-door, so this projection can
//! never grant a capability the engine would not.
//!
//! REAL backbone — the D14 authorization matrix
//! ([`hugit_refstore::matrix`]). This handler does NOT read the
//! [`EventLog`](hugit_refstore::EventLog): the matrix is per-principal-CLASS,
//! repo-agnostic, and static, so every cell is derived from the caller-asserted
//! principal chain only — never from log content. Each capability mirrors the
//! EXACT endpoint the corresponding write-door actually authorizes on, so the
//! render gate is faithful to engine enforcement:
//!
//! - `land`     → `matrix(class, Endpoint::Land)` (the land write-door checks Land).
//! - `verdict`  → `matrix(class, Endpoint::Land)` (`write_verdict` rides Land).
//! - `comment`  → `matrix(class, Endpoint::Push)` (`write_comment` checks Push).
//! - `dispatch` → `matrix(class, Endpoint::Land)` (`write_dispatch` rides Land).
//! - `policy`   → `matrix(class, Endpoint::Land)` (`write_policy` rides Land +
//!   a STEP-UP gate; STEP-UP is enforced at the write-door BEYOND the matrix —
//!   the render gate reflects the matrix cell only, NOT the human-only
//!   `Endpoint::Policy` cell).
//! - `erasure`  → `matrix(class, Endpoint::Land)` (`write_erasure_decide` rides
//!   Land + STEP-UP — same caveat as `policy`).
//!
//! For the dev/stub principal chain `["orchestrator:hugit"]` the class is
//! [`Orchestrator`](hugit_refstore::PrincipalClass::Orchestrator), so every
//! capability is `true` (Orchestrator is allowed Land and every class is allowed
//! Push). An unclassifiable principal fails CLOSED — every capability `false` —
//! mirroring `authorize()`'s fail-closed law.
//!
//! HONEST-DEFAULT: there are no honest-default fields and no log-backed
//! free-text — the VM is six booleans, each with REAL matrix backing. Nothing is
//! fabricated; an unrecognized principal yields the all-false honest default.

use hugit_http_contracts::viewer_can::ViewerCanVm;
use hugit_refstore::{Endpoint, PrincipalClass, matrix};

/// Capability for one matrix cell: `true` iff the principal's class is allowed
/// the endpoint. Fail-closed — an unclassifiable principal (`None`) yields
/// `false`, exactly as [`hugit_refstore::authorize`] denies an unrecognized
/// actor.
fn can(class: Option<PrincipalClass>, endpoint: Endpoint) -> bool {
    class.map(|c| matrix(c, endpoint)).unwrap_or(false)
}

/// Build the viewer-capability view-model from the caller-asserted principal
/// chain (the router supplies the dev/stub chain `["orchestrator:hugit"]` until
/// the P2 Clerk identity seam).
///
/// The acting identity is the chain TAIL — the principal that directly drives a
/// mutation — mirroring [`hugit_refstore::authorize`]. Its class is read
/// fail-closed; an empty or unclassifiable chain projects all-false.
///
/// This handler is repo-agnostic by construction (the D14 matrix is per-class,
/// not per-repo): no path param is read, so nothing is fabricated from one.
pub fn build_viewer_can(principal_chain: &[String]) -> ViewerCanVm {
    let class = principal_chain
        .last()
        .and_then(|actor| PrincipalClass::classify(actor));
    ViewerCanVm {
        // The land write-door authorizes on Endpoint::Land.
        land: can(class, Endpoint::Land),
        // write_verdict rides the SAME Endpoint::Land its write-door checks.
        verdict: can(class, Endpoint::Land),
        // write_comment authorizes on Endpoint::Push (every class allowed).
        comment: can(class, Endpoint::Push),
        // write_dispatch rides Endpoint::Land.
        dispatch: can(class, Endpoint::Land),
        // write_policy rides Endpoint::Land (+ STEP-UP at the door — matrix cell only here).
        policy: can(class, Endpoint::Land),
        // write_erasure_decide rides Endpoint::Land (+ STEP-UP — matrix cell only here).
        erasure: can(class, Endpoint::Land),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::EventLog;

    fn chain(actor: &str) -> Vec<String> {
        vec![actor.to_string()]
    }

    /// Empty / unclassifiable principal → all-false honest default (fail-closed),
    /// the same law `authorize()` applies to an unrecognized actor.
    #[test]
    fn empty_chain_fails_closed_all_false() {
        let vm = build_viewer_can(&[]);
        assert_eq!(
            vm,
            ViewerCanVm {
                land: false,
                verdict: false,
                comment: false,
                dispatch: false,
                policy: false,
                erasure: false,
            }
        );
        // An unknown-prefix principal is equally unclassifiable → all-false.
        let unknown = build_viewer_can(&chain("alien:x"));
        assert!(!unknown.land);
        assert!(!unknown.verdict);
        assert!(!unknown.comment);
        assert!(!unknown.dispatch);
        assert!(!unknown.policy);
        assert!(!unknown.erasure);
    }

    /// REAL matrix projection for the dev/stub orchestrator principal: Orchestrator
    /// is allowed Land (land/verdict/dispatch/policy/erasure) and every class is
    /// allowed Push (comment) → every capability true.
    #[test]
    fn orchestrator_gets_full_capability_set() {
        let vm = build_viewer_can(&chain("orchestrator:hugit"));
        assert!(vm.land, "Orchestrator allowed Land");
        assert!(vm.verdict, "verdict rides Land → allowed");
        assert!(vm.comment, "Push allowed for every class");
        assert!(vm.dispatch, "dispatch rides Land → allowed");
        assert!(vm.policy, "policy rides Land (matrix cell) → allowed");
        assert!(vm.erasure, "erasure rides Land (matrix cell) → allowed");
    }

    /// REAL matrix projection for a worker: Push only. A worker may comment but
    /// may NOT land/verdict/dispatch/policy/erasure — faithful to the engine's
    /// fail-closed denial of a self-declared worker on Land.
    #[test]
    fn worker_gets_comment_only() {
        let vm = build_viewer_can(&chain("agent:runner-03"));
        assert!(vm.comment, "worker may push → may comment");
        assert!(!vm.land);
        assert!(!vm.verdict);
        assert!(!vm.dispatch);
        assert!(!vm.policy);
        assert!(!vm.erasure);
    }

    /// REAL matrix projection driven from APPENDED records: the matrix is the
    /// source even though this handler is log-independent. The actor is the chain
    /// TAIL (a human-delegated worker is a worker), mirroring `authorize()`.
    #[test]
    fn real_projection_from_appended_records() {
        // Append a real authorized record so the test exercises the engine append
        // path (the matrix this handler projects is the SAME one the append door
        // enforced here). The handler does not read it — the projection is
        // matrix-only — so the VM stays a pure function of the principal class.
        let mut log = EventLog::new();
        let payload = serde_json::json!({"k": "v"}).to_string();
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            "pr.landed",
            vec!["o".into()],
            payload,
            1_000,
        )
        .expect("orchestrator may land");

        // Delegation chain: human → worker; the worker (tail) is the actor.
        let delegated = vec!["user:gustavo".to_string(), "agent:runner".to_string()];
        let vm = build_viewer_can(&delegated);
        assert!(vm.comment, "tail worker may push → comment");
        assert!(!vm.land, "tail worker may NOT land");
        assert!(!vm.verdict);
        assert!(!vm.dispatch);
        assert!(!vm.policy);
        assert!(!vm.erasure);

        // And the orchestrator tail gets the full set.
        let orch = build_viewer_can(&chain("orchestrator:opus"));
        assert!(orch.land && orch.verdict && orch.dispatch && orch.policy && orch.erasure);
    }

    /// Redaction: a `ghp_…` PAT placed in a record payload free-text field MUST
    /// NOT surface in the serialized VM. This handler projects only the boolean
    /// matrix and never echoes log free-text, so no secret can leak — proven by
    /// constructing exactly such a record and asserting the PAT is absent from the
    /// VM JSON (the read-boundary redaction invariant, by-construction here).
    #[test]
    fn secret_pat_never_reaches_vm_json() {
        let pat = "ghp_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        let mut log = EventLog::new();
        let payload = serde_json::json!({"note": pat, "rule_id": "secrets"}).to_string();
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            "policy.set",
            vec!["o".into()],
            payload,
            2_000,
        )
        .expect("orchestrator may land");

        let vm = build_viewer_can(&chain("orchestrator:hugit"));
        let j = serde_json::to_string(&vm).unwrap();
        assert!(
            !j.contains(pat),
            "no log free-text (and no PAT) may reach the viewer-can VM"
        );
        assert!(!j.contains("ghp_"), "no PAT prefix may appear in the VM");
    }

    #[test]
    fn vm_round_trips() {
        let vm = build_viewer_can(&chain("orchestrator:hugit"));
        let j = serde_json::to_string(&vm).unwrap();
        assert_eq!(vm, serde_json::from_str::<ViewerCanVm>(&j).unwrap());
    }
}
