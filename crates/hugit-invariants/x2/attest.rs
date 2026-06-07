//! X2 attestation-end-to-end verification surface.
//!
//! WP-X2 proves the full provenance attestation chain
//! ([`AttestationChain`](hugit_contracts::AttestationChain)) resolves
//! **cryptographically** end-to-end, that tampered/unsigned attestations are
//! rejected at the promotion boundary, that verification is a PUBLIC documented
//! procedure, and that a cross-tenant-shared hit attests to an anonymized
//! PLATFORM identity — never leaking the producing tenant.
//!
//! # Why asymmetric (ed25519), not a keyed-MAC
//!
//! The contract item ③ demands **public** verification: a third party must be
//! able to verify an attestation holding ONLY a public key, and that same party
//! must be UNABLE to forge a new attestation. A symmetric keyed-MAC (HMAC) fails
//! this — verifying an HMAC requires the shared secret, and anyone who can
//! verify can also forge. We therefore use a real asymmetric signature scheme,
//! [Ed25519](https://ed25519.cr.yp.to/) (audited, pure-Rust `ed25519-dalek`):
//! the **signing** (private) key produces the `sig`; the **verifying** (public)
//! key alone checks it. This is the cryptographic basis of public
//! verifiability (item ③) and of tamper-evidence (item ②).
//!
//! The frozen [`AttestationChain::sig`](hugit_contracts::AttestationChain::sig)
//! field already documents its format as a base64-encoded Ed25519 signature;
//! this module is the verification logic over that frozen surface. It modifies
//! no production path.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{
    PUBLIC_KEY_LENGTH, SIGNATURE_LENGTH, Signature, Signer, SigningKey, Verifier, VerifyingKey,
};
use hugit_contracts::{AttestationChain, RegenGate};

/// Domain-separation tag mixed into every signed payload. Binds a signature to
/// the hugit-attestation context so a signature minted for another purpose can
/// never be replayed as an attestation.
const DOMAIN_TAG: &[u8] = b"hugit/attestation/v1";

// ── canonical signing payload ─────────────────────────────────────────────────

/// Serialize the five provenance links into the canonical, deterministic byte
/// string that is signed and verified.
///
/// Format: `DOMAIN_TAG` followed, for each link, by a 4-byte big-endian length
/// prefix and the link's UTF-8 bytes. The `principal` chain is encoded as its
/// element count (4-byte BE) followed by each element length-prefixed, in order.
/// Length-prefixing makes the encoding injective: no two distinct chains can
/// produce the same payload (no field-boundary ambiguity), so a tamper in any
/// link changes the bytes that were signed and the signature stops verifying.
///
/// The `sig` field is deliberately excluded — it is the signature OVER this
/// payload, not part of it.
pub fn signing_payload(chain: &AttestationChain) -> Vec<u8> {
    fn put(buf: &mut Vec<u8>, s: &str) {
        buf.extend_from_slice(&(s.len() as u32).to_be_bytes());
        buf.extend_from_slice(s.as_bytes());
    }
    let mut buf = Vec::new();
    buf.extend_from_slice(DOMAIN_TAG);
    put(&mut buf, &chain.tree);
    put(&mut buf, &chain.def);
    put(&mut buf, &chain.runner);
    put(&mut buf, &chain.model);
    buf.extend_from_slice(&(chain.principal.len() as u32).to_be_bytes());
    for p in &chain.principal {
        put(&mut buf, p);
    }
    buf
}

// ── signing (private side) ────────────────────────────────────────────────────

/// Sign the five links of `chain` with `signing_key`, returning a new chain
/// whose `sig` is the base64-encoded Ed25519 signature over [`signing_payload`].
///
/// The input `chain.sig` is ignored and replaced. This is the ONLY function in
/// this module that needs the private signing key; everything verifier-facing
/// (resolution, tamper-check, promotion gate) uses the public key alone.
pub fn sign_chain(signing_key: &SigningKey, chain: &AttestationChain) -> AttestationChain {
    let sig = signing_key.sign(&signing_payload(chain));
    AttestationChain {
        sig: B64.encode(sig.to_bytes()),
        ..chain.clone()
    }
}

/// Export the public verifying key as base64 — the bytes a third-party verifier
/// receives. The signing key never leaves the producer.
pub fn export_public_key(signing_key: &SigningKey) -> String {
    B64.encode(signing_key.verifying_key().to_bytes())
}

// ── verification (public side — needs ONLY the public key) ────────────────────

