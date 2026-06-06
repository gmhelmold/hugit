//! Up-zoom under degradation (WP-D9 ⑤, R7).
//!
//! When the ranking inputs (`blast_radius` from D10 impact and/or `confidence`
//! from D7 verdicts) are UNAVAILABLE, the composite score cannot be trusted.
//! The queue must NOT go silently dark on items that must reach a human:
//! policy-mandatory items STILL surface, each carrying an honest
//! `ranking degraded` state so the human knows the order is unreliable.
//!
//! Non-mandatory items, whose only claim on attention came from the now-missing
//! ranking inputs, are withheld from the degraded view rather than presented in
//! a false order — but mandatory items are surfaced unconditionally.

use super::rank::{AttentionItem, PolicyClass};

/// Which ranking inputs are unavailable in a degraded queue build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DegradedInputs {
    /// Blast-radius input (D10 impact) is unavailable.
    pub blast_unavailable: bool,
    /// Confidence input (D7 verdict) is unavailable.
    pub confidence_unavailable: bool,
}

impl DegradedInputs {
    /// Whether any ranking input is unavailable (i.e. ranking is degraded).
    pub fn is_degraded(self) -> bool {
        self.blast_unavailable || self.confidence_unavailable
    }
}

/// The honest per-item ranking state shown to the human.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RankingState {
    /// Ranking inputs were available; the composite order is trustworthy.
    Ranked,
    /// Ranking inputs were unavailable; the item still surfaces but its
    /// position is NOT trustworthy. The queue is honest about being degraded.
    Degraded {
        /// Human-facing honest label.
        label: &'static str,
        /// Which inputs were missing.
        inputs: DegradedInputs,
    },
}

impl RankingState {
    /// The honest label for a degraded queue.
    pub const DEGRADED_LABEL: &'static str = "ranking degraded";

    /// Whether this state is degraded.
    pub fn is_degraded(&self) -> bool {
        matches!(self, RankingState::Degraded { .. })
    }
}

/// An item as surfaced in the (possibly degraded) human view.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfacedItem {
    /// The underlying attention item.
    pub item: AttentionItem,
    /// The honest ranking state for this surfacing.
    pub state: RankingState,
}

/// Build the human view when ranking inputs are degraded (item ⑤).
///
/// Guarantees:
/// - Every policy-mandatory item STILL surfaces (the queue never goes silently
///   dark on items that must reach a human).
/// - Each surfaced item carries an honest [`RankingState::Degraded`] label
///   naming the missing inputs.
/// - Non-mandatory items are withheld rather than shown in a false order.
///
/// If `degraded.is_degraded()` is false this is a no-op honest "ranked" view
/// over all items.
pub fn surface_degraded(items: Vec<AttentionItem>, degraded: DegradedInputs) -> Vec<SurfacedItem> {
    if !degraded.is_degraded() {
        return items
            .into_iter()
            .map(|item| SurfacedItem {
                item,
                state: RankingState::Ranked,
            })
            .collect();
    }

    items
        .into_iter()
        .filter(|i| i.class == PolicyClass::Mandatory)
        .map(|item| SurfacedItem {
            item,
            state: RankingState::Degraded {
                label: RankingState::DEGRADED_LABEL,
                inputs: degraded,
            },
        })
        .collect()
}
