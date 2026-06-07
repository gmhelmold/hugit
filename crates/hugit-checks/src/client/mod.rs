//! The checks-as-code CLIENT path (WP-B2a).
//!
//! The memoized-CI wedge — "your green checks never re-run" — assembled from
//! four pieces, all on the real production surface:
//!
//!   - [`parser`]   — parse + validate authored `CheckDef` JSON (fail-closed).
//!   - [`memo_key`] — the three-axis key `H(tree_root ‖ def_digest ‖
//!     toolchain_digest)`, delegating the final formula to
//!     [`hugit_refstore::compute_memo_key`].
//!   - [`ac`]       — the [`ActionCache`](ac::ActionCache) trait, the in-process
//!     reference cache, and the live HTTP seam (P2).
//!   - [`executor`] — `hugit check --local`: derive key → lookup → hit (0 exec)
//!     | miss (execute once, store).
//!
//! B2a owns the client/local path; the runner-side byte-identity proof is B2b.

pub mod ac;
pub mod executor;
pub mod glob;
pub mod memo_key;
pub mod parser;
