//! The verifiable transparency-log surface (WP-X8 ①).
//!
//! A release is published to an **append-only transparency log** whose entries
//! are **independently checkable from the log alone**: holding only the entry
//! and the release public key, a third party re-derives the canonical preimage
//! and verifies the ed25519 signature. No producer secret, no side channel.
//!
//! # The live-log seam (P2)
//! [`TransparencyLog`] is the trait boundary. [`InMemoryTransparencyLog`] is the
//! hermetic, always-runnable impl. A LIVE public transparency log (e.g. Rekor)
//! is the documented P2 seam behind the SAME trait — a Rekor-backed impl makes
//! publication globally verifiable without changing the boot self-verify path.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{SIGNATURE_LENGTH, Signature, Verifier, VerifyingKey};
use hugit_contracts::AttestationChain;

use crate::x8::release::{ReleaseArtifact, SignedRelease, release_preimage};

/// A monotonic, stable index identifying an entry in the transparency log — the
/// inclusion handle a publisher gets back and a verifier uses to fetch the entry.
pub type LogIndex = u64;

/// Why a transparency-log entry failed verification. Every variant is a
/// fail-closed rejection; verification succeeds only when the recorded
/// attestation's ed25519 signature verifies against the supplied public key over
/// the frozen canonical preimage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryVerifyError {
    /// The `sig` field is empty — an UNSIGNED entry.
    Unsigned,
    /// The `sig` field is not valid base64 / not a 64-byte ed25519 signature.
    MalformedSignature,
    /// The signature did not verify against the public key over the canonical
    /// preimage — the entry was TAMPERED with, forged, or signed by a different
    /// key.
    SignatureMismatch,
}

impl std::fmt::Display for EntryVerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntryVerifyError::Unsigned => write!(f, "unsigned transparency-log entry (empty sig)"),
            EntryVerifyError::MalformedSignature => write!(f, "malformed ed25519 signature"),
            EntryVerifyError::SignatureMismatch => {
                write!(f, "signature mismatch (tampered or forged entry)")
            }
        }
    }
}

impl std::error::Error for EntryVerifyError {}

/// One recorded transparency-log entry: the published release attestation plus
/// the attested artifact's content digest. The entry is self-contained — a
/// verifier needs ONLY the entry and the release public key.
///
/// `Eq` is intentionally NOT derived: the embedded frozen
/// [`AttestationChain`](hugit_contracts::AttestationChain) is `PartialEq` only.
#[derive(Debug, Clone, PartialEq)]
pub struct LogEntry {
    /// The attested artifact's content digest (the "what was released").
    artifact_digest: String,
    /// The signed attestation (the release signature lives in `sig`).
    attestation: AttestationChain,
}

impl LogEntry {
    /// The attested artifact content digest.
    #[must_use]
    pub fn artifact_digest(&self) -> &str {
        &self.artifact_digest
    }

    /// The recorded attestation.
    #[must_use]
    pub fn attestation(&self) -> &AttestationChain {
        &self.attestation
    }

    /// Verify this entry from the log alone: re-derive the canonical preimage
    /// from the recorded attestation links and verify the ed25519 signature
    /// against `verifying_key`. Fails CLOSED on unsigned / malformed / mismatch.
    ///
    /// # Errors
    /// Returns [`EntryVerifyError`] when the entry is not a genuine, untampered
    /// signature by the holder of `verifying_key`.
    pub fn verify(&self, verifying_key: &VerifyingKey) -> Result<(), EntryVerifyError> {
        if self.attestation.sig.is_empty() {
            return Err(EntryVerifyError::Unsigned);
        }
        let sig_bytes = B64
            .decode(self.attestation.sig.as_bytes())
            .map_err(|_| EntryVerifyError::MalformedSignature)?;
        let arr: [u8; SIGNATURE_LENGTH] = sig_bytes
            .try_into()
            .map_err(|_| EntryVerifyError::MalformedSignature)?;
        let signature = Signature::from_bytes(&arr);
        // The preimage is re-derived from the RECORDED links via the frozen
        // canonical function, so any post-publication tamper of those links (or
        // of the artifact digest, which is the `tree` link) breaks the match.
        let preimage = release_preimage(&self.attestation);
        verifying_key
            .verify(&preimage, &signature)
            .map_err(|_| EntryVerifyError::SignatureMismatch)
    }

