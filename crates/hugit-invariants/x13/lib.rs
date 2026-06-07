//! hugit-invariants — Squad-X legibility × degradation/erasure invariant (WP-X13).
//!
//! This module proves the **legibility intersection** holds as a standing,
//! falsifiable invariant: the HUMAN can ALWAYS follow, in every substrate state.
//! It consumes the human-facing down-zoom surfaces — the raw-commit view (D2,
//! plain git), deep links (D5, `hugit_ledger::deeplink`) and `why` (D10,
//! `hugit_cli::why::resolver`) — plus the X7 erasure cascade and the canonical
//! `hugit_refstore::verify_chain`, all read-only; it modifies none of them.
//!
//! # The two invariants (WP-X13 owned items)
//! ① **Intelligence layer DEGRADED:** the human's down-zoom (raw-commit view,
//!    deep links, `why`) still resolves via plain git OR fails HONESTLY
//!    (explicit "layer unavailable") — never a silent 404/blank. (Whitepaper §9
//!    lock 5: a valid git repo keeps serving; the human is *told* when the smart
//!    layer is down.)
//! ② **Erasure cascade:** following any chain reaches an honest TOMBSTONE (or a
//!    live target) — never a broken link. The human can always follow.
//!
//! The load-bearing distinction is **honesty vs. silence**: an explicit
//! "layer unavailable" and a tamper-evident tombstone are PASSES; a silent
//! blank/404 or a dangling/broken link are FAILS. The oracle in
//! `tests/acceptance_x13.rs` is RED on the silent/broken cases.

pub mod legibility;
