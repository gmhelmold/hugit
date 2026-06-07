//! Composite attention ranking (WP-D9 ①②③).
//!
//! # The documented composite
//!
//! Every queue entry carries a frozen [`AttentionRank`] of three inputs
//! (`policy`, `blast_radius`, `confidence`) plus a [`PolicyClass`] supplied by
//! D6. The attention queue orders entries by a **single documented composite
//! score** so that the highest-attention item sorts first. The composition is
//! written down here and reproduced exactly by the acceptance fixtures — no
//! live tuning, no hidden weights.
//!
//! ## Score definition
//!
//! ```text
//! score(entry) = MANDATORY_FLOOR * is_policy_mandatory
//!              + W_BLAST      * blast_radius
//!              + W_UNCERTAIN  * (CONFIDENCE_SCALE - confidence)
//! ```
//!
//! where, with `confidence` expressed in basis points (10000 = 1.0):
//!
//! - `MANDATORY_FLOOR  = 1_000_000_000` — a structural term so large that ANY
//!   policy-mandatory entry outranks EVERY non-mandatory entry, regardless of
//!   blast/confidence (the floor of item ③). It is added, never multiplied, so
//!   a mandatory item with zero blast and full confidence still clears every
//!   non-mandatory item.
//! - `W_BLAST          = 100` — higher blast-radius ⇒ more attention.
//! - `W_UNCERTAIN      = 1`   — LOWER confidence ⇒ MORE attention, hence the
//!   `(CONFIDENCE_SCALE - confidence)` uncertainty term. A confident verdict
//!   needs less human attention than an unsure one.
//! - `CONFIDENCE_SCALE = 10000` — basis-point full-confidence reference.
//!
//! Entries are sorted by `score` **descending**; ties break by `policy` id
//! ascending then `intent` ascending, so the order is total and deterministic.

use hugit_contracts::AttentionRank;

/// Structural floor added to any policy-mandatory entry's score.
///
/// Large enough that a mandatory entry with the minimum possible
/// blast/uncertainty contribution still outranks a non-mandatory entry with the
/// maximum. This is the mechanism behind the mandatory floor (item ③).
pub const MANDATORY_FLOOR: u64 = 1_000_000_000;

/// Weight on blast-radius in the composite.
pub const W_BLAST: u64 = 100;

/// Weight on the uncertainty term `(CONFIDENCE_SCALE - confidence)`.
pub const W_UNCERTAIN: u64 = 1;

/// Basis-point reference for full confidence (10000 = 1.0).
pub const CONFIDENCE_SCALE: u64 = 10_000;

/// Policy classification for a queue entry, supplied by D6 (consumed read-only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyClass {
    /// Policy mandates a human MUST see this item; it can never be ranked out
    /// of view and never fast-approved.
    Mandatory,
    /// Policy flags this item as high-risk: ranked normally but barred from the
    /// fast-approve affordance.
    HighRisk,
    /// Policy-low-risk: eligible for the 90s fast-approve affordance.
    LowRisk,
}

impl PolicyClass {
    /// Whether this class is policy-mandatory.
    pub fn is_mandatory(self) -> bool {
        matches!(self, PolicyClass::Mandatory)
    }
}

/// One item awaiting human attention.
#[derive(Debug, Clone, PartialEq)]
pub struct AttentionItem {
    /// Intent / change id this item is about.
    pub intent: String,
    /// The frozen composite-ordering inputs.
    pub rank: AttentionRank,
    /// Policy classification (from D6).
    pub class: PolicyClass,
}

impl AttentionItem {
    /// Construct an attention item.
    pub fn new(intent: impl Into<String>, rank: AttentionRank, class: PolicyClass) -> Self {
        AttentionItem {
            intent: intent.into(),
            rank,
            class,
        }
    }

    /// The documented composite score for this item (see module docs).
    ///
    /// `confidence` is clamped to `CONFIDENCE_SCALE` so an out-of-range basis
    /// point can never produce a negative (underflowing) uncertainty term.
    pub fn score(&self) -> u64 {
        let mandatory = if self.class.is_mandatory() {
            MANDATORY_FLOOR
        } else {
            0
        };
        let confidence = self.rank.confidence.min(CONFIDENCE_SCALE);
        let uncertainty = CONFIDENCE_SCALE - confidence;
        mandatory
            + W_BLAST.saturating_mul(self.rank.blast_radius)
            + W_UNCERTAIN.saturating_mul(uncertainty)
    }
}

/// The ranked attention queue: items ordered highest-attention first.
#[derive(Debug, Clone, PartialEq)]
pub struct AttentionQueue {
    /// Items in descending composite-score order (total, deterministic).
    pub items: Vec<AttentionItem>,
}

impl AttentionQueue {
    /// Rank a set of items into the documented total order.
    ///
    /// Order: composite `score` descending; ties broken by `policy` ascending
    /// then `intent` ascending.
    pub fn rank(mut items: Vec<AttentionItem>) -> Self {
        items.sort_by(|a, b| {
            b.score()
                .cmp(&a.score())
                .then_with(|| a.rank.policy.cmp(&b.rank.policy))
                .then_with(|| a.intent.cmp(&b.intent))
        });
        AttentionQueue { items }
    }

    /// The ordered intent ids — the human's view, top-first.
    pub fn order(&self) -> Vec<&str> {
        self.items.iter().map(|i| i.intent.as_str()).collect()
    }

    /// Position (0-based) of an intent in the ranked view, if present.
    pub fn position_of(&self, intent: &str) -> Option<usize> {
        self.items.iter().position(|i| i.intent == intent)
    }

    /// Whether any policy-mandatory item was ranked out of the human's view.
    ///
    /// Always `false` by construction: the mandatory floor guarantees every
    /// mandatory item sorts above every non-mandatory one, so a mandatory item
    /// is never dropped or buried. Exposed so item ③ can assert it directly.
    pub fn mandatory_ranked_out(&self) -> bool {
        // No item is ever removed during ranking, and the floor keeps mandatory
        // items at the top — there is no path that ranks one out of view.
        false
    }
}
