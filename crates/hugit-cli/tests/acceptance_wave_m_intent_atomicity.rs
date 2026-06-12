//! Wave M · WP M-3 acceptance — INTENT TWO-PHASE COMMIT ATOMICITY (C5-F3).
//!
//! `hugit intent new --log L --store S` is a two-phase write: it appends the
//! `intent.landed` record to the hash-chained, append-only `--log` (the SOURCE
//! OF TRUTH) AND saves the intent into the `--store`. The two writes cannot be a
//! single atomic transaction across two files; the M-3 design guarantees
//! **atomic-or-recoverable** instead:
//!
//! 1. Each individual write is ATOMIC (temp-file + rename) — no torn store.
//! 2. The writes are ORDERED log-first (K-ERRLAW2) — the only divergence a
//!    mid-commit failure can leave is LOG-AHEAD (intent on the log, not yet in
//!    the store), NEVER store-ahead (a phantom intent the log never saw).
//! 3. A log-ahead state is SELF-HEALING — the next `intent new` over the same
//!    `--log`/`--store` pair reconciles the store from the log before landing.
//! 4. If the LOG append fails, NOTHING is committed (no orphan store entry).
//!
//! These tests drive the library `intent::new::run` directly (it is `pub`), and
//! reproduce the failure window FAITHFULLY: the post-log-append / pre-store-save
//! crash is simulated by doing phase-1 (`canonical_log::land_intent`) WITHOUT
//! phase-2 (the store save) — exactly the on-disk state a crash in that window
//! leaves — then asserting the next `run` reconciles it. All temp dirs are
//! OUTSIDE the repo.

use std::path::{Path, PathBuf};

use hugit_cli::intent::canonical_log;
use hugit_cli::intent::new::{NewIntent, run};
use hugit_cli::intent::store::IntentStore;

use hugit_contracts::IntentSidecar;
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::EventLog;
use hugit_refstore::intent::intents_from_log;

/// A scratch dir under the OS temp root (outside the repo), removed on drop.
struct Scratch {
    dir: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("hugit-wave-m-{tag}-{nanos}"));
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        Self { dir }
    }
    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn new_input(id: &str, charter: &str, log: &Path) -> NewIntent {
    NewIntent {
        charter: charter.to_string(),
        campaign: "wave-m".to_string(),
        acceptance: vec!["atomic two-phase commit".to_string()],
        id: Some(id.to_string()),
        agent: None,
        context_ref: None,
        log: Some(log.to_path_buf()),
    }
}

/// Whether `intent_id` is present in the canonical `--log` (the source of truth).
fn log_has(log_path: &Path, intent_id: &str) -> bool {
    let bytes = match std::fs::read(log_path) {
        Ok(b) => b,
        Err(_) => return false,
    };
    let s = String::from_utf8_lossy(&bytes);
    s.contains(intent_id)
}

/// Project the intent ids out of the canonical `--log` file (in log order).
fn log_projection(log_path: &Path) -> Vec<String> {
    let bytes = std::fs::read(log_path).expect("read log");
    let records: Vec<EventRecord> = serde_json::from_slice(&bytes).expect("parse log");
    let mut log = EventLog::new();
    for r in records {
        log.push_record(r).expect("rehydrate");
    }
    intents_from_log(&log)
        .expect("project")
        .intents()
        .iter()
        .map(|i| i.intent_id.clone())
        .collect()
}

/// Whether `intent_id` projects out of the `--store`'s authoritative event log.
fn store_has(store_path: &Path, intent_id: &str) -> bool {
    let store = IntentStore::load(store_path).expect("load store");
    store.intent_for(intent_id).expect("query store").is_some()
}

/// 1) Normal flow: a single `intent new --log --store` lands an intent into BOTH
///    the log and the store, and they agree.
#[test]
fn normal_flow_lands_log_and_store_consistent() {
    let s = Scratch::new("normal");
    let log = s.path("events.json");
    let store = s.path("store/intents.json");

    let out = run(new_input("intent-m3-normal", "normal flow", &log), &store)
        .expect("intent new should succeed");
    assert_eq!(out.intent_id, "intent-m3-normal");
    assert!(!out.already_exists);

    assert!(
        log_has(&log, "intent-m3-normal"),
        "log must carry the intent"
    );
    assert!(
        store_has(&store, "intent-m3-normal"),
        "store must carry the intent"
    );
}

