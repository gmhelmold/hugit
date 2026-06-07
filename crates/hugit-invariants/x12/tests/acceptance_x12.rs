//! WP-X12 acceptance oracle — erasure × provenance × mirror.
//! Contract: `docs/plan/wp-contracts/WP-X12.md`.
//!
//! Owned items (VERBATIM from the contract; one `#[test] item_<n>_…` each, plus
//! adversarial guards proving each oracle goes RED on the gamed/broken case):
//!
//!   ① after an erasure request the attestation chain remains independently
//!      verifiable with the erased object as a tamper-evident TOMBSTONE (never
//!      silently re-linked)
//!   ② the mirror-side erasure obligation (data already replicated to GitHub) is
//!      discharged or explicitly surfaced as residual risk — and that disclosure
//!      is part of the export/exit proof
//!
//! WHY THE COMPOSITION IS LOAD-BEARING: X7 owns the cascade, X2/refstore the
//! attestation chain, E1 the mirror, E5 the export. X12 proves they COMPOSE: an
//! erasure must not silently break or re-point provenance, and the residual risk
//! that the mirror leg cannot fully discharge must travel inside the export/exit
//! proof. The oracle uses the REAL canonical `hugit_refstore::verify_chain` /
//! `compute_this_hash` and the frozen `hugit_contracts::{ExportSchema}` so it
//! tests the production surfaces, never a hand-rolled stand-in.

#[path = "../erasure.rs"]
mod erasure;

use erasure::{
    ExitProof, ExitProofError, MIRROR_OBLIGATION_CLASS, MirrorObligation, MirrorObligationOutcome,
    OBJECT_LINK_KIND, ObjectStore, Resolution, detect_silent_relink, link_target,
    object_link_payload, verify_after_erasure,
};
use hugit_contracts::event_record::EventRecord;
use hugit_contracts::export_schema::ExportSchema;
use hugit_refstore::{EventLog, TamperError, compute_this_hash, verify_chain};

// ── shared fixtures ──────────────────────────────────────────────────────────

/// The content hash of the personal-data object that will be erased.
const SUBJECT_OBJECT: &str = "sha256:subject-personal-data-0001";
/// A substitute object an attacker might silently re-link the provenance to.
const SUBSTITUTE_OBJECT: &str = "sha256:attacker-substitute-9999";

/// Build a real provenance chain (via the canonical `EventLog::append`) that
/// links to `SUBJECT_OBJECT`, and seed the matching object store. Returns the
/// chain's records, the live store, and the seq of the object-link record.
fn seed_provenance_and_store() -> (Vec<EventRecord>, ObjectStore, u64) {
    let mut log = EventLog::new();
    // genesis: a workspace snapshot event.
    log.append(
        "tree.snapshot",
        vec!["agent:planner".to_string()],
        object_link_payload("sha256:tree-root"),
        1_000,
    );
    // the object-link event: provenance references SUBJECT_OBJECT by content hash.
    let link = log.append(
        OBJECT_LINK_KIND,
        vec!["agent:executor".to_string(), "human:owner".to_string()],
        object_link_payload(SUBJECT_OBJECT),
        2_000,
    );
    // a later event chained on top (proves the link sits mid-chain).
    log.append(
        "check.result",
        vec!["runner:box-01".to_string()],
        object_link_payload("sha256:check-out"),
        3_000,
    );

    let mut store = ObjectStore::new();
    store.put("sha256:tree-root", "tree bytes");
    store.put(SUBJECT_OBJECT, "the data subject's personal data");
    store.put("sha256:check-out", "check output bytes");
    store.put(SUBSTITUTE_OBJECT, "unrelated substitute bytes");

    (log.records().to_vec(), store, link.seq)
}

fn export_schema_with_obligation_class() -> ExportSchema {
    ExportSchema {
        version: "1.0.0".to_string(),
        object_classes: vec![
            "event_record".to_string(),
            "attestation_chain".to_string(),
            MIRROR_OBLIGATION_CLASS.to_string(),
        ],
        redaction_manifest: "sha256:redaction-manifest-aaaa".to_string(),
    }
}

fn export_schema_without_obligation_class() -> ExportSchema {
    ExportSchema {
        version: "1.0.0".to_string(),
        object_classes: vec!["event_record".to_string(), "attestation_chain".to_string()],
        redaction_manifest: "sha256:redaction-manifest-aaaa".to_string(),
    }
}

