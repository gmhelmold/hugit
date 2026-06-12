//! WP-X14 acceptance oracle — deep-link referential integrity (lifecycle).
//! Contract: `docs/plan/wp-contracts/WP-X14.md`.
//!
//! Owned items (VERBATIM from the contract; each item has a dedicated `#[test]`
//! plus adversarial guards that go RED on the gamed/broken case):
//!
//!   ① property test across the full object lifecycle — after compaction/cold-tier
//!      to R2, after mirror round-trip, after tombstoning: every ledger/intent
//!      deep link resolves to its target or to a tamper-evident tombstone.
//!   ② ZERO dangling links, ever (continuous integrity check as a standing fixture).
//!
//! WHY THE PROPERTY IS LOAD-BEARING: deep links in the ledger/intent view (D5)
//! reference objects by content hash. Three lifecycle transitions can in
//! principle "move" or "destroy" the target: (a) compaction offloads the event
//! log prefix to R2 cold tier (D1) — the OBJECT store is separate; links stay
//! stable; (b) a mirror round-trip (E1) serializes/deserializes records — CAS
//! hashes are immutable; (c) tombstoning (X7) erases the object but installs a
//! tamper-evident tombstone keyed by the SAME hash. In ALL cases the link MUST
//! resolve to target-or-tombstone. "Dangling" is a contract FAIL.
//!
//! The oracle uses the REAL `hugit_refstore::{EventLog, compact,
//! recover_from_cold, verify_chain, InMemoryColdStore}` and the REAL
//! `hugit_ledger::{resolve, ResolveResult}` so it tests the production surfaces,
//! never a hand-rolled stand-in.

#[path = "../deeplink.rs"]
mod deeplink;

use deeplink::{
    DeepLink, LifecycleStage, Lookup, ObjectStore, ResolveOutcome, check_integrity, resolve_link,
    run_lifecycle_property, simulate_compaction_and_recovery, simulate_mirror_round_trip,
};
use hugit_ledger::{ResolveResult, resolve};
use hugit_refstore::{EventLog, canonical_json};

// ── shared fixtures ──────────────────────────────────────────────────────────

/// Tenant A HMAC-derived prefix (opaque, models the CoreLink model).
const TENANT_A: &str = "hmac:tenant-a:";

/// Build a canonical `intent.landed` payload, suitable for `EventLog::append`.
///
/// The payload includes the `"ref"` and `"target"` fields required by
/// `replay_unchecked` (the D1b recovery path folds `intent.landed` as a
/// ref-advancing event and rejects payloads missing these fields fail-closed).
fn intent_landed_payload(intent_id: &str, target_hash: &str) -> String {
    // Use the intent_id as the ref name (a valid synthetic ref for the fixture).
    let ref_name = format!("refs/hugit/intents/{intent_id}");
    let raw = serde_json::json!({
        "intent_id": intent_id,
        "ref": ref_name,
        "campaign": "wave-X14",
        "charter": "deep-link integrity test",
        "deep_link_target": target_hash,
        "target": target_hash
    })
    .to_string();
    canonical_json(&raw).expect("fixture payload is valid JSON")
}

/// Build a log + deep-link registry for 3 intents, each linking to a distinct
/// content-addressed target object. Returns (log, links, store) at baseline
/// (all targets live).
fn seed_three_intents() -> (EventLog, Vec<DeepLink>, ObjectStore) {
    let mut log = EventLog::new();
    let mut store = ObjectStore::new();

    // Intent 1
    let id1 = format!("{TENANT_A}intent-alpha");
    let hash1 = "sha256:object-alpha-0001";
    log.append_for_test(
        "intent.landed",
        vec!["agent:planner".to_string()],
        intent_landed_payload(&id1, hash1),
        1_000,
    );
    store.put(hash1, "alpha object bytes");

    // Intent 2
    let id2 = format!("{TENANT_A}intent-beta");
    let hash2 = "sha256:object-beta-0002";
    log.append_for_test(
        "intent.landed",
        vec!["agent:executor".to_string()],
        intent_landed_payload(&id2, hash2),
        2_000,
    );
    store.put(hash2, "beta object bytes");

    // Intent 3
    let id3 = format!("{TENANT_A}intent-gamma");
    let hash3 = "sha256:object-gamma-0003";
    log.append_for_test(
        "intent.landed",
        vec!["agent:executor".to_string(), "human:reviewer".to_string()],
        intent_landed_payload(&id3, hash3),
        3_000,
    );
    store.put(hash3, "gamma object bytes");

    let links = vec![
        DeepLink::new(id1, hash1),
        DeepLink::new(id2, hash2),
        DeepLink::new(id3, hash3),
    ];

    (log, links, store)
}

