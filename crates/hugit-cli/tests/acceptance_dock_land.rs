//! WP-DOCK-6 — landing per-dock: byte-identity + acceptance verification.
//!
//! Proves the acceptance hermetically AND over REAL git:
//!
//! - **L6 (safety — verified transport)** — the land records SUCCESS only
//!   when the worktree tip byte-matches the recorded tip; a divergence is
//!   FAIL-CLOSED (never "landed" with a mismatch). No recorded tip ⇒ honest
//!   `no_recorded_tip` (no success claim, L8).
//! - **L7 (safety — acceptance gate)** — a RED acceptance never lands the
//!   dock (excluded, honest); GREEN lands.
//! - **L8 (safety — no fabrication)** — a fault in the acceptance execution is
//!   surfaced RED, never silently "accepted".
//! - **L9 (liveness — settles)** — green byte-identity + green acceptance ⇒
//!   the verb completes with `dock.landed` recorded (idempotent replay).
//! - **F2 (cost-independence)** — landing works with cost present AND absent;
//!   cost is reported honestly (never fabricated).

use std::path::{Path, PathBuf};

use hugit_cli::dock::land::{AcceptanceOutcome, ByteIdentityOutcome, DOCK_LANDED_KIND, land_dock};
use hugit_refstore::EventLog;

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-dock6-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn git_in(cwd: &Path, args: &[&str]) -> (i32, String) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// A REAL git repo (non-bare) with one commit on `branch`; returns (path, tip).
/// The dock's gitdir is `<path>/.git` — a real object store the byte-identity
/// read can resolve. Each caller MUST pass a UNIQUE `tag` (tests run in
/// parallel; a shared scratch tag would collide).
fn real_repo_at(tag: &str, branch: &str) -> (PathBuf, String) {
    let d = scratch(tag);
    std::fs::write(d.join("a.txt"), "a").unwrap();
    let _ = git_in(&d, &["init", "-q", "-b", "main"]);
    let _ = git_in(&d, &["config", "user.email", "t@t"]);
    let _ = git_in(&d, &["config", "user.name", "t"]);
    let _ = git_in(&d, &["add", "-A"]);
    let _ = git_in(&d, &["commit", "-q", "-m", "init"]);
    let _ = git_in(&d, &["switch", "-q", "-c", branch]);
    std::fs::write(d.join("b.txt"), "b").unwrap();
    let _ = git_in(&d, &["add", "-A"]);
    let _ = git_in(&d, &["commit", "-q", "-m", "feature"]);
    let tip = git_in(&d, &["rev-parse", "HEAD"]).1.trim().to_string();
    (d, tip)
}

/// Append a dock record + commit-kind ref.update (the canonical-log inputs).
fn seed_log(log: &mut EventLog, dock_id: &str, branch: &str, gitdir: &str, tip: &str) {
    log.append_for_test(
        "dock.record",
        vec!["t".to_string()],
        serde_json::json!({
            "dock_id": dock_id, "gitdir": gitdir, "branch": branch,
            "charter": "test", "charter_derived": true, "state": "open",
            "origin": "worktree", "created_ts": 1000, "pid": 1,
        })
        .to_string(),
        1,
    );
    log.append_for_test(
        "ref.update",
        vec!["t".to_string()],
        serde_json::json!({
            "ref": format!("refs/heads/{branch}"), "target": tip, "branch": branch,
        })
        .to_string(),
        2,
    );
}

/// The file-backed ActionCache the `hugit check run --store` seam uses.
fn file_ac(tag: &str) -> hugit_cli::checks::run::FileAc {
    let dir = scratch(tag);
    hugit_cli::checks::run::FileAc::new(dir.join("ac.json"))
}

// ── L6 — byte-identity verified/diverged ────────────────────────────────────

