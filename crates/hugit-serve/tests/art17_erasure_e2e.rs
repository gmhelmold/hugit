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

/// A repo log that is already TOMBSTONED (repo.erased) but whose cleartext PII survives —
/// the exact state after a first run persisted `erasure.executed` then faulted inside the
/// PII step (repos tombstoned, not yet redacted).
fn seed_tombstoned_repo_with_cleartext(dir: &std::path::Path, slug: &str) {
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
        canon(serde_json::json!({"author":format!("clerk:{ACCOUNT}:user-1")})),
        2,
    )
    .unwrap();
    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        "repo.erased",
        vec!["orchestrator:hugit".into()],
        canon(serde_json::json!({"reason":"erasure","state":"erased"})),
        3,
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("{slug}.json")),
        serde_json::to_string(log.records()).unwrap(),
    )
    .unwrap();
}

/// MF1 + MF2 self-heal: a first run that persisted `erasure.executed` (a record that ITSELF
/// carries the cleartext account slug + `clerk:` principal) then FAULTED inside the PII step
/// leaves the account reading "executed" with cleartext intact + the key un-shredded. The
/// operator's retry hits the AlreadyExecuted short-circuit, which MUST re-drive the PII step
/// to convergence (redact — incl. the executed record — + shred) rather than skip it; and a
/// further post-shred re-execute must succeed idempotently (never 503 on the shred tombstone).
#[test]
fn already_executed_retry_self_heals_the_pii_step_and_is_not_fail_open() {
    let dir = scratch_dir();
    seed_tombstoned_repo_with_cleartext(&dir, "alpha");

    // Seed the fail-open STATE: requested + executed persisted (both cleartext), but NO
    // accountability record + NO redaction markers (the PII step never completed).
    let adir = dir.join("_accounts");
    std::fs::create_dir_all(&adir).unwrap();
    let mut alog = EventLog::new();
    alog.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        "erasure.requested",
        vec![format!("clerk:{ACCOUNT}:user-1")],
        canon(serde_json::json!({"account":ACCOUNT,"subject":ACCOUNT,"state":"requested"})),
        1,
    )
    .unwrap();
    alog.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        "erasure.executed",
        vec![format!("clerk:{ACCOUNT}:user-1")], // the executed record's OWN cleartext principal
        canon(serde_json::json!({"account":ACCOUNT,"state":"executed","repos_tombstoned":1})),
        2,
    )
    .unwrap();
    std::fs::write(
        adir.join(format!("{ACCOUNT}.json")),
        serde_json::to_string(alog.records()).unwrap(),
    )
    .unwrap();

    let st = AppState::new(dir.clone(), "dev-token".into());
    // Simulate the first run having MINTED the key before it faulted (key present, unshred).
    let _ = st.subject_key_ensure(ACCOUNT).expect("mint");

    // Pre-condition: the account reads executed AND cleartext (incl. the executed record) is present.
    let (before, _) = st.load_account_log(ACCOUNT).unwrap();
    assert!(
        before
            .records()
            .iter()
            .any(|r| r.kind == "erasure.executed")
    );
    assert!(
        before.records().iter().any(|r| r.payload.contains(ACCOUNT)),
        "cleartext survives before the self-heal"
    );

    // The operator's RETRY: AlreadyExecuted — but it must self-heal the PII step.
    let outcome = execute_account_erasure(&st, ACCOUNT, vec!["orchestrator:hugit".into()], 500)
        .expect("retry");
    assert_eq!(outcome, ErasureOutcome::AlreadyExecuted);

    // 1. The cleartext is now GONE from the account log — INCLUDING the executed record — and
    //    the chain still verifies; the accountability record was retained.
    let (alog2, _) = st.load_account_log(ACCOUNT).unwrap();
    verify_chain(alog2.records()).expect("healed account chain verifies");
    no_cleartext_survives(&alog2);
    let executed = alog2
        .records()
        .iter()
        .find(|r| r.kind == "erasure.executed")
        .expect("executed record still present");
    assert!(
        !executed.payload.contains(ACCOUNT)
            && executed
                .principal_chain
                .iter()
                .all(|p| !p.contains(ACCOUNT)),
        "the executed claim record itself is now redacted"
    );
    assert!(
        alog2
            .records()
            .iter()
            .any(|r| r.kind == "erasure.pii_shredded"),
        "the accountability record was retained on the self-heal"
    );

    // 2. The tombstoned repo log was redacted too, and still verifies.
    let rlog = st.load_verified("alpha").expect("repo log loads");
    verify_chain(rlog.records()).expect("healed repo chain verifies");
    no_cleartext_survives(&rlog);

    // 3. The key is SHREDDED.
    assert!(
        st.subject_key_for(ACCOUNT).unwrap().is_none(),
        "key shredded on heal"
    );

    // 4. A FURTHER post-shred re-execute is idempotent SUCCESS (MF2: no 503 on the tombstone).
    let again = execute_account_erasure(&st, ACCOUNT, vec!["orchestrator:hugit".into()], 501)
        .expect("post-shred re-execute must succeed, not 503");
    assert_eq!(again, ErasureOutcome::AlreadyExecuted);
    assert!(st.subject_key_for(ACCOUNT).unwrap().is_none());
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

