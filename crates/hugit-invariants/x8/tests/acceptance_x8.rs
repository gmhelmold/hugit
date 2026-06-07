//! WP-X8 acceptance oracle — self-release attestation. Contract:
//! `docs/plan/wp-contracts/WP-X8.md`. Items encoded VERBATIM, one `#[test]`
//! `item_<n>_<slug>` each, RED on a tree without the x8 surface.
//!
//! Owned items (verbatim from the contract):
//!   ① every hugit App/CLI/runner-image release signed + published to a
//!      verifiable transparency log
//!   ② the running App verifies its own provenance at boot
//!   ③ unsigned/tampered self-build fails CLOSED
//!
//! Hermetic by construction: a release manifest is signed with a real ed25519
//! key over the contract-frozen canonical preimage
//! (`hugit_refstore::attestation_sig_preimage` — never re-transcribed), recorded
//! to an in-process append-only transparency-log surface, and the boot
//! self-verify checks its OWN provenance against the published entry and fails
//! CLOSED on unsigned/tampered. The LIVE public transparency log (e.g. Rekor)
//! is the documented P2 seam behind the same [`TransparencyLog`] trait — the
//! in-memory log is one impl, so the oracle drives the real trait surface, not a
//! parallel mock.
//!
//! Oracle goes RED if a tampered/unsigned build verifies: items ② and ③ assert
//! the boot gate REFUSES TO SERVE on a missing/unsigned/tampered self-build and
//! emits an audit event. There is no skip lane — every assertion runs in the
//! bare `cargo test` gate (no network, no env).

use ed25519_dalek::SigningKey;
use hugit_invariants::x8::boot::{BootOutcome, boot_self_verify};
use hugit_invariants::x8::release::{ReleaseArtifact, ReleaseKind, sign_release};
use hugit_invariants::x8::tlog::{InMemoryTransparencyLog, TransparencyLog};

/// Deterministic signing key for the hermetic oracle (32 fixed bytes). The
/// public verifying key derived from it is what the boot path checks against —
/// no secret material is printed or persisted.
fn release_key() -> SigningKey {
    SigningKey::from_bytes(&[7u8; 32])
}

/// A distinct, WRONG key — used to forge a signature that must NOT verify
/// against the published release public key (tamper-by-foreign-key).
fn attacker_key() -> SigningKey {
    SigningKey::from_bytes(&[9u8; 32])
}

/// The three releasable hugit artifact kinds the charter names explicitly.
fn all_release_kinds() -> [ReleaseKind; 3] {
    [ReleaseKind::App, ReleaseKind::Cli, ReleaseKind::RunnerImage]
}

/// A sample release artifact for a given kind, with a content digest.
fn sample_release(kind: ReleaseKind) -> ReleaseArtifact {
    ReleaseArtifact::new(
        kind,
        "hugit",
        "1.0.0",
        // 64-hex content digest of the artifact bytes (the attested object).
        "a3f1c0de00000000000000000000000000000000000000000000000000000abc",
    )
}

// ── item ① — every release signed + published to a verifiable transparency log ─

/// Item ①: every hugit App/CLI/runner-image release is signed AND published to
/// a transparency log whose entry is **independently verifiable from the log
/// alone** — i.e. holding only the log entry + the release public key, a third
/// party confirms the signature over the artifact. This holds for ALL THREE
/// release kinds (App, CLI, runner image).
#[test]
fn item_1_every_release_signed_and_published_to_verifiable_log() {
    let key = release_key();
    let pubkey = ed25519_dalek::VerifyingKey::from(&key);
    let mut log = InMemoryTransparencyLog::new();

    for kind in all_release_kinds() {
        let release = sample_release(kind);

        // SIGNED: a real ed25519 signature over the canonical release preimage.
        let signed = sign_release(&key, &release);
        assert!(
            !signed.attestation().sig.is_empty(),
            "{kind:?} release must be SIGNED (non-empty sig)"
        );

        // PUBLISHED: the signed release is appended to the transparency log,
        // which returns a stable, monotonic log index (the inclusion handle).
        let index = log.publish(&signed).expect("publish to transparency log");

        // VERIFIABLE FROM THE LOG ALONE: fetch the entry back by index and
        // verify it holding ONLY the public key — no producer secret, no
        // side-channel. This is the "independently checkable" property.
        let entry = log.get(index).expect("entry retrievable from the log");
        assert!(
            entry.verify(&pubkey).is_ok(),
            "{kind:?} log entry must be independently verifiable from the log alone"
        );
    }

    // The log is append-only and now holds exactly the three published releases.
    assert_eq!(
        log.len(),
        3,
        "exactly the three published releases recorded, append-only"
    );
}

