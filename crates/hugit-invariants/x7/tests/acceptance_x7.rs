//! WP-X7 acceptance oracle — the right-to-erasure CASCADE.
//! Contract: `docs/plan/wp-contracts/WP-X7.md`.
//!
//! Owned items (VERBATIM from the contract; one `#[test] item_<n>_…` each, plus
//! adversarial guards proving each oracle goes RED on the gamed/broken case):
//!
//!   ① a data subject's personal data provably erased across CAS +
//!      provenance/ledger + context store + the GitHub mirror + the experiment
//!      corpus (R10)
//!   ② no orphaned provenance refs survive erasure
//!   ③ attestation chains re-seal or fail CLOSED after erasure (never silently
//!      broken)
//!   ④ erasure × seal precedence: lawful erasure of a corpus datapoint is
//!      PERMITTED despite the seal, and the gate verdict invalidates FAIL-CLOSED
//!      (claims/regen re-pin until re-evaluation) — never blocked by the seal,
//!      never a silently broken seal
//!
//! WHY THE CASCADE IS LOAD-BEARING: X3 owns the context-store purge in isolation
//! (a PARTIAL while no cross-store cascade existed); D1 the ledger, B2 the CAS,
//! E1 the mirror, D8 the sealed corpus. X7 proves a SINGLE erasure sweeps ALL
//! FIVE leaving no orphan and no silently broken seal — and supplies the cascade
//! that closes the X3③ PARTIAL. The oracle drives the REAL canonical
//! `hugit_refstore::{verify_chain, compute_this_hash, attestation_sig_preimage}`
//! and the frozen `hugit_contracts::{AttestationChain, RegenGate, EventRecord}`
//! so it tests the production surfaces, never a hand-rolled stand-in. Item ③ uses
//! REAL ed25519 (asymmetric) signatures — a stale seal cannot be silently
//! accepted.

#[path = "../cascade.rs"]
mod cascade;

use cascade::{
    AttestationError, CascadeAbsenceScan, ContextStore, CorpusDatapoint, CorpusErasureError,
    MirrorObligation, MirrorObligationOutcome, OBJECT_LINK_KIND, ObjectStore, Resolution,
    SealedCorpus, detect_silent_relink, link_target, object_link_payload, reseal_attestation,
    scan_cascade_absence, scan_orphans, sign_attestation, verify_attestation,
};
use ed25519_dalek::{SigningKey, VerifyingKey};
use hugit_contracts::attestation_chain::AttestationChain;
use hugit_contracts::event_record::EventRecord;
use hugit_contracts::regen_gate::RegenGate;
use hugit_refstore::{EventLog, TamperError, compute_this_hash, verify_chain};

// ── shared fixtures ──────────────────────────────────────────────────────────

/// The personal-data needle that must be ABSENT from every store after erasure.
const SUBJECT_PII: &str = "alice@example.com:ssn-123-45-6789";
/// The CAS content hash of the subject's personal-data object.
const SUBJECT_OBJECT: &str = "sha256:subject-personal-data-0001";
/// A substitute object an attacker might silently re-link the provenance to.
const SUBSTITUTE_OBJECT: &str = "sha256:attacker-substitute-9999";
/// The corpus datapoint id carrying the subject's data.
const SUBJECT_DATAPOINT: &str = "dp-0007";
/// The right-to-erasure request id authorising the cascade.
const RTBF_REQUEST: &str = "rtbf-request:case-42";

/// Deterministic signing key for the producer (no RNG — pure constructor).
fn producer_signing_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// Build a real provenance chain (via canonical `EventLog::append`) linking to
/// `SUBJECT_OBJECT`, returning the records and the object-link seq.
fn seed_provenance() -> (Vec<EventRecord>, u64) {
    let mut log = EventLog::new();
    log.append(
        "tree.snapshot",
        vec!["agent:planner".to_string()],
        object_link_payload("sha256:tree-root"),
        1_000,
    );
    let link = log.append(
        OBJECT_LINK_KIND,
        vec!["agent:executor".to_string(), "human:owner".to_string()],
        object_link_payload(SUBJECT_OBJECT),
        2_000,
    );
    log.append(
        "check.result",
        vec!["runner:box-01".to_string()],
        object_link_payload("sha256:check-out"),
        3_000,
    );
    (log.records().to_vec(), link.seq)
}