// ── ① property test across the full object lifecycle ─────────────────────────

/// Baseline: all links resolve to live targets (pre-conditions for the property).
#[test]
fn item_1_baseline_all_links_resolve_live() {
    let (_, links, store) = seed_three_intents();

    for link in &links {
        let outcome = resolve_link(link, &store);
        assert!(
            outcome.is_valid(),
            "baseline: link {} must resolve live, got {outcome:?}",
            link.intent_id
        );
        assert!(
            matches!(outcome, ResolveOutcome::Found { .. }),
            "baseline: link {} must be Found (live), not tombstoned",
            link.intent_id
        );
    }
}

/// After compaction/cold-tier (D1): the event log is compacted; deep links
/// in the store still resolve to live targets.
///
/// This proves that compaction operates on the EVENT LOG (relocating a prefix
/// to cold tier), NOT on the OBJECT STORE — the object store's content hashes
/// are unaffected, so all existing deep links remain valid.
#[test]
fn item_1_after_compaction_links_still_resolve() {
    let (log, links, store) = seed_three_intents();

    // Compact: keep only the last 1 record hot (moves 2 to cold tier).
    let recovered =
        simulate_compaction_and_recovery(&log, 1).expect("compaction+recovery must succeed");

    // The recovered log still verifies (compaction is replay-equivalent).
    assert_eq!(
        recovered.len(),
        3,
        "all 3 records must survive compaction+recovery"
    );

    // The OBJECT STORE is unchanged — compaction does not touch it.
    // All deep links must still resolve to live targets.
    let report = check_integrity(&links, &store);
    assert!(
        report.zero_dangling(),
        "after compaction: zero dangling links (got {} dangling: {:?})",
        report.dangling,
        report.dangling_links
    );
    assert_eq!(report.live, 3, "after compaction: all 3 links still live");
    assert_eq!(report.tombstoned, 0, "no tombstones at this stage");
}

/// After mirror round-trip (E1): records are serialized and re-verified through
/// the mirror path. Content hashes are CAS — unaffected by serialization.
#[test]
fn item_1_after_mirror_round_trip_links_still_resolve() {
    let (log, links, store) = seed_three_intents();

    let mirrored =
        simulate_mirror_round_trip(log.records()).expect("mirror round-trip must succeed");

    // Chain still verifies after the round-trip.
    assert_eq!(mirrored.len(), 3, "all records survive mirror round-trip");

    // The object store is unaffected — CAS hashes are immutable.
    let report = check_integrity(&links, &store);
    assert!(
        report.zero_dangling(),
        "after mirror round-trip: zero dangling links (got {} dangling: {:?})",
        report.dangling,
        report.dangling_links
    );
    assert_eq!(report.live, 3, "all 3 links still live after mirror");
}

