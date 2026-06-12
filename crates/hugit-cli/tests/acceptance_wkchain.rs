//! Acceptance — WP-K-CHAIN: `verify_chain` on `hugit why` + `hugit export` input
//! (closes the two read-path integrity gaps; honors PS-8).
//!
//! These tests prove that a tampered/reordered/corrupt hash chain is rejected
//! with `chain_broken`/exit-2 on both `why` and `export` — the two read verbs
//! that previously skipped `verify_chain` while every other read verb (campaign
//! show, checks show, queue show, pr, intent) already ran it.
//!
//! Design mirrored from the WF-3 test in `acceptance_wb2.rs`
//! (`tampered_chain_is_chain_broken_exit_two_on_checks_and_queue`), which
//! established the canonical chain-broken error shape + exit.
//!
//! What is pinned:
//!   - `hugit why --log <tampered>` → `chain_broken`/exit-2 (was: projected
//!     silently as authoritative provenance).
//!   - `hugit export --log <tampered>` → `chain_broken`/exit-2 (was: emitted
//!     a forged/arbitrary export corpus unchecked).
//!   - The tamper is naive (flip a byte in the payload without recomputing the
//!     hash) — the weakest corruption `verify_chain` must catch.
//!   - Happy-path: a well-formed log continues to resolve/export (no regression).

use std::path::PathBuf;
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::{Value, json};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-kchain-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit <args>` and return `(exit_code, parsed_stdout_json)`.
fn run(args: &[&str]) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

/// Build a `why`-format log `[{record, attestation: null, sidecar: null}, …]`
/// with a REAL hash chain (via the engine's canonical [`EventLog::append`]).
fn write_why_log(path: &std::path::Path, events: &[(&str, Value)]) {
    let mut log = EventLog::new();
    for (kind, payload) in events {
        log.append(*kind, vec![], payload.to_string(), 0);
    }
    let entries: Vec<Value> = log
        .records()
        .iter()
        .map(|r| json!({"record": r, "attestation": null, "sidecar": null}))
        .collect();
    std::fs::write(path, serde_json::to_string(&entries).unwrap()).unwrap();
}

/// Build a canonical `[EventRecord, …]` log with a REAL hash chain (for `export`
/// and other canonical-format verbs).
fn write_canonical_log(path: &std::path::Path, events: &[(&str, Value)]) {
    let mut log = EventLog::new();
    for (kind, payload) in events {
        log.append(*kind, vec![], payload.to_string(), 0);
    }
    std::fs::write(path, serde_json::to_string_pretty(log.records()).unwrap()).unwrap();
}

// ─────────────────────────────────────────────────────────────────────────────
// `hugit why` — tampered chain → `chain_broken`/exit-2.
// ─────────────────────────────────────────────────────────────────────────────

/// A tampered `why` log is rejected as `chain_broken`/exit-2. Before K-CHAIN,
/// `why` skipped `verify_chain` and projected the forged log as authoritative
/// provenance. Now it fails closed — the same boundary every sibling read verb
/// already enforces.
#[test]
fn tampered_chain_is_chain_broken_exit_two_on_why() {
    let dir = scratch("why-tamper");
    let log = dir.join("tampered.json");

    // Build a REAL, well-formed 2-record chain through the engine's append path.
    write_why_log(
        &log,
        &[
            (
                "intent.landed",
                json!({
                    "intent_id": "intent-why-1",
                    "charter": "Add the parser",
                    "path": "src/parser.rs"
                }),
            ),
            (
                "intent.landed",
                json!({
                    "intent_id": "intent-why-2",
                    "charter": "Add the router",
                    "path": "src/router.rs"
                }),
            ),
        ],
    );

    // Tamper: mutate the first record's payload on disk WITHOUT recomputing the
    // hash chain — the records still parse (monotonic seq intact), but the stored
    // hash no longer matches the payload, so `verify_chain` fails.
    // The payload is stored as a JSON STRING in the why-format, so the field
    // name is escaped on disk (`\"parser\"`); match the escaped form.
    let raw = std::fs::read_to_string(&log).unwrap();
    let tampered = raw.replacen(r#"\"intent-why-1\""#, r#"\"intent-TAMPERED\""#, 1);
    assert_ne!(raw, tampered, "the tamper must actually change a byte");
    std::fs::write(&log, tampered).unwrap();
    let log_s = log.to_str().unwrap();

    let (code, v) = run(&["why", "--log", log_s, "--path", "src/parser.rs"]);
    assert_eq!(code, 2, "a tampered why log is exit 2: {v}");
    assert_eq!(
        v["error"]["kind"], "chain_broken",
        "the tampered log must be rejected as chain_broken: {v}"
    );
    assert!(v["error"]["fix"].is_string(), "fix key present: {v}");
    // Nested under "error", never flat.
    assert!(
        v.get("kind").is_none(),
        "error must be nested, not flat: {v}"
    );
}

