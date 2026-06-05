//! ② feedback capture: distinguishes UNPROMPTED from PROMPTED ("I'd pay").
//!
//! Only UNPROMPTED signals count toward the ≥3 gate (⑤/R4).

use serde::{Deserialize, Serialize};

/// Discriminant for how a feedback signal was elicited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeedbackKind {
    /// The user volunteered this statement without being asked.
    /// Only UNPROMPTED signals count toward the ≥3 gate.
    Unprompted,
    /// The user responded to a direct question or prompt.
    /// PROMPTED signals do NOT count toward the ≥3 gate.
    Prompted,
}

/// A single captured feedback signal indicating willingness to pay.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedbackSignal {
    /// Anonymized install or team identifier (no PII).
    pub install_id: String,
    /// The feedback kind: UNPROMPTED or PROMPTED.
    pub kind: FeedbackKind,
    /// Raw feedback text (stored for auditability; should not contain PII).
    pub text: String,
}

impl FeedbackSignal {
    /// Returns `true` if this signal counts toward the ≥3 unprompted gate.
    pub fn counts_toward_gate(&self) -> bool {
        self.kind == FeedbackKind::Unprompted
    }
}

/// Count how many signals in the slice are UNPROMPTED (gate-eligible).
pub fn count_unprompted(signals: &[FeedbackSignal]) -> usize {
    signals.iter().filter(|s| s.counts_toward_gate()).count()
}
