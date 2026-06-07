//! WP-X13 acceptance oracle — legibility × degradation/erasure.
//! Contract: `docs/plan/wp-contracts/WP-X13.md`.
//!
//! Owned items (VERBATIM from the contract; `#[test] item_<n>_…` each, plus
//! adversarial guards proving each oracle goes RED on the gamed/broken case):
//!
//!   ① with the intelligence layer DEGRADED: the human's down-zoom (raw-commit
//!      view, deep links, `why`) still resolves via plain git OR fails HONESTLY
//!      (explicit "layer unavailable"), never a silent 404/blank
//!   ② after an erasure cascade: following any chain reaches an honest tombstone,
//!      never a broken link — the human can ALWAYS follow, in every substrate
//!      state
//!
//! WHY THE COMPOSITION IS LOAD-BEARING: X11 owns degradation, X7 the erasure
//! cascade, X14 deep-link integrity, D5/D10/D2 the legibility surfaces. X13
//! proves they COMPOSE into the HUMAN-following property: under a degraded
//! intelligence layer the down-zoom never goes silently blank, and after an
//! erasure cascade following any chain never hits a dangling link. The oracle
//! drives the REAL canonical surfaces — `hugit_ledger::deeplink::resolve`,
//! `hugit_cli::why::resolver::resolve_why`, `hugit_refstore::verify_chain` /
//! `EventLog` — so it tests production behavior, never a hand-rolled stand-in.
//!
//! THE ORACLE GOES RED on a silent blank (an honest-looking success carrying no
//! object/summary) and on a broken/dangling link — never a gamed oracle.

#[path = "../legibility.rs"]
mod legibility;

use legibility::{
    ChainFollow, ChainStore, DownZoom, DownZoomReport, Follow, IntelligenceLayer,
    LAYER_UNAVAILABLE_MSG, Tombstone, deep_link_down_zoom, follow_attestation_after_erasure,
    follow_chain, human_down_zoom, raw_commit_view, why_down_zoom,
};

use hugit_cli::why::resolver::{LogEntry as WhyLogEntry, WhyQuery, fixture_event};
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::EventLog;

// ── shared fixtures ───────────────────────────────────────────────────────────

const RAW_COMMIT: &str = "commit:abc123def456";
const INTENT_ID: &str = "intent-7f";
const DEEP_LINK_TARGET: &str = "oid:deadbeefcafe";
const WHY_PATH: &str = "src/landing.rs";

/// Plain-git object store: the raw-commit view (D2) is served from HERE,
/// independent of the intelligence layer.
fn git_objects() -> Vec<&'static str> {
    vec![RAW_COMMIT, "commit:parent000", "blob:tree-root"]
}

/// A real ledger event stream carrying an `intent.landed` deep-link the
/// canonical `hugit_ledger::deeplink::resolve` can follow.
fn ledger_records() -> Vec<EventRecord> {
    let mut log = EventLog::new();
    log.append(
        "intent.landed",
        vec!["human:owner".to_string()],
        serde_json::json!({
            "intent_id": INTENT_ID,
            "deep_link_target": DEEP_LINK_TARGET,
        })
        .to_string(),
        1_000,
    );
    log.records().to_vec()
}

/// A `why` log slice attributing `WHY_PATH` to a landed intent (D10).
fn why_entries() -> Vec<WhyLogEntry> {
    vec![WhyLogEntry {
        record: fixture_event(
            0,
            "intent.landed",
            vec!["human:owner".to_string()],
            serde_json::json!({
                "intent_id": INTENT_ID,
                "charter": "land the landing layer",
                "path": WHY_PATH,
            }),
        ),
        attestation: None,
        sidecar: None,
    }]
}

fn why_query() -> WhyQuery {
    WhyQuery {
        path: WHY_PATH.to_string(),
        line: None,
        symbol: None,
    }
}

// ── ① with the intelligence layer DEGRADED: the human's down-zoom (raw-commit ─
//     view, deep links, `why`) still resolves via plain git OR fails HONESTLY
//     (explicit "layer unavailable"), never a silent 404/blank