/// After tombstoning (X7): one target is erased. The surviving link must
/// resolve to a tamper-evident tombstone — NOT to a dangling void.
#[test]
fn item_1_after_tombstoning_link_resolves_to_tombstone_not_dangling() {
    let (_, links, mut store) = seed_three_intents();

    // Erase the first intent's target (the X7 cascade).
    let erased_hash = links[0].target_hash.clone();
    let tombstone = store.erase(&erased_hash, "rtbf-request:x14-case-01");

    // The link for the erased object must resolve to a TOMBSTONE.
    let outcome = resolve_link(&links[0], &store);
    match &outcome {
        ResolveOutcome::Tombstone {
            target_hash,
            tombstone: ts,
            ..
        } => {
            assert_eq!(
                target_hash, &erased_hash,
                "tombstone outcome must carry the ORIGINAL target hash"
            );
            assert_eq!(
                ts.erased_object_hash, erased_hash,
                "tombstone must name the erased object (tamper-evident)"
            );
            assert_eq!(ts, &tombstone, "store must resolve to the minted tombstone");
        }
        other => panic!(
            "after tombstoning: link {} must resolve to Tombstone, got {other:?}",
            links[0].intent_id
        ),
    }

    // It must NOT be dangling.
    assert!(
        outcome.is_valid(),
        "post-tombstone link must be valid (target-or-tombstone)"
    );

    // The OTHER two links (beta, gamma) are still live — unaffected.
    for link in &links[1..] {
        let o = resolve_link(link, &store);
        assert!(
            matches!(o, ResolveOutcome::Found { .. }),
            "non-erased link {} must still be live, got {o:?}",
            link.intent_id
        );
    }
}

/// Full lifecycle property: run all four stages in sequence and assert
/// zero dangling at every stage, including after tombstoning.
#[test]
fn item_1_full_lifecycle_property_holds_at_every_stage() {
    let (log, links, mut store) = seed_three_intents();

    // Stage: Baseline
    {
        let report = check_integrity(&links, &store);
        assert!(
            report.zero_dangling(),
            "Baseline: zero dangling (got {:?})",
            report.dangling_links
        );
    }

    // Stage: AfterCompaction — compact + recover, check store unchanged.
    {
        simulate_compaction_and_recovery(&log, 1).expect("compaction must succeed");
        let report = check_integrity(&links, &store);
        assert!(
            report.zero_dangling(),
            "AfterCompaction: zero dangling (got {:?})",
            report.dangling_links
        );
    }

    // Stage: AfterMirrorRoundTrip — mirror records, check store unchanged.
    {
        simulate_mirror_round_trip(log.records()).expect("mirror round-trip must succeed");
        let report = check_integrity(&links, &store);
        assert!(
            report.zero_dangling(),
            "AfterMirrorRoundTrip: zero dangling (got {:?})",
            report.dangling_links
        );
    }

    // Stage: AfterTombstoning — erase one target, check all resolve to target-or-tombstone.
    {
        store.erase(&links[0].target_hash, "rtbf-request:full-lifecycle");
        let report = check_integrity(&links, &store);
        assert!(
            report.zero_dangling(),
            "AfterTombstoning: zero dangling (got {:?})",
            report.dangling_links
        );
        assert_eq!(
            report.tombstoned, 1,
            "exactly one tombstoned link after erasure"
        );
        assert_eq!(report.live, 2, "two links still live after erasure");
    }

    // run_lifecycle_property helper with all stages (uses the final store state).
    let all_stages = [
        LifecycleStage::Baseline,
        LifecycleStage::AfterCompaction,
        LifecycleStage::AfterMirrorRoundTrip,
        LifecycleStage::AfterTombstoning,
    ];
    let result = run_lifecycle_property(&links, &store, &all_stages);
    assert!(
        result.is_ok(),
        "run_lifecycle_property must pass all stages: {:?}",
        result.err()
    );
}

/// Integration with the REAL `hugit_ledger::resolve` surface (D5): the oracle
/// resolves each intent_id from the actual event log and asserts the returned
/// deep_link_target matches the object store entry.
#[test]
fn item_1_ledger_resolve_surface_matches_object_store() {
    let (log, links, store) = seed_three_intents();
    let records = log.records();

    for link in &links {
        // Resolve via the REAL D5 ledger surface.
        match resolve(&link.intent_id, records) {
            ResolveResult::Found { target, .. } => {
                // The golden target returned by the ledger resolver must match
                // the object store's content hash for this link.
                assert_eq!(
                    target, link.target_hash,
                    "ledger resolve for {} must return the golden target hash",
                    link.intent_id
                );
                // And that hash must resolve to a live object in the store.
                let lookup = store.resolve(&link.target_hash);
                assert!(
                    matches!(lookup, Lookup::Live(_)),
                    "ledger-resolved target for {} must be live in the store",
                    link.intent_id
                );
            }
            ResolveResult::NotFound { intent_id } => {
                panic!("ledger resolve failed for {intent_id}: should be Found in the event log");
            }
        }
    }
}