/// Seed the CAS object store with the subject object (carrying the PII) live.
fn seed_cas() -> ObjectStore {
    let mut store = ObjectStore::new();
    store.put("sha256:tree-root", "tree bytes");
    store.put(SUBJECT_OBJECT, format!("personal data: {SUBJECT_PII}"));
    store.put("sha256:check-out", "check output bytes");
    store.put(SUBSTITUTE_OBJECT, "unrelated substitute bytes");
    store
}

/// Seed the context store with the subject's journal datum (carrying the PII).
fn seed_context() -> ContextStore {
    let mut ctx = ContextStore::new();
    ctx.put("ws:42/intent:land", "benign session note");
    ctx.put("ws:42/journal:subject", format!("captured: {SUBJECT_PII}"));
    ctx
}

/// Seed a sealed experiment corpus containing the subject's datapoint.
fn seed_sealed_corpus() -> SealedCorpus {
    let mut corpus = SealedCorpus::new(vec![
        CorpusDatapoint {
            id: "dp-0001".to_string(),
            data: "wave-1 disjoint green".to_string(),
        },
        CorpusDatapoint {
            id: SUBJECT_DATAPOINT.to_string(),
            data: format!("wave-2 datapoint authored by {SUBJECT_PII}"),
        },
    ]);
    corpus.seal();
    corpus
}

fn passing_gate() -> RegenGate {
    RegenGate {
        optin_scope: "acme/repo".to_string(),
        repass: true,
        indep_verdict: "sha256:independent-verdict-PASS".to_string(),
    }
}

fn well_formed_attestation() -> AttestationChain {
    AttestationChain {
        tree: "sha256:tree-pre-erasure".to_string(),
        def: "sha256:checkdef".to_string(),
        runner: "sha256:runner".to_string(),
        model: "claude-opus-4-8".to_string(),
        principal: vec!["agent:executor".to_string(), "human:owner".to_string()],
        sig: String::new(),
    }
}

// ── ① a data subject's personal data provably erased across CAS + ─────────────
//     provenance/ledger + context store + the GitHub mirror + the experiment
//     corpus

#[test]
fn item_1_personal_data_erased_across_all_five_stores() {
    let (records, link_seq) = seed_provenance();
    let mut cas = seed_cas();
    let mut context = seed_context();
    let mut corpus = seed_sealed_corpus();

    let linked = link_target(&records[link_seq as usize]).expect("object-link target");
    assert_eq!(linked, SUBJECT_OBJECT);

    // SANITY: before erasure the subject's PII is PRESENT in every byte-scannable
    // store, and the CAS object is live.
    assert!(
        cas.live_bytes_present(SUBJECT_OBJECT),
        "CAS object live pre"
    );
    assert!(!context.datum_absent(SUBJECT_PII), "context has PII pre");
    assert!(!corpus.datum_absent(SUBJECT_PII), "corpus has PII pre");

    // ── the cascade: one right-to-erasure request sweeps all five stores. ──
    // 1. CAS: erase the object → tamper-evident tombstone (never touches chain).
    cas.erase(SUBJECT_OBJECT, RTBF_REQUEST);
    // 2. provenance/ledger: NOT rewritten (append-only) — proven by chain below.
    // 3. context store: genuinely PURGE the bytes (closes the X3③ leg).
    context.purge("ws:42/journal:subject");
    // 4. GitHub mirror: the P2 seam — honest residual-risk disclosure.
    let mirror = MirrorObligation {
        object_hash: SUBJECT_OBJECT.to_string(),
        mirror_target: "github.com/acme/repo".to_string(),
        outcome: MirrorObligationOutcome::ResidualRisk {
            disclosure: "GitHub mirror copy may persist in third-party forks/caches \
                 beyond hugit's physical control; canonical-copy deletion requested, \
                 full erasure cannot be guaranteed."
                .to_string(),
        },
    };
    // 5. experiment corpus: erase the sealed datapoint (precedence — item ④).
    corpus
        .erase_datapoint(SUBJECT_DATAPOINT, RTBF_REQUEST, &passing_gate())
        .expect("sealed datapoint erasure is permitted");

    // ── absence scan across ALL FIVE stores (re-read each, not a flag). ──
    let scan: CascadeAbsenceScan = scan_cascade_absence(
        &records,
        &cas,
        SUBJECT_OBJECT,
        &context,
        &corpus,
        &mirror,
        SUBJECT_PII,
    );

    assert!(scan.cas_live_absent, "CAS: live subject bytes must be gone");
    assert!(scan.no_orphans, "ledger: no orphaned provenance refs");
    assert_eq!(scan.chain_verifies, Ok(()), "ledger: chain still verifies");
    assert!(scan.context_absent, "context: PII bytes must be purged");
    assert!(scan.mirror_resolved, "mirror: obligation honestly resolved");
    assert!(scan.corpus_absent, "corpus: datapoint PII must be absent");
    assert!(scan.fully_erased(), "item ①: erased across ALL FIVE stores");

    // The CAS object resolves to a tamper-evident tombstone, not live, not void.
    match cas.resolve(SUBJECT_OBJECT) {
        Resolution::Tombstone(ts) => {
            assert!(ts.is_tombstone());
            assert_eq!(ts.erased_object_hash, SUBJECT_OBJECT);
        }
        other => panic!("erased CAS object must be a tombstone, got {other:?}"),
    }
}

