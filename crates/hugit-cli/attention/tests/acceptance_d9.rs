//! WP-D9 acceptance tests — attention queue.
//!
//! Contract: the work-package contract.
//! Oracle:   tests/acceptance/wp-d9/run.sh.
//!
//! Naming convention (binding, pre-decided by the lead):
//!   item_<n>_<slug>
//!
//! PURE / fixture proofs only — composite ordering, the mandatory floor, the
//! fast-approve gate, and the degraded up-zoom are deterministic functions over
//! local fixtures (see `hugit_cli::attention::fixtures`). No live inputs.

use hugit_cli::attention::degraded::{DegradedInputs, RankingState, surface_degraded};
use hugit_cli::attention::fast_approve::{BlockReason, FAST_APPROVE_WINDOW_SECS, evaluate};
use hugit_cli::attention::fixtures;
use hugit_cli::attention::rank::{AttentionQueue, PolicyClass};

// ─── item ①: documented composite ordering reproduced ─────────────────────────

/// ① A known policy/blast/confidence fixture reproduces the EXACT documented
/// composite ordering (see the score definition in `attention::rank`).
#[test]
fn item_1_composite_ordering_reproduced() {
    let queue = AttentionQueue::rank(fixtures::baseline_items());
    assert_eq!(
        queue.order(),
        fixtures::baseline_expected_order(),
        "documented composite ordering must be reproduced exactly"
    );

    // The order is driven by the hand-computed scores, not insertion order.
    let scores: Vec<u64> = queue.items.iter().map(|i| i.score()).collect();
    let mut sorted = scores.clone();
    sorted.sort_unstable_by(|a, b| b.cmp(a));
    assert_eq!(
        scores, sorted,
        "queue must be in descending composite score"
    );
}

// ─── item ②: perturbing one input moves the entry ─────────────────────────────

/// ② Perturbing a SINGLE input (calm's blast-radius 5 → 400) moves that entry
/// from last to its expected new position — and nothing else's relative order
/// changes beyond what that one input dictates.
#[test]
fn item_2_perturbation_moves_entry() {
    let before = AttentionQueue::rank(fixtures::baseline_items());
    assert_eq!(before.position_of("calm"), Some(3), "calm starts last");

    let after = AttentionQueue::rank(fixtures::perturbed_items());
    assert_eq!(
        after.order(),
        fixtures::perturbed_expected_order(),
        "one-input perturbation must move the entry to its expected position"
    );
    assert_eq!(
        after.position_of("calm"),
        Some(1),
        "calm must move from last to second after its blast-radius jumps"
    );
}

// ─── item ③: policy-mandatory floor — never ranked out of view ─────────────────

/// ③ A policy-mandatory item with the WORST attention inputs still outranks many
/// non-mandatory items with the BEST attention inputs — it can never be ranked
/// out of the human's view.
#[test]
fn item_3_mandatory_floor_never_ranked_out() {
    let queue = AttentionQueue::rank(fixtures::mandatory_floor_items());

    assert_eq!(
        queue.position_of("must-see"),
        Some(0),
        "policy-mandatory item must sit at the top despite minimal attention inputs"
    );
    assert!(
        !queue.mandatory_ranked_out(),
        "no policy-mandatory item may be ranked out of view"
    );

    // Every mandatory item must outscore every non-mandatory item, structurally.
    let min_mandatory = queue
        .items
        .iter()
        .filter(|i| i.class == PolicyClass::Mandatory)
        .map(|i| i.score())
        .min()
        .expect("fixture has a mandatory item");
    let max_non_mandatory = queue
        .items
        .iter()
        .filter(|i| i.class != PolicyClass::Mandatory)
        .map(|i| i.score())
        .max()
        .expect("fixture has non-mandatory items");
    assert!(
        min_mandatory > max_non_mandatory,
        "mandatory floor must dominate the maximum non-mandatory score"
    );
}

// ─── item ④ (R2): fast-approve gate — block high-risk/mandatory, permit low ────

/// ④ The 90s fast-approve affordance is BLOCKED for high-risk and
/// policy-mandatory items (forced through full review) and PERMITTED only for
/// policy-low-risk items. Both the block and permit paths are asserted.
#[test]
fn item_4_fast_approve_blocked_high_risk_mandatory_permitted_low_risk() {
    // permit path — policy-low-risk.
    let low = evaluate(&fixtures::low_risk_item());
    assert!(
        low.is_permitted(),
        "policy-low-risk must be fast-approvable"
    );
    match low {
        hugit_cli::attention::fast_approve::FastApprove::Permitted { window_secs } => {
            assert_eq!(window_secs, FAST_APPROVE_WINDOW_SECS, "the 90s window");
            assert_eq!(window_secs, 90);
        }
        other => panic!("expected Permitted, got {other:?}"),
    }

    // block path — high-risk.
    let high = evaluate(&fixtures::high_risk_item());
    assert!(
        high.is_blocked(),
        "high-risk must be barred from fast-approve"
    );
    assert!(matches!(
        high,
        hugit_cli::attention::fast_approve::FastApprove::Blocked {
            reason: BlockReason::HighRisk
        }
    ));

    // block path — policy-mandatory.
    let mand = evaluate(&fixtures::mandatory_item());
    assert!(
        mand.is_blocked(),
        "policy-mandatory must be barred from fast-approve"
    );
    assert!(matches!(
        mand,
        hugit_cli::attention::fast_approve::FastApprove::Blocked {
            reason: BlockReason::PolicyMandatory
        }
    ));
}

// ─── item ⑤ (R7): up-zoom under degradation — mandatory still surfaces ─────────

/// ⑤ With ranking inputs (blast/confidence) unavailable, policy-mandatory items
/// STILL surface, each carrying an honest "ranking degraded" state — the queue
/// never goes silently dark on items that must reach a human.
#[test]
fn item_5_degradation_mandatory_still_surface_labeled() {
    let degraded = DegradedInputs {
        blast_unavailable: true,
        confidence_unavailable: true,
    };
    assert!(degraded.is_degraded());

    let surfaced = surface_degraded(fixtures::degraded_mix_items(), degraded);

    // every mandatory item still surfaces — the queue is not dark.
    let surfaced_intents: Vec<&str> = surfaced.iter().map(|s| s.item.intent.as_str()).collect();
    for must in fixtures::degraded_mandatory_intents() {
        assert!(
            surfaced_intents.contains(&must),
            "policy-mandatory item {must} must still surface under degradation"
        );
    }

    // every surfaced item is honestly labeled "ranking degraded".
    assert!(
        !surfaced.is_empty(),
        "degraded queue must not go silently dark"
    );
    for s in &surfaced {
        assert!(
            s.state.is_degraded(),
            "surfaced item must carry degraded state"
        );
        match &s.state {
            RankingState::Degraded { label, inputs } => {
                assert_eq!(*label, "ranking degraded", "honest degraded label");
                assert!(inputs.blast_unavailable && inputs.confidence_unavailable);
            }
            RankingState::Ranked => panic!("expected Degraded state, got Ranked"),
        }
    }

    // only mandatory items are surfaced (non-mandatory withheld vs. false order).
    for s in &surfaced {
        assert_eq!(
            s.item.class,
            PolicyClass::Mandatory,
            "non-mandatory items must not be shown in a false degraded order"
        );
    }
}