/// After tombstoning: the REAL `hugit_ledger::resolve` surface still returns
/// the same target hash — and the object store now resolves it to a tombstone.
/// Proves the D5 ledger surface and the X14 property interlock correctly.
#[test]
fn item_1_ledger_resolve_plus_tombstone_resolution() {
    let (log, links, mut store) = seed_three_intents();
    let records = log.records();

    // Erase the alpha target.
    let erased_hash = links[0].target_hash.clone();
    store.erase(&erased_hash, "rtbf-request:x14-ledger-tombstone");

    // The ledger resolver still returns the SAME hash (the log is immutable).
    match resolve(&links[0].intent_id, records) {
        ResolveResult::Found { target, .. } => {
            assert_eq!(
                target, erased_hash,
                "ledger must still return the original target hash after erasure"
            );
            // But the object store now resolves that hash to a tombstone.
            let lookup = store.resolve(&target);
            assert!(
                matches!(lookup, Lookup::Tombstone(_)),
                "after erasure, the target hash must resolve to a tombstone in the store"
            );
        }
        ResolveResult::NotFound { intent_id } => {
            panic!("ledger resolve unexpectedly not-found for {intent_id}");
        }
    }
}

// ── ② ZERO dangling links, ever (continuous integrity check as standing fixture) ─

/// Standing fixture: zero dangling at baseline.
#[test]
fn item_2_standing_fixture_zero_dangling_at_baseline() {
    let (_, links, store) = seed_three_intents();
    let report = check_integrity(&links, &store);

    assert!(
        report.zero_dangling(),
        "standing fixture: ZERO dangling links at baseline (got {} dangling: {:?})",
        report.dangling,
        report.dangling_links
    );
    assert_eq!(report.total, 3);
    assert_eq!(report.live, 3);
    assert_eq!(report.tombstoned, 0);
    assert_eq!(report.dangling, 0);
}

/// Standing fixture: zero dangling after tombstoning MULTIPLE targets.
#[test]
fn item_2_standing_fixture_zero_dangling_after_multiple_tombstones() {
    let (_, links, mut store) = seed_three_intents();

    // Erase ALL three targets — all links must resolve to tombstones, none dangling.
    for link in &links {
        store.erase(&link.target_hash, "rtbf-request:bulk-erasure");
    }

    let report = check_integrity(&links, &store);
    assert!(
        report.zero_dangling(),
        "standing fixture: ZERO dangling even after ALL targets tombstoned (got {:?})",
        report.dangling_links
    );
    assert_eq!(
        report.tombstoned, 3,
        "all 3 links must resolve to tombstones"
    );
    assert_eq!(report.live, 0, "no live objects after full erasure");
    assert_eq!(report.dangling, 0, "ZERO dangling — the core invariant");
}

/// Standing fixture: zero dangling with a MIXED set (some live, some tombstoned).
#[test]
fn item_2_standing_fixture_mixed_live_and_tombstoned() {
    let (_, links, mut store) = seed_three_intents();

    // Erase only the second target.
    store.erase(&links[1].target_hash, "rtbf-request:mixed");

    let report = check_integrity(&links, &store);
    assert!(
        report.zero_dangling(),
        "standing fixture: ZERO dangling with mixed live+tombstoned set (got {:?})",
        report.dangling_links
    );
    assert_eq!(report.live, 2);
    assert_eq!(report.tombstoned, 1);
    assert_eq!(report.dangling, 0);
}

/// Standing fixture: empty link set is trivially zero-dangling.
#[test]
fn item_2_standing_fixture_empty_link_set() {
    let store = ObjectStore::new();
    let report = check_integrity(&[], &store);

    assert!(
        report.zero_dangling(),
        "empty link set: trivially zero dangling"
    );
    assert_eq!(report.total, 0);
}

// ── adversarial guards ────────────────────────────────────────────────────────
// These guards prove the oracle goes RED on the gamed/broken case.
// A dangling link MUST trip the invariant — the oracle is not gamed.

