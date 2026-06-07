//! hugit-invariants — Squad-X right-to-erasure CASCADE invariant (WP-X7).
//!
//! This module proves a data subject's personal data is **provably erased across
//! EVERY store** — CAS, provenance/ledger, context, the GitHub mirror, and the
//! experiment corpus — with no orphaned provenance refs, attestation chains that
//! re-seal OR fail CLOSED, and the erasure×seal precedence resolved (lawful
//! corpus erasure is permitted despite the seal and invalidates the gate
//! fail-closed). It consumes the five stores' surfaces (B2/D1/X3/E1/D8 as-built)
//! plus the canonical hash/attestation preimage from `hugit-refstore` and the
//! frozen `hugit-contracts` types read-only; it modifies none of them.
//!
//! Where X12 proves erasure × provenance × mirror *compose* at the seam, X7 owns
//! the broader cross-store cascade and the corpus seal-precedence leg X12 does
//! not. They compose at the property level (same tamper-evident-tombstone law),
//! not by sharing source files.
//!
//! # The four owned items (WP-X7)
//! ① **Five-store cascade.** A data subject's data is erased across CAS +
//!    provenance/ledger + context store + the GitHub mirror + the experiment
//!    corpus; each leg exposes an ABSENCE scan (re-read the store, not a flag).
//! ② **No orphans.** After erasure every provenance link resolves to a live
//!    object or a tamper-evident tombstone — [`cascade::scan_orphans`] returns
//!    zero dangling refs.
//! ③ **Attestation re-seal OR fail-closed.** The [`cascade::AttestationChain`]
//!    is re-sealed over the post-erasure manifest (verifies via REAL ed25519
//!    over the canonical preimage) OR a stale chain fails CLOSED — never a
//!    silently broken seal.
//! ④ **Erasure × seal precedence.** [`cascade::SealedCorpus::erase_datapoint`]
//!    permits lawful erasure of a SEALED datapoint AND invalidates the
//!    [`cascade::GateInvalidation`] fail-closed (re-pinned `RegenGate`,
//!    audited) — never blocked by the seal, never silent.
//!
//! Everything here is cascade logic over the *consumed* surfaces — no production
//! behavior ships from this module beyond the cascade + tombstone + invalidation
//! surface. This is also the erasure surface that closes the X3③ context-store
//! purge PARTIAL (the cross-store cascade that proof was waiting on).

pub mod cascade;
