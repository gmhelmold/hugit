//! hugit-refstore — the append-only, hash-chained event log that is the source
//! of truth for one repository (one Durable Object per repo owns it).
//!
//! WP-D1a delivers the *core* of that log:
//!
//! - [`log`] — the single append point + the frozen per-record hash chain.
//! - [`replay`] — deterministic projection of the log into a ref state.
//! - [`tamper`] — chain re-verification + fail-closed tamper detection.
//!
//! Refs are a **derived view**, never primary state: a ref state is the fold of
//! [`replay::replay`] over the log. Replay is pure and deterministic — the same
//! log always projects to the same ref state, byte-for-byte. Tamper detection
//! fails *closed*: a broken chain refuses to serve a derived view, it never
//! silently repairs.
//!
//! Compaction / cold-tier / recovery / undo (D1b) and concurrency / perf (D1c)
//! build on the primitives sealed here; this crate makes no concurrency or
//! throughput claims (single-writer append correctness only).

pub mod log;
pub mod replay;
pub mod tamper;

pub use log::{EventLog, GENESIS_PREV_HASH, compute_this_hash};
pub use replay::{RefState, replay};
pub use tamper::{TamperError, verify_chain};
