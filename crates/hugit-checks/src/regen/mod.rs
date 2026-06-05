//! Derived-file regeneration subsystem (WP-C4).
//!
//! Module layout:
//! - `driver/` — the regen driver framework + Cargo.lock + pnpm-lock drivers
//!   (WP-C4 Claims boundary).
//! - `gate/` — promotion gate (D12, not in this WP).

pub mod driver;