    /// Tamper the recorded artifact digest (and the bound `tree` link) — TEST
    /// ONLY. Used by the oracle to prove the log is tamper-evident: after this
    /// mutation [`LogEntry::verify`] must fail.
    #[doc(hidden)]
    pub fn tamper_digest_for_test(&mut self, new_digest: impl Into<String>) {
        let d = new_digest.into();
        self.artifact_digest = d.clone();
        self.attestation.tree = d;
    }
}

/// The append-only transparency-log surface. Publishing returns a stable
/// inclusion index; entries are immutable once recorded and retrievable by
/// index. A LIVE public log (e.g. Rekor) is the P2 impl behind this trait.
pub trait TransparencyLog {
    /// Publish a signed release; returns its stable, monotonic log index.
    ///
    /// # Errors
    /// Implementations may fail on transport / inclusion-proof errors (the live
    /// P2 impl); the in-memory impl is infallible but keeps the `Result` shape
    /// so the boot path is identical across impls.
    fn publish(&mut self, release: &SignedRelease) -> Result<LogIndex, String>;

    /// Fetch a recorded entry by index, if present.
    fn get(&self, index: LogIndex) -> Option<LogEntry>;

    /// Number of entries recorded.
    fn len(&self) -> usize;

    /// `true` iff no entries are recorded.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Find the recorded entry for an artifact by its content digest, if any.
    /// The boot self-verify path uses this to locate the entry for exactly the
    /// build that is running.
    fn find_by_digest(&self, digest: &str) -> Option<LogEntry>;
}

/// In-process append-only transparency log — the hermetic impl exercised by the
/// X8 acceptance oracle. Entries are stored by insertion order; the index is the
/// position. There is no mutation or removal API (append-only).
#[derive(Debug, Default, Clone)]
pub struct InMemoryTransparencyLog {
    entries: Vec<LogEntry>,
}

impl InMemoryTransparencyLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl TransparencyLog for InMemoryTransparencyLog {
    fn publish(&mut self, release: &SignedRelease) -> Result<LogIndex, String> {
        let index = self.entries.len() as LogIndex;
        self.entries.push(LogEntry {
            artifact_digest: release.artifact().digest().to_string(),
            attestation: release.attestation().clone(),
        });
        Ok(index)
    }

    fn get(&self, index: LogIndex) -> Option<LogEntry> {
        self.entries.get(index as usize).cloned()
    }

    fn len(&self) -> usize {
        self.entries.len()
    }

    fn find_by_digest(&self, digest: &str) -> Option<LogEntry> {
        self.entries
            .iter()
            .find(|e| e.artifact_digest == digest)
            .cloned()
    }
}

/// Convenience: locate AND verify the entry for `artifact` against
/// `verifying_key` in one step. Returns the entry on success, or a fail-closed
/// reason describing why no verifying entry exists for what is running.
///
/// # Errors
/// `Err(reason)` when there is no entry for the artifact's digest, or the entry
/// exists but does not verify (unsigned / tampered / foreign key).
pub fn locate_and_verify<L: TransparencyLog + ?Sized>(
    log: &L,
    verifying_key: &VerifyingKey,
    artifact: &ReleaseArtifact,
) -> Result<LogEntry, String> {
    let Some(entry) = log.find_by_digest(artifact.digest()) else {
        return Err(format!(
            "no transparency-log entry for running artifact digest {} \
             (unsigned/unpublished/tampered self-build)",
            artifact.digest()
        ));
    };
    entry
        .verify(verifying_key)
        .map_err(|e| format!("published entry does not verify: {e}"))?;
    Ok(entry)
}
