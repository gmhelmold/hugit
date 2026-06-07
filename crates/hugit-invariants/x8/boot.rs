//! Boot-time self-provenance verification (WP-X8 ②③).
//!
//! The running App verifies its OWN provenance at boot BEFORE serving: it
//! locates its published attestation in the transparency log for exactly the
//! build that is running (matched by content digest) and verifies the ed25519
//! signature against the trusted release public key. The check GATES serving —
//! [`boot_self_verify`] returns [`BootOutcome::Serving`] ONLY on success.
//!
//! An unsigned (unpublished / empty-sig), tampered (digest-mismatched), or
//! foreign-key-signed self-build makes boot **fail CLOSED**: the App does not
//! serve, and an audit event is emitted. This is the self-turned form of X4③.

use ed25519_dalek::VerifyingKey;

use crate::x8::release::ReleaseArtifact;
use crate::x8::tlog::{TransparencyLog, locate_and_verify};

/// One fail-closed audit event emitted by the boot self-verify gate. Mirrors the
/// X2 promotion-gate audit shape so the invariant family records refusals
/// uniformly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    /// What was attempted (always `"boot-self-verify"` here).
    pub action: &'static str,
    /// Whether serving was admitted.
    pub admitted: bool,
    /// Machine-readable reason on refusal (empty when admitted).
    pub reason: String,
}

/// The outcome of the boot self-verify gate. The discriminant records whether
/// the App may serve: [`BootOutcome::Serving`] is reachable ONLY after the
/// running build's own provenance verified against the published, trusted
/// attestation. A [`BootOutcome::FailedClosed`] proves the App refused to serve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootOutcome {
    /// Self-provenance verified — the App proceeds to serve. Always audited.
    Serving,
    /// Self-provenance did NOT verify — the App fails CLOSED and does not serve.
    /// Carries the audit event with the machine-readable reason.
    FailedClosed {
        /// The fail-closed audit event.
        audit: AuditEvent,
    },
}

impl BootOutcome {
    /// `true` iff the App proceeds to serve.
    #[must_use]
    pub fn is_serving(&self) -> bool {
        matches!(self, BootOutcome::Serving)
    }
}

/// Verify the running App's OWN provenance at boot against the transparency log,
/// holding ONLY the trusted release public key, BEFORE serving (WP-X8 ②③).
///
/// The gate locates the published entry for exactly the build that is running
/// (matched by `running.digest()`) and verifies its ed25519 signature against
/// `release_pubkey`. It returns:
/// - [`BootOutcome::Serving`] only when a verifying entry exists for the running
///   build (item ②: the check gates serving and is not skippable);
/// - [`BootOutcome::FailedClosed`] — with an audit event — when the build is
///   unsigned/unpublished, tampered (no matching entry), or signed by a foreign
///   key (item ③: fail CLOSED).
///
/// `release_pubkey` is the trust anchor: the genuine hugit release public key,
/// distributed out of band. The boot path never holds a private key.
#[must_use]
pub fn boot_self_verify<L: TransparencyLog + ?Sized>(
    log: &L,
    release_pubkey: &VerifyingKey,
    running: &ReleaseArtifact,
) -> BootOutcome {
    match locate_and_verify(log, release_pubkey, running) {
        Ok(_entry) => BootOutcome::Serving,
        Err(reason) => BootOutcome::FailedClosed {
            audit: AuditEvent {
                action: "boot-self-verify",
                admitted: false,
                reason,
            },
        },
    }
}
