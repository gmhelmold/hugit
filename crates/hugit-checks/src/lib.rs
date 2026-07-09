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

pub mod attest_v2;

/// The off-box cost-attestation verdict (the CONSUME side of the cost-killer):
/// turn a dispatch outcome's propagated `intent_metrics_sig` into a fail-closed
/// `Attested`/`Unattested(reason)` verdict. Renders nothing.
pub mod cost_attest;

/// The pinned canonical `CheckDef` for hugit's own CI gate on the moat
/// check-host (#68 — the verified `toolchain_ref` digest + the gate command).
pub mod gate;

pub mod attest_keyset;