#[test]
fn item_1_incomplete_cascade_is_caught_red() {
    // ADVERSARIAL: a cascade that erases CAS + ledger + corpus but FORGETS the
    // context store (the leg the X3③ PARTIAL covers). The five-store absence
    // scan must go RED — a partial cascade is not erasure. If the oracle passed
    // here it would be gamed (claiming erasure while PII survives in context).
    let (records, _link_seq) = seed_provenance();
    let mut cas = seed_cas();
    let context = seed_context(); // NOT purged — the planted gap.
    let mut corpus = seed_sealed_corpus();

    cas.erase(SUBJECT_OBJECT, RTBF_REQUEST);
    corpus
        .erase_datapoint(SUBJECT_DATAPOINT, RTBF_REQUEST, &passing_gate())
        .unwrap();
    let mirror = MirrorObligation {
        object_hash: SUBJECT_OBJECT.to_string(),
        mirror_target: "github.com/acme/repo".to_string(),
        outcome: MirrorObligationOutcome::Discharged {
            verification: "deleted-ref 404".to_string(),
        },
    };
    // A discharged mirror obligation is honestly resolved with no residual text.
    assert!(
        mirror.is_discharged(),
        "discharged obligation reports discharged"
    );
    assert!(
        mirror.residual_disclosure().is_none(),
        "discharged carries no residual"
    );

    let scan = scan_cascade_absence(
        &records,
        &cas,
        SUBJECT_OBJECT,
        &context,
        &corpus,
        &mirror,
        SUBJECT_PII,
    );
    assert!(
        !scan.context_absent,
        "the un-purged context leg must scan PRESENT"
    );
    assert!(
        !scan.fully_erased(),
        "an incomplete cascade must NOT report full erasure (oracle not gamed)",
    );
}

#[test]
fn item_1_empty_mirror_disclosure_fails_the_mirror_leg() {
    // ADVERSARIAL: the mirror leg "discloses" residual risk with EMPTY text — an
    // omission masquerading as a disclosure. The mirror leg must scan unresolved.
    let mirror = MirrorObligation {
        object_hash: SUBJECT_OBJECT.to_string(),
        mirror_target: "github.com/acme/repo".to_string(),
        outcome: MirrorObligationOutcome::ResidualRisk {
            disclosure: "   ".to_string(),
        },
    };
    assert!(
        !mirror.is_honestly_resolved(),
        "an empty residual-risk disclosure is not an honest resolution",
    );
}

// ── ② no orphaned provenance refs survive erasure ─────────────────────────────

#[test]
fn item_2_no_orphaned_provenance_refs_survive_erasure() {
    let (records, _link_seq) = seed_provenance();
    let mut cas = seed_cas();

    // Pre-erasure: zero orphans (every link resolves live).
    assert!(
        scan_orphans(&records, &cas).is_empty(),
        "no orphans pre-erasure"
    );

    // Erase the subject object → tombstone (the correct cascade leaves a marker).
    cas.erase(SUBJECT_OBJECT, RTBF_REQUEST);

    // Post-erasure: STILL zero orphans — the erased link resolves to a tombstone,
    // every other link still resolves live. No dangling ref survives.
    let orphans = scan_orphans(&records, &cas);
    assert!(
        orphans.is_empty(),
        "no orphaned provenance refs may survive erasure, found {orphans:?}",
    );
    // And the surviving link still NAMES the original subject object (not voided,
    // not re-pointed).
    assert_eq!(
        cas.resolve(SUBJECT_OBJECT),
        Resolution::Tombstone(cascade::Tombstone::new(SUBJECT_OBJECT, RTBF_REQUEST)),
    );
}