#[test]
fn item_1_degraded_down_zoom_resolves_via_git_or_fails_honestly_never_blank() {
    let git = git_objects();
    let records = ledger_records();
    let entries = why_entries();
    let q = why_query();

    // ── Substrate state: intelligence layer DEGRADED. ──
    let report: DownZoomReport = human_down_zoom(
        IntelligenceLayer::Degraded,
        RAW_COMMIT,
        &git,
        INTENT_ID,
        &records,
        &q,
        &entries,
    );

    // The CORE assertion: every leg lands on an HONEST endpoint — NEVER a silent
    // 404/blank. (A success carrying an empty object/summary fails is_honest.)
    assert!(
        report.all_honest(),
        "every down-zoom leg must be honest under degradation, got {report:?}",
    );

    // Clause A — the raw-commit view STILL resolves via plain git (lock 5: a
    // valid git repo keeps serving even when the intelligence layer is down).
    assert!(
        report.git_still_serves(),
        "raw-commit view (plain git) must keep serving under degradation",
    );
    match &report.raw_commit {
        DownZoom::ViaPlainGit { object } => {
            assert_eq!(object, RAW_COMMIT, "git leg must land on the real commit");
            assert!(!object.trim().is_empty(), "git leg must NOT be a blank");
        }
        other => panic!("raw-commit view must resolve via plain git, got {other:?}"),
    }

    // Clause B — the deep-link leg fails HONESTLY: explicit "layer unavailable",
    // NOT a silent blank/NotFound a human could mistake for "no such intent".
    assert!(
        report.deep_link.is_layer_unavailable(),
        "deep-link leg must fail honestly under degradation, got {:?}",
        report.deep_link,
    );
    assert!(report.deep_link.is_honest());

    // Clause C — the `why` leg fails HONESTLY too: explicit "layer unavailable".
    assert!(
        report.why.is_layer_unavailable(),
        "`why` leg must fail honestly under degradation, got {:?}",
        report.why,
    );
    assert!(report.why.is_honest());

    // Sanity that the honest message exists and is non-empty (the human is told).
    assert!(!LAYER_UNAVAILABLE_MSG.trim().is_empty());
}

#[test]
fn item_1_healthy_layer_resolves_all_legs_via_intelligence_layer() {
    // CONTRAST: with the layer HEALTHY the deep-link and `why` legs resolve via
    // the intelligence layer (using the REAL canonical resolvers), and the
    // raw-commit view still resolves via git. Every leg honest, none blank.
    let git = git_objects();
    let records = ledger_records();
    let entries = why_entries();
    let q = why_query();

    let report = human_down_zoom(
        IntelligenceLayer::Healthy,
        RAW_COMMIT,
        &git,
        INTENT_ID,
        &records,
        &q,
        &entries,
    );

    assert!(report.all_honest(), "all legs honest when healthy");
    assert!(report.git_still_serves());
    assert!(
        matches!(report.deep_link, DownZoom::ViaIntelligenceLayer { .. }),
        "healthy deep-link must resolve via the intelligence layer, got {:?}",
        report.deep_link,
    );
    assert!(
        matches!(report.why, DownZoom::ViaIntelligenceLayer { .. }),
        "healthy `why` must resolve via the intelligence layer, got {:?}",
        report.why,
    );
    // The deep-link leg used the REAL canonical resolver: it carries the golden
    // target the ledger record declared.
    if let DownZoom::ViaIntelligenceLayer { summary } = &report.deep_link {
        assert!(
            summary.contains(DEEP_LINK_TARGET),
            "deep-link summary must carry the canonical golden target",
        );
    }
}

#[test]
fn item_1_git_leg_independent_of_layer_state() {
    // Lock 5, sharpened: the raw-commit view resolves IDENTICALLY whether the
    // intelligence layer is healthy or degraded — it is served by plain git.
    let git = git_objects();
    let healthy = raw_commit_view(IntelligenceLayer::Healthy, RAW_COMMIT, &git);
    let degraded = raw_commit_view(IntelligenceLayer::Degraded, RAW_COMMIT, &git);
    assert_eq!(
        healthy, degraded,
        "the plain-git raw-commit view must not depend on the intelligence layer",
    );
    assert!(matches!(healthy, DownZoom::ViaPlainGit { .. }));
}