// ── ① after an erasure request the attestation chain remains independently ────
//     verifiable with the erased object as a tamper-evident TOMBSTONE (never
//     silently re-linked)

#[test]
fn item_1_chain_verifiable_over_tamper_evident_tombstone_no_silent_relink() {
    let (records, mut store, link_seq) = seed_provenance_and_store();

    // Sanity: BEFORE erasure the chain verifies and the link resolves LIVE.
    assert!(
        verify_chain(&records).is_ok(),
        "pre-erasure chain must verify",
    );
    let linked = link_target(&records[link_seq as usize])
        .expect("object-link record must expose its link target");
    assert_eq!(linked, SUBJECT_OBJECT);
    assert!(
        matches!(store.resolve(&linked), Resolution::Live(_)),
        "pre-erasure the subject object is live",
    );

    // ── the erasure request: erase the subject's object (the X7 cascade leg). ──
    // Erasure operates on the OBJECT STORE only; the provenance chain is never
    // touched. The link keeps referencing the SAME content hash.
    let tombstone = store.erase(SUBJECT_OBJECT, "rtbf-request:case-42");

    // The records are byte-identical to before (erasure did not rewrite them).
    let after = verify_after_erasure(&records, &store, &linked);

    // Clause 1 — the attestation chain remains INDEPENDENTLY verifiable: the
    // canonical verify_chain still passes byte-for-byte over the immutable chain.
    assert_eq!(
        after.chain,
        Ok(()),
        "post-erasure: the canonical hash-chain must STILL verify (independently)",
    );

    // Clause 2 — the erased object resolves to a TAMPER-EVIDENT TOMBSTONE that
    // carries the SAME content hash the surviving link points at (a deliberate
    // erasure marker, not a broken link, not a substitute).
    match &after.link_resolution {
        Resolution::Tombstone(ts) => {
            assert!(ts.is_tombstone(), "must be a well-formed tombstone marker");
            assert_eq!(
                ts.erased_object_hash, SUBJECT_OBJECT,
                "tombstone must name the ORIGINAL erased object (tamper-evident)",
            );
            assert_eq!(ts, &tombstone, "store must resolve to the minted tombstone");
        }
        other => panic!("erased link must resolve to a tombstone, got {other:?}"),
    }
    assert!(
        after.is_independently_verifiable_over_tombstone(&linked),
        "item ① composite predicate must hold post-erasure",
    );

    // Clause 3 — NEVER silently re-linked: the link STILL points at the original
    // subject hash, not at any substitute.
    assert_eq!(
        link_target(&records[link_seq as usize]).as_deref(),
        Some(SUBJECT_OBJECT),
        "the provenance link must NOT be re-pointed by erasure",
    );
    assert!(
        !matches!(store.resolve(SUBSTITUTE_OBJECT), Resolution::Tombstone(_)),
        "the substitute object must not be the resolution target",
    );
}

#[test]
fn item_1_silent_relink_is_caught_fail_closed() {
    // ADVERSARIAL: a silent re-link — instead of leaving a tombstone, an actor
    // re-points the provenance record at a SUBSTITUTE object so the link "still
    // resolves". This rewrites the record's payload; the canonical chain must
    // catch it fail-closed. If the oracle did NOT catch this it would be gamed.
    let (mut records, _store, link_seq) = seed_provenance_and_store();

    let idx = link_seq as usize;
    // Tamper: swap the payload to point at the substitute, leaving this_hash
    // stale (the silent-re-link attack — payload changed, hash not re-sealed,
    // because re-sealing would also break prev_hash of the NEXT record).
    records[idx].payload = object_link_payload(SUBSTITUTE_OBJECT);

    // The dedicated detector flags the re-link and reads the new target.
    assert_eq!(
        detect_silent_relink(&records[idx]).as_deref(),
        Some(SUBSTITUTE_OBJECT),
        "silent re-link must be detected and its substitute target surfaced",
    );

    // And the canonical chain verifier fails CLOSED at exactly that seq.
    match verify_chain(&records) {
        Err(TamperError::ThisHashMismatch { seq, .. }) => {
            assert_eq!(seq, link_seq, "tamper detected at the re-linked record");
        }
        other => panic!("silent re-link must fail-closed via verify_chain, got {other:?}"),
    }
}

