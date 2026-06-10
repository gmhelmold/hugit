//! hugit-web — githugr, the forge web surface (read-only MVP spine).
//!
//! Module layout (wave githugr-spine-w1):
//!   - [`provider`] — the FROZEN Provider trait + view-models (the contract).
//!   - [`fixture`]  — the seeded fixture world (WP-W1: real in-process wave).
//!   - [`layout`]   — shared page chrome (topbar/tabbar/footer/⌘K).
//!   - [`screens`]  — one module per spine screen (WP-W2..W6), mockup-faithful.
//!   - [`routes`]   — the router; handlers are thin provider→VM→render.
//!
//! Design source of truth: `../githugr/design/` (DDD — the mockup is the
//! spec). Live infra (per-repo DO event-log + CoreLink CAS) binds at P2 behind
//! the same `Provider` trait.

pub mod fixture;
pub mod layout;
pub mod provider;
pub mod routes;
pub mod screens;

pub use routes::app;
