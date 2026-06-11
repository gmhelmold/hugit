//! WP-PC2 acceptance — `hugit intent new/show`.
//!
//! Spec: docs/plan/2026-06-10-cli-porcelain-wave.md (Design law: stable JSON
//! stdout, structured errors w/ suggested fix, idempotency, hermetic-first) +
//! docs/product/headless-engine.md §5.4.
//!
//! These tests drive the REAL refstore/projection paths — `intent new` lands a
//! frozen `IntentSidecar` through `hugit_refstore::intent::import_sidecar` onto
//! a hash-chained `EventLog`, and `intent show` reads it back via
//! `intents_from_log`. NOTHING is hand-faked: every assertion is over state the
//! production path actually produced. The store is a temp JSON file (the
//! hermetic `--store` seam; live DO/CAS binding is the P2 disclosed seam).
//!
//! Owned items:
//!   ① new: charter/acceptance/campaign → real refstore landing → intent id
//!   ② new returns the stable `{"intent_id","already_exists":false}` object
//!   ③ idempotency: a double-run on the SAME id returns already_exists:true and
//!      appends NO second event (the log length is unchanged)
//!   ④ show: native projection + sidecar + envelope ref + verdicts, honestly
//!      null/absent when not captured
//!   ⑤ structured errors carry a kind + message + suggested_fix
//!   ⑥ the persisted store re-verifies its hash chain on load (tamper fails closed)

use std::path::PathBuf;

use hugit_cli::intent::error::PorcelainError;
use hugit_cli::intent::new::{self, NewIntent};
use hugit_cli::intent::show::{self, ShowIntent};
use hugit_cli::intent::store::IntentStore;
use hugit_refstore::intent::intents_from_log;

/// A unique temp store path for one test (no cross-test contamination).
fn temp_store(tag: &str) -> PathBuf {
    let mut p = std::env::temp_dir();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    p.push(format!(
        "hugit-pc2-{tag}-{nanos}-{}.json",
        std::process::id()
    ));
    p
}

fn sample(id: Option<&str>) -> NewIntent {
    NewIntent {
        charter: "Add the JSON porcelain for intents".to_string(),
        campaign: "cli-porcelain".to_string(),
        acceptance: vec![
            "new returns an intent id".to_string(),
            "show projects honestly".to_string(),
        ],
        id: id.map(str::to_string),
        agent: None,
        context_ref: None,
        log: None,
    }
}

// ── ① + ② new lands through the REAL refstore path and returns the id ───────────

#[test]
fn item_1_new_lands_real_intent_and_returns_id() {
    let store = temp_store("new-real");
    let result = new::run(sample(None), &store).expect("new must succeed");

    assert!(!result.intent_id.is_empty(), "an id must be returned");
    assert!(!result.already_exists, "first landing is not a re-run");

    // The REAL projection: the intent is on the hash-chained log at the intent
    // altitude, carrying the charter we authored (no hand-faked state).
    let loaded = IntentStore::load(&store).expect("store loads + chain verifies");
    let intents = intents_from_log(&loaded.log).expect("intent altitude projects");
    let intent = intents
        .by_id(&result.intent_id)
        .expect("the landed intent is on the log");
    assert_eq!(intent.charter, "Add the JSON porcelain for intents");
    assert_eq!(intents.len(), 1, "exactly one intent landed");

    let _ = std::fs::remove_file(&store);
}

#[test]
fn item_2_new_result_is_the_stable_object() {
    let store = temp_store("new-shape");
    let result = new::run(sample(Some("intent-fixed-1")), &store).expect("new succeeds");
    // Stable key-set (WB0): intent_id, already_exists, campaign, agent always present.
    assert_eq!(result.intent_id, "intent-fixed-1");
    assert!(!result.already_exists);
    assert_eq!(result.campaign, "cli-porcelain");
    assert_eq!(result.agent, "main"); // default agent
    let v: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&result).unwrap()).unwrap();
    assert!(v.get("intent_id").is_some());
    assert!(v.get("already_exists").is_some());
    assert!(v.get("campaign").is_some());
    assert!(v.get("agent").is_some());
    let _ = std::fs::remove_file(&store);
}

// ── ③ idempotency: double-run on the same id is a no-op landing ─────────────────

#[test]
fn item_3_double_run_is_idempotent() {
    let store = temp_store("idempotent");

    let first = new::run(sample(Some("intent-dup")), &store).expect("first new");
    assert!(!first.already_exists);

    // Second run, SAME explicit id → already_exists, exit-0 semantics.
    let second = new::run(sample(Some("intent-dup")), &store).expect("second new");
    assert!(second.already_exists, "re-run reports already_exists");
    assert_eq!(first.intent_id, second.intent_id);

    // And it appended NO second event: the log still holds exactly one intent.
    let loaded = IntentStore::load(&store).expect("store reloads");
    let intents = intents_from_log(&loaded.log).expect("project");
    assert_eq!(intents.len(), 1, "idempotent: no duplicate landing event");

    let _ = std::fs::remove_file(&store);
}

