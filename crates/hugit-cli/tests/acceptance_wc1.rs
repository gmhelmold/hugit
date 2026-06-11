//! WP-WC1 acceptance — the atomic, lock-disciplined porcelain file seam, the
//! structured corrupt/truncated-log error paths, and the concurrency proof, all
//! against the REAL `hugit` binary (`CARGO_BIN_EXE_hugit`).
//!
//! Proves:
//!   ① **Corrupt-log rejection per module** — `intent`/`campaign`/`pr` each
//!      reject a half-written (truncated) `--log`, an invalid-UTF-8 `--log`, and
//!      a valid-JSON-WRONG-SHAPE `--log` with a deterministic structured error
//!      and a non-success exit — NEVER a fake success, NEVER a silent empty
//!      world, NEVER a clobber of the bytes on disk.
//!   ② **Lock serialization (TOCTOU dead)** — while one verb holds the
//!      `<log>.lock`, a second `pr`/`campaign`/`intent` verb on the same log
//!      fails structured (`log_busy`/`store_busy`) rather than clobbering.
//!   ③ **Concurrency proof** — two SIMULTANEOUS `intent new --log` processes on
//!      one log either both serialize (two records) or exactly one wins
//!      (`log_busy` for the loser); the log ends VALID with NO truncation and the
//!      hash chain still verifies.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-wc1-{tag}-{}-{}",
        std::process::id(),
        nanos()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Run `hugit <args>` → `(exit_success, parsed_stdout_json_or_null, stderr)`.
fn run(args: &[&str]) -> (bool, Value, String) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.success(), v, stderr)
}

/// Seed a one-record valid canonical `[EventRecord, …]` log so `pr`/`campaign`
/// verbs that require an existing log have something honest to read — via the
/// real binary (a `campaign open`), so the chain is genuinely valid.
fn seed_valid_log(dir: &Path) -> PathBuf {
    let log = dir.join("valid.json");
    let (ok, _, err) = run(&[
        "campaign",
        "open",
        "--log",
        log.to_str().unwrap(),
        "--campaign",
        "wc1",
        "--charter",
        "c",
        "--owner",
        "o@h.com",
    ]);
    assert!(ok, "seed campaign open must succeed: {err}");
    log
}

// ── ① corrupt-log rejection per module ───────────────────────────────────────

