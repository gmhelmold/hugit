//! `write_account_erase` — a user requests erasure of THEIR OWN account (GDPR1, PART 1:
//! STAGE the request; the X7/X12 cascade EXECUTION is Part 2). See the decided design
//! `docs/design/2026-07-04-gdpr1-account-erase-design.md`.
//!
//! SAFE by construction: records an `erasure.requested` lifecycle event AS-THE-USER
//! (subject DERIVED from the verified principal — never operator/anon, never a request
//! param); it NEVER deletes anything. Persistence (the reserved `_erasure/{account_slug}`
//! log) + the step-up/idempotency door are the HANDLER's job; this verb is the pure,
//! testable append that mutates a `&mut EventLog`.

use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests::AccountEraseReq;
use hugit_refstore::{Endpoint, EventLog};
use serde_json::json;

use crate::error::EngineErr;
use crate::writes::verbs::write_provision::derive_owner_tenant;

/// The event kind appended by this verb — an account-scoped erasure REQUEST (staging).
/// The matching `erasure.executed` is Part 2 (the audit-gated cascade execution), never
/// written here.
pub const ERASURE_REQUESTED_KIND: &str = "erasure.requested";

/// Append an `erasure.requested` for the CALLER'S OWN account.
///
/// The account (subject) is DERIVED from the verified principal via
/// [`derive_owner_tenant`] (refuses operator/anon/malformed → no god-erase / no
/// anon-erase, exactly like provision's self-create). `req.confirm` MUST equal the
/// caller's account slug — a typed guard validated BEFORE the append (fail-closed
/// against an accidental irreversible erase). Appended AS-THE-USER
/// ([`crate::writes::asserted_class`]), NOT the hardcoded `Orchestrator` the `decide`
/// verb uses.
///
/// # Errors
/// - `401 UNAUTHORIZED` — operator/anonymous/malformed principal (no own account to erase).
/// - `400 INVALID_REQUEST` — `confirm` does not match the caller's account slug.
/// - `503 ENGINE_UNAVAILABLE` — append denied (fail-honest).
pub fn write_account_erase(
    log: &mut EventLog,
    req: &AccountEraseReq,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<Accepted, EngineErr> {
    // The account being erased = the CALLER'S OWN account, derived from the verified
    // principal. `derive_owner_tenant` refuses operator/anon → no god-erase / anon-erase.
    let account = derive_owner_tenant(&principal_chain)?;
    // Typed confirmation: the caller must type their OWN account slug — a fail-closed
    // guard against an accidental IRREVERSIBLE erase, validated BEFORE any write.
    if req.confirm != account {
        return Err(EngineErr::invalid_request(
            "confirm deve ser exatamente o slug da sua própria conta",
        ));
    }
    let payload_value = json!({
        "account": account,
        "subject": account,
        "state": "requested",
    });
    let payload = hugit_refstore::canonical_json(&payload_value.to_string())
        .unwrap_or_else(|| payload_value.to_string());
    // As-the-user: the REAL caller's chain-derived class (a tenant → the Orchestrator
    // integration authority), NOT the decide verb's hardcoded Orchestrator.
    let class = crate::writes::asserted_class(&principal_chain)?;
    let record = log
        .append_authorized(
            class,
            Endpoint::Land,
            ERASURE_REQUESTED_KIND,
            principal_chain,
            payload,
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!(
                "erasure.requested append denied: {}",
                d.reason.code()
            ))
        })?;
    Ok(Accepted {
        seq: record.seq,
        note: format!("apagamento de conta solicitado: {account}"),
        extra: None,
        queue_pos: None,
        pr_number: None,
        branch: None,
        state: Some("requested".to_string()),
        charter_preview: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tenant(org: &str) -> Vec<String> {
        vec![format!("clerk:{org}:user-1")]
    }
    fn req(confirm: &str) -> AccountEraseReq {
        AccountEraseReq {
            confirm: confirm.into(),
        }
    }

    #[test]
    fn own_account_with_matching_confirm_stages_a_request() {
        let mut log = EventLog::new();
        let acc = write_account_erase(&mut log, &req("org-a"), tenant("org-a"), 1).unwrap();
        assert_eq!(acc.state.as_deref(), Some("requested"));
        let rec = log.records().last().expect("a record was appended");
        assert_eq!(rec.kind, ERASURE_REQUESTED_KIND);
        assert!(
            rec.payload.contains("\"account\":\"org-a\""),
            "the subject is the caller's own account: {}",
            rec.payload
        );
    }

    #[test]
    fn wrong_confirm_is_rejected_and_writes_nothing() {
        let mut log = EventLog::new();
        let err = write_account_erase(&mut log, &req("org-b"), tenant("org-a"), 1).unwrap_err();
        assert_eq!(err.status, 400, "a mismatched confirm is a 400");
        assert!(
            log.records().is_empty(),
            "a mismatched confirm leaves ZERO state (fail-closed)"
        );
    }

    #[test]
    fn operator_and_anon_are_refused_no_god_or_anon_erase() {
        let mut log = EventLog::new();
        assert_eq!(
            write_account_erase(
                &mut log,
                &req("hugit"),
                vec!["orchestrator:hugit".into()],
                1
            )
            .unwrap_err()
            .status,
            401,
            "the operator has no own-account to erase (no god-erase)"
        );
        assert_eq!(
            write_account_erase(&mut log, &req(""), vec![], 1)
                .unwrap_err()
                .status,
            401,
            "an anonymous caller cannot erase (no anon-erase)"
        );
        assert!(log.records().is_empty(), "neither refusal writes anything");
    }
}