#[test]
fn item_1_relink_then_reseal_breaks_downstream_chain() {
    // ADVERSARIAL (stronger): the actor re-links AND re-seals this_hash to hide
    // the ThisHashMismatch. The hash-chain still catches it — re-sealing this
    // record changes its this_hash, so the NEXT record's prev_hash no longer
    // matches: the chain breaks downstream. There is NO silent re-link that
    // survives verify_chain.
    let (mut records, _store, link_seq) = seed_provenance_and_store();
    let idx = link_seq as usize;

    records[idx].payload = object_link_payload(SUBSTITUTE_OBJECT);
    // Re-seal this record's hash to defeat the ThisHashMismatch check.
    records[idx].this_hash = compute_this_hash(
        &records[idx].prev_hash,
        &records[idx].kind,
        &records[idx].principal_chain,
        &records[idx].payload,
        records[idx].seq,
    );

    // Now THIS record self-verifies, but the downstream record's prev_hash is
    // stale → PrevHashMismatch. Re-link is still impossible to hide.
    match verify_chain(&records) {
        Err(TamperError::PrevHashMismatch { seq, .. }) => {
            assert_eq!(
                seq,
                link_seq + 1,
                "re-seal propagates a break to the next record",
            );
        }
        other => panic!("re-sealed re-link must still break the chain, got {other:?}"),
    }
}

#[test]
fn item_1_erasure_never_yields_an_orphan() {
    // An erased object must resolve to a tombstone, NEVER to Missing (a void).
    // A void would be an orphaned/broken link — a contract FAIL distinct from a
    // tamper-evident tombstone.
    let (records, mut store, link_seq) = seed_provenance_and_store();
    let linked = link_target(&records[link_seq as usize]).unwrap();
    store.erase(&linked, "rtbf-request:case-42");

    assert!(
        !matches!(store.resolve(&linked), Resolution::Missing),
        "an erased object must leave a tombstone, never an orphan/void",
    );
}

// ── ② the mirror-side erasure obligation (data already replicated to GitHub) ──
//     is discharged or explicitly surfaced as residual risk — and that
//     disclosure is part of the export/exit proof

#[test]
fn item_2_residual_risk_disclosure_is_part_of_export_exit_proof() {
    // The honest case: the mirror copy cannot be guaranteed erased (forks /
    // caches / backups beyond hugit's physical control). The obligation is
    // surfaced as RESIDUAL RISK, and that disclosure is a stated element of the
    // export/exit proof (validated against the frozen ExportSchema).
    let obligation = MirrorObligation {
        object_hash: SUBJECT_OBJECT.to_string(),
        mirror_target: "github.com/acme/repo".to_string(),
        outcome: MirrorObligationOutcome::ResidualRisk {
            disclosure: "Mirror copy may persist in third-party forks/caches \
                 beyond hugit's physical control; deletion of the canonical \
                 GitHub copy was requested but full erasure cannot be \
                 guaranteed."
                .to_string(),
        },
    };

    let proof = ExitProof {
        schema: export_schema_with_obligation_class(),
        obligations: vec![obligation.clone()],
    };

    // The disclosure exists and is non-empty.
    assert!(!obligation.is_discharged());
    assert!(
        obligation
            .residual_disclosure()
            .is_some_and(|d| !d.is_empty())
    );

    // The exit proof validates: the disclosure travels INSIDE the export proof,
    // structurally tied via the obligation object-class in the ExportSchema.
    assert_eq!(proof.validate(), Ok(()), "exit proof must validate");
    assert!(proof.discloses_or_discharges_all());
    assert!(
        proof
            .schema
            .object_classes
            .iter()
            .any(|c| c == MIRROR_OBLIGATION_CLASS),
        "the disclosure must be a stated element of the export/exit proof",
    );

    // Round-trips through serde (the export is a machine-validatable artifact).
    let json = serde_json::to_string(&proof).expect("exit proof serializes");
    let back: ExitProof = serde_json::from_str(&json).expect("exit proof deserializes");
    assert_eq!(back, proof);
    assert!(
        json.contains("beyond hugit's physical control"),
        "the residual-risk disclosure text must be present in the exported bytes",
    );
}

