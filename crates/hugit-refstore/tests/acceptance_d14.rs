//! WP-D14 acceptance oracle — hugit-refstore forge authz.
//!
//! Owned items (VERBATIM from decomposition v2.0 — D14):
//!   ① mutating endpoints (push/land/undo/policy) reject unauthorized principals
//!   ② permission model documented + golden-tested per principal class
//!   ③ authz denials audited
//!
//! Driven by `tests/acceptance/wp-d14/run.sh`.

use hugit_refstore::authz::{
    ALL_CLASSES, ALL_ENDPOINTS, AUTHZ_DENIED_KIND, AuditedGuard, Decision, DenyReason, Endpoint,
    PrincipalClass, authorize, matrix,
};
use hugit_refstore::log::EventLog;

/// A single-link principal chain whose actor is `actor`.
fn chain(actor: &str) -> Vec<String> {
    vec![actor.to_string()]
}

/// A representative *recognized* identity for each principal class, using the
/// documented `<class>:<id>` principal-chain convention.
fn identity_for(class: PrincipalClass) -> &'static str {
    match class {
        PrincipalClass::Human => "user:gustavo",
        PrincipalClass::Orchestrator => "orchestrator:opus",
        PrincipalClass::Worker => "agent:runner-03",
        PrincipalClass::Model => "model:claude",
    }
}

/// ① Every mutating endpoint (push/land/undo/policy) REJECTS an unauthorized
/// principal, fail-closed. An unrecognized identity is denied on every
/// endpoint; and for each endpoint, every principal class the documented matrix
/// does NOT permit is also rejected (never defaulted-allow).
#[test]
fn item_1_mutating_endpoints_reject_unauthorized() {
    // (a) An unrecognized principal is rejected on EVERY mutating endpoint.
    for endpoint in ALL_ENDPOINTS {
        let decision = authorize(&chain("intruder"), endpoint);
        assert_eq!(
            decision,
            Decision::Deny(DenyReason::UnrecognizedPrincipal),
            "unrecognized principal must be denied on {} (fail-closed)",
            endpoint.as_str()
        );
        // empty chain (no actor) is likewise denied — never defaulted-allow.
        assert!(
            !authorize(&[], endpoint).is_allowed(),
            "empty principal chain must be denied on {}",
            endpoint.as_str()
        );
    }

    // (b) Each endpoint rejects every RECOGNIZED-but-unauthorized class.
    //     There is at least one such class per endpoint, so the reject path is
    //     exercised for every mutation surface.
    let mut endpoints_with_a_reject = 0;
    for endpoint in ALL_ENDPOINTS {
        let mut rejected_a_recognized_class = false;
        for class in ALL_CLASSES {
            let decision = authorize(&chain(identity_for(class)), endpoint);
            if matrix(class, endpoint) {
                assert!(
                    decision.is_allowed(),
                    "{} must be ALLOWED on {} per the documented matrix",
                    class.as_str(),
                    endpoint.as_str()
                );
            } else {
                assert_eq!(
                    decision,
                    Decision::Deny(DenyReason::NotPermitted { class }),
                    "{} must be REJECTED on {} (recognized but not permitted)",
                    class.as_str(),
                    endpoint.as_str()
                );
                rejected_a_recognized_class = true;
            }
        }
        if rejected_a_recognized_class {
            endpoints_with_a_reject += 1;
        }
    }
    // land/undo/policy each reject ≥1 recognized class; push allows all four.
    assert_eq!(
        endpoints_with_a_reject, 3,
        "exactly land/undo/policy must each reject a recognized-but-unauthorized class"
    );
}