/// Why an attestation failed to resolve or verify. Every variant is a
/// fail-closed rejection; resolution succeeds only when ALL links are present
/// AND the asymmetric signature verifies against the supplied public key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyError {
    /// A provenance link (tree/def/runner/model/principal) is unresolved
    /// (empty). The named link could not be resolved end-to-end.
    UnresolvedLink(&'static str),
    /// The supplied public-key bytes are not a valid Ed25519 verifying key.
    MalformedPublicKey,
    /// The `sig` field is empty — an UNSIGNED attestation.
    Unsigned,
    /// The `sig` field is not valid base64 / not a 64-byte Ed25519 signature.
    MalformedSignature,
    /// The signature did not verify against the public key over the canonical
    /// payload — the attestation was TAMPERED with, forged, or signed by a
    /// different key.
    SignatureMismatch,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VerifyError::UnresolvedLink(l) => write!(f, "unresolved provenance link: {l}"),
            VerifyError::MalformedPublicKey => write!(f, "malformed ed25519 public key"),
            VerifyError::Unsigned => write!(f, "unsigned attestation (empty sig)"),
            VerifyError::MalformedSignature => write!(f, "malformed ed25519 signature"),
            VerifyError::SignatureMismatch => {
                write!(f, "signature mismatch (tampered or forged attestation)")
            }
        }
    }
}

impl std::error::Error for VerifyError {}

/// Parse a base64-encoded Ed25519 public key into a [`VerifyingKey`].
///
/// This is all a third-party verifier needs to construct from the published
/// public-key bytes — no secret is involved.
pub fn parse_public_key(public_key_b64: &str) -> Result<VerifyingKey, VerifyError> {
    let bytes = B64
        .decode(public_key_b64.as_bytes())
        .map_err(|_| VerifyError::MalformedPublicKey)?;
    let arr: [u8; PUBLIC_KEY_LENGTH] = bytes
        .try_into()
        .map_err(|_| VerifyError::MalformedPublicKey)?;
    VerifyingKey::from_bytes(&arr).map_err(|_| VerifyError::MalformedPublicKey)
}

/// Assert all five provenance links are resolved (non-empty). The `principal`
/// chain must contain at least one principal.
fn require_links(chain: &AttestationChain) -> Result<(), VerifyError> {
    if chain.tree.is_empty() {
        return Err(VerifyError::UnresolvedLink("tree"));
    }
    if chain.def.is_empty() {
        return Err(VerifyError::UnresolvedLink("def"));
    }
    if chain.runner.is_empty() {
        return Err(VerifyError::UnresolvedLink("runner"));
    }
    if chain.model.is_empty() {
        return Err(VerifyError::UnresolvedLink("model"));
    }
    if chain.principal.is_empty() || chain.principal.iter().any(|p| p.is_empty()) {
        return Err(VerifyError::UnresolvedLink("principal"));
    }
    Ok(())
}

/// Verify ONLY the cryptographic signature of `chain` against `verifying_key`.
///
/// Holds the **public** key alone. Rejects unsigned (`sig` empty), malformed,
/// and mismatched (tampered/forged) signatures fail-closed. Does not check link
/// resolution — see [`resolve_chain`] for the full end-to-end resolution.
pub fn verify_signature(
    verifying_key: &VerifyingKey,
    chain: &AttestationChain,
) -> Result<(), VerifyError> {
    if chain.sig.is_empty() {
        return Err(VerifyError::Unsigned);
    }
    let sig_bytes = B64
        .decode(chain.sig.as_bytes())
        .map_err(|_| VerifyError::MalformedSignature)?;
    let arr: [u8; SIGNATURE_LENGTH] = sig_bytes
        .try_into()
        .map_err(|_| VerifyError::MalformedSignature)?;
    let signature = Signature::from_bytes(&arr);
    verifying_key
        .verify(&signing_payload(chain), &signature)
        .map_err(|_| VerifyError::SignatureMismatch)
}

/// Resolve the full attestation chain end-to-end: assert every provenance link
/// (tree + def + runner + model + principal) is present AND the asymmetric
/// signature verifies against `verifying_key`.
///
/// This is item ① — a full, cryptographic resolution. It needs ONLY the public
/// key; the producer's private key is neither required nor present. A `chain`
/// that resolves here has every link bound, by the signature, to exactly the
/// bytes the producer signed.
pub fn resolve_chain(
    verifying_key: &VerifyingKey,
    chain: &AttestationChain,
) -> Result<(), VerifyError> {
    require_links(chain)?;
    verify_signature(verifying_key, chain)
}

// ── promotion gate (item ②) ───────────────────────────────────────────────────

/// Why a promotion was refused. Promotion is the boundary that lands a checked
/// artifact; it must reject any attestation that does not cryptographically
/// resolve, fail-closed, before the artifact is admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromotionError {
    /// The attestation failed to resolve/verify (tampered, unsigned, forged, or
    /// missing a link).
    AttestationRejected(VerifyError),
    /// The regen gate's own promotion criteria were not met (defence in depth —
    /// even a valid attestation does not bypass the policy gate).
    GateNotSatisfied,
}

