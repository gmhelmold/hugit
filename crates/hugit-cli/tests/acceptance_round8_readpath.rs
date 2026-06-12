//! Acceptance — Wave L, WP L-B: `intent list` read-path integrity (Round-8 C2).
//!
//! Confirmed hole (Round 8 Class-2): `intent list --log <path>` built an
//! `EventLog` via a hand-rolled `push_record` loop WITHOUT calling
//! `verify_chain`, so a hand-tampered `intent.landed` payload returned
//! `"landed":true` exit 0 with no `chain_broken`.
//!
//! Fix: `resolve_landed` now routes through `crate::checks::load_event_log` —
//! the shared, verified choke-point (also used by `verdict`, `checks show`,
//! and `queue show`) that always calls `verify_chain` before projecting.
//!
//! Tests:
//!
//!   ① Baseline: a well-formed log → `intent list --log` returns `landed:true`,
//!     exit 0, NO error envelope.
//!
//!   ② Tamper repro (the L-B fix): a hand-tampered `intent.landed` payload
//!     (payload bytes mutated, hash chain not recomputed) → `intent list --log`
//!     MUST exit 2 with `chain_broken`, MUST NOT project `landed:true`.
//!
//! The test uses only the REAL `hugit` binary and temp dirs OUTSIDE the repo
//! tree — no internal crate imports needed for the binary-path assertions.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "hugit-r8-readpath-{tag}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit <args>` → `(exit_code, parsed_stdout_json_or_null)`.
fn run(args: &[&str]) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

/// Build a real canonical `[EventRecord, …]` log with one `intent.landed`
/// event via `hugit intent new --log`.  Returns `(store_path, log_path)`.
fn seed_intent_log(dir: &std::path::Path, intent_id: &str, campaign: &str) {
    let store_s = dir.join("store.json");
    let log_s = dir.join("log.json");

    let (code, v) = run(&[
        "intent",
        "new",
        "--store",
        store_s.to_str().unwrap(),
        "--log",
        log_s.to_str().unwrap(),
        "--campaign",
        campaign,
        "--charter",
        "L-B read-path integrity test intent",
        "--id",
        intent_id,
    ]);
    assert_eq!(
        code, 0,
        "intent new --log must exit 0 to seed the test log: {v}"
    );
    assert_eq!(v["intent_id"], intent_id, "intent id echoed: {v}");
}

// ── ① Baseline: a well-formed log projects landed:true, exit 0 ───────────────

#[test]
fn baseline_well_formed_log_projects_landed_true_exit_0() {
    let dir = scratch("baseline");
    let store_s = dir.join("store.json");
    let log_s = dir.join("log.json");
    let intent_id = "lb-baseline-intent";
    let campaign = "lb-baseline-camp";

    seed_intent_log(&dir, intent_id, campaign);

    let (code, v) = run(&[
        "intent",
        "list",
        "--store",
        store_s.to_str().unwrap(),
        "--log",
        log_s.to_str().unwrap(),
    ]);

    assert_eq!(code, 0, "intent list on a clean log must exit 0: {v}");
    let intents = v["intents"].as_array().expect("top-level intents array");
    assert_eq!(intents.len(), 1, "one intent in the list: {v}");

    let item = &intents[0];
    assert_eq!(item["id"], intent_id, "id matches: {v}");
    assert_eq!(
        item["landed"],
        serde_json::json!(true),
        "landed must be true on a clean log (baseline): {v}"
    );

    // No error envelope on the success path.
    assert!(v.get("error").is_none(), "no error key on success: {v}");
}

// ── ② Tamper repro — the L-B fix: tampered log → chain_broken, exit 2 ────────
//
// Before the fix: `resolve_landed` skipped `verify_chain` → `landed:true`, exit 0.
// After the fix:  routes through `load_event_log` → `chain_broken`, exit 2.

#[test]
fn tampered_log_is_chain_broken_exit_2_on_intent_list() {
    let dir = scratch("tamper");
    let store_s = dir.join("store.json");
    let log_s = dir.join("log.json");
    let intent_id = "lb-tamper-intent";
    let campaign = "lb-tamper-camp";

    // Step 1: build a valid log.
    seed_intent_log(&dir, intent_id, campaign);

    // Step 2: verify the CLEAN state — `intent list --log` must return
    // landed:true, exit 0 (establishes the before-tamper baseline).
    let (before_code, before_v) = run(&[
        "intent",
        "list",
        "--store",
        store_s.to_str().unwrap(),
        "--log",
        log_s.to_str().unwrap(),
    ]);
    assert_eq!(
        before_code, 0,
        "BEFORE tamper: intent list must exit 0: {before_v}"
    );
    {
        let intents = before_v["intents"]
            .as_array()
            .expect("intents array present before tamper");
        assert_eq!(intents.len(), 1, "one intent before tamper: {before_v}");
        assert_eq!(
            intents[0]["landed"],
            serde_json::json!(true),
            "landed:true BEFORE tamper (baseline): {before_v}"
        );
    }

    // Step 3: hand-tamper the payload — mutate the `intent_id` string inside the
    // JSON payload WITHOUT recomputing the hash chain.  The record still parses
    // (monotonic seq intact), but the stored `this_hash` no longer matches the
    // mutated bytes, so `verify_chain` MUST fail.
    let raw = std::fs::read_to_string(&log_s).unwrap();
    // The payload is a JSON string value; the intent_id appears as `"lb-tamper-intent"`
    // inside the serialised payload (double-escaped as `\"lb-tamper-intent\"` in the
    // outer JSON, or as the bare string if pretty-printed without escaping).  We
    // do a simple string substitution — the tamper is valid JSON but breaks the
    // hash chain.
    let tampered = raw.replacen("lb-tamper-intent", "lb-tamper-intent-TAMPERED", 1);
    assert_ne!(raw, tampered, "the tamper must actually mutate a byte");
    std::fs::write(&log_s, &tampered).unwrap();

    // Step 4: re-run `intent list --log` on the tampered file.
    let (after_code, after_v) = run(&[
        "intent",
        "list",
        "--store",
        store_s.to_str().unwrap(),
        "--log",
        log_s.to_str().unwrap(),
    ]);

    // THE CORE ASSERTION — the L-B fix: a tampered chain is never projected.
    assert_eq!(
        after_code, 2,
        "AFTER tamper: intent list must exit 2 (chain_broken), not project landed:true. \
         stdout: {after_v}"
    );
    assert_eq!(
        after_v["error"]["kind"], "chain_broken",
        "AFTER tamper: error kind must be chain_broken: {after_v}"
    );
    assert!(
        after_v["error"]["fix"].is_string(),
        "error must carry a fix string: {after_v}"
    );
    // Must NOT be nested flat.
    assert!(
        after_v.get("kind").is_none(),
        "error shape must be nested under 'error', not flat: {after_v}"
    );

    // The tamper must NOT have been silently ignored and projected as landed:true.
    assert!(
        after_v.get("intents").is_none(),
        "a tampered log must not produce an intents array: {after_v}"
    );
}
