//! The self-release artifact and its signed attestation (WP-X8 ①).
//!
//! A hugit release — the App, the CLI, or a runner image — is the **attested
//! object**. We map it onto the frozen
//! [`AttestationChain`](hugit_contracts::AttestationChain): the artifact's
//! content digest is the `tree` link (the content-addressed snapshot of what is
//! released), the release identity is carried in `def`/`runner`/`model`/
//! `principal`, and `sig` is the **release signature** — a real ed25519
//! signature over the contract-frozen canonical preimage.
//!
//! The signature is produced over exactly
//! [`hugit_refstore::attestation_sig_preimage`] — the single canonical
//! realisation of the `AttestationChain::sig` doc. It is imported and called,
//! never re-transcribed, so the X8 oracle signs/verifies the SAME bytes the rest
//! of hugit does.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signer, SigningKey};
use hugit_contracts::AttestationChain;
use serde::{Deserialize, Serialize};

/// Which hugit release artifact is being attested. The charter names all three
/// explicitly: every App/CLI/runner-image release must be signed + published.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReleaseKind {
    /// The hugit App (the forge service binary).
    App,
    /// The hugit CLI.
    Cli,
    /// A hugit runner image.
    RunnerImage,
}

impl ReleaseKind {
    /// A stable string tag used inside the attestation's `runner`-link identity
    /// so distinct kinds never collide on the same preimage.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            ReleaseKind::App => "hugit-app",
            ReleaseKind::Cli => "hugit-cli",
            ReleaseKind::RunnerImage => "hugit-runner-image",
        }
    }
}

/// A releasable hugit artifact: a kind, a name, a version, and the content
/// digest of the released bytes (the attested object). Two artifacts with the
/// same `(kind, name, version, digest)` are the same release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseArtifact {
    kind: ReleaseKind,
    name: String,
    version: String,
    /// 64-char hex content digest of the released artifact bytes.
    digest: String,
}

impl ReleaseArtifact {
    /// Build a release artifact descriptor.
    pub fn new(
        kind: ReleaseKind,
        name: impl Into<String>,
        version: impl Into<String>,
        digest: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            name: name.into(),
            version: version.into(),
            digest: digest.into(),
        }
    }

    /// The artifact kind.
    #[must_use]
    pub fn kind(&self) -> ReleaseKind {
        self.kind
    }

    /// The content digest of the released bytes (the attested object).
    #[must_use]
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// The release name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The release version.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// The unsigned [`AttestationChain`] form of this release: the artifact
    /// digest is the content-addressed `tree` link; identity is carried in the
    /// remaining links; `sig` is left empty (it is filled by [`sign_release`]).
    ///
    /// This is deterministic — the same artifact always maps to the same links,
    /// so the signed preimage is stable and the published entry is reproducible.
    #[must_use]
    pub fn unsigned_chain(&self) -> AttestationChain {
        AttestationChain {
            // The released artifact bytes, content-addressed.
            tree: self.digest.clone(),
            // The release definition: kind + name + version, the release identity.
            def: format!("{}:{}:{}", self.kind.tag(), self.name, self.version),
            // The producer of the release (the kind tag — App/CLI/runner image).
            runner: self.kind.tag().to_string(),
            // No AI model participates in a release build; the link is the
            // explicit "no model" sentinel rather than empty (every link must
            // resolve for the attestation to be well-formed).
            model: "release/no-model".to_string(),
            // The release principal: hugit attesting its own build.
            principal: vec!["platform:hugit/self-release".to_string()],
            sig: String::new(),
        }
    }
}

/// A release artifact paired with its signed attestation (the release
/// signature). Construction is via [`sign_release`]; the `sig` is always present.
///
/// `Eq` is intentionally NOT derived: the embedded frozen
/// [`AttestationChain`](hugit_contracts::AttestationChain) is `PartialEq` only,
/// so this type mirrors that contract surface rather than widening it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignedRelease {
    artifact: ReleaseArtifact,
    attestation: AttestationChain,
}

impl SignedRelease {
    /// The released artifact.
    #[must_use]
    pub fn artifact(&self) -> &ReleaseArtifact {
        &self.artifact
    }

    /// The signed attestation (the release signature lives in `attestation.sig`).
    #[must_use]
    pub fn attestation(&self) -> &AttestationChain {
        &self.attestation
    }
}

/// Build the ed25519 message for an attestation chain using the **single
/// contract-frozen canonical preimage** — imported from
/// [`hugit_refstore::attestation_sig_preimage`], never re-transcribed.
#[must_use]
pub fn release_preimage(chain: &AttestationChain) -> Vec<u8> {
    hugit_refstore::attestation_sig_preimage(
        &chain.tree,
        &chain.def,
        &chain.runner,
        &chain.model,
        &chain.principal,
    )
}

/// Sign a release with `signing_key`, producing a [`SignedRelease`] whose
/// `attestation.sig` is the base64-encoded ed25519 signature over the frozen
/// canonical preimage of the release's attestation links (WP-X8 ①).
///
/// This is the ONLY function that needs the private release key; everything the
/// transparency log and the boot self-verify do uses the public key alone.
#[must_use]
pub fn sign_release(signing_key: &SigningKey, artifact: &ReleaseArtifact) -> SignedRelease {
    let mut chain = artifact.unsigned_chain();
    let sig = signing_key.sign(&release_preimage(&chain));
    chain.sig = B64.encode(sig.to_bytes());
    SignedRelease {
        artifact: artifact.clone(),
        attestation: chain,
    }
}