/// Item ① (transparency-log integrity): the log is append-only and
/// tamper-evident — a recorded entry cannot be silently mutated and still
/// verify. A verifier holding only the public key catches a post-publication
/// tamper of the log content.
#[test]
fn item_1_published_entry_is_tamper_evident() {
    let key = release_key();
    let pubkey = ed25519_dalek::VerifyingKey::from(&key);
    let mut log = InMemoryTransparencyLog::new();

    let signed = sign_release(&key, &sample_release(ReleaseKind::App));
    let index = log.publish(&signed).expect("publish");

    // Take the entry, tamper its attested artifact digest, and confirm the
    // signature over the canonical preimage no longer verifies.
    let mut entry = log.get(index).expect("entry");
    entry
        .tamper_digest_for_test("deadbeef00000000000000000000000000000000000000000000000000000000");
    assert!(
        entry.verify(&pubkey).is_err(),
        "a tampered published entry must FAIL verification — log is tamper-evident"
    );
}

// ── item ② — the running App verifies its own provenance at boot ───────────────

/// Item ②: the running App verifies its OWN provenance at boot against its
/// published attestation, and the verification GATES serving — a boot with a
/// verifiable published attestation proceeds to serve; the check is not
/// skippable. We model boot as `boot_self_verify`, which returns
/// [`BootOutcome::Serving`] ONLY after a successful self-provenance check.
#[test]
fn item_2_running_app_verifies_own_provenance_at_boot() {
    let key = release_key();
    let pubkey = ed25519_dalek::VerifyingKey::from(&key);
    let mut log = InMemoryTransparencyLog::new();

    // The running App is this exact self-build; its release was signed +
    // published at release time.
    let self_build = sign_release(&key, &sample_release(ReleaseKind::App));
    log.publish(&self_build).expect("publish self-build");

    // At boot the App verifies its own provenance against the published log,
    // holding only the public key, BEFORE serving.
    let outcome = boot_self_verify(&log, &pubkey, self_build.artifact());
    assert!(
        matches!(outcome, BootOutcome::Serving),
        "a self-build with a verifiable published attestation must boot to Serving"
    );
    assert!(
        outcome.is_serving(),
        "the boot gate proceeds to serve only after self-provenance verifies"
    );
}

// ── item ③ — unsigned/tampered self-build fails CLOSED ─────────────────────────

/// Item ③ (unsigned): an UNSIGNED self-build (no signature published / empty
/// sig) fails CLOSED at boot — the App does NOT serve, and an audit event is
/// emitted. There is no entry to verify against, so the gate refuses.
#[test]
fn item_3_unsigned_self_build_fails_closed() {
    let key = release_key();
    let pubkey = ed25519_dalek::VerifyingKey::from(&key);
    let log = InMemoryTransparencyLog::new(); // EMPTY — nothing published.

    // The running App claims to be this self-build, but it was never signed +
    // published to the log (unsigned/unattested release).
    let unsigned_self_build = sample_release(ReleaseKind::App);

    let outcome = boot_self_verify(&log, &pubkey, &unsigned_self_build);
    assert!(
        !outcome.is_serving(),
        "an unsigned/unpublished self-build must FAIL CLOSED — must not serve"
    );
    match outcome {
        BootOutcome::FailedClosed { audit, .. } => {
            assert!(
                !audit.admitted,
                "fail-closed audit event must record admitted = false"
            );
            assert!(
                !audit.reason.is_empty(),
                "fail-closed audit event must carry a machine-readable reason"
            );
        }
        BootOutcome::Serving => panic!("unsigned self-build must NOT reach Serving"),
    }
}

