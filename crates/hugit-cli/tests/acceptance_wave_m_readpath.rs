//! Acceptance — Wave M, WP M-1 (PS-13): the single verified-loader chokepoint.
//!
//! PS-13 root (recurring across R7 why/export, R8 intent list): every canonical
//! `[EventRecord, …]` read verb re-implemented its OWN
//! `EventLog::new() + push_record + verify_chain` loop, so a NEW read verb could
//! forget `verify_chain` and project a tampered log as authoritative.
//!
//! Wave M converges every canonical-log disk loader onto ONE chokepoint —
//! `crate::checks::rehydrate_and_verify` — the SOLE production site of the
//! rehydrate-and-verify sequence. This suite proves, by live tamper repro
//! against the REAL `hugit` binary (temp dirs OUTSIDE the repo tree), that
//! EVERY canonical read verb fails closed on a tampered chain:
//!
//!   - a VALID log → normal projection, exit 0;
//!   - a TAMPERED log (one `this_hash`/payload byte broken) → `chain_broken`,
//!     exit 2, NEVER a projected tampered state.
//!
//! Read verbs covered: `intent list --log`, `pr show --log`, `campaign show
//! --log`, `export --log`. Plus a SOURCE-INVARIANT test (mirroring L-D's
//! `no_out_of_crate_raw_append`) that proves no canonical-log loader
//! re-implements `verify_chain`/`push_record` outside the chokepoint — so
//! forgetting becomes structurally impossible, not merely "remembered".

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

static SCRATCH_CTR: AtomicU64 = AtomicU64::new(0);

fn scratch(tag: &str) -> PathBuf {
    let n = SCRATCH_CTR.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "hugit-wave-m-readpath-{tag}-{}-{n}",
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

/// Seed a full canonical log: `campaign open` → `intent new` → `pr open` on ONE
/// shared `[EventRecord, …]` log, so every read verb has real state to project.
/// Returns `(log_path, store_path)`.
fn seed_full_log(dir: &Path, campaign: &str, intent: &str, pr: &str) -> (PathBuf, PathBuf) {
    let log = dir.join("L.json");
    let store = dir.join("store.json");
    let log_s = log.to_str().unwrap();
    let store_s = store.to_str().unwrap();

    let (code, v) = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        campaign,
        "--charter",
        "wave-m read-path",
        "--owner",
        "o@h.com",
    ]);
    assert_eq!(code, 0, "campaign open seeds the log: {v}");

    let (code, v) = run(&[
        "intent",
        "new",
        "--log",
        log_s,
        "--store",
        store_s,
        "--campaign",
        campaign,
        "--charter",
        "wave-m intent",
        "--id",
        intent,
    ]);
    assert_eq!(code, 0, "intent new seeds the log: {v}");

    let (code, v) = run(&[
        "pr",
        "open",
        "--log",
        log_s,
        "--pr",
        pr,
        "--campaign",
        campaign,
        "--author-kind",
        "orchestrator",
        "--run-id",
        "r",
        "--intent",
        intent,
    ]);
    assert_eq!(code, 0, "pr open seeds the log: {v}");

    (log, store)
}

/// Hand-tamper the log: mutate the FIRST occurrence of `needle` (a payload byte)
/// WITHOUT recomputing the hash chain — the record still parses (monotonic seq
/// intact) but `this_hash` no longer matches the bytes, so `verify_chain` fails.
fn tamper(log: &Path, needle: &str, replacement: &str) {
    let raw = std::fs::read_to_string(log).unwrap();
    let tampered = raw.replacen(needle, replacement, 1);
    assert_ne!(raw, tampered, "the tamper must actually mutate a byte");
    std::fs::write(log, &tampered).unwrap();
}

/// Assert a `(code, json)` pair is a clean `chain_broken`/exit-2 envelope and
/// projected NO authoritative state (no top-level projection key present).
fn assert_chain_broken(code: i32, v: &Value, projection_key: &str) {
    assert_eq!(
        code, 2,
        "tampered log must exit 2 (chain_broken), never project: {v}"
    );
    assert_eq!(
        v["error"]["kind"], "chain_broken",
        "error kind must be chain_broken: {v}"
    );
    assert!(
        v["error"]["fix"].is_string(),
        "error must carry a fix string: {v}"
    );
    assert!(
        v.get("kind").is_none(),
        "error must be nested under 'error', not flat: {v}"
    );
    assert!(
        v.get(projection_key).is_none(),
        "a tampered log must NOT produce a '{projection_key}' projection: {v}"
    );
}

// ── intent list ──────────────────────────────────────────────────────────────

#[test]
fn intent_list_valid_projects_then_tamper_is_chain_broken() {
    let dir = scratch("intent-list");
    let (log, store) = seed_full_log(&dir, "m-camp", "m-i1", "1");
    let (log_s, store_s) = (log.to_str().unwrap(), store.to_str().unwrap());

    let (code, v) = run(&["intent", "list", "--store", store_s, "--log", log_s]);
    assert_eq!(code, 0, "valid log → exit 0: {v}");
    assert_eq!(v["intents"][0]["landed"], serde_json::json!(true), "{v}");

    tamper(&log, "wave-m intent", "wave-m intent TAMPERED");
    let (code, v) = run(&["intent", "list", "--store", store_s, "--log", log_s]);
    assert_chain_broken(code, &v, "intents");
}

// ── pr show ──────────────────────────────────────────────────────────────────