#[test]
fn item_2_void_erasure_yields_a_detected_orphan() {
    // ADVERSARIAL: a broken cascade that DELETES the object without leaving a
    // tombstone (a void). The orphan scan MUST flag the dangling ref — if it did
    // not, an erasure that orphaned provenance would pass undetected (gamed).
    let (records, link_seq) = seed_provenance();
    // A store missing the subject object entirely (simulating a void delete).
    let mut cas = ObjectStore::new();
    cas.put("sha256:tree-root", "tree bytes");
    cas.put("sha256:check-out", "check output bytes");
    // SUBJECT_OBJECT deliberately absent and NOT tombstoned → it will resolve
    // Missing for the surviving link.
    assert!(matches!(cas.resolve(SUBJECT_OBJECT), Resolution::Missing));

    let orphans = scan_orphans(&records, &cas);
    assert_eq!(orphans.len(), 1, "the void must surface exactly one orphan");
    assert_eq!(orphans[0].seq, link_seq);
    assert_eq!(orphans[0].target, SUBJECT_OBJECT);
}

#[test]
fn item_2_silent_relink_is_caught_fail_closed() {
    // ADVERSARIAL: instead of a tombstone, an actor re-points the provenance
    // record at a SUBSTITUTE so the link "still resolves". This rewrites the
    // payload; the canonical chain catches it fail-closed.
    let (mut records, link_seq) = seed_provenance();
    let idx = link_seq as usize;
    records[idx].payload = object_link_payload(SUBSTITUTE_OBJECT);

    assert_eq!(
        detect_silent_relink(&records[idx]).as_deref(),
        Some(SUBSTITUTE_OBJECT),
        "silent re-link must be detected with its substitute surfaced",
    );
    match verify_chain(&records) {
        Err(TamperError::ThisHashMismatch { seq, .. }) => assert_eq!(seq, link_seq),
        other => panic!("silent re-link must fail-closed via verify_chain, got {other:?}"),
    }
}

#[test]
fn item_2_relink_then_reseal_breaks_downstream_chain() {
    // ADVERSARIAL (stronger): re-link AND re-seal this_hash to hide the mismatch.
    // The chain still catches it — the NEXT record's prev_hash goes stale.
    let (mut records, link_seq) = seed_provenance();
    let idx = link_seq as usize;
    records[idx].payload = object_link_payload(SUBSTITUTE_OBJECT);
    records[idx].this_hash = compute_this_hash(
        &records[idx].prev_hash,
        &records[idx].kind,
        &records[idx].principal_chain,
        &records[idx].payload,
        records[idx].seq,
    );
    match verify_chain(&records) {
        Err(TamperError::PrevHashMismatch { seq, .. }) => assert_eq!(seq, link_seq + 1),
        other => panic!("re-sealed re-link must still break the chain, got {other:?}"),
    }
}

// ── ③ attestation chains re-seal or fail CLOSED after erasure ─────────────────
//     (never silently broken)

#[test]
fn item_3_attestation_reseals_over_post_erasure_manifest() {
    // The "re-seals" branch: after erasure the attestation is re-pointed at the
    // post-erasure (tombstone-bearing) tree AND re-signed. It verifies via REAL
    // ed25519 over the canonical preimage — the chain is intact over the new
    // state, not silently broken.
    let sk = producer_signing_key();
    let vk: VerifyingKey = sk.verifying_key();

    // Pre-erasure: a signed attestation that verifies.
    let pre = sign_attestation(&sk, &well_formed_attestation());
    assert_eq!(
        verify_attestation(&vk, &pre),
        Ok(()),
        "pre-erasure verifies"
    );

    // Erasure re-seals over the post-erasure manifest.
    let resealed = reseal_attestation(&sk, &pre, "sha256:tree-post-erasure-tombstone");
    assert_eq!(
        verify_attestation(&vk, &resealed),
        Ok(()),
        "③ re-seal branch: re-sealed attestation must verify (intact over new state)",
    );
    // The seal genuinely moved to the post-erasure manifest (not the old tree).
    assert_eq!(resealed.tree, "sha256:tree-post-erasure-tombstone");
    assert_ne!(resealed.sig, pre.sig, "re-seal produced a fresh signature");
}

