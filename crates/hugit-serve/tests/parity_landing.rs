//! Parity test for the landing handler (`build_landing`).
//!
//! Asserts: (1) on an EMPTY event log the VM serializes and round-trips through
//! the frozen `hugit_http_contracts::LandingVm`; (2) the honest defaults hold
//! (zero PRs across all columns, `main_green == false`, `list_groups` empty);
//! (3) the externally-tagged `LandingItemVm` round-trips when constructed.

use hugit_http_contracts::common::{CampaignChipVm, CostVm, MirrorVm, UnionVm};
use hugit_http_contracts::landing::{ChecksBadgeVm, LandingItemVm, PrCardVm, PrDrawerVm, PrState};
use hugit_http_contracts::{DiffVm, LandingVm};
use hugit_refstore::EventLog;
use hugit_serve::handlers::landing::build_landing;

/// Append a RAW record of `kind` with a JSON `payload` via the `test-support`
/// door (the repo-standard synthetic-fixture seam). Raw on purpose: it bypasses
/// the write-path scrub so the read-path scrub is proven in isolation
/// (defence-in-depth).
fn push(log: &mut EventLog, kind: &str, payload: serde_json::Value, at: u64) {
    log.append_for_test(kind, vec!["test".to_string()], payload.to_string(), at);
}

/// Collect every lone `PrCardVm` across all columns of a `LandingVm`.
fn all_cards(vm: &LandingVm) -> Vec<&PrCardVm> {
    vm.columns
        .iter()
        .flat_map(|c| c.items.iter())
        .filter_map(|item| match item {
            LandingItemVm::Card(c) => Some(c.as_ref()),
            _ => None,
        })
        .collect()
}

#[test]
fn empty_log_landing_round_trips_and_is_honest() {
    let log = EventLog::new();
    let vm = build_landing(&log, "hugit");

    // ── REAL fields reflect the empty log ──────────────────────────────────
    assert_eq!(vm.repo, "hugit");
    assert_eq!(vm.open_count, 0, "no opened PRs on an empty log");
    assert_eq!(vm.merged_count, 0, "no landed PRs on an empty log");
    assert!(
        vm.campaigns.is_empty(),
        "no campaign.opened on an empty log"
    );

    // Every column renders, all empty (zero PRs).
    let total_items: usize = vm.columns.iter().map(|c| c.items.len()).sum();
    assert_eq!(total_items, 0, "no PR cards in any column on an empty log");

    // ── STUB / honest defaults ─────────────────────────────────────────────
    assert!(
        !vm.main_green,
        "main_green is the honest false (no main-CI seam)"
    );
    assert_eq!(vm.main_status, "", "main_status is empty (no main-CI seam)");
    assert_eq!(vm.draft_count, 0, "no draft state in the engine");
    assert!(vm.list_groups.is_empty(), "no fleet grouping seam");

    // ── Round-trip through the frozen contract type ─────────────────────────
    let json = serde_json::to_string(&vm).expect("LandingVm serializes");
    let reparsed: LandingVm =
        serde_json::from_str(&json).expect("LandingVm deserializes from its own JSON");
    assert_eq!(vm, reparsed, "LandingVm round-trip is lossless");
}

/// SECRET-MATRIX: a `campaign.opened` whose campaign id is secret-shaped is
/// scrubbed by the read path (`build_landing`) before it reaches the chip —
/// defence-in-depth: the read scrub fires even on a RAW (un-write-scrubbed) log.
#[test]
fn campaign_chip_id_and_label_are_scrubbed() {
    let mut log = EventLog::new();
    // A GitHub PAT shape echoed RAW into a campaign id (the detector's canonical
    // classic-PAT specimen).
    let secret = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
    push(
        &mut log,
        "campaign.opened",
        serde_json::json!({ "campaign": secret }),
        1_000,
    );

    let vm = build_landing(&log, "hugit");

    assert_eq!(vm.campaigns.len(), 1, "one campaign chip projected");
    let chip = &vm.campaigns[0];
    assert!(
        chip.id.contains("[REDACTED]"),
        "campaign chip id is scrubbed, got: {}",
        chip.id
    );
    assert!(
        chip.label.contains("[REDACTED]"),
        "campaign chip label is scrubbed, got: {}",
        chip.label
    );
    assert!(
        chip.display_label.contains("[REDACTED]"),
        "campaign chip display_label is scrubbed, got: {}",
        chip.display_label
    );
    assert!(
        !chip.id.contains("ghp_"),
        "the raw PAT never reaches the chip id"
    );
}

