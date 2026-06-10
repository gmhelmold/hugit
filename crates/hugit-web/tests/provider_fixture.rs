//! Parity oracle for the seeded fixture world (WP-W1).
//!
//! The fixture world is built by running hugit's REAL dogfood wave and
//! projecting the resulting event log through the REAL projections
//! (`hugit_ledger::Ledger` / `hugit_refstore::intent`). This suite re-derives
//! those same projections over the SAME world records the provider exposes and
//! holds the view-models to them — so a fabricated number cannot pass. It also
//! pins the MEASURED checks numbers to the wave and verifies the world's event
//! chain through the real `verify_chain`.
//!
//! The provider exposes `#[doc(hidden)]` test seams (`world_records`,
//! `wave_cold_measured_exec_ms`, `wave_warm_executions`, `world_intent_ids`) —
//! allowed: `fixture.rs` is W1's owned file.

use hugit_ledger::Ledger;
use hugit_refstore::intent::intents_from_log;
use hugit_refstore::{EventLog, verify_chain};
use hugit_web::fixture::FixtureProvider;
use hugit_web::provider::{LandingItemVm, Provider};

/// Rebuild an `EventLog` from a record slice through the public `push_record`
/// path so we can run `intents_from_log` over the SAME records the provider
/// built its VMs from.
fn log_from_records(records: &[hugit_contracts::event_record::EventRecord]) -> EventLog {
    let mut log = EventLog::new();
    for r in records {
        log.push_record(r.clone())
            .expect("world records are a gap-free, monotonic chain");
    }
    log
}

/// 1 — the Ledger numbers on the Insights VM EQUAL the real projection over the
/// same world records, and proven rows correspond to entries with proven=true.
#[test]
fn ledger_numbers_match_the_real_projection() {
    let provider = FixtureProvider::seed();
    let insights = provider.insights("hugit").expect("insights served");

    // Recompute the Ledger over the SAME world records.
    let ledger = Ledger::from_records(provider.world_records());

    // The world must actually carry landed intents + at least one proven one,
    // otherwise this oracle proves nothing.
    assert!(
        !ledger.entries().is_empty(),
        "world ledger must have entries"
    );
    assert!(
        ledger.entries().iter().any(|e| e.proven),
        "world ledger must have at least one proven entry"
    );

    // Every campaign group on the VM must match the Ledger's per-campaign
    // asked/done/proven exactly.
    for camp in &insights.ledger.campaigns {
        let cid = &camp.campaign.id;
        assert_eq!(
            camp.asked,
            ledger.asked(cid),
            "asked mismatch for campaign {cid}"
        );
        assert_eq!(
            camp.done,
            ledger.done(cid),
            "done mismatch for campaign {cid}"
        );
        assert_eq!(
            camp.proven,
            ledger.proven(cid),
            "proven mismatch for campaign {cid}"
        );

        // A row is marked "provado" iff its underlying ledger entry is proven.
        for row in &camp.rows {
            let entry = ledger
                .entries()
                .iter()
                .find(|e| e.intent_id == row.intent_id)
                .unwrap_or_else(|| panic!("ledger entry for {} must exist", row.intent_id));
            let row_proven = row.proven_status == "provado";
            assert_eq!(
                row_proven, entry.proven,
                "proven_status for {} must agree with the projection",
                row.intent_id
            );
            // A proven row carries the verdict outcome; an unproven one does not.
            assert_eq!(
                row.verdict.is_some(),
                entry.proven,
                "verdict presence for {} must follow proven",
                row.intent_id
            );
        }
    }

    // The VM's total proven count equals the projection's total proven count.
    let vm_proven: usize = insights.ledger.campaigns.iter().map(|c| c.proven).sum();
    let proj_proven = ledger.entries().iter().filter(|e| e.proven).count();
    assert_eq!(
        vm_proven, proj_proven,
        "total proven on the VM must equal the projection"
    );
}

/// 2 — every projected intent is served by `intent("hugit", id)` with the SAME
/// charter, and the landed cards reference exactly those intent ids in their
/// drawers.
#[test]
fn intents_match_the_real_projection() {
    let provider = FixtureProvider::seed();
    let log = log_from_records(provider.world_records());
    let projected = intents_from_log(&log).expect("world log projects");

    assert!(
        !projected.intents().is_empty(),
        "world must project at least one intent"
    );

    // Every projected intent is served, with the same charter.
    for intent in projected.intents() {
        let detail = provider
            .intent("hugit", &intent.intent_id)
            .unwrap_or_else(|| panic!("intent {} must be served", intent.intent_id));
        assert_eq!(detail.id, intent.intent_id);
        assert_eq!(
            detail.charter, intent.charter,
            "served charter for {} must equal the projected charter",
            intent.intent_id
        );
        // The detail snapshot tree is the target oid from the landed event.
        assert_eq!(
            detail.snapshot.tree, intent.target,
            "snapshot tree for {} must be the landed target oid",
            intent.intent_id
        );
    }

    // The provider's reported world intent ids match the projection exactly.
    let projected_ids: Vec<String> = projected
        .intents()
        .iter()
        .map(|i| i.intent_id.clone())
        .collect();
    assert_eq!(
        provider.world_intent_ids(),
        projected_ids,
        "provider world intent ids must equal the projection (order included)"
    );

    // Landing's landed cards reference exactly the projected intent ids in their
    // drawers (the Pousado hoje column, bundles flattened).
    let landing = provider.landing("hugit").expect("landing served");
    let pousado = landing
        .columns
        .iter()
        .find(|c| c.title == "Pousado hoje")
        .expect("Pousado hoje column present");
    let mut card_intent_ids: Vec<String> = Vec::new();
    for item in &pousado.items {
        match item {
            LandingItemVm::Card(card) => {
                for ix in &card.drawer.intents {
                    card_intent_ids.push(ix.id.clone());
                }
            }
            LandingItemVm::Bundle { cards, .. } => {
                for card in cards {
                    for ix in &card.drawer.intents {
                        card_intent_ids.push(ix.id.clone());
                    }
                }
            }
        }
    }
    card_intent_ids.sort();
    let mut expected_ids = projected_ids.clone();
    expected_ids.sort();
    assert_eq!(
        card_intent_ids, expected_ids,
        "landed cards must reference exactly the projected intent ids"
    );
}

