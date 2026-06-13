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
/// F2 fix: the previous version scanned a HARD-CODED array of 5 files, so a
/// brand-new read verb in a NEW file was invisible to the guard — the exact
/// failure mode that recurred in R7/R8 (why/export/intent-list each "forgot
/// verify_chain").  This version WALKS the entire `src/**/*.rs` tree and flags
/// any non-exempt file that hand-rolls `verify_chain(`/`.push_record(`.
///
/// ## Exemption rationale (documented per-file)
///
/// - `checks/mod.rs`  — the ONE legitimate home of the loop (the chokepoint
///   itself); write-path append primitives live here too.
/// - `intent/store.rs` — reads `IntentStoreFile` (a DIFFERENT on-disk shape
///   with its OWN embedded spine); not a canonical-log loader.
/// - `export/cut.rs`  — `verify_chain` here is tagged `readpath-verify-exempt`
///   (re-verifies a SUB-RANGE of an already-chokepoint-loaded in-memory log,
///   NOT a disk read); `.push_record` reconstructs the in-memory EventLog from
///   an already-verified cut (write-path, not disk-read).
/// - `export/mod.rs`  — `.push_record` usages are write-path/restore operations
///   (`export` format serialisation, `restore_from_bytes`); no canonical-log
///   disk read.
/// - `why/` (`why/mod.rs`, `why/resolver.rs`) — reads `why`'s OWN wrapper shape
///   (`[{record,attestation?,sidecar?},…]`), not the bare `[EventRecord,…]`
///   canonical log; has its own K-CHAIN verify (correctly documented).
/// - `main.rs` — hosts `run_why`; same special-shape exemption as `why/`.
///
/// Any file NOT in this exemption set that contains `verify_chain(` or
/// `.push_record(` (outside comments or `readpath-verify-exempt` lines) is
/// a NEW canonical-log inline bypass — the test FAILS.
#[test]
fn canonical_log_loaders_route_through_the_chokepoint() {
    use std::collections::HashSet;

    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let src = manifest.join("src");

    // ── Exempted files (paths relative to `src/`, normalised for comparison) ─
    //
    // All exemptions are by SUFFIX of the canonical path so they are
    // OS-independent (no hard-coded separators).
    //
    // Exempt: the chokepoint itself + write-path + special-shape siblings.
    let exempt_suffixes: HashSet<&str> = [
        // The chokepoint: the ONE legitimate home of verify_chain + push_record.
        "checks/mod.rs",
        // IntentStoreFile — different on-disk shape, own embedded spine.
        "intent/store.rs",
        // export: write-path push_record (in-memory log construction/restore)
        // and readpath-verify-exempt sub-range re-verify.
        "export/cut.rs",
        "export/mod.rs",
        // `why` verb: reads its OWN wrapper shape (not bare [EventRecord,…]).
        "why/mod.rs",
        "why/resolver.rs",
        // main.rs hosts run_why; same special-shape exemption.
        "main.rs",
    ]
    .iter()
    .copied()
    .collect();

    /// Collect all `.rs` files under `dir` recursively via `std::fs::read_dir`.
    fn collect_rs(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_rs(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }

    let mut all_rs: Vec<PathBuf> = Vec::new();
    collect_rs(&src, &mut all_rs);
    all_rs.sort(); // deterministic order for readable failure messages

    let mut offenders: Vec<String> = Vec::new();

    for path in &all_rs {
        // Check whether this path ends with any exempted suffix.
        // We normalise to forward slashes so the suffix match is cross-platform.
        let path_str = path.to_string_lossy().replace('\\', "/");
        let is_exempt = exempt_suffixes.iter().any(|suf| path_str.ends_with(suf));
        if is_exempt {
            continue;
        }

        let text = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("read {}: {e}", path.display()));

        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            // Skip comment lines — they legitimately NAME the patterns in docs.
            if trimmed.starts_with("//") {
                continue;
            }
            // An inline `readpath-verify-exempt` marker on the same line
            // documents a deliberate non-canonical-log use of the primitives.
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
        "F2 / PS-13: a source file outside the exempt set hand-rolls \
         verify_chain/push_record instead of routing through the single chokepoint \
         `checks::rehydrate_and_verify`.  Adding verify_chain inline in a new file \
         is the recurring R7/R8 regression class.  Either route through the chokepoint \
         or add the file to the exempt list with a documented rationale.\n\
         Offenders:\n{}",
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