/// POPULATED LOG: a real `pr.opened` (+ a matching `campaign.opened`) projects a
/// real `PrCardVm` into a column with the real number/state/campaign, and the
/// full `LandingVm` round-trips through the frozen contract.
#[test]
fn populated_log_projects_a_real_card() {
    let mut log = EventLog::new();
    push(
        &mut log,
        "campaign.opened",
        serde_json::json!({ "campaign": "wave-x" }),
        1_000,
    );
    push(
        &mut log,
        "pr.opened",
        serde_json::json!({
            "pr_id": "42",
            "campaign": "wave-x",
            "author_kind": "orchestrator",
            "intent_ids": ["i1", "i2"],
            "principal": serde_json::Value::Null,
            "run_id": "r-1",
        }),
        2_000,
    );

    let vm = build_landing(&log, "hugit");

    // Counts reflect the one open (non-terminal) PR.
    assert_eq!(vm.open_count, 1, "one opened, non-terminal PR");
    assert_eq!(vm.merged_count, 0, "nothing landed");

    // The campaign chip is real (no secret → not scrubbed).
    assert_eq!(vm.campaigns.len(), 1);
    assert_eq!(vm.campaigns[0].id, "wave-x");

    // Exactly one real card, with the real number/state/campaign.
    let cards = all_cards(&vm);
    assert_eq!(cards.len(), 1, "one PR card projected across the columns");
    let card = cards[0];
    assert_eq!(card.number, 42, "numeric pr_id → card number");
    assert_eq!(card.state, PrState::Open, "opened, not queued/landed");
    assert_eq!(card.intent_count, 2, "two bundled intents");
    assert_eq!(
        card.campaign.as_ref().map(|c| c.id.as_str()),
        Some("wave-x"),
        "the card carries its campaign chip"
    );

    // The card sits in the "Na fila" column (Open → Na fila).
    let na_fila = vm
        .columns
        .iter()
        .find(|c| c.title == "Na fila")
        .expect("Na fila column exists");
    assert_eq!(na_fila.items.len(), 1, "the card is in Na fila");

    // Full round-trip through the frozen contract.
    let json = serde_json::to_string(&vm).expect("LandingVm serializes");
    let reparsed: LandingVm = serde_json::from_str(&json).expect("LandingVm deserializes");
    assert_eq!(vm, reparsed, "populated LandingVm round-trip is lossless");
}

/// The externally-tagged `LandingItemVm::Card` round-trips (`{"Card":{…}}`).
#[test]
fn landing_item_card_round_trips_externally_tagged() {
    let card = PrCardVm {
        number: 7,
        title: String::new(),
        author: String::new(),
        model: String::new(),
        campaign: Some(CampaignChipVm {
            id: "wave-x".to_string(),
            label: "wave-x".to_string(),
            color_class: String::new(),
            display_label: "wave-x".to_string(),
        }),
        intent_count: 2,
        file_count: 0,
        checks: ChecksBadgeVm {
            passed: 0,
            total: 0,
            cache_hits: 0,
        },
        state: PrState::Queued,
        stack: None,
        date: "há 2h".to_string(),
        list_badge: "na fila #1".to_string(),
        change_id: String::new(),
        landed_ago: String::new(),
        drawer: PrDrawerVm {
            union: UnionVm {
                batch: vec![],
                verdict: String::new(),
                green: false,
            },
            cost: CostVm {
                tokens_total: 0,
                usd: 0.0,
                model_breakdown: vec![],
                cache_savings: String::new(),
            },
            mirror: MirrorVm {
                synced: false,
                detail: String::new(),
            },
            intents: vec![],
            files: vec![],
            conflict_note: None,
            summary_diff: None,
        },
    };
    let item = LandingItemVm::Card(Box::new(card));

    let json = serde_json::to_string(&item).expect("LandingItemVm serializes");
    assert!(
        json.starts_with("{\"Card\":"),
        "externally tagged as Card: {json}"
    );
    let reparsed: LandingItemVm = serde_json::from_str(&json).expect("LandingItemVm round-trips");
    assert_eq!(
        item, reparsed,
        "externally-tagged Card round-trip is lossless"
    );

    // A DiffVm atom (used inside the drawer) is reachable from the crate root.
    let _ = DiffVm {
        files: vec![],
        hunks: vec![],
    };
}

// ── F5: queue_position and eta_seconds ────────────────────────────────────────

/// F5: empty log → `queue_position` is `None` (no queued PRs).
/// `eta_seconds` is always `None` (no estimator seam exists).
#[test]
fn empty_log_queue_position_none_eta_none() {
    let log = EventLog::new();
    let vm = build_landing(&log, "hugit");
    assert_eq!(
        vm.queue_position, None,
        "queue_position is None when no PRs are queued"
    );
    assert_eq!(
        vm.eta_seconds, None,
        "eta_seconds is always None (no estimator seam)"
    );
}

/// F5: a PR that is queued via `pr.queued` → `queue_position` is `Some(1)` (the
/// head of the active queue). `eta_seconds` stays `None` (honest).
#[test]
fn queued_pr_makes_queue_position_some() {
    let mut log = EventLog::new();

    // Open a PR.
    push(
        &mut log,
        "pr.opened",
        serde_json::json!({
            "pr_id": "42",
            "campaign": "wave-f5",
            "intent_ids": ["i-1"],
            "title": "test pr"
        }),
        1_000,
    );

    // Queue the PR (pr.queued — the `all_pr_queued` seam reads this record kind).
    push(
        &mut log,
        "pr.queued",
        serde_json::json!({
            "pr_id": "42",
            "item_id": "42#0",
            "order_index": 0,
            "mode": "union"
        }),
        2_000,
    );

    let vm = build_landing(&log, "hugit");
    assert_eq!(
        vm.queue_position,
        Some(1),
        "queue_position is Some(1) when one PR is queued"
    );
    assert_eq!(
        vm.eta_seconds, None,
        "eta_seconds stays None (no estimator)"
    );
}