#[test]
fn l6_and_l9_verified_tip_lands_exactly_once() {
    let (repo, tip) = real_repo_at("rep6v", "feat/x");
    let gitdir = repo.join(".git");
    let ac = file_ac("l6v");
    let mut log = EventLog::new();
    seed_log(&mut log, "dock-a", "feat/x", gitdir.to_str().unwrap(), &tip);

    let res = land_dock(&mut log, &ac, "dock-a", 0, false, false).unwrap();
    assert_eq!(res.byte_identity, ByteIdentityOutcome::Verified);
    assert_eq!(res.acceptance, AcceptanceOutcome::Green);
    assert!(res.landed, "L9 — green byte + green accept ⇒ dock settles");
    assert!(log.records().iter().any(|r| r.kind == DOCK_LANDED_KIND));

    // Idempotent replay: no SECOND record, honest `landed:false` (already
    // landed — the verb does not re-assert a new settle).
    let replay = land_dock(&mut log, &ac, "dock-a", 0, false, false).unwrap();
    assert!(
        !replay.landed,
        "idempotent — settled once, replay is a no-op"
    );
    let n = log
        .records()
        .iter()
        .filter(|r| r.kind == DOCK_LANDED_KIND)
        .count();
    assert_eq!(n, 1, "exact-once — one dock.landed");
}

#[test]
fn l6_diverged_tip_fails_closed_never_lands() {
    let (repo, tip) = real_repo_at("rep6d", "feat/x");
    // The recorded tip is a DIFFERENT object than the worktree's current HEAD.
    let other = {
        // a second commit on the SAME repo after the tip → a real diverged sha
        std::fs::write(repo.join("c.txt"), "c").unwrap();
        let _ = git_in(&repo, &["add", "-A"]);
        let _ = git_in(&repo, &["commit", "-q", "-m", "diverged"]);
        git_in(&repo, &["rev-parse", "HEAD"]).1.trim().to_string()
    };
    // Record a tip that does NOT match the current worktree state (`tip`, the
    // pre-divergence sha): gitdir now at `other`, log records `tip`.
    let gitdir = repo.join(".git");
    let ac = file_ac("l6d");
    let mut log = EventLog::new();
    seed_log(&mut log, "dock-b", "feat/x", gitdir.to_str().unwrap(), &tip);

    let res = land_dock(&mut log, &ac, "dock-b", 0, false, false).unwrap();
    match res.byte_identity {
        ByteIdentityOutcome::Diverged {
            expected,
            observed: obs,
        } => {
            assert_eq!(expected, tip);
            assert_eq!(obs, other);
        }
        other => panic!("FAIL-CLOSED — expected Diverged, got {other:?}"),
    }
    assert!(!res.landed, "L6 — divergence never lands");
    assert!(
        !log.records().iter().any(|r| r.kind == DOCK_LANDED_KIND),
        "no dock.landed on divergence"
    );
}

#[test]
fn l8_no_recorded_tip_is_honest_no_success() {
    // A dock with NO commit-kind ref.update for its branch — nothing verified.
    let (repo, _tip) = real_repo_at("rep8c", "feat/x");
    let gitdir = repo.join(".git");
    let ac = file_ac("l8");
    let mut log = EventLog::new();
    log.append_for_test(
        "dock.record",
        vec!["t".to_string()],
        serde_json::json!({
            "dock_id": "dock-c", "gitdir": gitdir.to_str().unwrap(), "branch": "feat/x",
            "charter": "test", "charter_derived": true, "state": "open",
            "origin": "worktree", "created_ts": 1000, "pid": 1,
        })
        .to_string(),
        1,
    );

    let res = land_dock(&mut log, &ac, "dock-c", 0, false, false).unwrap();
    assert_eq!(
        res.byte_identity,
        ByteIdentityOutcome::NoRecordedTip,
        "L8 — no recorded tip ⇒ no success claim"
    );
    assert!(!res.landed, "never lands without a verified tip");
}

// ── L7 — the acceptance gate ─────────────────────────────────────────────────

/// The `land_dock` signature proxies the real runner; to force RED I use the
/// `force_red_accept` hook (production would swap a real runner behind the
/// trait). This proves the gate excludes a RED even when byte-identity holds.
#[test]
fn l7_red_acceptance_excluded_even_when_byte_identity_verified() {
    let (repo, tip) = real_repo_at("rep7d", "feat/x");
    let gitdir = repo.join(".git");
    let ac = file_ac("l7");
    let mut log = EventLog::new();
    seed_log(&mut log, "dock-d", "feat/x", gitdir.to_str().unwrap(), &tip);

    let res = land_dock(&mut log, &ac, "dock-d", 0, false, true).unwrap();
    assert_eq!(res.byte_identity, ByteIdentityOutcome::Verified);
    assert_eq!(res.acceptance, AcceptanceOutcome::Red);
    assert!(!res.landed, "L7 — a RED acceptance never lands the dock");
    assert!(
        !log.records().iter().any(|r| r.kind == DOCK_LANDED_KIND),
        "RED excluded — no dock.landed"
    );
}

