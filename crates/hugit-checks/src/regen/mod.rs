//! Derived-file regeneration subsystem (WP-C4).
//!
//! Module layout:
//! - `driver/` — the regen driver framework + Cargo.lock + pnpm-lock drivers
//!   (WP-C4 Claims boundary).
//! - `gate/` — regenerative-rebase landing gate (WP-D12 Claims boundary).

pub mod driver;
pub mod gate;