// ── ADR-0004 leg 2: FORWARD write-path pseudonymisation, driven END-TO-END through the
//    real write-door with the real (durable-key-backed) pseudonymizer ──────────────────

use hugit_http_contracts::write_requests::CommentReq;
use hugit_serve::writes::{LogSink, verbs, with_write};

/// Seed a repo owned by `owner` (cleartext `owner_tenant` — the authz key) with an OPEN PR #1,
/// so a `comment` verb has a target. The subject-authored records here are still cleartext; the
/// FORWARD write under test appends the pseudonymised one.
fn seed_repo_with_pr(dir: &std::path::Path, slug: &str, owner: &str) {
    let mut log = EventLog::new();
    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        "repo.meta",
        vec!["orchestrator:hugit".into()],
        canon(serde_json::json!({"visibility":"private","owner_tenant":owner})),
        1,
    )
    .unwrap();
    log.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        "pr.opened",
        vec!["orchestrator:hugit".into()],
        canon(serde_json::json!({
            "author_kind":"orchestrator","campaign":"c","intent_ids":["i"],
            "pr_id":"1","principal":null,"run_id":"r"
        })),
        2,
    )
    .unwrap();
    std::fs::write(
        dir.join(format!("{slug}.json")),
        serde_json::to_string(log.records()).unwrap(),
    )
    .unwrap();
}

#[test]
fn forward_write_through_the_door_pseudonymises_dedups_and_owner_authz_survives() {
    // DoD (1)+(2)+(3), end-to-end through `with_write` + the durable key store:
    // (1) a re-submitted identical `comment` dedups to ONE effect under pseudonymised storage;
    // (2) NO new record stores the cleartext account/clerk principal in its chain (the whole
    //     comment record is clean — its payload carries no slug); (3) the owner's authz still
    //     resolves on the reloaded repo (pseudonymising the stored chain is authz-neutral).
    use hugit_serve::authz::{authorize_write, project_repo_meta};

    let dir = scratch_dir();
    seed_repo_with_pr(&dir, "alpha", ACCOUNT);
    let st = AppState::new(dir.clone(), "dev-token".into());
    let sink: &dyn LogSink = &st;

    let chain = vec![format!("clerk:{ACCOUNT}:user-1")];
    let req = CommentReq {
        body: "looks good".into(),
        anchor: None,
    };
    let body = serde_json::to_vec(&req).unwrap();

    let first = with_write(
        sink,
        "alpha",
        "comment",
        "prs/1/comments",
        "IDEM-1",
        &body,
        true,
        chain.clone(),
        10,
        &st,
        |log, p, at| verbs::write_comment::write_comment(log, "alpha", 1, &req, p, at),
    )
    .expect("first comment ok");
    // A lost-response retry (same key + body) must REPLAY, not post a second comment.
    let replay = with_write(
        sink,
        "alpha",
        "comment",
        "prs/1/comments",
        "IDEM-1",
        &body,
        true,
        chain.clone(),
        11,
        &st,
        |log, p, at| verbs::write_comment::write_comment(log, "alpha", 1, &req, p, at),
    )
    .expect("replay ok");
    assert_eq!(
        first.seq, replay.seq,
        "the replay returns the original outcome"
    );

    // Reload the durably-persisted, chain-verified log.
    let (log, _) = sink.load("alpha").expect("repo log loads + verifies");
    verify_chain(log.records()).expect("the pseudonymised forward chain verifies");
    // (1) Exactly ONE pr.comment despite the resubmission.
    assert_eq!(
        log.records()
            .iter()
            .filter(|r| r.kind == verbs::write_comment::PR_COMMENT_KIND)
            .count(),
        1,
        "the resubmission dedups to ONE comment"
    );
    // (2) The forward comment record carries NO cleartext account/clerk principal anywhere.
    let comment = log
        .records()
        .iter()
        .find(|r| r.kind == verbs::write_comment::PR_COMMENT_KIND)
        .unwrap();
    assert!(
        comment
            .principal_chain
            .iter()
            .all(|p| p.starts_with("subj:")),
        "the stored chain is pseudonymised: {:?}",
        comment.principal_chain
    );
    assert!(
        comment.principal_chain.iter().all(|p| !p.contains(ACCOUNT))
            && !comment.payload.contains(ACCOUNT),
        "no cleartext account survives in the forward record"
    );
    // (3) The owner's authz is UNAFFECTED — it still owns + can write the repo.
    let meta = project_repo_meta(&log);
    assert_eq!(meta.owner_tenant.as_deref(), Some(ACCOUNT));
    assert!(
        authorize_write(&chain, &meta),
        "the non-erased owner still resolves write authz after pseudonymised writes"
    );
    assert!(
        !authorize_write(&["clerk:other-org:u".to_string()], &meta),
        "a different tenant is still denied"
    );
}