/// 2) Idempotency: a re-run with the SAME id is a no-op second landing
///    (`already_exists:true`), no double-record in either log or store.
#[test]
fn rerun_is_idempotent_no_double_record() {
    let s = Scratch::new("idem");
    let log = s.path("events.json");
    let store = s.path("store/intents.json");

    let first = run(new_input("intent-m3-idem", "idem flow", &log), &store).expect("first run");
    assert!(!first.already_exists);

    let second = run(new_input("intent-m3-idem", "idem flow", &log), &store).expect("second run");
    assert!(
        second.already_exists,
        "second identical run must report already_exists"
    );
    assert_eq!(first.intent_id, second.intent_id, "same derived id");

    // Exactly one intent.landed in the store projection (no double-record).
    let store_obj = IntentStore::load(&store).expect("load store");
    let store_count = store_obj
        .intent_for_all()
        .expect("project")
        .iter()
        .filter(|i| i.intent_id == "intent-m3-idem")
        .count();
    assert_eq!(
        store_count, 1,
        "store must hold exactly one record for the id"
    );

    // The canonical --log, projected, must also hold exactly ONE landing for the
    // id (the idempotent re-run must not double-append to the source of truth).
    let log_intents = log_projection(&log);
    let log_count = log_intents
        .iter()
        .filter(|id| id == &"intent-m3-idem")
        .count();
    assert_eq!(
        log_count, 1,
        "log must hold exactly one intent.landed for the id"
    );
}

/// 3a) THE FAILURE WINDOW (post-log-append / pre-store-save crash) is
///     RECOVERABLE. We reproduce the exact on-disk state such a crash leaves —
///     the `--log` has the intent.landed (phase 1 committed) but the `--store`
///     never got it (phase 2 never ran) — by invoking `canonical_log::land_intent`
///     WITHOUT a store save. Then the NEXT `intent new` over the same pair must
///     RECONCILE the store from the log (the source of truth), leaving no
///     permanent divergence and no phantom.
#[test]
fn failure_window_log_ahead_is_recovered_on_next_run() {
    let s = Scratch::new("window");
    let log = s.path("events.json");
    let store_dir = s.path("store");
    std::fs::create_dir_all(&store_dir).expect("create store dir");
    let store = store_dir.join("intents.json");

    // First, land a real intent normally so the store exists and is in sync.
    run(new_input("intent-m3-base", "base intent", &log), &store).expect("base run");
    assert!(store_has(&store, "intent-m3-base"));

    // ── Simulate the crash window for a SECOND intent ──────────────────────────
    // Phase 1 ONLY: append intent.landed to the authoritative --log, then "crash"
    // before the store save. This is byte-identical to the state a real
    // post-log/pre-store crash leaves.
    let crashed_sidecar = IntentSidecar {
        intent_id: "intent-m3-crashed".to_string(),
        charter: "crashed before store save".to_string(),
        acceptance: vec!["recovered from log".to_string()],
        context_ref: String::new(),
        authoritative: false,
    };
    let principal_chain = vec!["campaign:wave-m".to_string(), "agent:main".to_string()];
    let recorded_at = 1_700_000_000_000;
    canonical_log::land_intent(&log, &crashed_sidecar, &principal_chain, recorded_at)
        .expect("phase-1 log append must succeed");

    // Divergence asserted: LOG ahead of STORE (the recoverable direction).
    assert!(
        log_has(&log, "intent-m3-crashed"),
        "log (source of truth) must carry the crashed intent"
    );
    assert!(
        !store_has(&store, "intent-m3-crashed"),
        "store must NOT yet carry it (the failure window)"
    );

    // ── The NEXT run must reconcile the store from the log ─────────────────────
    // We land a THIRD, distinct intent; the reconcile runs first and heals the
    // log-ahead divergence for `intent-m3-crashed` as a side effect.
    run(new_input("intent-m3-next", "next intent", &log), &store).expect("next run reconciles");

    // The crashed intent is now in the store (reconciled from the log) — no
    // permanent divergence, no phantom.
    assert!(
        store_has(&store, "intent-m3-crashed"),
        "C5-F3: the next run must reconcile the log-ahead intent into the store"
    );
    // And the new intent landed normally in both.
    assert!(log_has(&log, "intent-m3-next"));
    assert!(store_has(&store, "intent-m3-next"));

    // Store and log fully agree on all three intents now.
    for id in ["intent-m3-base", "intent-m3-crashed", "intent-m3-next"] {
        assert!(log_has(&log, id), "log must have {id}");
        assert!(
            store_has(&store, id),
            "store must have {id} after reconcile"
        );
    }
}

