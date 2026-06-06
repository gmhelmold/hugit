//! Fast-approve (90s) affordance gate (WP-D9 ④, R2).
//!
//! The attention queue offers a 90-second fast-approve affordance so a human
//! can clear policy-low-risk items quickly. That affordance is **blocked** for
//! high-risk and policy-mandatory items: those are forced through full review.
//! Only policy-low-risk items may be fast-approved.

use super::rank::{AttentionItem, PolicyClass};

/// The fast-approve affordance window, in seconds (the "approve in 90s" path).
pub const FAST_APPROVE_WINDOW_SECS: u64 = 90;

/// Outcome of evaluating the fast-approve affordance for an item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FastApprove {
    /// The 90s fast-approve affordance is permitted (policy-low-risk).
    Permitted {
        /// The affordance window in seconds.
        window_secs: u64,
    },
    /// The affordance is blocked; the item must go through full review.
    Blocked {
        /// Human-readable reason the fast path is barred.
        reason: BlockReason,
    },
}

impl FastApprove {
    /// Whether the fast-approve affordance is permitted.
    pub fn is_permitted(&self) -> bool {
        matches!(self, FastApprove::Permitted { .. })
    }

    /// Whether the fast-approve affordance is blocked (forced to full review).
    pub fn is_blocked(&self) -> bool {
        matches!(self, FastApprove::Blocked { .. })
    }
}

/// Why a fast-approve was blocked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockReason {
    /// Policy mandates a human must fully review this item.
    PolicyMandatory,
    /// The item is high-risk and must go through full review.
    HighRisk,
}

impl std::fmt::Display for BlockReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BlockReason::PolicyMandatory => {
                write!(f, "policy-mandatory: full review required")
            }
            BlockReason::HighRisk => write!(f, "high-risk: full review required"),
        }
    }
}

/// Evaluate the fast-approve affordance for an item.
///
/// - [`PolicyClass::Mandatory`] ⇒ blocked ([`BlockReason::PolicyMandatory`]).
/// - [`PolicyClass::HighRisk`]  ⇒ blocked ([`BlockReason::HighRisk`]).
/// - [`PolicyClass::LowRisk`]   ⇒ permitted with the 90s window.
pub fn evaluate(item: &AttentionItem) -> FastApprove {
    match item.class {
        PolicyClass::Mandatory => FastApprove::Blocked {
            reason: BlockReason::PolicyMandatory,
        },
        PolicyClass::HighRisk => FastApprove::Blocked {
            reason: BlockReason::HighRisk,
        },
        PolicyClass::LowRisk => FastApprove::Permitted {
            window_secs: FAST_APPROVE_WINDOW_SECS,
        },
    }
}
