//! ADR-0004 legs 4/5 — end-to-end acceptance for the Art.17 provenance-PII erasure,
//! driven through the PUBLIC engine API only (no internal test hooks).
//!
//! Proves, on a COMPLETED erase: the subject's cleartext account slug + `clerk:` principal
//! are UNRECOVERABLE from the account log AND the tombstoned repo log; the pseudonymous
//! accountability record is RETAINED; `verify_chain` STILL passes on the redacted chains
//! (tamper-evidence intact); and the subject key is SHREDDED. And on a PARTIAL erase (no
//! CAS-erase transport): the key is NOT shredded (a partial must leave the subject's
//! non-erased data resolvable).

use hugit_refstore::{Endpoint, EventLog, PrincipalClass, verify_chain};
use hugit_serve::error::EngineErr;
use hugit_serve::state::AppState;
use hugit_serve::writes::erasure::{
    CasEraseTransport, ErasureOutcome, execute_account_erasure, execute_account_erasure_with_erase,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const ACCOUNT: &str = "acme";

fn scratch_dir() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "hugit-art17-e2e-{}-{nanos}-{seq}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn canon(v: serde_json::Value) -> String {
    hugit_refstore::canonical_json(&v.to_string()).unwrap()
}

/// A repo log carrying the subject's cleartext: `repo.meta{owner_tenant}` + a
/// subject-authored record with a `clerk:{account}:user` principal and the slug in payload.
fn seed_repo(dir: &std::path::Path, slug: &str) {
    let mut log = EventLog::new();
    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        "repo.meta",
        vec!["orchestrator:hugit".into()],
        canon(serde_json::json!({"visibility":"private","owner_tenant":ACCOUNT})),
        1,
    )
    .unwrap();
    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        "pr.opened",
        vec![format!("clerk:{ACCOUNT}:user-1")],
        canon(serde_json::json!({"author":format!("clerk:{ACCOUNT}:user-1"),"pr":1})),
        2,
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("{slug}.json")),
        serde_json::to_string(log.records()).unwrap(),
    )
    .unwrap();
}

/// An account log with a standing `erasure.requested` carrying the cleartext slug +
/// a `clerk:{account}:user` principal.
fn seed_account(dir: &std::path::Path) {
    let adir = dir.join("_accounts");
    std::fs::create_dir_all(&adir).unwrap();
    let mut log = EventLog::new();
    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        "erasure.requested",
        vec![format!("clerk:{ACCOUNT}:user-1")],
        canon(serde_json::json!({"account":ACCOUNT,"subject":ACCOUNT,"state":"requested","dsr_id":"dsr-7"})),
        1,
    )
    .unwrap();
    std::fs::write(
        adir.join(format!("{ACCOUNT}.json")),
        serde_json::to_string(log.records()).unwrap(),
    )
    .unwrap();
}

/// A mock CAS-erase transport that confirms every digest gone (so the cascade COMPLETES).
struct AllGone;
impl CasEraseTransport for AllGone {
    fn erase(&self, _t: &str, _d: &str, _dsr: &str, _r: &str) -> Result<(), EngineErr> {
        Ok(())
    }
    fn is_gone(&self, _t: &str, _d: &str) -> Result<bool, EngineErr> {
        Ok(true)
    }
}

fn no_cleartext_survives(log: &EventLog) {
    for r in log.records() {
        // The account slug can appear NOWHERE post-erase — not in any payload nor any
        // principal. (The pseudonym is 64-hex, which can never contain the substring
        // "acme" — 'm' is not a hex digit — so this is an exact, strong check.)
        assert!(
            !r.payload.contains(ACCOUNT),
            "no payload carries the cleartext account slug: {}",
            r.payload
        );
        assert!(
            r.principal_chain.iter().all(|p| !p.contains(ACCOUNT)),
            "no principal_chain carries the cleartext account: {:?}",
            r.principal_chain
        );
    }
}

#[test]
fn completed_erase_shreds_the_key_redacts_cleartext_and_the_chains_still_verify() {
    let dir = scratch_dir();
    seed_repo(&dir, "alpha");
    seed_account(&dir);
    let st = AppState::new(dir.clone(), "dev-token".into());

    // Pre-condition: the cleartext IS present before the erase.
    let (before, _) = st.load_account_log(ACCOUNT).unwrap();
    assert!(before.records().iter().any(|r| r.payload.contains(ACCOUNT)));

    // EXECUTE with a transport (empty exclusive set → the cascade completes → Executed).
    let outcome = execute_account_erasure_with_erase(
        &st,
        ACCOUNT,
        vec!["orchestrator:hugit".into()],
        100,
        &AllGone,
        "d863fafb",
        &[],
        "dsr-7",
    )
    .expect("execute");
    assert!(
        matches!(outcome, ErasureOutcome::Executed { .. }),
        "a complete cascade claims executed: {outcome:?}"
    );

    // 1. The account log: cleartext GONE, chain STILL verifies, accountability RETAINED.
    let (alog, _) = st.load_account_log(ACCOUNT).unwrap();
    verify_chain(alog.records()).expect("redacted account chain verifies");
    no_cleartext_survives(&alog);
    assert!(
        alog.records()
            .iter()
            .any(|r| r.kind == "erasure.pii_shredded"),
        "the pseudonymous accountability record is retained (Art.5(2))"
    );
    assert!(
        alog.records()
            .iter()
            .any(|r| r.kind == "provenance.redaction"),
        "a redaction marker was appended"
    );

    // 2. The tombstoned repo log: cleartext GONE, chain STILL verifies, and it is erased.
    let rlog = st.load_verified("alpha").expect("repo log loads");
    verify_chain(rlog.records()).expect("redacted repo chain verifies");
    no_cleartext_survives(&rlog);
    assert!(
        rlog.records().iter().any(|r| r.kind == "repo.erased"),
        "the repo is tombstoned"
    );

    // 3. The subject key is SHREDDED — the pseudonyms are now one-way/unrecoverable.
    assert!(
        st.subject_key_for(ACCOUNT).unwrap().is_none(),
        "the subject key is shredded after a completed erase"
    );

    // 4. Idempotent: a re-execute is AlreadyExecuted and leaves the key shredded.
    let again = execute_account_erasure_with_erase(
        &st,
        ACCOUNT,
        vec!["orchestrator:hugit".into()],
        101,
        &AllGone,
        "d863fafb",
        &[],
        "dsr-7",
    )
    .expect("re-execute");
    assert_eq!(again, ErasureOutcome::AlreadyExecuted);
    assert!(st.subject_key_for(ACCOUNT).unwrap().is_none());
}

