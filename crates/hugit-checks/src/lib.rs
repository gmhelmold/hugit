//! hugit-checks — check execution engine.
//!
//! Module layout (body lands per owning WP):
//!   - `client/`  — checks-as-code client: parser + memo key + AC + local
//!     executor (WP-B2a)
//!   - `runner/`  — runner-side execution: byte-identity (local≡runner),
//!     non-determinism detection, honest partial hit-rate (WP-B2b)
//!   - `affected/` — affected-target computation (WP-B3)
//!   - `regen/`   — derived-file regeneration drivers (WP-C4)
//!   - `shadow/`  — snapshot-cadence shadow scheduler (WP-C8)

pub mod client;

pub mod runner;

#[path = "../affected/mod.rs"]
pub mod affected;

pub mod regen;

pub mod shadow;
