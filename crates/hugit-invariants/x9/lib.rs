//! hugit-invariants — cross-phase object identity (WP-X9).
//!
//! This module proves that **object identity holds ACROSS phases** as a
//! standing, falsifiable invariant — without owning any production path. It
//! consumes the B2 (memoization), D7 (verdict panels), B6 (sidecar), and D4
//! (native intent) surfaces as-built, plus the canonical hashing in
//! `hugit-refstore` and the frozen types in `hugit-contracts`, read-only; it
//! modifies none of them.
//!
//! # The two identity laws (WP-X9 owned items)
//!
//! ① **A `CheckResult` memoized by the phase-B App is BIT-IDENTICAL to the one
//!    served as evidence in a phase-D verdict panel for the same
//!    `(tree, def, toolchain)`.** Identity is *byte* identity, not value
//!    equality: both phases serve the result through one canonical serializer
//!    ([`identity::canonical_bytes`]), so the phase-B memo and the phase-D
//!    evidence are the same bytes — and therefore the same content hash
//!    ([`identity::content_id`]) — for the same memo key.
//!
//! ② **A mismatch fails CLOSED + alerts.** If the bytes the phase-D panel would
//!    serve as evidence diverge from the bytes phase-B memoized (a re-serialized
//!    or tampered object), the cross-phase check
//!    ([`identity::serve_evidence`]) REFUSES to serve and raises an alert —
//!    never silently serves a divergent object as evidence.
//!
//! ③ **`intent_id` identity (the R10 addition): the id minted by a phase-B
//!    sidecar is IDENTICAL and NON-COLLIDING with the native phase-D intent for
//!    the same logical intent — one lifecycle, one id.** A phase-B
//!    [`IntentSidecar`](hugit_contracts::IntentSidecar) mints an `intent_id`;
//!    the phase-D native intent (carried on the
//!    [`VerdictObject`](hugit_contracts::VerdictObject)'s `intent` field) for the
//!    SAME logical intent must carry that EXACT id. Divergence (a second id
//!    minted) and collision (two distinct logical intents folding onto one id)
//!    both fail CLOSED. Identity is NOT resolution: X14 covers deep-link
//!    resolution; X9③ covers that the id is the SAME id.
//!
//! Everything here is identity/composition logic over the *consumed* surfaces —
//! there is no production behavior to ship beyond the canonical-identity and
//! intent-lifecycle surface.

pub mod identity;