/// Item ③ (tampered): a TAMPERED self-build fails CLOSED at boot. The log holds
/// a genuine signed entry for digest D, but the running App's artifact has a
/// DIFFERENT digest D' (tampered binary) — the self-provenance check finds no
/// matching, verifying entry for what is actually running, and refuses to serve.
#[test]
fn item_3_tampered_self_build_fails_closed() {
    let key = release_key();
    let pubkey = ed25519_dalek::VerifyingKey::from(&key);
    let mut log = InMemoryTransparencyLog::new();

    // A genuine release for digest D is signed + published.
    let genuine = sign_release(&key, &sample_release(ReleaseKind::App));
    log.publish(&genuine).expect("publish genuine");

    // The running App is a TAMPERED build: same name/version, different content
    // digest (D' != D). No verifying entry exists for what is actually running.
    let tampered_running = ReleaseArtifact::new(
        ReleaseKind::App,
        "hugit",
        "1.0.0",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
    );

    let outcome = boot_self_verify(&log, &pubkey, &tampered_running);
    assert!(
        !outcome.is_serving(),
        "a tampered self-build must FAIL CLOSED — must not serve"
    );
    assert!(
        matches!(outcome, BootOutcome::FailedClosed { .. }),
        "a tampered self-build must yield a FailedClosed audited outcome"
    );
}

/// Item ③ (forged signature): a self-build whose published entry was signed by
/// a FOREIGN key (not the release key the boot path trusts) fails CLOSED — the
/// signature does not verify against the trusted release public key. This is the
/// gamed-oracle guard: a forged-but-present entry must NOT let the App serve.
#[test]
fn item_3_foreign_key_signature_fails_closed() {
    let release_pubkey = ed25519_dalek::VerifyingKey::from(&release_key());
    let mut log = InMemoryTransparencyLog::new();

    // Attacker signs the SAME artifact with their OWN key and publishes it.
    let artifact = sample_release(ReleaseKind::App);
    let forged = sign_release(&attacker_key(), &artifact);
    log.publish(&forged).expect("publish forged");

    // Boot trusts the genuine RELEASE public key; the forged entry's signature
    // does not verify against it, so the gate fails CLOSED.
    let outcome = boot_self_verify(&log, &release_pubkey, &artifact);
    assert!(
        !outcome.is_serving(),
        "an entry signed by a FOREIGN key must FAIL CLOSED against the trusted release key"
    );
    assert!(
        matches!(outcome, BootOutcome::FailedClosed { .. }),
        "foreign-key signature must yield a FailedClosed audited outcome"
    );
}

/// Cross-check the canonical preimage IS the single contract-frozen source: the
/// x8 release signature is produced over exactly
/// `hugit_refstore::attestation_sig_preimage(...)` for the release's
/// attestation links — never a re-transcribed copy. A signature minted directly
/// over that preimage must verify identically to one produced by `sign_release`.
#[test]
fn release_signature_uses_the_frozen_canonical_preimage() {
    use ed25519_dalek::Signer;

    let key = release_key();
    let pubkey = ed25519_dalek::VerifyingKey::from(&key);
    let release = sample_release(ReleaseKind::Cli);

    let signed = sign_release(&key, &release);
    let chain = signed.attestation();

    // Re-derive the preimage straight from the refstore canonical function and
    // sign it independently; it must match the sig sign_release produced.
    let preimage = hugit_refstore::attestation_sig_preimage(
        &chain.tree,
        &chain.def,
        &chain.runner,
        &chain.model,
        &chain.principal,
    );
    let independent = key.sign(&preimage);

    use base64::Engine as _;
    let independent_b64 = base64::engine::general_purpose::STANDARD.encode(independent.to_bytes());
    assert_eq!(
        chain.sig, independent_b64,
        "release sig MUST be ed25519 over the frozen canonical preimage \
         (hugit_refstore::attestation_sig_preimage) — no re-transcription"
    );

    // And it verifies against the public key (sanity: the chain is well-formed).
    use ed25519_dalek::Verifier;
    let sig_bytes = base64::engine::general_purpose::STANDARD
        .decode(chain.sig.as_bytes())
        .expect("sig b64");
    let arr: [u8; 64] = sig_bytes.try_into().expect("64-byte sig");
    assert!(
        pubkey
            .verify(&preimage, &ed25519_dalek::Signature::from_bytes(&arr))
            .is_ok(),
        "the frozen-preimage signature must verify against the release public key"
    );
}
