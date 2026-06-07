# Public verification procedure — hugit attestation (WP-X2)

A hugit attestation (`AttestationChain`) is signed with **Ed25519**, a real
asymmetric signature scheme. This means **anyone** can verify an attestation
holding **only the producer's public key** — and **nobody** can forge a new
attestation without the producer's private (signing) key.

This is the load-bearing difference from a keyed-MAC such as HMAC: an HMAC can
only be checked by a party that holds the shared secret, and that same party can
therefore also forge. With Ed25519 the verifying (public) key and the signing
(private) key are different keys; the verifier **never** needs and **never**
receives the signing key / secret / private key.

This document is the runnable procedure. It is executed by the acceptance suite
(`x2/tests/acceptance_x2.rs`, `item_3_public_verification_procedure`) against a
known-good and a known-bad attestation, so the steps below are proven sufficient,
not aspirational.

## What you need

1. The producer's **public key**, as published base64 bytes (32 bytes decoded).
   This is all you need. You do **not** need the signing key.
2. The attestation to check: an `AttestationChain { tree, def, runner, model,
   principal, sig }`.

## What is signed

The signature in `sig` covers the canonical, length-prefixed encoding of the
five provenance links — `tree`, `def`, `runner`, `model`, and the ordered
`principal` chain — under a domain-separation tag (`hugit/attestation/v1`). The
`sig` field itself is excluded from the signed bytes. See
`attest::signing_payload`. Mutating any link changes the signed bytes, so the
signature stops verifying — that is the tamper-evidence.

## Procedure (public-key-only)

```rust
use hugit_invariants::x2::attest;

// Step 1 — reconstruct the verifying key from the PUBLISHED public bytes alone.
//          No signing key, no secret, no private key is involved.
let vk = attest::parse_public_key(public_key_b64)?;

// Step 2 — verify the full chain end-to-end: every link present AND the Ed25519
//          signature verifies against the public key.
match attest::resolve_chain(&vk, &attestation) {
    Ok(())  => { /* ACCEPT: authentic, untampered, fully resolved */ }
    Err(e)  => { /* REJECT fail-closed: tampered, unsigned, forged, or a
                    missing link. `e` names the reason. */ }
}

// (Equivalently, to check only the cryptographic signature without the
//  link-resolution step, use `attest::verify_signature(&vk, &attestation)`.)
```

## Guarantees this procedure proves

- **Accept-valid.** A correctly signed, fully-resolved attestation returns
  `Ok(())` using only the public key.
- **Reject-tampered.** Any post-signing mutation of a link returns
  `Err(SignatureMismatch)`.
- **Reject-unsigned.** An empty `sig` returns `Err(Unsigned)`.
- **Un-forgeable.** A signature made with any key other than the producer's
  private key returns `Err(SignatureMismatch)` — a verifier (who holds only the
  public key) can check but can **never** forge.

## Cross-tenant shared hits

For a public-deterministic artifact shared across tenants, `hugit why` surfaces
an **anonymized platform attestation** (`attest::anonymized_platform_attestation`):
the public-deterministic links (`tree`, `def`, `model`) are preserved and the
`runner`/`principal` are replaced by neutral platform sentinels, then signed by
the platform key. It is verified by exactly the procedure above, using the
**platform** public key. It never leaks the producing tenant's identity.