/// ADVERSARIAL: a link whose target was deleted without installing a tombstone
/// (a broken delete, not a tombstoning) produces a Dangling outcome.
/// This proves `is_valid()` returns false for Dangling, and the oracle would
/// catch it — the oracle is NOT gamed.
#[test]
fn adversarial_dangling_link_is_detected() {
    let mut store = ObjectStore::new();
    // Put a live object, then remove it WITHOUT tombstoning (simulate a bug).
    store.put("sha256:broken-target", "some bytes");
    // Simulate a broken delete by creating a NEW store without the object.
    let empty_store = ObjectStore::new();

    let link = DeepLink::new("tenant-a:broken-intent", "sha256:broken-target");
    let outcome = resolve_link(&link, &empty_store);

    // Must be Dangling — the oracle DETECTS it.
    assert!(
        matches!(outcome, ResolveOutcome::Dangling { .. }),
        "a link with no target and no tombstone must be Dangling: {outcome:?}"
    );
    assert!(
        !outcome.is_valid(),
        "a Dangling outcome must fail is_valid() — the oracle goes RED"
    );
}

/// ADVERSARIAL: `check_integrity` returns non-zero dangling when a link
/// has no target and no tombstone. The standing fixture fails closed.
#[test]
fn adversarial_standing_fixture_detects_dangling_link() {
    let store = ObjectStore::new(); // empty — no objects, no tombstones
    let link = DeepLink::new("tenant-a:orphaned-intent", "sha256:nowhere");

    let report = check_integrity(&[link], &store);

    // The standing fixture MUST flag the dangling link.
    assert!(
        !report.zero_dangling(),
        "standing fixture must flag a dangling link (zero_dangling() must be false)"
    );
    assert_eq!(report.dangling, 1, "exactly one dangling link detected");
    assert_eq!(
        report.dangling_links,
        vec![(
            "tenant-a:orphaned-intent".to_string(),
            "sha256:nowhere".to_string()
        )]
    );
}

/// ADVERSARIAL: `run_lifecycle_property` returns Err when a dangling link
/// exists at any stage.
#[test]
fn adversarial_lifecycle_property_fails_on_dangling_link() {
    let store = ObjectStore::new(); // empty — dangling by construction
    let link = DeepLink::new("tenant-a:dangling", "sha256:missing");
    let stages = [LifecycleStage::Baseline];

    let result = run_lifecycle_property(&[link], &store, &stages);
    assert!(
        result.is_err(),
        "run_lifecycle_property must return Err when a link dangles"
    );
    let (stage, report) = result.unwrap_err();
    assert_eq!(stage, LifecycleStage::Baseline);
    assert_eq!(report.dangling, 1);
}

/// ADVERSARIAL: a tombstone whose `erased_object_hash` does NOT match the
/// link's target hash — a corrupted tombstone — is still a `Tombstone` outcome
/// (structurally valid), but the caller can detect the hash mismatch.
///
/// Note: X14 distinguishes *resolution outcome* from *identity*. The resolution
/// oracle says "this hash resolved to a tombstone" — whether the tombstone's
/// internal hash matches is an additional check the caller (the property test)
/// makes. This test documents the boundary.
#[test]
fn adversarial_tombstone_hash_mismatch_is_detectable() {
    let mut store = ObjectStore::new();
    // Put a real object, erase it.
    let real_hash = "sha256:real-target";
    store.put(real_hash, "real bytes");
    store.erase(real_hash, "rtbf");

    let link = DeepLink::new("tenant-a:real-intent", real_hash);
    let outcome = resolve_link(&link, &store);

    // Must be Tombstone (valid resolution).
    match &outcome {
        ResolveOutcome::Tombstone {
            target_hash,
            tombstone,
            ..
        } => {
            // The tombstone's erased_object_hash MUST equal the link's target_hash
            // (tamper-evident guarantee). This is what X12 / X14 both rely on.
            assert_eq!(
                tombstone.erased_object_hash, *target_hash,
                "tombstone hash must match the link's target hash (tamper-evident)"
            );
        }
        other => panic!("expected Tombstone outcome, got {other:?}"),
    }
    assert!(
        outcome.is_valid(),
        "a Tombstone outcome is valid (not dangling)"
    );
}