#[test]
fn item_1_a_silent_blank_would_be_caught_red() {
    // ADVERSARIAL: prove the oracle is RED on a SILENT BLANK — a success-shaped
    // outcome that carries no object/summary (a 404/blank masquerading as an
    // answer). is_honest() must reject it; if it did not, the oracle would be
    // gamed (a blank screen would pass item ①).
    let blank_git = DownZoom::ViaPlainGit {
        object: "   ".to_string(),
    };
    let blank_layer = DownZoom::ViaIntelligenceLayer {
        summary: String::new(),
    };
    assert!(
        !blank_git.is_honest(),
        "a blank-object 'success' must NOT pass as honest",
    );
    assert!(
        !blank_layer.is_honest(),
        "an empty-summary 'success' must NOT pass as honest",
    );

    // And a report containing such a blank fails all_honest() — RED.
    let gamed = DownZoomReport {
        raw_commit: blank_git,
        deep_link: DownZoom::LayerUnavailable {
            leg: "deep_link".to_string(),
        },
        why: DownZoom::LayerUnavailable {
            leg: "why".to_string(),
        },
    };
    assert!(
        !gamed.all_honest(),
        "a report with a silent blank leg must be caught (oracle RED)",
    );
}

#[test]
fn item_1_degraded_deep_link_is_explicit_not_a_silent_not_found() {
    // The honest-failure state is EXPLICIT. Under degradation the deep-link leg
    // is `LayerUnavailable` — distinguishable from a healthy-layer "no such
    // intent". A human is told "layer unavailable", not handed silence.
    let records = ledger_records();
    let degraded = deep_link_down_zoom(IntelligenceLayer::Degraded, INTENT_ID, &records);
    assert!(degraded.is_layer_unavailable());

    // Same for `why`.
    let entries = why_entries();
    let q = why_query();
    let degraded_why = why_down_zoom(IntelligenceLayer::Degraded, &q, &entries);
    assert!(degraded_why.is_layer_unavailable());
}

// ── ② after an erasure cascade: following any chain reaches an honest tombstone,
//     never a broken link — the human can ALWAYS follow, in every substrate state

const A: &str = "sha256:obj-a";
const B: &str = "sha256:obj-b";
const C: &str = "sha256:obj-c";

fn seeded_chain_store() -> ChainStore {
    let mut s = ChainStore::new();
    s.put(A, "alpha bytes");
    s.put(B, "beta bytes");
    s.put(C, "gamma bytes");
    s
}

#[test]
fn item_2_following_any_chain_reaches_tombstone_never_broken_after_erasure() {
    let mut store = seeded_chain_store();

    // ── the erasure cascade (X7 leg): erase B and C across the chain. ──
    store.erase(B, "rtbf-request:case-13");
    store.erase(C, "rtbf-request:case-13");

    // Follow the WHOLE chain A → B → C. A is live; B, C are erased.
    let followed: ChainFollow = follow_chain(&[A, B, C], &store);

    // THE CORE assertion: the human can ALWAYS follow — every hop reaches a
    // target-or-tombstone, NEVER a broken/dangling link.
    assert!(
        followed.human_can_always_follow(),
        "every hop must reach a target-or-tombstone, got {followed:?}",
    );
    // The cascade left honest tombstones the human actually reached.
    assert!(
        followed.reaches_tombstone(),
        "following the chain must reach the erasure tombstones",
    );

    // Per-hop truth: live target, then two tamper-evident tombstones.
    assert!(matches!(followed.hops[0], Follow::Target { .. }));
    for (i, hash) in [(1usize, B), (2usize, C)] {
        match &followed.hops[i] {
            Follow::Tombstone(ts) => {
                assert!(ts.is_tombstone(), "must be a well-formed tombstone");
                assert_eq!(
                    ts.erased_object_hash, hash,
                    "tombstone must name the ORIGINAL erased object (tamper-evident)",
                );
            }
            other => panic!("erased hop {i} must reach a tombstone, got {other:?}"),
        }
    }
}

#[test]
fn item_2_attestation_chain_followable_over_tombstone_after_erasure() {
    // The chain followed in ② is an attestation/provenance chain: erasure
    // operates on the OBJECT STORE, never the append-only chain, so the canonical
    // `verify_chain` still verifies AND following the surviving link reaches a
    // tombstone — the human follows to a truthful endpoint.
    let mut log = EventLog::new();
    log.append(
        "tree.snapshot",
        vec!["agent:planner".to_string()],
        serde_json::json!({ "object": A }).to_string(),
        1_000,
    );
    let link = log.append(
        "object.link",
        vec!["human:owner".to_string()],
        serde_json::json!({ "object": B }).to_string(),
        2_000,
    );
    let records = log.records().to_vec();
    let _ = link; // the linked object is B.

    let mut store = seeded_chain_store();
    store.erase(B, "rtbf-request:case-13");

    let follow = follow_attestation_after_erasure(&records, B, &store);
    assert!(
        follow.human_can_always_follow(),
        "chain must verify AND its link reach a truthful endpoint, got {follow:?}",
    );
    assert_eq!(
        follow.chain,
        Ok(()),
        "append-only chain still verifies post-erasure"
    );
    assert!(matches!(follow.endpoint, Follow::Tombstone(_)));
}