#[test]
fn erase_renders_forward_pseudonymised_records_unrecoverable_no_double_handling() {
    // DoD (4): a forward-PSEUDONYMISED write composes cleanly with the leg-4/5 erase — after a
    // completed erase the forward record's identity is UNRECOVERABLE (its `subj:*` pseudonym is
    // one-way once the key is shredded), the chain STILL verifies, and there is no double-
    // handling (a record already pseudonymous forward needs no redaction marker).
    let dir = scratch_dir();
    seed_repo_with_pr(&dir, "alpha", ACCOUNT);
    seed_account(&dir);
    let st = AppState::new(dir.clone(), "dev-token".into());
    let sink: &dyn LogSink = &st;

    // A forward pseudonymised comment by the subject.
    let req = CommentReq {
        body: "mine".into(),
        anchor: None,
    };
    let body = serde_json::to_vec(&req).unwrap();
    with_write(
        sink,
        "alpha",
        "comment",
        "prs/1/comments",
        "IDEM-9",
        &body,
        true,
        vec![format!("clerk:{ACCOUNT}:user-1")],
        10,
        &st,
        |log, p, at| verbs::write_comment::write_comment(log, "alpha", 1, &req, p, at),
    )
    .expect("forward comment ok");

    // Now run the completed erase cascade (empty exclusive set → Executed) over the repo.
    let outcome = execute_account_erasure_with_erase(
        &st,
        ACCOUNT,
        vec!["orchestrator:hugit".into()],
        100,
        &AllGone,
        "d863fafb",
        &["alpha".to_string()],
        "dsr-7",
    )
    .expect("execute");
    assert!(
        matches!(outcome, ErasureOutcome::Executed { .. }),
        "the cascade completes: {outcome:?}"
    );

    // The repo log: cleartext GONE (forward pseudonym is now one-way), chain STILL verifies.
    let rlog = st.load_verified("alpha").expect("repo log loads");
    verify_chain(rlog.records()).expect("post-erase repo chain verifies");
    no_cleartext_survives(&rlog);
    // The forward comment is still present, pseudonymous, and unrecoverable.
    assert!(
        rlog.records()
            .iter()
            .any(|r| r.kind == verbs::write_comment::PR_COMMENT_KIND),
        "the forward comment record is retained (pseudonymously)"
    );
    // The subject key is shredded — the forward `subj:*` can no longer be linked to cleartext.
    assert!(
        st.subject_key_for(ACCOUNT).unwrap().is_none(),
        "the key is shredded after a completed erase"
    );
}