#[test]
fn item_3_stale_seal_after_relink_fails_closed_never_silent() {
    // The "fail CLOSED" branch: an actor re-points the attestation's tree at the
    // post-erasure manifest WITHOUT re-signing (a silently broken seal attempt).
    // verify_attestation must REJECT it fail-closed — there is no accepted stale
    // seal.
    let sk = producer_signing_key();
    let vk = sk.verifying_key();
    let signed = sign_attestation(&sk, &well_formed_attestation());

    // Mutate a link AFTER signing; keep the old sig (the stale-seal attack).
    let stale = AttestationChain {
        tree: "sha256:tree-post-erasure-tombstone".to_string(),
        ..signed.clone()
    };
    assert_eq!(
        verify_attestation(&vk, &stale),
        Err(AttestationError::SignatureMismatch),
        "③ fail-closed branch: a stale (silently broken) seal must be rejected",
    );

    // An unsigned chain is likewise rejected fail-closed (never silently OK).
    let unsigned = well_formed_attestation();
    assert_eq!(
        verify_attestation(&vk, &unsigned),
        Err(AttestationError::Unsigned)
    );
}

#[test]
fn item_3_asymmetry_attacker_cannot_reseal() {
    // The seal is ASYMMETRIC: only the producer's private key can re-seal. An
    // attacker re-signing with a different key fails against the producer's
    // public key — a re-seal cannot be forged.
    let producer = producer_signing_key();
    let attacker = SigningKey::from_bytes(&[42u8; 32]);
    let producer_vk = producer.verifying_key();

    let pre = sign_attestation(&producer, &well_formed_attestation());
    let forged = reseal_attestation(&attacker, &pre, "sha256:tree-post-erasure-tombstone");
    assert_eq!(
        verify_attestation(&producer_vk, &forged),
        Err(AttestationError::SignatureMismatch),
        "an attacker-signed re-seal must not verify against the producer key",
    );
}

// ── ④ erasure × seal precedence ───────────────────────────────────────────────
//     lawful erasure of a corpus datapoint is PERMITTED despite the seal, and the
//     gate verdict invalidates FAIL-CLOSED (claims/regen re-pin until
//     re-evaluation) — never blocked by the seal, never a silently broken seal

#[test]
fn item_4_erasure_of_sealed_datapoint_permitted_and_invalidates_gate_failclosed() {
    let mut corpus = seed_sealed_corpus();
    assert!(
        corpus.is_sealed(),
        "corpus is sealed before evaluation (D8④)"
    );
    assert!(
        corpus.contains(SUBJECT_DATAPOINT),
        "datapoint present pre-erasure"
    );

    let gate = passing_gate();
    assert!(
        gate.repass,
        "the standing gate verdict is a PASS pre-erasure"
    );

    // (a) PERMITTED despite the seal — erasure is NOT blocked, returns audit.
    let invalidation = corpus
        .erase_datapoint(SUBJECT_DATAPOINT, RTBF_REQUEST, &gate)
        .expect("④a: lawful erasure of a SEALED datapoint must be PERMITTED");

    // The datapoint is genuinely gone (erasure wins over the seal).
    assert!(
        !corpus.contains(SUBJECT_DATAPOINT),
        "datapoint erased despite seal"
    );
    assert!(
        corpus.datum_absent(SUBJECT_PII),
        "the datapoint's PII is absent"
    );

    // (b) the gate verdict INVALIDATES fail-closed — repinned advisory/OFF/blocked.
    assert!(
        invalidation.is_failclosed(),
        "④b: gate verdict must invalidate FAIL-CLOSED (claims/regen re-pin)",
    );
    assert!(
        !invalidation.repinned_gate.repass,
        "regen promotion re-pinned blocked"
    );
    assert!(
        invalidation.repinned_gate.indep_verdict.is_empty(),
        "the prior independent PASS verdict no longer counts (cleared until re-eval)",
    );
    // The opt-in scope survives (only the verdict is invalidated, not the policy).
    assert_eq!(invalidation.repinned_gate.optin_scope, gate.optin_scope);

    // (c) the invalidation is RECORDED, never silent — it is a first-class,
    // serialisable audit object naming the erased datapoint and the request.
    assert_eq!(invalidation.erased_datapoint, SUBJECT_DATAPOINT);
    assert_eq!(invalidation.reason, RTBF_REQUEST);
    let json = serde_json::to_string(&invalidation).expect("invalidation serialises (audited)");
    assert!(
        json.contains(SUBJECT_DATAPOINT) && json.contains(RTBF_REQUEST),
        "④c: the invalidation must be an audited (serialisable) record, not silent",
    );
    let back: cascade::GateInvalidation =
        serde_json::from_str(&json).expect("invalidation round-trips");
    assert_eq!(back, invalidation);
}