#[test]
fn pr_show_valid_projects_then_tamper_is_chain_broken() {
    let dir = scratch("pr-show");
    let (log, _store) = seed_full_log(&dir, "m-camp", "m-i1", "1");
    let log_s = log.to_str().unwrap();

    let (code, v) = run(&["pr", "show", "--log", log_s, "--pr", "1"]);
    assert_eq!(code, 0, "valid log → exit 0: {v}");
    assert_eq!(v["pr_id"], "1", "pr projection present on a clean log: {v}");

    tamper(&log, "wave-m intent", "wave-m intent TAMPERED");
    let (code, v) = run(&["pr", "show", "--log", log_s, "--pr", "1"]);
    assert_chain_broken(code, &v, "pr_id");
}

// ── campaign show ────────────────────────────────────────────────────────────

#[test]
fn campaign_show_valid_projects_then_tamper_is_chain_broken() {
    let dir = scratch("campaign-show");
    let (log, _store) = seed_full_log(&dir, "m-camp", "m-i1", "1");
    let log_s = log.to_str().unwrap();

    let (code, v) = run(&["campaign", "show", "--log", log_s, "--campaign", "m-camp"]);
    assert_eq!(code, 0, "valid log → exit 0: {v}");
    assert!(
        v.get("progress").is_some(),
        "campaign projection present on a clean log: {v}"
    );

    tamper(&log, "wave-m intent", "wave-m intent TAMPERED");
    let (code, v) = run(&["campaign", "show", "--log", log_s, "--campaign", "m-camp"]);
    assert_chain_broken(code, &v, "progress");
}

// ── export ───────────────────────────────────────────────────────────────────

#[test]
fn export_valid_projects_then_tamper_is_chain_broken() {
    let dir = scratch("export");
    let (log, _store) = seed_full_log(&dir, "m-camp", "m-i1", "1");
    let log_s = log.to_str().unwrap();
    let out_dir = dir.join("out");

    let (code, v) = run(&["export", "--log", log_s, "--out", out_dir.to_str().unwrap()]);
    assert_eq!(code, 0, "valid log → exit 0: {v}");
    assert!(
        v["exported"]["git_dir"].is_string(),
        "export projection present on a clean log: {v}"
    );

    tamper(&log, "wave-m intent", "wave-m intent TAMPERED");
    let out_dir2 = dir.join("out2");
    let (code, v) = run(&[
        "export",
        "--log",
        log_s,
        "--out",
        out_dir2.to_str().unwrap(),
    ]);
    assert_chain_broken(code, &v, "exported");
}

// ── source invariant (PS-13 compile/source-time guard) ───────────────────────

/// PS-13 SOURCE INVARIANT (mirrors L-D's `no_out_of_crate_raw_append`): the
/// canonical-`[EventRecord, …]`-log disk loaders MUST NOT re-implement the
/// `verify_chain` / `push_record` rehydrate loop — they route through the ONE
/// chokepoint `crate::checks::rehydrate_and_verify`. This converts "every read
/// verb must remember to verify_chain" from an enforced-by-convention rule into
/// a SOURCE-ENFORCED invariant: a new read verb that hand-rolls the loop, or an
/// existing loader that regresses to inline verify, FAILS this test.
///
/// Scope: the canonical-log loader files. The chokepoint file (`checks/mod.rs`)
/// is the ONE legitimate home of the loop. `export/cut.rs`'s prefix re-verify is
/// a DIFFERENT operation (re-verify a sub-range of an already-chokepoint-loaded
/// in-memory log, not a disk read) and is tagged `readpath-verify-exempt`.
/// `intent/store.rs` reads a DIFFERENT on-disk shape (`IntentStoreFile`, its own
/// embedded spine with its own verify) and is out of the canonical-loader set.
#[test]
fn canonical_log_loaders_route_through_the_chokepoint() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = manifest.join("src");

    // The canonical-log disk loaders converged in Wave M. NONE of these may
    // contain an inline `verify_chain(` or `push_record(` (they route through
    // `checks::rehydrate_and_verify`).
    let converged = [
        src.join("intent").join("list.rs"),
        src.join("intent").join("canonical_log.rs"),
        src.join("pr").join("cli.rs"),
        src.join("campaign").join("world.rs"),
    ];

    let mut offenders: Vec<String> = Vec::new();
    for path in &converged {
        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // Skip comments and doc-comments (they legitimately NAME the pattern).
            if trimmed.starts_with("//") {
                continue;
            }
            // An explicit, documented exemption marker keeps a deliberate
            // non-disk-read verify legal (none in these files today).
            if line.contains("readpath-verify-exempt") {
                continue;
            }
            if line.contains("verify_chain(") || line.contains(".push_record(") {
                offenders.push(format!("{}:{}: {}", path.display(), i + 1, line.trim()));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "a canonical-log loader re-implements verify_chain/push_record instead of \
         routing through the single chokepoint `checks::rehydrate_and_verify` — \
         the PS-13 read-path chokepoint is bypassable. Offenders:\n{}",
        offenders.join("\n")
    );
}

/// PS-13: the chokepoint exists and is the ONE place the rehydrate-and-verify
/// loop lives in `checks/mod.rs`. A guard against silently deleting the
/// chokepoint (which would push the loop back out to the verbs).
#[test]
fn the_chokepoint_function_exists_in_checks() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let checks = manifest.join("src").join("checks").join("mod.rs");
    let text = std::fs::read_to_string(&checks).unwrap();
    assert!(
        text.contains("fn rehydrate_and_verify("),
        "the PS-13 chokepoint `rehydrate_and_verify` must live in checks/mod.rs"
    );
    assert!(
        text.contains("verify_chain(") && text.contains(".push_record("),
        "the chokepoint must run the push_record + verify_chain sequence"
    );
}