#[test]
fn item_2_discharged_obligation_validates() {
    // The other lawful branch: mirror-side erasure was executed AND verified.
    let proof = ExitProof {
        schema: export_schema_with_obligation_class(),
        obligations: vec![MirrorObligation {
            object_hash: SUBJECT_OBJECT.to_string(),
            mirror_target: "github.com/acme/repo".to_string(),
            outcome: MirrorObligationOutcome::Discharged {
                verification: "deleted-ref confirmed 404 at 2026-06-07T10:00:00Z".to_string(),
            },
        }],
    };
    assert_eq!(proof.validate(), Ok(()));
    assert!(proof.obligations[0].is_discharged());
    assert!(proof.obligations[0].residual_disclosure().is_none());
}

#[test]
fn item_2_omitted_disclosure_from_export_schema_is_rejected() {
    // ADVERSARIAL: an obligation exists (residual risk) but the export schema
    // does NOT advertise the obligation object-class — i.e. the disclosure is
    // not actually a stated element of the export proof. This must be REJECTED;
    // otherwise the oracle would be gamed (an export that hides the residual
    // risk would pass).
    let proof = ExitProof {
        schema: export_schema_without_obligation_class(),
        obligations: vec![MirrorObligation {
            object_hash: SUBJECT_OBJECT.to_string(),
            mirror_target: "github.com/acme/repo".to_string(),
            outcome: MirrorObligationOutcome::ResidualRisk {
                disclosure: "residual risk present".to_string(),
            },
        }],
    };
    assert_eq!(
        proof.validate(),
        Err(ExitProofError::DisclosureNotInExportSchema),
        "an obligation whose disclosure is not in the export proof must be rejected",
    );
    assert!(!proof.discloses_or_discharges_all());
}

#[test]
fn item_2_empty_residual_disclosure_is_rejected() {
    // ADVERSARIAL: an "omission masquerading as a disclosure" — a residual-risk
    // outcome with empty disclosure text. The honest disclosure IS the
    // deliverable; an empty one is not a disclosure and must fail closed.
    let proof = ExitProof {
        schema: export_schema_with_obligation_class(),
        obligations: vec![MirrorObligation {
            object_hash: SUBJECT_OBJECT.to_string(),
            mirror_target: "github.com/acme/repo".to_string(),
            outcome: MirrorObligationOutcome::ResidualRisk {
                disclosure: "   ".to_string(),
            },
        }],
    };
    assert_eq!(
        proof.validate(),
        Err(ExitProofError::EmptyResidualDisclosure {
            object_hash: SUBJECT_OBJECT.to_string(),
        }),
        "an empty residual disclosure must be rejected fail-closed",
    );
}

// ── composition: ① ∧ ② together (the X12 intersection) ───────────────────────

#[test]
fn composition_erasure_leaves_verifiable_tombstone_and_discloses_mirror_residual() {
    // The full three-way composition in one flow: erase the subject object →
    // (①) the provenance chain still verifies over a tamper-evident tombstone,
    // AND (②) the mirror-side residual risk for that SAME object is disclosed in
    // the export/exit proof.
    let (records, mut store, link_seq) = seed_provenance_and_store();
    let linked = link_target(&records[link_seq as usize]).unwrap();

    store.erase(&linked, "rtbf-request:case-42");
    let after = verify_after_erasure(&records, &store, &linked);
    assert!(
        after.is_independently_verifiable_over_tombstone(&linked),
        "①: chain verifiable over tamper-evident tombstone post-erasure",
    );

    let proof = ExitProof {
        schema: export_schema_with_obligation_class(),
        obligations: vec![MirrorObligation {
            object_hash: linked.clone(),
            mirror_target: "github.com/acme/repo".to_string(),
            outcome: MirrorObligationOutcome::ResidualRisk {
                disclosure: "GitHub mirror copy of the erased object may persist \
                     in forks/caches; residual risk disclosed."
                    .to_string(),
            },
        }],
    };
    assert_eq!(
        proof.validate(),
        Ok(()),
        "②: residual risk disclosed in exit proof"
    );
    // The exit proof's obligation names the SAME object the tombstone marks.
    assert_eq!(proof.obligations[0].object_hash, linked);
}