impl std::fmt::Display for PromotionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PromotionError::AttestationRejected(e) => write!(f, "attestation rejected: {e}"),
            PromotionError::GateNotSatisfied => write!(f, "regen gate criteria not satisfied"),
        }
    }
}

impl std::error::Error for PromotionError {}

/// One fail-closed audit event emitted by the promotion gate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEvent {
    /// What was attempted (always `"promote"` here).
    pub action: &'static str,
    /// Whether the attempt was admitted.
    pub admitted: bool,
    /// Machine-readable reason on refusal (empty when admitted).
    pub reason: String,
}

/// The promotion boundary: admit `chain` only if it resolves cryptographically
/// against `verifying_key` AND the [`RegenGate`] criteria are met. Returns the
/// audit event alongside the verdict — a refusal is ALWAYS audited.
///
/// This is item ② — tampered and unsigned attestations are rejected here,
/// fail-closed, with an audit trail. The gate holds ONLY the public key.
pub fn promote(
    verifying_key: &VerifyingKey,
    chain: &AttestationChain,
    gate: &RegenGate,
) -> (Result<(), PromotionError>, AuditEvent) {
    if let Err(e) = resolve_chain(verifying_key, chain) {
        let reason = e.to_string();
        return (
            Err(PromotionError::AttestationRejected(e)),
            AuditEvent {
                action: "promote",
                admitted: false,
                reason,
            },
        );
    }
    // Policy gate: even a cryptographically valid attestation does not bypass
    // the regen-gate promotion criteria (repass + a fresh independent verdict).
    if !gate.repass || gate.indep_verdict.is_empty() {
        return (
            Err(PromotionError::GateNotSatisfied),
            AuditEvent {
                action: "promote",
                admitted: false,
                reason: PromotionError::GateNotSatisfied.to_string(),
            },
        );
    }
    (
        Ok(()),
        AuditEvent {
            action: "promote",
            admitted: true,
            reason: String::new(),
        },
    )
}

// ── cross-tenant shared-hit honesty (item ④) ──────────────────────────────────

/// The sentinel principal used by the anonymized PLATFORM attestation. It names
/// the platform itself, never any tenant — surfacing it can leak nothing.
pub const PLATFORM_PRINCIPAL: &str = "platform:hugit";

/// The sentinel runner identity used by the anonymized PLATFORM attestation —
/// it discloses no producing-tenant runner.
pub const PLATFORM_RUNNER: &str = "platform:shared-deterministic";

/// Produce the anonymized PLATFORM attestation for a cross-tenant shared hit.
///
/// When tenant B does `why`/attestation on a **public-deterministic shared
/// artifact** that tenant A originally produced, B must NOT see A's
/// `principal`/`runner` identity, and B must NOT be mis-attributed as the
/// producer. The honest answer is a platform-level attestation: the
/// content-addressed links that make the artifact reproducible (`tree`, `def`,
/// `model`) are preserved — they are public-deterministic and carry no tenant
/// identity — while `runner` and `principal` are replaced by neutral platform
/// sentinels.
///
/// The producer's chain (`producer`) is read only to copy the
/// public-deterministic links; not one byte of its `runner` or `principal` is
/// carried through. The result is then signed by the PLATFORM key so it is
/// itself publicly verifiable.
pub fn anonymized_platform_attestation(
    platform_key: &SigningKey,
    producer: &AttestationChain,
) -> AttestationChain {
    let neutral = AttestationChain {
        tree: producer.tree.clone(),
        def: producer.def.clone(),
        model: producer.model.clone(),
        // Tenant-identifying links are dropped and replaced by platform
        // sentinels — never copied from the producer.
        runner: PLATFORM_RUNNER.to_string(),
        principal: vec![PLATFORM_PRINCIPAL.to_string()],
        sig: String::new(),
    };
    sign_chain(platform_key, &neutral)
}

/// True iff `chain` carries no byte of any string in `tenant_secrets` across
/// any of its fields — the leak-check used by item ④.
pub fn leaks_none_of(chain: &AttestationChain, tenant_secrets: &[&str]) -> bool {
    let haystacks = std::iter::once(chain.tree.as_str())
        .chain(std::iter::once(chain.def.as_str()))
        .chain(std::iter::once(chain.runner.as_str()))
        .chain(std::iter::once(chain.model.as_str()))
        .chain(chain.principal.iter().map(String::as_str))
        .chain(std::iter::once(chain.sig.as_str()));
    let joined: String = haystacks.collect::<Vec<_>>().join("\u{0}");
    !tenant_secrets
        .iter()
        .any(|secret| !secret.is_empty() && joined.contains(secret))
}
