//! hugit-invariants — self-release attestation invariant (WP-X8).
//!
//! This module proves hugit attests its OWN releases — without owning any
//! production path. It consumes the frozen
//! [`AttestationChain`](hugit_contracts::AttestationChain) form and the
//! contract-frozen canonical signature preimage
//! ([`hugit_refstore::attestation_sig_preimage`]) read-only; it modifies
//! neither. The self-release attestation is the supply-chain invariant (X4)
//! turned on hugit itself: X4 proves third-party images are pinned/verified;
//! X8 proves hugit's OWN App/CLI/runner-image releases are signed and verified.
//!
//! # The three invariants (WP-X8 owned items)
//! ① **Every release signed + published to a verifiable transparency log.**
//!    Every hugit App/CLI/runner-image release is signed with a real ed25519
//!    key over the frozen canonical preimage and published to an append-only
//!    transparency log whose entries are **independently checkable from the log
//!    alone** (holding only the log entry + the release public key). See
//!    [`release`] and [`tlog`].
//! ② **The running App verifies its own provenance at boot.** Before serving,
//!    the App checks its OWN provenance against its published attestation in the
//!    transparency log; the check GATES serving and is not skippable. See
//!    [`boot::boot_self_verify`].
//! ③ **Unsigned/tampered self-build fails CLOSED.** An unsigned (unpublished /
//!    empty-sig), tampered (digest-mismatched), or foreign-key-signed self-build
//!    makes boot fail CLOSED — the App does not serve, with an audit event.
//!    This is the self-turned form of X4③ (tampered third-party image fails
//!    closed).
//!
//! # The live-transparency-log seam (P2)
//! The append-only log surface is the [`tlog::TransparencyLog`] trait. The
//! in-process [`tlog::InMemoryTransparencyLog`] is the hermetic, always-runnable
//! impl exercised by the acceptance oracle. A LIVE public transparency log (e.g.
//! [Rekor](https://docs.sigstore.dev/logging/overview/)) is the documented P2
//! seam behind the **same trait**: a Rekor-backed `TransparencyLog` impl makes
//! the publication globally, independently verifiable without changing one line
//! of the boot self-verify path or the oracle. The seam is the trait boundary,
//! so the in-memory impl is not a parallel mock — it is one realisation of the
//! interface the live log also satisfies.
//!
//! Everything here is verification logic over the *consumed* surfaces — there is
//! no production behavior to ship from this module.

pub mod boot;
pub mod release;
pub mod tlog;
