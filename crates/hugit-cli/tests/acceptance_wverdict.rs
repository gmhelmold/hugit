//! Acceptance — WP-W-INT (W-VERDICT logic on W0's `verdict` entry point):
//! `hugit verdict` records an adversarial-panel verdict to the canonical log,
//! end-to-end, against the REAL `hugit` binary.
//!
//! Drives the built binary via `CARGO_BIN_EXE_hugit` — no extra dev-deps. All
//! assertions use the real event-log seam (the canonical `[EventRecord, …]`
//! array). The entry point is W0's frozen flat `verdict` verb (`--intent --log
//! [--store]`); `--store` triggers the append, `--lens`/`--result` carry the
//! per-lens panel.
//!
//! # Coverage
//!
//! 1. **Happy path** — record a 3-lens verdict for an intent; confirm the
//!    `verdict.recorded` event is on the log with the correct fields, exit 0,
//!    stable JSON on stdout.
//! 2. **Null → real proof** — the log had no verdict before the `--store` call;
//!    after it the log carries exactly one `verdict.recorded`, exit 0.
//! 3. **Idempotency** — a second identical record call is a no-op
//!    (`"already_recorded":true`, no duplicate event, exit 0).
//! 4. **Canonical log seam** — the appended event's `verdict.recorded` payload
//!    round-trips as a valid [`VerdictObject`].
//! 5. **Missing log under `--store` → `log_not_found`/exit-2** (canonical law).
//! 6. **Lens/result mismatch → `lens_result_mismatch`/exit-2**.
//! 7. **Invalid result token → `invalid_result`/exit-2**.
//! 8. **Aggregate logic** — all-approve → "approve"; any reject → "reject".

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_contracts::event_record::EventRecord;
use hugit_contracts::verdict_object::{Verdict, VerdictObject};

use hugit_cli::verdict::VERDICT_RECORDED_KIND;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-wverdict-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write an empty canonical `[EventRecord, …]` log at `path`.
fn write_empty_log(path: &Path) {
    std::fs::write(path, "[]").unwrap();
}

/// Read the log at `path` and return the parsed records.
fn read_log(path: &Path) -> Vec<EventRecord> {
    let bytes = std::fs::read(path).unwrap_or_default();
    serde_json::from_slice(&bytes).unwrap_or_default()
}

fn count_verdicts(path: &Path) -> usize {
    read_log(path)
        .iter()
        .filter(|r| r.kind == VERDICT_RECORDED_KIND)
        .count()
}

