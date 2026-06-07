//! hugit-invariants — Squad-X tenant-isolation red-team invariant (WP-X1).
//!
//! This module proves the CoreLink-inherited **tenant boundary** holds for
//! hugit, without owning any production path. It consumes the canonical
//! memo-key surface ([`hugit_refstore::compute_memo_key`]) and the frozen
//! [`AttestationChain`](hugit_contracts::AttestationChain) read-only; it
//! modifies neither.
//!
//! # The four red-team invariants (WP-X1 owned items)
//! ① **Cross-tenant private lookup → miss/deny, never served.** Tenant B
//!    requesting a key whose private bytes came from tenant A resolves to
//!    [`Lookup::Miss`](isolation::Lookup::Miss) — B physically cannot derive A's
//!    HMAC-namespaced storage key, so the bytes are never served.
//! ② **Forged / collision memo key → deny + alert, no poisoning.** A forged
//!    namespaced key, and a collision write that would overwrite an existing
//!    entry with different bytes, are both refused fail-closed with an ALERT
//!    audit event; the existing entry's bytes are left byte-for-byte unchanged.
//! ③ **Public-deterministic artifact IS shared, with proof no private bytes
//!    rode along.** A public-deterministic artifact is served cross-tenant from
//!    the [`SharedRegistry`](isolation::SharedRegistry); its attestation is the
//!    platform-anonymized form, proven to carry NO byte of the producing
//!    tenant's private principal/runner.
//! ④ **No private-artifact side-channel.** A private miss and a genuine absent
//!    are the SAME response shape (`Miss`) and the SAME work (one HMAC
//!    derivation + one map probe), so no existence/timing signal betrays a
//!    private artifact. The existence signal inherent to public-deterministic
//!    sharing is **by-design disclosure** — documented, not a leak.
//!
//! Everything here is verification logic over the consumed surfaces — there is
//! no production behaviour to ship from this module.

pub mod isolation;
