//! hugit-invariants — erasure × provenance × mirror invariant (WP-X12).
//!
//! This module proves the **three-way composition** of erasure, provenance, and
//! the GitHub mirror holds as a standing, falsifiable invariant — without owning
//! any production path. It consumes the X7 cascade, the E1 mirror, and the E5
//! export/exit proof as-built (plus the canonical attestation/hash from
//! `hugit-refstore`) read-only; it modifies none of them.
//!
//! # The two invariants (WP-X12 owned items)
//! ① **After an erasure request the attestation chain remains independently
//!    verifiable with the erased object as a tamper-evident TOMBSTONE — never
//!    silently re-linked.** Erasure operates on the object store, not the
//!    append-only chain: the surviving provenance link keeps referencing the
//!    same content hash, which now resolves to a [`erasure::Tombstone`]. The
//!    canonical `hugit_refstore::verify_chain` still verifies; a silent re-link
//!    is caught fail-closed.
//! ② **The mirror-side erasure obligation is discharged OR explicitly surfaced
//!    as residual risk — and that disclosure is part of the export/exit proof.**
//!    A [`erasure::MirrorObligation`] resolves to discharge-or-residual-risk and
//!    the [`erasure::ExitProof`] (a frozen `ExportSchema` envelope) validates
//!    only when the disclosure is a stated element of the export proof.
//!
//! Everything here is composition logic over the *consumed* surfaces — there is
//! no production behavior to ship from this module beyond the tombstone +
//! disclosure surface.

pub mod erasure;