// ── F2 — cost present/absent is independent and honest ──────────────────────

#[test]
fn f2_cost_present_and_absent_both_independent_and_report_honest() {
    let (repo, tip) = real_repo_at("repf2", "feat/x");
    let gitdir = repo.join(".git");

    // Absent cost: a dock with NO cost.sample → honest zero, still lands.
    let ac = file_ac("f2a");
    let mut log = EventLog::new();
    seed_log(&mut log, "dock-e", "feat/x", gitdir.to_str().unwrap(), &tip);
    let res = land_dock(&mut log, &ac, "dock-e", 0, false, false).unwrap();
    assert_eq!(res.cost_usd_micros, 0, "F2 — honest zero when absent");
    assert!(res.landed, "F2 — absent cost never blocks landing");

    // Present cost: a cost.sample on the dock lands and reports it.
    let ac2 = file_ac("f2p");
    let mut log2 = EventLog::new();
    seed_log(
        &mut log2,
        "dock-f",
        "feat/x",
        gitdir.to_str().unwrap(),
        &tip,
    );
    log2.append_for_test(
        "cost.sample",
        vec!["t".to_string()],
        serde_json::json!({
            "run_id": "r-1", "dock_id": "dock-f", "model": "m",
            "input_tokens": 1, "output_tokens": 1,
            "cost_usd_micros": 123_456, "ts_ms": 2000,
            "content_hash": "h", "is_unlabeled": false,
        })
        .to_string(),
        3,
    );
    let res2 = land_dock(&mut log2, &ac2, "dock-f", 0, false, false).unwrap();
    assert_eq!(res2.cost_usd_micros, 123_456, "F2 — cost reported honestly");
    assert!(res2.landed);
}

/// F1 regression (cold-verify audit) — the CLI `dock land` shell must persist
/// the `dock.close` AFTER the `dock.landed`, NOT swallow a lock re-entrancy
/// failure. This drives the REAL binary, so the `run()` shell (which no
/// library test covers) is exercised end to end.
#[test]
fn f1_cli_dock_land_persists_the_close_record() {
    let (repo, tip) = real_repo_at("repf1", "feat/f1");
    let gitdir = repo.join(".git");
    // Real log on disk (the CLI reads/writes a file, not an in-memory log).
    let dir = scratch("f1log");
    let log_path = dir.join(".hugit/log.json");
    std::fs::create_dir_all(dir.join(".hugit")).unwrap();
    std::fs::write(&log_path, "[]").unwrap();
    let mut log = hugit_cli::checks::load_event_log(&log_path).unwrap();
    seed_log(
        &mut log,
        "dock-f1",
        "feat/f1",
        gitdir.to_str().unwrap(),
        &tip,
    );
    let bytes = serde_json::to_vec_pretty(log.records()).unwrap();
    std::fs::write(&log_path, bytes).unwrap();
    drop(log);

    // Drive the REAL binary via `dock land`.
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_hugit"))
        .args([
            "dock",
            "land",
            "--id",
            "dock-f1",
            "--log",
            log_path.to_str().unwrap(),
        ])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "CLI dock land exits 0 — stdout: {}",
        String::from_utf8_lossy(&out.stdout)
    );

    // The log now carries BOTH dock.landed AND dock.close (A4 closed).
    let log = hugit_cli::checks::load_event_log(&log_path).unwrap();
    let kinds: Vec<&str> = log.records().iter().map(|r| r.kind.as_str()).collect();
    assert!(
        log.records().iter().any(|r| r.kind == DOCK_LANDED_KIND),
        "dock.landed recorded (was: {kinds:?})"
    );
    assert!(
        log.records().iter().any(|r| r.kind == "dock.close"),
        "F1 — dock.close recorded AFTER land (fixes the swallowed re-entrant close) — was: {kinds:?}"
    );
    assert!(
        log.records()
            .iter()
            .filter(|r| r.kind == "dock.close")
            .count()
            == 1,
        "exactly one dock.close (idempotent)"
    );
}
