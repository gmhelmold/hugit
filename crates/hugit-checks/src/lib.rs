//! hugit-checks — check execution engine.
//!
//! Module layout (body lands per owning WP):
//!   - `client/`  — checks-as-code client: parser + memo key + AC + local
//!     executor (WP-B2a)
//!   - `affected/` — affected-target computation (WP-B3)
//!   - `regen/`   — derived-file regeneration drivers (WP-C4)
//!   - `shadow/`  — snapshot-cadence shadow scheduler (WP-C8)

pub mod client;

#[path = "../affected/mod.rs"]
pub mod affected;

pub mod regen;

pub mod shadow;