#[test]
fn two_users_in_one_org_do_not_collide_in_the_idempotency_ledger() {
    // MUST-FIX 1 (end-to-end, real key store): the pseudonym is FULL-PRINCIPAL, so two DISTINCT
    // users in ONE org do NOT share an idem tuple. Same Idempotency-Key + verb + resource + body
    // by user-1 then user-2 → TWO comments (no silent lost write, no spurious replay); a user-1
    // resubmit → deduped to ONE for user-1. (Both users own the repo — org == owner_tenant.)
    let dir = scratch_dir();
    seed_repo_with_pr(&dir, "alpha", ACCOUNT);
    let st = AppState::new(dir.clone(), "dev-token".into());
    let sink: &dyn LogSink = &st;
    let req = CommentReq {
        body: "hi".into(),
        anchor: None,
    };
    let body = serde_json::to_vec(&req).unwrap();
    let post = |user: &str, at: u64| {
        with_write(
            sink,
            "alpha",
            "comment",
            "prs/1/comments",
            "SHARED-KEY",
            &body,
            true,
            vec![format!("clerk:{ACCOUNT}:{user}")],
            at,
            &st,
            |log, p, at| verbs::write_comment::write_comment(log, "alpha", 1, &req, p, at),
        )
    };
    post("user-1", 10).expect("user-1 first ok");
    post("user-2", 11).expect("user-2 must NOT dedup against user-1 (distinct pseudonyms)");
    post("user-1", 12).expect("user-1 resubmit replays");

    let (log, _) = sink.load("alpha").expect("repo log loads + verifies");
    verify_chain(log.records()).expect("chain verifies");
    let comment_chains: Vec<String> = log
        .records()
        .iter()
        .filter(|r| r.kind == verbs::write_comment::PR_COMMENT_KIND)
        .flat_map(|r| r.principal_chain.clone())
        .collect();
    assert_eq!(
        comment_chains.len(),
        2,
        "two distinct users → TWO comments (no idem collision); user-1 resubmit deduped to one"
    );
    assert_ne!(
        comment_chains[0], comment_chains[1],
        "user-1 and user-2 get DISTINCT full-principal pseudonyms"
    );
    assert!(
        comment_chains
            .iter()
            .all(|p| p.starts_with("subj:") && !p.contains(ACCOUNT)),
        "both stored pseudonyms are opaque (no cleartext account/user): {comment_chains:?}"
    );
}

// ── HOLE 1: the per-tenant repo REGISTRY (`_tenants/{account}.json`) must carry no cleartext
//    clerk principal — closed at BOTH the write door (1a) and the erase redaction pass (1b) ──

/// Assert `_tenants/{ACCOUNT}.json` carries NO cleartext `clerk:` principal / `acme:user-1` user
/// id in ANY record's `principal_chain` (the `repo` payload legitimately contains the org slug,
/// so we do NOT use the bare-slug `no_cleartext_survives` here).
fn registry_has_no_cleartext_principal(reg: &EventLog) {
    for r in reg.records() {
        assert!(
            r.principal_chain.iter().all(|p| !p.contains("clerk:")),
            "a cleartext clerk principal survives in the tenant registry: {:?}",
            r.principal_chain
        );
        assert!(
            r.principal_chain
                .iter()
                .all(|p| !p.contains(&format!("{ACCOUNT}:user-1"))),
            "a cleartext user id survives in the tenant registry: {:?}",
            r.principal_chain
        );
    }
}