/// Run `hugit <args>` and return `(exit_code, parsed_stdout_json)`.
fn run(args: &[&str]) -> (i32, serde_json::Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v = serde_json::from_str(stdout.trim()).unwrap_or(serde_json::Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

// ── ① Happy path: record a 3-lens verdict ─────────────────────────────────

#[test]
fn item_1_record_3_lens_verdict_exit_zero_stable_json() {
    let dir = scratch("happy3");
    let log = dir.join("log.json");
    write_empty_log(&log);

    let (code, v) = run(&[
        "verdict",
        "--log",
        log.to_str().unwrap(),
        "--store",
        "--intent",
        "intent-A",
        "--lens",
        "security",
        "--result",
        "approve",
        "--lens",
        "contracts",
        "--result",
        "approve",
        "--lens",
        "impact",
        "--result",
        "approve",
    ]);

    assert_eq!(code, 0, "verdict (all approve, --store) must exit 0: {v}");
    assert_eq!(v["verdict_recorded"], true, "verdict_recorded must be true");
    assert_eq!(v["already_recorded"], false, "first run: not a duplicate");
    assert_eq!(v["stored"], true, "--store recorded the row");
    assert_eq!(v["intent"], "intent-A");
    assert_eq!(
        v["aggregate"], "approve",
        "all 3 lenses approve → aggregate approve"
    );

    let lenses = v["lenses"].as_array().expect("lenses is array");
    assert_eq!(lenses.len(), 3, "3 lens entries");
    assert_eq!(lenses[0]["lens"], "security");
    assert_eq!(lenses[0]["result"], "approve");
    assert_eq!(lenses[1]["lens"], "contracts");
    assert_eq!(lenses[2]["lens"], "impact");
}

// ── ② null → real: the log starts empty; after record it carries the event ──

#[test]
fn item_2_verdict_null_to_real_after_record() {
    let dir = scratch("null-to-real");
    let log = dir.join("log.json");
    write_empty_log(&log);

    // Before: log has zero verdict.recorded events.
    assert_eq!(count_verdicts(&log), 0, "log starts with no verdicts");

    let (code, _v) = run(&[
        "verdict",
        "--log",
        log.to_str().unwrap(),
        "--store",
        "--intent",
        "intent-B",
        "--lens",
        "security",
        "--result",
        "approve",
    ]);
    assert_eq!(code, 0, "record exits 0");

    // After: exactly one verdict.recorded — the null→real flip.
    assert_eq!(
        count_verdicts(&log),
        1,
        "after --store the log carries exactly one verdict.recorded"
    );
}

// ── ③ idempotency: a second identical record is a no-op ─────────────────────

#[test]
fn item_3_idempotent_second_record_is_a_noop() {
    let dir = scratch("idem");
    let log = dir.join("log.json");
    write_empty_log(&log);

    let args = [
        "verdict",
        "--log",
        log.to_str().unwrap(),
        "--store",
        "--intent",
        "intent-C",
        "--lens",
        "security",
        "--result",
        "approve",
        "--lens",
        "contracts",
        "--result",
        "reject",
    ];

    let (code1, v1) = run(&args);
    assert_eq!(code1, 0, "first record exits 0: {v1}");
    assert_eq!(v1["already_recorded"], false, "first is fresh");
    assert_eq!(count_verdicts(&log), 1, "one event after first record");

    let (code2, v2) = run(&args);
    assert_eq!(code2, 0, "second (identical) record exits 0: {v2}");
    assert_eq!(
        v2["already_recorded"], true,
        "the identical re-run is recognised as already-recorded: {v2}"
    );
    assert_eq!(
        count_verdicts(&log),
        1,
        "no duplicate event is appended on the idempotent re-run"
    );
}

// ── ④ canonical-log seam: the payload round-trips as a VerdictObject ────────

#[test]
fn item_4_payload_round_trips_as_verdict_object() {
    let dir = scratch("roundtrip");
    let log = dir.join("log.json");
    write_empty_log(&log);

    let (code, _) = run(&[
        "verdict",
        "--log",
        log.to_str().unwrap(),
        "--store",
        "--intent",
        "intent-D",
        "--lens",
        "security",
        "--result",
        "approve",
        "--lens",
        "contracts",
        "--result",
        "fix_first",
    ]);
    assert_eq!(code, 0, "record exits 0");

    let records = read_log(&log);
    let rec = records
        .iter()
        .find(|r| r.kind == VERDICT_RECORDED_KIND)
        .expect("a verdict.recorded event is on the log");
    let vo: VerdictObject =
        serde_json::from_str(&rec.payload).expect("payload is a valid VerdictObject");
    assert_eq!(vo.intent, "intent-D");
    // Any non-approve lens fails the aggregate panel.
    assert_eq!(vo.verdict, Verdict::Reject);
    // The per-lens breakdown is carried in claims_checked (lens:result).
    assert!(vo.claims_checked.contains(&"security:approve".to_string()));
    assert!(vo.claims_checked.contains(&"contracts:fix_first".to_string()));
}

// ── ⑤ missing log under --store → log_not_found / exit-2 ────────────────────

#[test]
fn item_5_missing_log_is_log_not_found_exit_2() {
    let dir = scratch("nolog");
    let missing = dir.join("does-not-exist.json");
    let (code, v) = run(&[
        "verdict",
        "--log",
        missing.to_str().unwrap(),
        "--store",
        "--intent",
        "intent-E",
        "--lens",
        "security",
        "--result",
        "approve",
    ]);
    assert_eq!(code, 2, "a missing --log under --store is exit 2: {v}");
    assert_eq!(v["error"]["kind"], "log_not_found", "canonical envelope: {v}");
}

// ── ⑥ lens/result mismatch → lens_result_mismatch / exit-2 ──────────────────

#[test]
fn item_6_lens_result_mismatch_is_exit_2() {
    let dir = scratch("mismatch");
    let log = dir.join("log.json");
    write_empty_log(&log);
    let (code, v) = run(&[
        "verdict",
        "--log",
        log.to_str().unwrap(),
        "--store",
        "--intent",
        "intent-F",
        "--lens",
        "security",
        "--lens",
        "contracts",
        "--result",
        "approve",
    ]);
    assert_eq!(code, 2, "2 lenses but 1 result is exit 2: {v}");
    assert_eq!(
        v["error"]["kind"], "lens_result_mismatch",
        "canonical envelope: {v}"
    );
}

// ── ⑦ invalid result token → invalid_result / exit-2 ────────────────────────

#[test]
fn item_7_invalid_result_token_is_exit_2() {
    let dir = scratch("badtoken");
    let log = dir.join("log.json");
    write_empty_log(&log);
    let (code, v) = run(&[
        "verdict",
        "--log",
        log.to_str().unwrap(),
        "--store",
        "--intent",
        "intent-G",
        "--lens",
        "security",
        "--result",
        "maybe",
    ]);
    assert_eq!(code, 2, "an unknown --result token is exit 2: {v}");
    assert_eq!(
        v["error"]["kind"], "invalid_result",
        "canonical envelope: {v}"
    );
}

// ── ⑧ aggregate logic: all-approve vs any-reject ────────────────────────────

#[test]
fn item_8_aggregate_all_approve_vs_any_reject() {
    let dir = scratch("aggregate");

    // All approve → approve.
    let log_a = dir.join("a.json");
    write_empty_log(&log_a);
    let (code, v) = run(&[
        "verdict",
        "--log",
        log_a.to_str().unwrap(),
        "--store",
        "--intent",
        "intent-H",
        "--lens",
        "x",
        "--result",
        "approve",
        "--lens",
        "y",
        "--result",
        "approve",
    ]);
    assert_eq!(code, 0);
    assert_eq!(v["aggregate"], "approve", "all approve → approve: {v}");

    // One reject → reject.
    let log_b = dir.join("b.json");
    write_empty_log(&log_b);
    let (code, v) = run(&[
        "verdict",
        "--log",
        log_b.to_str().unwrap(),
        "--store",
        "--intent",
        "intent-I",
        "--lens",
        "x",
        "--result",
        "approve",
        "--lens",
        "y",
        "--result",
        "reject",
    ]);
    assert_eq!(code, 0);
    assert_eq!(v["aggregate"], "reject", "any reject → reject: {v}");
}

// ── ⑨ dry panel: no --store appends nothing to the log ──────────────────────

#[test]
fn item_9_dry_panel_without_store_records_nothing() {
    let dir = scratch("dry");
    let log = dir.join("log.json");
    write_empty_log(&log);
    let (code, v) = run(&[
        "verdict",
        "--log",
        log.to_str().unwrap(),
        "--intent",
        "intent-J",
        "--lens",
        "security",
        "--result",
        "approve",
    ]);
    assert_eq!(code, 0, "a dry panel exits 0: {v}");
    assert_eq!(v["stored"], false, "no --store ⇒ not recorded: {v}");
    assert_eq!(v["aggregate"], "approve", "the panel still convened: {v}");
    assert_eq!(
        count_verdicts(&log),
        0,
        "a dry panel appends nothing to the log"
    );
}