/// A well-formed `why` log still resolves without error (no regression).
#[test]
fn well_formed_why_log_still_resolves_after_k_chain() {
    let dir = scratch("why-ok");
    let log = dir.join("log.json");
    write_why_log(
        &log,
        &[(
            "intent.landed",
            json!({
                "intent_id": "intent-kchain-ok",
                "charter": "Happy path — verify_chain does not block valid logs",
                "path": "src/happy.rs"
            }),
        )],
    );

    let (code, v) = run(&[
        "why",
        "--log",
        log.to_str().unwrap(),
        "--path",
        "src/happy.rs",
    ]);
    assert_eq!(code, 0, "a valid why log exits 0: {v}");
    assert_eq!(v["intent_id"], "intent-kchain-ok");
    assert!(v["charter"].is_string());
}

// ─────────────────────────────────────────────────────────────────────────────
// `hugit export` — tampered chain → `chain_broken`/exit-2.
// ─────────────────────────────────────────────────────────────────────────────

/// A tampered export input log is rejected as `chain_broken`/exit-2. Before
/// K-CHAIN, `export` rebuilt an EventLog from raw `.append()` over an
/// `{events:[…]}` format with no chain verification — arbitrary/forged events
/// could enter the export corpus unchecked. Now export reads the canonical
/// `[EventRecord, …]` format and runs `verify_chain`, so a tampered input is
/// rejected before any artifact is written.
#[test]
fn tampered_chain_is_chain_broken_exit_two_on_export() {
    let dir = scratch("export-tamper");
    let log = dir.join("tampered.json");
    let out_dir = dir.join("artifact");

    // Build a REAL, well-formed 2-record chain through the engine's canonical
    // append path (same as every other porcelain write verb uses).
    write_canonical_log(
        &log,
        &[
            (
                "intent.landed",
                json!({"intent_id": "exp-1", "charter": "first intent"}),
            ),
            (
                "intent.landed",
                json!({"intent_id": "exp-2", "charter": "second intent"}),
            ),
        ],
    );

    // Tamper: mutate the first record's payload on disk WITHOUT recomputing the
    // hash chain — the records still parse + rehydrate (monotonic seq intact),
    // but the stored hash no longer matches the payload, so `verify_chain` fails.
    // The payload is a JSON STRING escaped on disk (`\"exp-1\"`); match it.
    let raw = std::fs::read_to_string(&log).unwrap();
    let tampered = raw.replacen(r#"\"exp-1\""#, r#"\"exp-TAMPERED\""#, 1);
    assert_ne!(raw, tampered, "the tamper must actually change a byte");
    std::fs::write(&log, tampered).unwrap();
    let log_s = log.to_str().unwrap();

    let (code, v) = run(&["export", "--log", log_s, "--out", out_dir.to_str().unwrap()]);
    assert_eq!(code, 2, "a tampered export input is exit 2: {v}");
    assert_eq!(
        v["error"]["kind"], "chain_broken",
        "the tampered log must be rejected as chain_broken: {v}"
    );
    assert!(v["error"]["fix"].is_string(), "fix key present: {v}");
    assert!(
        v.get("kind").is_none(),
        "error must be nested, not flat: {v}"
    );
    // No partial artifact written — export fails closed.
    assert!(
        !out_dir.exists(),
        "no artifact must be written when the input chain is broken"
    );
}

/// A well-formed export log still produces a valid artifact (no regression).
#[test]
fn well_formed_export_log_still_exports_after_k_chain() {
    let dir = scratch("export-ok");
    let log = dir.join("log.json");
    let out_dir = dir.join("artifact");
    write_canonical_log(
        &log,
        &[(
            "ref.update",
            json!({"ref": "refs/heads/main", "target": "deadbeef"}),
        )],
    );

    let (code, v) = run(&[
        "export",
        "--log",
        log.to_str().unwrap(),
        "--out",
        out_dir.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "a valid export log exits 0: {v}");
    assert!(v["exported"]["git_dir"].is_string());
    assert!(v["exported"]["envelope_json"].is_string());
    assert!(out_dir.join("export.json").is_file(), "envelope written");
}
