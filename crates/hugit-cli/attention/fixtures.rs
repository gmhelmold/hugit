//! Deterministic fixtures for the attention-queue acceptance lane (WP-D9).
//!
//! These carry KNOWN policy/blast/confidence triples with an expected composite
//! ordering, so the acceptance suite reproduces the documented order exactly and
//! a one-input perturbation moves an entry to a known position. No live inputs.

use hugit_contracts::AttentionRank;

use super::rank::{AttentionItem, PolicyClass};

/// Build an [`AttentionRank`] from raw inputs.
fn rank(policy: &str, blast_radius: u64, confidence: u64) -> AttentionRank {
    AttentionRank {
        policy: policy.into(),
        blast_radius,
        confidence,
    }
}

/// The baseline known-input fixture (item ①).
///
/// Hand-computed composite scores (see `rank` module docs):
/// `score = MANDATORY_FLOOR*mandatory + 100*blast + 1*(10000-confidence)`.
///
/// | intent | class     | blast | conf  | score          |
/// |--------|-----------|-------|-------|----------------|
/// | sec    | Mandatory |    10 |  9000 | 1_000_002_000  |
/// | big    | HighRisk  |   200 |  5000 |        25_000  |
/// | mid    | LowRisk   |    50 |  8000 |         7_000  |
/// | calm   | LowRisk   |     5 |  9900 |           600  |
///
/// Expected order (score desc): `[sec, big, mid, calm]`.
pub fn baseline_items() -> Vec<AttentionItem> {
    vec![
        // deliberately unsorted on input — ranking must impose the order.
        AttentionItem::new("mid", rank("p-mid", 50, 8000), PolicyClass::LowRisk),
        AttentionItem::new("sec", rank("p-sec", 10, 9000), PolicyClass::Mandatory),
        AttentionItem::new("calm", rank("p-calm", 5, 9900), PolicyClass::LowRisk),
        AttentionItem::new("big", rank("p-big", 200, 5000), PolicyClass::HighRisk),
    ]
}

/// The documented expected ordering for [`baseline_items`].
pub fn baseline_expected_order() -> Vec<&'static str> {
    vec!["sec", "big", "mid", "calm"]
}

/// Perturbation fixture (item ②): baseline, but `calm`'s blast-radius jumps
/// from 5 → 400. New score for `calm` = 100*400 + (10000-9900) = 40_100, which
/// now outranks `big` (25_000) and `mid` (7_000) but stays below the mandatory
/// `sec`. Expected order moves `calm` from last to second: `[sec, calm, big, mid]`.
pub fn perturbed_items() -> Vec<AttentionItem> {
    vec![
        AttentionItem::new("mid", rank("p-mid", 50, 8000), PolicyClass::LowRisk),
        AttentionItem::new("sec", rank("p-sec", 10, 9000), PolicyClass::Mandatory),
        AttentionItem::new("calm", rank("p-calm", 400, 9900), PolicyClass::LowRisk),
        AttentionItem::new("big", rank("p-big", 200, 5000), PolicyClass::HighRisk),
    ]
}

/// Expected ordering after the [`perturbed_items`] one-input change.
pub fn perturbed_expected_order() -> Vec<&'static str> {
    vec!["sec", "calm", "big", "mid"]
}

/// Mandatory-floor fixture (item ③): a mandatory item with the WORST possible
/// attention inputs (zero blast, full confidence ⇒ minimal contribution) pitted
/// against many non-mandatory items with the BEST possible attention inputs
/// (huge blast, zero confidence). The mandatory floor must still keep the
/// mandatory item at the top — it can never be ranked out of view.
pub fn mandatory_floor_items() -> Vec<AttentionItem> {
    let mut items = vec![AttentionItem::new(
        "must-see",
        rank("p-must", 0, 10_000),
        PolicyClass::Mandatory,
    )];
    for n in 0..5 {
        items.push(AttentionItem::new(
            format!("loud-{n}"),
            rank("p-loud", 1_000_000, 0),
            PolicyClass::HighRisk,
        ));
    }
    items
}

/// A policy-low-risk item — fast-approve must be PERMITTED (item ④).
pub fn low_risk_item() -> AttentionItem {
    AttentionItem::new("quick", rank("p-quick", 3, 9800), PolicyClass::LowRisk)
}

/// A high-risk item — fast-approve must be BLOCKED (item ④).
pub fn high_risk_item() -> AttentionItem {
    AttentionItem::new("risky", rank("p-risky", 300, 4000), PolicyClass::HighRisk)
}

/// A policy-mandatory item — fast-approve must be BLOCKED (item ④).
pub fn mandatory_item() -> AttentionItem {
    AttentionItem::new("forced", rank("p-forced", 7, 7000), PolicyClass::Mandatory)
}

/// Degradation fixture (item ⑤): a mix of mandatory + non-mandatory items where
/// the ranking inputs are removed. The mandatory items must STILL surface,
/// labeled "ranking degraded".
pub fn degraded_mix_items() -> Vec<AttentionItem> {
    vec![
        AttentionItem::new("m1", rank("p-m1", 12, 6000), PolicyClass::Mandatory),
        AttentionItem::new("lo", rank("p-lo", 4, 9700), PolicyClass::LowRisk),
        AttentionItem::new("m2", rank("p-m2", 88, 3000), PolicyClass::Mandatory),
        AttentionItem::new("hi", rank("p-hi", 150, 5000), PolicyClass::HighRisk),
    ]
}

/// The intents that MUST still surface under degradation (the mandatory ones).
pub fn degraded_mandatory_intents() -> Vec<&'static str> {
    vec!["m1", "m2"]
}