/// 3 — the checks numbers are MEASURED from the wave, not fabricated: saved_ms
/// equals wave A's cold measured exec time, warm executions are 0, the warm
/// hit-rate is 100%, and the shape string matches the hit_rate.rs vocabulary.
#[test]
fn checks_numbers_are_measured_not_fabricated() {
    let provider = FixtureProvider::seed();
    let checks = provider.checks("hugit").expect("checks served");

    // saved_ms is the wave's cold measured execution time (stored on the
    // provider), never a static constant.
    assert_eq!(
        checks.kpis.saved_ms,
        provider.wave_cold_measured_exec_ms(),
        "saved_ms must equal the wave's MEASURED cold exec time"
    );
    // A cold wave actually executed checks, so the figure is non-trivial.
    assert!(
        checks.kpis.saved_ms > 0,
        "the cold wave must have measured a non-zero exec time"
    );

    // The warm pass executed zero checks (the memoization wedge).
    assert_eq!(
        provider.wave_warm_executions(),
        0,
        "the warm pass must execute 0 checks (the wedge)"
    );

    // Warm pass = all hits → 100% hit-rate.
    assert_eq!(
        checks.kpis.hit_rate_pct, 100.0,
        "warm-pass hit-rate must be a measured 100%"
    );

    // The shape label uses the hit_rate.rs vocabulary, and for an all-hit warm
    // pass it is FULL.
    assert!(
        matches!(
            checks.kpis.shape.as_str(),
            "FULL" | "PARTIAL" | "NONE" | "NO DATA"
        ),
        "shape must be one of the hit_rate.rs labels; got {:?}",
        checks.kpis.shape
    );
    assert_eq!(
        checks.kpis.shape, "FULL",
        "an all-hit warm pass is FULL per hit_rate.rs"
    );

    // KPI counts: executed = cold executions; hits = warm hits. The cold wave
    // landed 5 PRs each with one check → 5 executions, 5 warm hits.
    assert_eq!(checks.kpis.executed, 5, "5 checks executed cold");
    assert_eq!(checks.kpis.hits, 5, "5 checks hit warm");

    // The rows tell the wedge story: every cold row executed (no cache hit) and
    // every warm row is an AC hit, all green, with a real 64-hex memo key.
    let cold_rows = checks.checks.iter().filter(|r| !r.cache_hit).count();
    let warm_rows = checks.checks.iter().filter(|r| r.cache_hit).count();
    assert_eq!(cold_rows, 5, "5 cold (executed) rows");
    assert_eq!(warm_rows, 5, "5 warm (cache-hit) rows");
    for r in &checks.checks {
        assert!(r.ok, "every check row is green");
        assert_eq!(r.memo_key.len(), 64, "memo key is a 64-hex digest");
        assert!(
            r.memo_key.chars().all(|c| c.is_ascii_hexdigit()),
            "memo key is lowercase hex"
        );
    }
}

/// 4 — the world event chain verifies clean (it was built through the real
/// append path).
#[test]
fn event_chain_verifies() {
    let provider = FixtureProvider::seed();
    verify_chain(provider.world_records())
        .expect("the world log was built through the real append path; it must verify");

    // And it actually carries events (an empty chain trivially verifies).
    assert!(
        !provider.world_records().is_empty(),
        "the world log must carry events"
    );
}

/// 5 — unknown repo and unknown intent are None across every route.
#[test]
fn unknown_repo_and_intent_are_none() {
    let provider = FixtureProvider::seed();

    // Unknown repo on every route.
    assert!(provider.repo_home("nope").is_none());
    assert!(provider.landing("nope").is_none());
    assert!(provider.checks("nope").is_none());
    assert!(provider.insights("nope").is_none());
    assert!(provider.intent("nope", "intent-pr-0").is_none());

    // Unknown intent on the known repo.
    assert!(provider.intent("hugit", "no-such-intent").is_none());

    // The standing smoke id MUST exist in the world.
    assert!(
        provider.intent("hugit", "intent-pr-0").is_some(),
        "intent-pr-0 must be served (the app_smoke oracle depends on it)"
    );
}