#[test]
fn item_3b_content_derived_id_is_idempotent_without_explicit_id() {
    let store = temp_store("derived");
    let a = new::run(sample(None), &store).expect("first");
    let b = new::run(sample(None), &store).expect("second, identical inputs");
    assert_eq!(a.intent_id, b.intent_id, "same content ⇒ same derived id");
    assert!(!a.already_exists);
    assert!(b.already_exists, "identical re-run is idempotent");
    let _ = std::fs::remove_file(&store);
}

// ── ④ show: honest projection, absent fields null/empty ─────────────────────────

#[test]
fn item_4_show_projects_honestly() {
    let store = temp_store("show");
    let created = new::run(sample(Some("intent-show-1")), &store).expect("new");

    let value = show::run(
        ShowIntent {
            intent_id: created.intent_id.clone(),
        },
        &store,
    )
    .expect("show succeeds");

    assert_eq!(value["intent_id"], "intent-show-1");
    assert_eq!(value["charter"], "Add the JSON porcelain for intents");
    // Acceptance comes from the (non-authoritative) sidecar corpus.
    assert_eq!(value["acceptance"].as_array().unwrap().len(), 2);
    assert_eq!(value["authoritative"], false);
    // No envelope captured ⇒ context_ref is honestly null (never invented).
    assert!(
        value["context_ref"].is_null(),
        "no envelope ⇒ null context_ref"
    );
    // No panel ran ⇒ verdicts is honestly empty.
    assert!(value["verdicts"].as_array().unwrap().is_empty());

    let _ = std::fs::remove_file(&store);
}

#[test]
fn item_4b_show_surfaces_captured_context_ref() {
    let store = temp_store("show-ctx");
    let mut input = sample(Some("intent-ctx"));
    input.context_ref = Some("cas:envelope-abc".to_string());
    new::run(input, &store).expect("new with envelope");

    let value = show::run(
        ShowIntent {
            intent_id: "intent-ctx".to_string(),
        },
        &store,
    )
    .expect("show");
    assert_eq!(value["context_ref"], "cas:envelope-abc");
    let _ = std::fs::remove_file(&store);
}

// ── ⑤ structured errors carry a fix ─────────────────────────────────────────────

#[test]
fn item_5_show_missing_intent_is_structured_not_found() {
    let store = temp_store("missing");
    // Land one intent so the store exists but does NOT hold the queried id.
    new::run(sample(Some("intent-present")), &store).expect("seed");

    let err = show::run(
        ShowIntent {
            intent_id: "intent-absent".to_string(),
        },
        &store,
    )
    .expect_err("absent id is an error");
    assert_eq!(err.kind, "not_found");
    assert!(!err.fix.is_empty(), "the fix is non-empty");

    // The structured JSON wire shape (WB0 canonical: "fix" not "suggested_fix").
    let json = err.to_json();
    assert!(json.contains(r#""kind":"not_found""#));
    assert!(json.contains(r#""fix""#));

    let _ = std::fs::remove_file(&store);
}

#[test]
fn item_5b_new_rejects_empty_charter() {
    let store = temp_store("empty-charter");
    let mut input = sample(Some("intent-x"));
    input.charter = "   ".to_string();
    let err = new::run(input, &store).expect_err("empty charter rejected");
    assert_eq!(err.kind, "invalid_argument");
    let _ = std::fs::remove_file(&store);
}

// ── ⑥ tamper fails closed on load ───────────────────────────────────────────────

#[test]
fn item_6_tampered_store_fails_closed() {
    let store = temp_store("tamper");
    new::run(sample(Some("intent-tamper")), &store).expect("seed");

    // Corrupt the persisted charter in the event payload WITHOUT recomputing the
    // hash chain — the load-time verify must reject it (no forged projection).
    let raw = std::fs::read_to_string(&store).unwrap();
    let tampered = raw.replace("Add the JSON porcelain", "Tampered charter text");
    assert_ne!(raw, tampered, "the test must actually mutate the payload");
    std::fs::write(&store, tampered).unwrap();

    let err = IntentStore::load(&store).expect_err("tampered store must fail closed");
    let msg = format!("{err}");
    assert!(
        msg.contains("verification") || msg.contains("chain"),
        "fail-closed error names the broken chain, got: {msg}"
    );

    let _ = std::fs::remove_file(&store);
}

// ── PorcelainError shape is the stable single-object contract ───────────────────

#[test]
fn porcelain_error_json_is_stable() {
    let e = PorcelainError::new("not_found", "no intent x", "create it first");
    let v: serde_json::Value = serde_json::from_str(&e.to_json()).unwrap();
    // WB0 canonical: {"error":{"kind":…,"message":…,"fix":…}} — "fix" not "suggested_fix".
    assert_eq!(v["error"]["kind"], "not_found");
    assert_eq!(v["error"]["message"], "no intent x");
    assert_eq!(v["error"]["fix"], "create it first");
    assert!(
        v["error"].get("suggested_fix").is_none(),
        "must be 'fix' not 'suggested_fix'"
    );
}
