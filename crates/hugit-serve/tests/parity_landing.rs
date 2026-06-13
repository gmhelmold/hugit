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