#[test]
fn provision_pseudonymises_the_registry_and_erase_leaves_no_cleartext_there() {
    // HOLE 1 end-to-end THROUGH THE REAL DOORS: provision a repo as `clerk:acme:user-1` (a real
    // cleartext-candidate registry write via the door #319 bypassed), then erase the account.
    // Asserts: (1a) the FRESH registry record stores `subj:`, not cleartext; and after erase the
    // registry has NO cleartext clerk principal AND `verify_chain` still passes. RED on the
    // un-fixed code (the door stored `clerk:acme:user-1` and the erase never touched `_tenants/`).
    let dir = scratch_dir();
    let st = AppState::new(dir.clone(), "dev-token".into());

    let principal = vec![format!("clerk:{ACCOUNT}:user-1")];
    let body =
        serde_json::to_vec(&serde_json::json!({"name":"alpha","visibility":"private"})).unwrap();
    hugit_serve::writes::verbs::write_provision::provision(&st, &body, &principal, 1)
        .expect("provision through the real door");

    // (1a) The fresh registry write stores the PSEUDONYM, never the cleartext clerk principal.
    let (reg0, _) = st.load_tenant_registry(ACCOUNT).expect("registry loads");
    verify_chain(reg0.records()).expect("registry chain verifies");
    let registered: Vec<_> = reg0
        .records()
        .iter()
        .filter(|r| r.kind == hugit_serve::tenant_registry::TENANT_REPO_REGISTERED_KIND)
        .collect();
    assert_eq!(registered.len(), 1, "exactly one repo registered");
    assert!(
        registered[0]
            .principal_chain
            .iter()
            .all(|p| p.starts_with("subj:")),
        "the fresh registry write stores subj:, not cleartext: {:?}",
        registered[0].principal_chain
    );
    registry_has_no_cleartext_principal(&reg0);

    // Request + EXECUTE the account erasure (empty exclusive set → Executed → shred+redact).
    seed_account(&dir);
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
        "the cascade completes: {outcome:?}"
    );

    // (1b) + INVARIANT: after a COMPLETED erase the registry carries NO cleartext clerk principal
    // anywhere and its tamper-evident chain STILL verifies. The key is shredded LAST, so any
    // surviving cleartext could NEVER be redacted afterward — it must already be gone.
    let (reg1, _) = st
        .load_tenant_registry(ACCOUNT)
        .expect("registry loads post-erase");
    verify_chain(reg1.records()).expect("post-erase registry chain still verifies");
    registry_has_no_cleartext_principal(&reg1);
    assert!(
        st.subject_key_for(ACCOUNT).unwrap().is_none(),
        "the subject key is shredded after the completed erase"
    );
}

#[test]
fn erase_redacts_a_legacy_cleartext_registry_record() {
    // HOLE 1b in ISOLATION: a registry record written with a CLEARTEXT clerk principal (the
    // kill-switch-OFF / legacy path the door pseudonymisation does not retroactively cover) must
    // be rendered unrecoverable by the erase redaction pass. RED if `redact_tenant_registry` is
    // not wired into `shred_and_redact_on_execute`.
    let dir = scratch_dir();
    // Seed `_tenants/acme.json` with a CLEARTEXT registered record via the raw registry primitive
    // (bypassing the now-pseudonymising door — exactly what a legacy/kill-switch-OFF write left).
    let tdir = dir.join("_tenants");
    std::fs::create_dir_all(&tdir).unwrap();
    let mut reg = EventLog::new();
    hugit_serve::tenant_registry::append_register(
        &mut reg,
        &format!("{ACCOUNT}/alpha"),
        &[format!("clerk:{ACCOUNT}:user-1")],
        1,
    )
    .unwrap();
    std::fs::write(
        tdir.join(format!("{ACCOUNT}.json")),
        serde_json::to_string(reg.records()).unwrap(),
    )
    .unwrap();
    seed_account(&dir);

    let st = AppState::new(dir.clone(), "dev-token".into());
    // Pre-condition: the cleartext clerk principal IS present in the registry.
    let (before, _) = st.load_tenant_registry(ACCOUNT).unwrap();
    assert!(
        before
            .records()
            .iter()
            .any(|r| r.principal_chain.iter().any(|p| p.contains("clerk:"))),
        "cleartext clerk principal is present before the erase"
    );

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
        "{outcome:?}"
    );

    // The registry cleartext is now redacted (hash-preservingly) and the chain still verifies.
    let (after, _) = st.load_tenant_registry(ACCOUNT).unwrap();
    verify_chain(after.records()).expect("redacted registry chain verifies");
    registry_has_no_cleartext_principal(&after);
    assert!(
        after
            .records()
            .iter()
            .any(|r| r.kind == "provenance.redaction"),
        "a redaction marker was appended to the registry"
    );
}

// ── HOLE 2: the post-claim shred+redact self-heal must be REACHABLE via the background sweep ──

