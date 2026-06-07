//! hugit-invariants — Squad-X cross-cutting invariant test crate.
//!
//! Root library entry point. Each WP owns a subtree; this file wires them
//! together additively. No subtree is modified when a new WP is added.
//!
//! # WP layout
//! - `x2/` — attestation e2e: full chain resolves cryptographically, tampered/unsigned rejected, public verification, cross-tenant honesty (WP-X2).
//! - `x4/` — supply-chain invariants: pinned images, verified deps, fail-closed (WP-X4).
//! - `x5/` — namespace-law invariants: no git-verb shadow, ref-namespace non-collision (WP-X5).
//! - `x6/` — resource non-interference: infra isolation config + accounting model + latency measurement (WP-X6).
//! - `x12/` — erasure × provenance × mirror: post-erasure chain stays verifiable over a tamper-evident tombstone (no silent re-link), mirror-obligation discharged-or-disclosed in the export/exit proof (WP-X12).
//! - `x8/` — self-release attestation: every App/CLI/runner-image release signed + published to a verifiable transparency log, the running App verifies its own provenance at boot, unsigned/tampered self-build fails CLOSED (WP-X8).

// ── WP-X2: attestation end-to-end invariants ─────────────────────────────────
#[path = "x2/lib.rs"]
pub mod x2;

// ── WP-X4: supply-chain invariants ───────────────────────────────────────────
// Re-export x4's public surface so existing consumers (acceptance_x4.rs) keep
// their `hugit_invariants::pin::…` import paths intact.
#[path = "x4/lib.rs"]
mod x4_root;

pub use x4_root::pin;

// ── WP-X5: namespace-law invariants ──────────────────────────────────────────
#[path = "x5/lib.rs"]
pub mod x5;

// ── WP-X6: resource non-interference invariants ───────────────────────────────
#[path = "x6/lib.rs"]
pub mod x6;
// ── WP-X12: erasure × provenance × mirror invariants ─────────────────────────
#[path = "x12/lib.rs"]
pub mod x12;
// ── WP-X8: self-release attestation invariants ───────────────────────────────
#[path = "x8/lib.rs"]
pub mod x8;
// ── WP-X9: cross-phase object identity invariants ─────────────────────────────
#[path = "x9/lib.rs"]
pub mod x9;