#[test]
fn item_2_erased_link_resolves_to_tombstone_never_void() {
    // An erased object resolves to a tombstone, NEVER to a broken link (a void).
    // Idempotent: even erasing a never-present hash leaves a tombstone, so the
    // cascade can never strand the human at a dangling link.
    let mut store = ChainStore::new();
    store.erase("sha256:never-stored", "rtbf");
    let f = store.follow("sha256:never-stored");
    assert!(
        !matches!(f, Follow::Broken { .. }),
        "an erased object must leave a tombstone, never a broken link",
    );
    assert!(f.is_followable());
}

#[test]
fn item_2_a_broken_link_would_be_caught_red() {
    // ADVERSARIAL: prove the oracle is RED on a BROKEN/dangling link — a hop that
    // reaches NOTHING (no target, no tombstone). If the cascade left a void
    // instead of a tombstone, following the chain hits Follow::Broken and
    // human_can_always_follow() must be FALSE. If it weren't, the oracle would be
    // gamed (a dangling link would pass item ②).
    let store = ChainStore::new(); // empty: nothing live, nothing erased.
    let followed = follow_chain(&[A, B], &store);

    // Each hop is Broken (reaches nothing) — NOT followable.
    assert!(
        followed
            .hops
            .iter()
            .all(|f| matches!(f, Follow::Broken { .. }))
    );
    assert!(
        !followed.human_can_always_follow(),
        "a chain with a broken link must be caught (oracle RED)",
    );

    // And a lone broken hop is explicitly not followable.
    assert!(!Follow::Broken { id: A.to_string() }.is_followable());
}

#[test]
fn item_2_tombstone_is_tamper_evident_not_a_substitute() {
    // The tombstone is a deliberate, self-describing marker carrying the ORIGINAL
    // hash — never a silent substitute. Following it lands on a truthful "this
    // was here, it was erased" endpoint, distinguishable from a live target.
    let ts = Tombstone::new(B, "rtbf-request:case-13");
    assert!(ts.is_tombstone());
    assert_eq!(ts.erased_object_hash, B);
    let f = Follow::Tombstone(ts);
    assert!(f.is_followable());
    assert!(!matches!(f, Follow::Target { .. }));
}

// ── composition: ① ∧ ② — the human can always follow in EVERY substrate state ─

#[test]
fn composition_human_can_always_follow_under_degradation_and_erasure() {
    // BOTH adverse substrate states at once: the intelligence layer is DEGRADED
    // AND an erasure cascade has run. The human still (①) down-zooms via plain
    // git or an honest "layer unavailable" — never blank — AND (②) follows the
    // chain to a target-or-tombstone — never broken. Legibility holds in every
    // substrate state.
    let git = git_objects();
    let records = ledger_records();
    let entries = why_entries();
    let q = why_query();

    // ① degraded down-zoom is fully honest.
    let report = human_down_zoom(
        IntelligenceLayer::Degraded,
        RAW_COMMIT,
        &git,
        INTENT_ID,
        &records,
        &q,
        &entries,
    );
    assert!(report.all_honest(), "①: degraded down-zoom honest");
    assert!(report.git_still_serves(), "①: plain git keeps serving");

    // ② erasure cascade, then follow the chain to tombstones.
    let mut store = seeded_chain_store();
    store.erase(B, "rtbf-request:case-13");
    store.erase(C, "rtbf-request:case-13");
    let followed = follow_chain(&[A, B, C], &store);
    assert!(
        followed.human_can_always_follow(),
        "②: human follows chain to target-or-tombstone under erasure",
    );

    // The whole point: under BOTH degradation and erasure, the human ALWAYS
    // lands on a truthful endpoint — never a silent blank, never a broken link.
    assert!(report.all_honest() && followed.human_can_always_follow());
}
