//! hugit-invariants — deep-link referential integrity (lifecycle) invariant (WP-X14).
//!
//! This module proves **deep-link referential integrity across the FULL object
//! lifecycle** — after compaction/cold-tier to R2, after mirror round-trip, after
//! tombstoning: every ledger/intent deep link resolves to its target or to a
//! tamper-evident tombstone, with ZERO dangling links, ever.
//!
//! It consumes D1 (compaction/cold-tier to R2), E1 (mirror round-trip), D5 (ledger/
//! intent deep links), and X7 (tombstoning) as-built read-only; it modifies none.
//!
//! # The two invariants (WP-X14 owned items)
//!
//! ① **Property test across the full object lifecycle — after compaction/cold-tier
//!    to R2, after mirror round-trip, after tombstoning: every ledger/intent deep
//!    link resolves to its target or to a tamper-evident tombstone.** The property
//!    holds across every lifecycle transition, not just at rest.
//!
//! ② **ZERO dangling links, ever — a continuous integrity check standing as a
//!    fixture.** Resolution (X14) is distinct from identity (X9③): this module
//!    proves a link LANDS on target-or-tombstone; X9③ proves the id is the SAME
//!    id across phases (non-colliding). They are distinct legs of the same integrity
//!    guarantee.
//!
//! # Resolution vs identity
//!
//! X14 covers deep-link RESOLUTION (the link lands on target-or-tombstone).
//! X9③ covers IDENTITY (the id is the same id, non-colliding). X13② is the
//! human-following reading; X14 is the mechanized property/standing-fixture
//! reading; X7② is the no-orphans-survive-erasure reading.
//!
//! # Tenant boundary
//!
//! Tenant boundary = HMAC-derived prefixes (CoreLink model). Within the model
//! the HMAC is abstracted as an opaque prefix tag on the intent_id — the
//! resolver is tenant-aware by construction.

pub mod deeplink;