#[test]
fn forward_pseudonymization_replaces_clerk_principals_and_authz_is_unaffected() {
    // Leg 2 + leg 3: a NEW write's stored principal_chain is pseudonymised (no cleartext
    // clerk principal enters the chain), while a NON-erased subject's authz is UNCHANGED
    // (authz keys on the projected owner_tenant + the LIVE request principal, never on the
    // stored principal_chain).
    use hugit_serve::authz::{authorize_read, authorize_write, project_repo_meta};
    use hugit_serve::provenance_pii_redact::pseudonymize_write_principal_chain;

    let dir = scratch_dir();
    let st = AppState::new(dir.clone(), "dev-token".into());

    // Forward-pseudonymise a live request chain: the clerk principal → subj:<hmac>, the
    // orchestrator principal untouched.
    let live = vec![
        "orchestrator:hugit".to_string(),
        format!("clerk:{ACCOUNT}:user-1"),
    ];
    let stored = pseudonymize_write_principal_chain(&st, &live).expect("pseudonymise");
    assert_eq!(
        stored[0], "orchestrator:hugit",
        "non-clerk is passed through"
    );
    assert!(
        stored[1].starts_with("subj:"),
        "the clerk principal is stored pseudonymously: {}",
        stored[1]
    );
    assert!(
        !stored[1].contains(ACCOUNT),
        "the stored principal carries no cleartext org"
    );
    // Stable: a repeat yields the SAME pseudonym (idempotency-key-safe).
    let stored2 = pseudonymize_write_principal_chain(&st, &live).unwrap();
    assert_eq!(stored, stored2, "pseudonym is stable while the key lives");

    // A repo owned by ACCOUNT (cleartext owner_tenant in repo.meta — the authz key, NOT
    // pseudonymised for a live subject). Its authored records may be pseudonymous.
    let mut log = EventLog::new();
    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        "repo.meta",
        stored.clone(), // stored records carry the PSEUDONYM in principal_chain
        canon(serde_json::json!({"visibility":"private","owner_tenant":ACCOUNT})),
        1,
    )
    .unwrap();
    let meta = project_repo_meta(&log);
    assert_eq!(meta.owner_tenant.as_deref(), Some(ACCOUNT));

    // Authz for the LIVE owner is UNAFFECTED by the pseudonymised chain.
    let live_owner = vec![format!("clerk:{ACCOUNT}:user-1")];
    assert!(
        authorize_read(&live_owner, &meta),
        "owner reads its private repo"
    );
    assert!(
        authorize_write(&live_owner, &meta),
        "owner writes its private repo"
    );
    let other = vec!["clerk:other-org:u".to_string()];
    assert!(
        !authorize_read(&other, &meta),
        "a different tenant is denied"
    );
    assert!(
        !authorize_write(&other, &meta),
        "a different tenant cannot write"
    );
}

#[test]
fn a_partial_erase_does_not_shred_the_key() {
    let dir = scratch_dir();
    seed_repo(&dir, "alpha");
    seed_account(&dir);
    let st = AppState::new(dir.clone(), "dev-token".into());

    // Pre-mint the subject key (as a forward-pseudonymised write would).
    let _ = st.subject_key_ensure(ACCOUNT).expect("mint key");
    assert!(st.subject_key_for(ACCOUNT).unwrap().is_some());

    // EXECUTE WITHOUT a transport: the account owns a repo → the CAS-GC obligation is
    // unmet → the cascade is PARTIAL (never over-claims executed).
    let outcome = execute_account_erasure(&st, ACCOUNT, vec!["orchestrator:hugit".into()], 100)
        .expect("execute");
    assert!(
        matches!(outcome, ErasureOutcome::Partial { .. }),
        "no transport + owned repo → partial: {outcome:?}"
    );

    // The key is INTACT — a partial must leave the subject's non-erased data resolvable.
    assert!(
        st.subject_key_for(ACCOUNT).unwrap().is_some(),
        "a PARTIAL erase must NOT shred the subject key"
    );
    // And no accountability/redaction was emitted on the partial path.
    let (alog, _) = st.load_account_log(ACCOUNT).unwrap();
    assert!(
        !alog
            .records()
            .iter()
            .any(|r| r.kind == "erasure.pii_shredded"),
        "no accountability record on a partial erase"
    );
}
