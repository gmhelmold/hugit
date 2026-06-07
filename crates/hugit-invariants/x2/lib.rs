//! hugit-invariants — Squad-X attestation-end-to-end invariant (WP-X2).
//!
//! This module proves the four attestation invariants behind the
//! [`AttestationChain`](hugit_contracts::AttestationChain) object class — using
//! a **real asymmetric signature** (Ed25519) so verification is genuinely
//! public — without owning any production path. It consumes the frozen
//! attestation/promotion contract surfaces read-only; it modifies neither.
//!
//! # The four invariants (WP-X2 owned items)
//! ① **Full chain resolves cryptographically.** All five provenance links
//!    (tree + def + runner + model + principal) are present AND the asymmetric
//!    signature verifies against the public key. No link may be unresolved.
//! ② **Tampered/unsigned rejected at promotion.** A mutated or unsigned chain
//!    presented at the promotion boundary ([`attest::promote`]) is rejected
//!    fail-closed, with an audit event.
//! ③ **Public, documented verification procedure.** A verifier holding ONLY
//!    the Ed25519 public key accepts a valid attestation, REJECTS a tampered
//!    one, and demonstrably never needs (nor possesses) the signing key. The
//!    runnable steps are committed as `x2/PUBLIC_VERIFICATION.md` and exercised
//!    by the acceptance suite.
//! ④ **Cross-tenant shared-hit honesty.** Tenant B's attestation on a
//!    public-deterministic shared artifact resolves to an anonymized PLATFORM
//!    attestation ([`attest::anonymized_platform_attestation`]) — never leaking
//!    tenant A's principal/runner, never mis-attributing B as producer.
//!
//! Everything here is verification logic over the *consumed* surfaces — there
//! is no production behavior to ship from this module.

pub mod attest;
