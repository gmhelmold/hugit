//! hugit-ledger — the human-facing read surface over the forge event stream.
//!
//! Three commands:
//! - [`ledger`] — `hugit ledger`: asked→done→proven projection per campaign.
//! - [`watch`]  — `hugit watch`: live TUI display with latency measurement (p95 < 2s).
//! - [`fleet`]  — `hugit fleet`: machine-readable fleet state schema emitter.
//!
//! All three are pure projections of the EventRecord stream — one store, two
//! zooms (intent ⇄ raw-commit), redaction at the view boundary.  Read-only;
//! never a source of truth.
//!
//! Plus the [`rollup`] module (WP-F3): the three-altitude metric rollups of
//! ADR-0001 §2.3 — intent → PR record → campaign — computed over captured
//! `ContextEnvelope`s + the queue/verdict/check projection seams.

pub mod deeplink;
pub mod fleet;
pub mod journal;
pub mod ledger;
pub mod redact;
pub mod rollup;
pub mod watch;

pub use deeplink::{ResolveResult, resolve};
pub use fleet::{
    AgentEntry, AgentState, FLEET_SCHEMA_VERSION, FleetState, WorkspaceEntry, WorkspaceState,
};
pub use ledger::{Ledger, LedgerEntry, REDACTED, VerdictView};
pub use redact::apply as redact_apply;
pub use rollup::{PrPhase, PrQueueInput, RollupError, campaign_rollup, pr_record};
pub use watch::{EventClass, LatencyMeasurement, WatchDisplay, WatchLine};