#[test]
fn item_4_seal_does_not_block_and_passing_verdict_is_not_silently_kept() {
    // ADVERSARIAL: the failure modes the precedence resolution forbids —
    //   (1) the seal BLOCKING the erasure, and
    //   (2) the gate verdict being SILENTLY KEPT (still PASS) after erasure.
    // Both must be impossible. We assert erase_datapoint never returns the input
    // gate unchanged and never errors due to the seal.
    let mut corpus = seed_sealed_corpus();
    let passing = passing_gate();

    let result = corpus.erase_datapoint(SUBJECT_DATAPOINT, RTBF_REQUEST, &passing);
    assert!(result.is_ok(), "the seal must NOT block a lawful erasure");
    let invalidation = result.unwrap();

    // The verdict is NOT silently kept: the re-pinned gate differs from the input
    // PASS gate on exactly the verdict axes (repass + indep_verdict).
    assert_ne!(
        invalidation.repinned_gate, passing,
        "the passing verdict must NOT be silently kept after erasure",
    );
    assert!(
        invalidation.repinned_gate.repass != passing.repass,
        "repass must flip from PASS to blocked",
    );
}

#[test]
fn item_4_erasing_absent_datapoint_errors_not_a_false_invalidation() {
    // Guard: erasing a datapoint that is not in the corpus is a NotFound error,
    // not a spurious gate invalidation (no verdict is invalidated for a no-op).
    let mut corpus = seed_sealed_corpus();
    let err = corpus
        .erase_datapoint("dp-does-not-exist", RTBF_REQUEST, &passing_gate())
        .expect_err("erasing an absent datapoint must error");
    assert_eq!(
        err,
        CorpusErasureError::NotFound {
            id: "dp-does-not-exist".to_string(),
        },
    );
}

// ── composition: ① ∧ ② ∧ ③ ∧ ④ together (the full cascade) ───────────────────

#[test]
fn composition_full_cascade_erases_everywhere_no_orphan_reseals_and_invalidates() {
    let (records, link_seq) = seed_provenance();
    let mut cas = seed_cas();
    let mut context = seed_context();
    let mut corpus = seed_sealed_corpus();
    let sk = producer_signing_key();
    let vk = sk.verifying_key();
    let linked = link_target(&records[link_seq as usize]).unwrap();

    // One cascade across all five stores.
    cas.erase(&linked, RTBF_REQUEST);
    context.purge("ws:42/journal:subject");
    let invalidation = corpus
        .erase_datapoint(SUBJECT_DATAPOINT, RTBF_REQUEST, &passing_gate())
        .unwrap();
    let mirror = MirrorObligation {
        object_hash: linked.clone(),
        mirror_target: "github.com/acme/repo".to_string(),
        outcome: MirrorObligationOutcome::ResidualRisk {
            disclosure: "mirror copy may persist in forks/caches; residual risk disclosed."
                .to_string(),
        },
    };

    // ① five-store absence.
    let scan = scan_cascade_absence(
        &records,
        &cas,
        &linked,
        &context,
        &corpus,
        &mirror,
        SUBJECT_PII,
    );
    assert!(scan.fully_erased(), "① erased across all five stores");
    // ② no orphans (also folded into the scan).
    assert!(
        scan_orphans(&records, &cas).is_empty(),
        "② no orphaned refs"
    );
    // ③ attestation re-seals over the post-erasure manifest and verifies.
    let resealed = reseal_attestation(
        &sk,
        &sign_attestation(&sk, &well_formed_attestation()),
        "sha256:tree-post-erasure-tombstone",
    );
    assert_eq!(
        verify_attestation(&vk, &resealed),
        Ok(()),
        "③ re-seal verifies"
    );
    // ④ the sealed corpus erasure invalidated the gate fail-closed, audited.
    assert!(
        invalidation.is_failclosed(),
        "④ gate invalidated fail-closed"
    );
    assert_eq!(invalidation.erased_datapoint, SUBJECT_DATAPOINT);
}