#[test]
fn background_sweep_reconciles_a_stranded_executed_account() {
    // HOLE 2 end-to-end THROUGH THE SWEEP'S per-account entry (`auto_execute_one` → the predicate
    // + `reconcile_pii_completion`), NOT a direct `execute_account_erasure` call: an account left
    // reading `executed` with cleartext un-redacted + key un-shredded (a transient durable fault
    // hit the post-claim PII step) is converged. RED on the un-fixed code (the sweep skipped it
    // because `executed` supersedes the standing request → the self-heal was dead code in prod).
    let dir = scratch_dir();
    seed_tombstoned_repo_with_cleartext(&dir, "alpha");

    // The STRANDED account log: requested + executed persisted (both cleartext), NO pii_shredded.
    let adir = dir.join("_accounts");
    std::fs::create_dir_all(&adir).unwrap();
    let mut alog = EventLog::new();
    alog.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        "erasure.requested",
        vec![format!("clerk:{ACCOUNT}:user-1")],
        canon(
            serde_json::json!({"account":ACCOUNT,"subject":ACCOUNT,"state":"requested","dsr_id":"dsr-7"}),
        ),
        1,
    )
    .unwrap();
    alog.append_authorized(
        PrincipalClass::Orchestrator,
        Endpoint::Land,
        "erasure.executed",
        vec![format!("clerk:{ACCOUNT}:user-1")],
        canon(serde_json::json!({"account":ACCOUNT,"state":"executed","repos_tombstoned":1})),
        2,
    )
    .unwrap();
    std::fs::write(
        adir.join(format!("{ACCOUNT}.json")),
        serde_json::to_string(alog.records()).unwrap(),
    )
    .unwrap();

    let st = AppState::new(dir.clone(), "dev-token".into());
    // The first run MINTED the key before it faulted (key live, un-shredded).
    let _ = st.subject_key_ensure(ACCOUNT).expect("mint");

    // Pre-condition: the predicate detects the stranded state + cleartext is present.
    let (before, _) = st.load_account_log(ACCOUNT).unwrap();
    assert!(
        hugit_serve::writes::erasure::pii_shred_incomplete(&before),
        "the pure predicate flags the stranded (executed, no pii_shredded) state"
    );
    assert!(
        before.records().iter().any(|r| r.payload.contains(ACCOUNT)),
        "cleartext survives before the reconcile"
    );
    assert!(st.subject_key_for(ACCOUNT).unwrap().is_some(), "key live");

    // Drive the RECONCILER via the sweep's per-account entry. grace/now are irrelevant to the
    // reconcile arm (executed supersedes the standing request, so the auto-EXECUTE arm is inert).
    let out = st
        .auto_execute_one("d863fafb", ACCOUNT, 0, 1_000_000)
        .expect("reconcile drive");
    assert_eq!(
        out,
        Some(ErasureOutcome::AlreadyExecuted),
        "the reconciler converged the stranded PII step"
    );

    // Converged: cleartext GONE from account + repo, key SHREDDED, accountability RECORDED, chains
    // still verify.
    let (alog2, _) = st.load_account_log(ACCOUNT).unwrap();
    verify_chain(alog2.records()).expect("healed account chain verifies");
    no_cleartext_survives(&alog2);
    assert!(
        alog2
            .records()
            .iter()
            .any(|r| r.kind == "erasure.pii_shredded"),
        "the accountability record is now recorded"
    );
    let rlog = st.load_verified("alpha").expect("repo log loads");
    verify_chain(rlog.records()).expect("healed repo chain verifies");
    no_cleartext_survives(&rlog);
    assert!(
        st.subject_key_for(ACCOUNT).unwrap().is_none(),
        "the subject key is shredded on the reconcile"
    );

    // Idempotent: a SECOND sweep pass is a clean NO-OP (predicate now false; no re-mint/re-shred).
    let (alog3, _) = st.load_account_log(ACCOUNT).unwrap();
    assert!(
        !hugit_serve::writes::erasure::pii_shred_incomplete(&alog3),
        "the converged account no longer matches the reconcile predicate"
    );
    let again = st
        .auto_execute_one("d863fafb", ACCOUNT, 0, 1_000_001)
        .expect("second pass");
    assert_eq!(
        again, None,
        "a converged account is skipped on the next sweep"
    );
    assert!(st.subject_key_for(ACCOUNT).unwrap().is_none());
}