/// ② The permission model is DOCUMENTED and golden-tested per principal class.
/// The golden fixture `authz/tests/permission_matrix.golden` is the written-down
/// model; this asserts `authz::matrix` agrees with EVERY (class × endpoint) cell,
/// and that the fixture covers all 16 cells exactly once (no missing/extra cell).
#[test]
fn item_2_permission_model_golden_per_principal_class() {
    let golden = include_str!("../authz/tests/permission_matrix.golden");

    let parse_class = |s: &str| -> PrincipalClass {
        match s {
            "human" => PrincipalClass::Human,
            "orchestrator" => PrincipalClass::Orchestrator,
            "worker" => PrincipalClass::Worker,
            "model" => PrincipalClass::Model,
            other => panic!("unknown principal class in golden: {other}"),
        }
    };
    let parse_endpoint = |s: &str| -> Endpoint {
        match s {
            "push" => Endpoint::Push,
            "land" => Endpoint::Land,
            "undo" => Endpoint::Undo,
            "policy" => Endpoint::Policy,
            other => panic!("unknown endpoint in golden: {other}"),
        }
    };

    // Track every (class, endpoint) cell the golden asserts; ensure full cover.
    let mut seen = std::collections::HashSet::new();
    let mut cells = 0;
    for line in golden.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split_whitespace().collect();
        assert_eq!(
            cols.len(),
            3,
            "golden line must be `<class> <endpoint> <ALLOW|DENY>`: {line:?}"
        );
        let class = parse_class(cols[0]);
        let endpoint = parse_endpoint(cols[1]);
        let expected_allow = match cols[2] {
            "ALLOW" => true,
            "DENY" => false,
            other => panic!("decision must be ALLOW|DENY in golden: {other}"),
        };
        assert_eq!(
            matrix(class, endpoint),
            expected_allow,
            "golden mismatch: {} × {} expected {}",
            class.as_str(),
            endpoint.as_str(),
            cols[2]
        );
        assert!(
            seen.insert((cols[0].to_string(), cols[1].to_string())),
            "duplicate cell in golden: {} × {}",
            cols[0],
            cols[1]
        );
        cells += 1;
    }

    // Exactly the full 4×4 matrix is golden-asserted — per principal class.
    assert_eq!(cells, ALL_CLASSES.len() * ALL_ENDPOINTS.len());
    for class in ALL_CLASSES {
        for endpoint in ALL_ENDPOINTS {
            assert!(
                seen.contains(&(class.as_str().to_string(), endpoint.as_str().to_string())),
                "golden missing cell {} × {}",
                class.as_str(),
                endpoint.as_str()
            );
        }
    }
}

/// ③ Every authz denial is AUDITED: a denial appends an EventRecord (kind
/// `authz.denied`) to the log, carrying the denied principal chain and endpoint,
/// so the denial is attributable and never silent. An ALLOW appends nothing.
#[test]
fn item_3_authz_denials_audited() {
    let mut log = EventLog::new();
    let mut denied = 0;

    // Walk every (class × endpoint) cell through the auditing guard.
    for class in ALL_CLASSES {
        for endpoint in ALL_ENDPOINTS {
            let pc = chain(identity_for(class));
            let recorded_at = 1_717_000_000_000 + (denied as u64);
            let mut guard = AuditedGuard::new(&mut log);
            let (decision, record) = guard.authorize(&pc, endpoint, recorded_at);

            if decision.is_allowed() {
                assert!(record.is_none(), "ALLOW must not emit an audit event");
            } else {
                let record = record.expect("every denial must emit an audit event");
                assert_eq!(record.kind, AUTHZ_DENIED_KIND);
                // Attributable: the denied principal chain is recorded verbatim.
                assert_eq!(record.principal_chain, pc);
                // The endpoint is captured in the payload.
                assert!(
                    record
                        .payload
                        .contains(&format!("\"endpoint\":\"{}\"", endpoint.as_str())),
                    "denial payload must name the endpoint: {}",
                    record.payload
                );
                denied += 1;
            }
        }
    }

    // The log holds exactly one audit event per denial, in append order, and the
    // hash chain is intact (denials are real events on the source-of-truth log).
    assert_eq!(log.len(), denied, "one audit event per denial");
    assert!(denied > 0, "the matrix must contain denials to audit");
    for rec in log.records() {
        assert_eq!(rec.kind, AUTHZ_DENIED_KIND);
    }

    // Also audit an UNRECOGNIZED principal — its denial is attributable too.
    let before = log.len();
    let mut guard = AuditedGuard::new(&mut log);
    let (decision, record) = guard.authorize(&chain("intruder"), Endpoint::Push, 1);
    assert!(!decision.is_allowed());
    let record = record.expect("unrecognized-principal denial must be audited");
    assert!(record.payload.contains("unrecognized_principal"));
    assert_eq!(log.len(), before + 1);
}