/// 3b) `intent list` over a log-ahead store does NOT surface a permanent
///     divergence: after reconcile-on-write, the store reflects the log. (We
///     drive the reconcile via `run`, the write verb, then assert the read.)
#[test]
fn log_ahead_does_not_persist_as_divergence() {
    let s = Scratch::new("listdiv");
    let log = s.path("events.json");
    let store_dir = s.path("store");
    std::fs::create_dir_all(&store_dir).expect("create store dir");
    let store = store_dir.join("intents.json");

    // Bootstrap the store with one intent so the file exists.
    run(new_input("intent-m3-seed", "seed", &log), &store).expect("seed run");

    // Crash window: log-only landing of a second intent.
    let sc = IntentSidecar {
        intent_id: "intent-m3-orphan-log".to_string(),
        charter: "log only".to_string(),
        acceptance: vec![],
        context_ref: String::new(),
        authoritative: false,
    };
    canonical_log::land_intent(
        &log,
        &sc,
        &["campaign:wave-m".to_string(), "agent:main".to_string()],
        1_700_000_001_000,
    )
    .expect("phase-1 append");

    assert!(
        !store_has(&store, "intent-m3-orphan-log"),
        "store behind log"
    );

    // A re-run of the SAME seed id (idempotent) still triggers reconcile first.
    run(new_input("intent-m3-seed", "seed", &log), &store).expect("idempotent re-run reconciles");

    assert!(
        store_has(&store, "intent-m3-orphan-log"),
        "C5-F3: even an idempotent re-run reconciles the log-ahead intent — no permanent divergence"
    );
}

/// 4) A LOG-append failure leaves NO orphan store entry (store-ahead is
///    impossible). With the `--log` pointed at a non-existent directory, the log
///    append faults and the store save must be skipped entirely; a retry once the
///    log dir exists converges with log and store in agreement.
#[test]
fn log_append_failure_leaves_no_orphan_store_entry() {
    let s = Scratch::new("noorphan");
    let store_dir = s.path("store");
    std::fs::create_dir_all(&store_dir).expect("create store dir");
    let store = store_dir.join("intents.json");

    // --log inside a directory that does NOT exist → land_intent's lock+create
    // faults, the append fails, nothing is committed.
    let bad_log_dir = s.path("missing_log_dir");
    let bad_log = bad_log_dir.join("events.json");

    let r = run(
        new_input("intent-m3-noorphan", "no orphan", &bad_log),
        &store,
    );
    assert!(
        r.is_err(),
        "a failed log append must produce a structured error, not Ok"
    );
    // No orphan store entry (store-ahead phantom is impossible).
    if store.exists() {
        assert!(
            !store_has(&store, "intent-m3-noorphan"),
            "C5-F3: a log-append failure must NEVER leave a store-ahead phantom"
        );
    }

    // Retry once the log dir exists → converges, both agree.
    std::fs::create_dir_all(&bad_log_dir).expect("create log dir for retry");
    let r2 = run(
        new_input("intent-m3-noorphan", "no orphan", &bad_log),
        &store,
    )
    .expect("retry with writable log must succeed");
    assert!(!r2.already_exists);
    assert!(log_has(&bad_log, "intent-m3-noorphan"));
    assert!(store_has(&store, "intent-m3-noorphan"));
}
