//! hugit-checks — check execution engine.
//!
//! Module layout (body lands per owning WP):
//!   - `affected/` — affected-target computation (WP-B3)
//!   - `regen/`   — derived-file regeneration drivers (WP-C4)
//!   - `shadow/`  — snapshot-cadence shadow scheduler (WP-C8)

#[path = "../affected/mod.rs"]
pub mod affected;

pub mod regen;

pub mod shadow;