/// The three corruption shapes the WP requires, as raw bytes.
fn corruptions() -> [(&'static str, Vec<u8>); 3] {
    [
        // Half-written JSON (a truncated array — the classic mid-write crash).
        ("truncated", b"[{\"seq\":0,\"prev_hash\":".to_vec()),
        // Invalid UTF-8 (a 0xff byte never appears in valid UTF-8).
        ("invalid_utf8", vec![b'[', 0xff, 0xfe, b']']),
        // Valid JSON, WRONG shape (a map where the canonical seam is an array).
        ("wrong_shape", br#"{"events":[]}"#.to_vec()),
    ]
}

#[test]
fn intent_rejects_every_corrupt_log_without_clobber() {
    for (tag, bytes) in corruptions() {
        let dir = scratch(&format!("intent-{tag}"));
        let log = dir.join("L.json");
        let store = dir.join("store.json");
        std::fs::write(&log, &bytes).unwrap();

        let (ok, v, stderr) = run(&[
            "intent",
            "new",
            "--log",
            log.to_str().unwrap(),
            "--store",
            store.to_str().unwrap(),
            "--campaign",
            "wc1",
            "--charter",
            "do a thing",
        ]);
        assert!(
            !ok,
            "[{tag}] intent must reject a corrupt --log: {v} {stderr}"
        );
        // Structured error on stdout (intent's canonical envelope), kind `parse`.
        assert_eq!(
            v["error"]["kind"], "parse",
            "[{tag}] corrupt --log is a structured parse error: {v}"
        );
        assert!(
            v["error"]["fix"].is_string(),
            "[{tag}] the parse error carries a fix: {v}"
        );
        // The corrupt bytes are NEVER clobbered into a fake-valid log.
        assert_eq!(
            std::fs::read(&log).unwrap(),
            bytes,
            "[{tag}] a rejected --log is left byte-for-byte untouched"
        );
    }
}

#[test]
fn campaign_rejects_every_corrupt_log_without_clobber() {
    for (tag, bytes) in corruptions() {
        let dir = scratch(&format!("camp-{tag}"));
        let log = dir.join("L.json");
        std::fs::write(&log, &bytes).unwrap();

        let (ok, v, stderr) = run(&[
            "campaign",
            "open",
            "--log",
            log.to_str().unwrap(),
            "--campaign",
            "wc1",
            "--charter",
            "c",
            "--owner",
            "o@h.com",
        ]);
        assert!(
            !ok,
            "[{tag}] campaign must reject a corrupt --log: {v} {stderr}"
        );
        assert_eq!(
            v["error"]["kind"], "parse",
            "[{tag}] corrupt --log is a structured parse error: {v}"
        );
        assert_eq!(
            std::fs::read(&log).unwrap(),
            bytes,
            "[{tag}] a rejected --log is left byte-for-byte untouched"
        );
    }
}

#[test]
fn pr_rejects_every_corrupt_log_without_clobber() {
    for (tag, bytes) in corruptions() {
        let dir = scratch(&format!("pr-{tag}"));
        let log = dir.join("L.json");
        std::fs::write(&log, &bytes).unwrap();

        // `pr show` reads the log; a corrupt log is a seam fault (the documented
        // pr `--log` seam-fault path: a `hugit: error:` line on stderr, a
        // non-success exit) — deterministic, never a fake success, never a
        // clobber. (The one-error-JSON convergence for pr is Wave-B1's; WC1
        // proves the rejection is structured + non-destructive.)
        let (ok, _v, stderr) = run(&["pr", "show", "--log", log.to_str().unwrap(), "--pr", "1"]);
        assert!(!ok, "[{tag}] pr must reject a corrupt --log: {stderr}");
        assert!(
            stderr.contains("parse log") || stderr.contains("rehydrate log"),
            "[{tag}] pr surfaces a deterministic parse/rehydrate fault: {stderr}"
        );
        assert_eq!(
            std::fs::read(&log).unwrap(),
            bytes,
            "[{tag}] a rejected --log is left byte-for-byte untouched"
        );
    }
}

// ── ② lock serialization — a held lock serializes the next verb ──────────────

#[test]
fn held_lock_makes_the_next_pr_verb_log_busy() {
    let dir = scratch("pr-busy");
    let log = seed_valid_log(&dir);
    // Simulate a live holder by planting a FRESH `<log>.lock` (a just-created
    // lock is never stale → never reclaimed, so the next verb must report busy).
    let lock = lock_path(&log);
    std::fs::write(&lock, b"pid=999999 at_unix=9999999999\n").unwrap();

    let (ok, v, stderr) = run(&[
        "pr",
        "open",
        "--log",
        log.to_str().unwrap(),
        "--pr",
        "1",
        "--campaign",
        "wc1",
        "--author-kind",
        "orchestrator",
        "--run-id",
        "r",
        "--intent",
        "i1",
    ]);
    assert!(
        !ok,
        "a held lock must block the mutating verb: {v} {stderr}"
    );
    assert_eq!(
        v["error"]["kind"], "log_busy",
        "the blocked verb reports the structured log_busy: {v}"
    );
    // The lock is still there (we never clobbered it / never wrote the log).
    assert!(lock.exists(), "the foreign lock is left intact");
}

#[test]
fn held_lock_makes_campaign_open_log_busy() {
    let dir = scratch("camp-busy");
    let log = seed_valid_log(&dir);
    let lock = lock_path(&log);
    std::fs::write(&lock, b"pid=999999 at_unix=9999999999\n").unwrap();

    let (ok, v, stderr) = run(&[
        "campaign",
        "open",
        "--log",
        log.to_str().unwrap(),
        "--campaign",
        "wc1b",
        "--charter",
        "c",
        "--owner",
        "o@h.com",
    ]);
    assert!(!ok, "a held lock must block campaign open: {v} {stderr}");
    assert_eq!(v["error"]["kind"], "log_busy", "structured log_busy: {v}");
}

#[test]
fn held_store_lock_makes_intent_new_store_busy() {
    let dir = scratch("store-busy");
    let store = dir.join("store.json");
    // Plant a fresh lock on the STORE path (intent new's save acquires it).
    let lock = lock_path(&store);
    std::fs::write(&lock, b"pid=999999 at_unix=9999999999\n").unwrap();

    let (ok, v, stderr) = run(&[
        "intent",
        "new",
        "--store",
        store.to_str().unwrap(),
        "--campaign",
        "wc1",
        "--charter",
        "blocked",
    ]);
    assert!(!ok, "a held store lock must block intent new: {v} {stderr}");
    assert_eq!(
        v["error"]["kind"], "store_busy",
        "the blocked save reports the structured store_busy: {v}"
    );
}

fn lock_path(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

// ── ③ the concurrency proof — two simultaneous `intent new --log` ────────────

#[test]
fn two_concurrent_intent_new_log_never_truncate_the_chain() {
    let dir = scratch("concurrency");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap().to_string();

    // Two DISTINCT intents (distinct charters → distinct content-derived ids), so
    // a clean serialization lands TWO records; a lock loss for one lands one. We
    // spawn both as close to simultaneously as possible (no sleeps — both child
    // processes contend for the same `<log>.lock`).
    let spawn = |charter: &str| {
        Command::new(hugit_bin())
            .args([
                "intent",
                "new",
                "--log",
                &log_s,
                "--store",
                dir.join(format!("store-{charter}.json")).to_str().unwrap(),
                "--campaign",
                "wc1",
                "--charter",
                charter,
            ])
            // Pipe the child's streams so `wait_with_output` actually captures
            // them (the default inherits the parent's stdout → empty capture).
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn intent new")
    };

    // Run the race a handful of times to actually exercise contention.
    for round in 0..8 {
        let _ = std::fs::remove_file(&log);
        let _ = std::fs::remove_file(lock_path(&log));

        let a = spawn(&format!("alpha-{round}"));
        let b = spawn(&format!("beta-{round}"));
        let oa = a.wait_with_output().expect("wait a");
        let ob = b.wait_with_output().expect("wait b");

        // At least one must have succeeded (someone wins the lock first).
        let a_ok = oa.status.success();
        let b_ok = ob.status.success();
        assert!(
            a_ok || b_ok,
            "round {round}: at least one writer must win; \
             a={:?} b={:?}",
            String::from_utf8_lossy(&oa.stdout),
            String::from_utf8_lossy(&ob.stdout),
        );

        // A loser (if any) must have failed STRUCTURED with log_busy — never a
        // silent clobber, never a crash.
        for (label, out) in [("a", &oa), ("b", &ob)] {
            if !out.status.success() {
                let v: Value = serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim())
                    .unwrap_or(Value::Null);
                assert_eq!(
                    v["error"]["kind"],
                    "log_busy",
                    "round {round}: loser {label} must fail structured log_busy: \
                     stdout={} stderr={}",
                    String::from_utf8_lossy(&out.stdout),
                    String::from_utf8_lossy(&out.stderr),
                );
            }
        }

        // INVARIANT: the log is a VALID, non-truncated canonical array whose
        // chain verifies, with at least one record and at most two — never a
        // half-write, never a fork.
        assert_chain_is_valid(&log, round);
    }
}

/// Assert the log at `path` parses as a canonical `[EventRecord, …]` array whose
/// hash chain verifies through the real engine, with 1..=2 records (a truncated
/// or forked log fails to parse / fails verification).
fn assert_chain_is_valid(path: &Path, round: usize) {
    let bytes = std::fs::read(path).expect("log exists after the race");
    let records: Vec<hugit_contracts::event_record::EventRecord> = serde_json::from_slice(&bytes)
        .unwrap_or_else(|e| {
            panic!(
                "round {round}: log must be VALID JSON (no truncation): {e}; bytes={:?}",
                String::from_utf8_lossy(&bytes)
            )
        });
    assert!(
        (1..=2).contains(&records.len()),
        "round {round}: 1..=2 records, got {}",
        records.len()
    );
    // The real engine's chain verifier — a forked/clobbered chain fails here.
    let mut log = hugit_refstore::EventLog::new();
    for r in records {
        log.push_record(r)
            .unwrap_or_else(|e| panic!("round {round}: record rehydrates: {e}"));
    }
    hugit_refstore::verify_chain(log.records())
        .unwrap_or_else(|e| panic!("round {round}: chain MUST verify after the race: {e}"));
}
