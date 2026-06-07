//! Attention queue — the human's inbox (WP-D9).
//!
//! The attention queue ranks what needs a human by a DOCUMENTED composite of
//! `policy × blast-radius × verdict-confidence` over the frozen
//! [`hugit_contracts::AttentionRank`]. It enforces three guarantees that keep a
//! human reliably in the loop:
//!
//! 1. **Documented composite order** ([`rank`]) — the score is written down and
//!    a known-input fixture reproduces the exact ordering; perturbing one input
//!    moves an entry to its expected position.
//! 2. **Fast-approve gating** ([`fast_approve`]) — the 90s fast-approve
//!    affordance is BLOCKED for high-risk/policy-mandatory items (forced through
//!    full review) and PERMITTED only for policy-low-risk.
//! 3. **Honest degradation** ([`degraded`]) — when the ranking inputs
//!    (blast/confidence) are unavailable, policy-mandatory items STILL surface
//!    carrying an honest "ranking degraded" state; the queue never goes silently
//!    dark on items that must reach a human.
//!
//! ## Consumes, never owns its inputs
//!
//! D9 reads the D10 `impact` blast-radius, the D7 `VerdictObject` confidence,
//! and the D6 policy classification. It modifies none of them — the seams are
//! frozen and consumed read-only. Blast/confidence land as the `blast_radius`
//! and `confidence` fields of [`hugit_contracts::AttentionRank`]; the policy
//! classification lands as [`rank::PolicyClass`].
//!
//! ## Acceptance lane note
//!
//! Everything here is **pure / fixture-driven**: composite ordering, the
//! mandatory floor, the fast-approve gate, and the degraded up-zoom are all
//! deterministic functions over local fixtures — no live model or network input.

pub mod degraded;
pub mod fast_approve;
pub mod fixtures;
pub mod rank;

pub use degraded::{DegradedInputs, RankingState, SurfacedItem, surface_degraded};
pub use fast_approve::{BlockReason, FAST_APPROVE_WINDOW_SECS, FastApprove, evaluate};
pub use rank::{
    AttentionItem, AttentionQueue, CONFIDENCE_SCALE, MANDATORY_FLOOR, PolicyClass, W_BLAST,
    W_UNCERTAIN,
};

pub use hugit_contracts::AttentionRank;
