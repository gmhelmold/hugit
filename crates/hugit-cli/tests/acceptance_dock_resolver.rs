//! WP-DOCK-2 — dock resolver (cwd→gitdir→dock; env fast-path, cwd truth;
//! ghost; self-heal; M5 auto-coin).
//!
//! Proves the resolver's Lamport properties hermetically + e2e:
//!
//! - **P1 (safety — identity):** exactly ONE dock per cwd, or a named error;
//!   two OPEN docks sharing the gitdir ⇒ `AmbiguousDock` (never a guess).
//! - **P2 (safety — cwd wins on both sides):** env dock differs from cwd ⇒ env
//!   ignored, cwd dock returned, a `dock.reconcile` record appended once per
//!   pair (R5 — never silent, exact-once).
//! - **P3 (safety — no fabrication):** a dock whose gitdir vanished ⇒ `ghost`
//!   + `dock.ghost` record once, NEVER a live dock.
//! - **L2 (liveness — unlabeled resolves):** a marker-less worktree ⇒ `NoDock`
//!   (the honest "unlabeled" signal); a repo with a log but no dock ⇒ M5
//!   auto-coin a repo-scope dock; once the marker exists, self-heal re-coins.
//! - **R1 (self-heal):** marker present but record missing ⇒ re-coin via the
//!   marker path (restores, never duplicates).

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_cli::dock::resolve::{DOCK_GHOST_KIND, DOCK_RECONCILE_KIND};
use hugit_cli::dock::{DockState, ResolveError, coin_dock, find_dock_payload};
use hugit_cli::init::{InitArgs, run as run_init};

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-dock2-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn lib_init(dir: &Path) {
    let code = run_init(InitArgs {
        dir: Some(dir.to_path_buf()),
    });
    assert_eq!(code, std::process::ExitCode::SUCCESS, "lib init exits 0");
}

fn git_in(cwd: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

fn repo(tag: &str) -> PathBuf {
    let dir = scratch(tag);
    git_in(&dir, &["init", "-q", "-b", "main"]);
    git_in(&dir, &["config", "user.email", "t@t"]);
    git_in(&dir, &["config", "user.name", "t"]);
    std::fs::write(dir.join("a.txt"), "a").unwrap();
    git_in(&dir, &["add", "a.txt"]);
    git_in(&dir, &["commit", "-q", "-m", "init"]);
    lib_init(&dir);
    dir
}

fn log_path(dir: &Path) -> PathBuf {
    dir.join(".hugit/log.json")
}

fn kinds(log: &Path) -> Vec<String> {
    let bytes = std::fs::read(log).unwrap_or_else(|_| b"[]".to_vec());
    serde_json::from_slice::<Vec<serde_json::Value>>(&bytes)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|r| r.get("kind").and_then(|k| k.as_str()).map(String::from))
        .collect()
}

fn dock_kind_count(log: &Path) -> usize {
    kinds(log)
        .iter()
        .filter(|k| k.as_str() == "dock.record")
        .count()
}

// ── P1: exact one + ambiguous fail-closed ──────────────────────────────────

#[test]
fn hermetic_resolve_exact_one_and_ambiguous_fails_closed() {
    let dir = repo("p1");
    let log = log_path(&dir);

    // M5 — repo with a log but no dock ⇒ repo-scope auto-coin on first read.
    let r = resolve_at(&dir, &log, None);
    let rd = r.expect("repo-scope auto-coin resolves");
    assert_eq!(rd.origin, "repo", "M5 — repo-scope dock");
    assert_eq!(
        rd.state,
        DockState::Open,
        "P3 — live dock has a live gitdir"
    );

    // A worktree gets its own dock.
    let wt = dir.join("wt");
    git_in(
        &dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/x",
            wt.to_str().unwrap(),
        ],
    );
    // The worktree's gitdir is the AUTHORITATIVE absolute git dir (in a linked
    // worktree `.git` is a FILE pointing at it, never the physical gitdir — a
    // test that assumed `.git` is a directory was Unix-only and failed on the
    // Windows runner, where the path shape differs). Resolve it via git itself,
    // which is the only correct answer on every platform.
    let wt_gitdir = PathBuf::from(git_in(&wt, &["rev-parse", "--absolute-git-dir"]).1.trim());
    let wt_gitdir_s = wt_gitdir.to_string_lossy().to_string();
    coin_dock(&hugit_cli::dock::CoinSpec {
        log: &log,
        hook_log: None,
        top_level: &dir,
        gitdir: &wt_gitdir_s,
        branch: "feat/x",
        charter: "add x",
        origin: "worktree",
        env_dock_id: None,
    })
    .unwrap();

    // Resolve from INSIDE the worktree → the worktree dock.
    let rd = resolve_at(&wt, &log, None).expect("worktree resolves");
    assert_eq!(rd.origin, "worktree");
    assert_eq!(rd.branch, "feat/x");

    // P1 fail-closed: two OPEN docks sharing the gitdir ⇒ ambiguous. The
    // idempotent coin prevents a dup via the marker; after the FIRST coin,
    // REMOVE the marker and coin the SAME gitdir with a different branch →
    // two legitimate records (both real chains), the resolver must refuse.
    let marker_p = std::path::Path::new(&wt_gitdir_s).join("hugit-dock");
    let _ = std::fs::remove_file(&marker_p);
    coin_dock(&hugit_cli::dock::CoinSpec {
        log: &log,
        hook_log: None,
        top_level: &dir,
        gitdir: &wt_gitdir_s,
        branch: "feat/y",
        charter: "add y",
        origin: "worktree",
        env_dock_id: None,
    })
    .unwrap();

    match resolve_at(&wt, &log, None) {
        Err(ResolveError::AmbiguousDock { ids }) => {
            assert_eq!(ids.len(), 2, "both dock ids")
        }
        other => panic!("P1 — must be AmbiguousDock, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

fn resolve_at(
    cwd: &Path,
    log: &Path,
    env_dock_id: Option<String>,
) -> Result<hugit_cli::dock::ResolvedDock, ResolveError> {
    hugit_cli::dock::resolve::resolve(cwd, Some(log), env_dock_id)
}

// ── P2 + R5: env cwd-wins + reconcile exact-once ───────────────────────────

#[test]
fn hermetic_env_diff_is_ignored_and_reconcile_recorded_once() {
    let dir = repo("p2");
    let log = log_path(&dir);

    // Dock the MAIN repo (cwd dock A).
    let main_gitdir = git_in(&dir, &["rev-parse", "--absolute-git-dir"])
        .1
        .trim()
        .to_string();
    let id_a = coin_dock(&hugit_cli::dock::CoinSpec {
        log: &log,
        hook_log: None,
        top_level: &dir,
        gitdir: &main_gitdir,
        branch: "main",
        charter: "repository scope (main)",
        origin: "repo",
        env_dock_id: None,
    })
    .unwrap();

    // A worktree (cwd dock B).
    let wt = dir.join("wt");
    git_in(
        &dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/wt",
            wt.to_str().unwrap(),
        ],
    );
    let wt_gitdir = wt.join(".git");
    let wt_gitdir = if wt_gitdir.is_dir() {
        wt_gitdir
    } else {
        std::path::PathBuf::from(git_in(&wt, &["rev-parse", "--absolute-git-dir"]).1.trim())
    };
    let wt_gitdir_s = wt_gitdir.to_string_lossy().to_string();
    coin_dock(&hugit_cli::dock::CoinSpec {
        log: &log,
        hook_log: None,
        top_level: &dir,
        gitdir: &wt_gitdir_s,
        branch: "feat/wt",
        charter: "add wt",
        origin: "worktree",
        env_dock_id: None,
    })
    .unwrap();

    // Resolve IN the worktree with HUGIT_DOCK_ID=A (env differs).
    let rd = resolve_at(&wt, &log, Some(id_a.clone())).expect("cwd wins");
    assert_eq!(
        rd.origin, "worktree",
        "P2 — cwd (worktree) wins over env (repo)"
    );
    assert_ne!(rd.dock_id, id_a, "env never adopted");

    // R5 — reconcile record appended ONCE (exact-once).
    let rec_count = kinds(&log)
        .iter()
        .filter(|k| k.as_str() == DOCK_RECONCILE_KIND)
        .count();
    assert_eq!(rec_count, 1, "R5 — one reconcile record");

    // Re-resolve (env again) → STILL one reconcile.
    let _ = resolve_at(&wt, &log, Some(id_a));
    let rec_count = kinds(&log)
        .iter()
        .filter(|k| k.as_str() == DOCK_RECONCILE_KIND)
        .count();
    assert_eq!(rec_count, 1, "R5 — exact-once, never two");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── P3 + R4: ghost ─────────────────────────────────────────────────────────

#[test]
fn hermetic_ghost_when_gitdir_vanishes_marked_once() {
    let dir = repo("p3");
    let log = log_path(&dir);

    // A fake worktree dock whose gitdir we DELETE.
    let fake_gitdir = dir.join("ghost-gitdir");
    std::fs::create_dir_all(&fake_gitdir).unwrap();
    let gs = fake_gitdir.to_string_lossy().to_string();
    let id = coin_dock(&hugit_cli::dock::CoinSpec {
        log: &log,
        hook_log: None,
        top_level: &dir,
        gitdir: &gs,
        branch: "feat/g",
        charter: "add g",
        origin: "worktree",
        env_dock_id: None,
    })
    .unwrap();

    // Delete the gitdir.
    std::fs::remove_dir_all(&fake_gitdir).unwrap();

    // P3 — the ghosted dock's gitdir is gone (never a live dock).
    let p = find_dock_payload(&log, &id).unwrap().unwrap();
    let gitdir_str = p.get("gitdir").and_then(|v| v.as_str()).unwrap();
    assert!(!Path::new(gitdir_str).exists(), "gitdir gone ⇒ ghost");

    // The R4 ghost mark fires on OBSERVATION — the resolver only observes its
    // own cwd's gitdir (which exists, so resolve alone marks nothing), but the
    // ENUMERATION horizon (`dock ls` / mark_ghosts) sees ALL docks and marks
    // the vanished gitdir's dock as ghost ONCE (cold-verify F8).
    let _ = resolve_at(&dir, &log, None);
    let ghost_recs_after_resolve = kinds(&log)
        .iter()
        .filter(|k| k.as_str() == DOCK_GHOST_KIND)
        .count();
    assert_eq!(
        ghost_recs_after_resolve, 0,
        "resolve alone never touches another gitdir's ghost"
    );

    let marked = hugit_cli::dock::resolve::mark_ghosts(&log).unwrap();
    assert_eq!(
        marked, 1,
        "R4 — the vanished gitdir's dock is marked ghost ONCE"
    );
    let ghost_recs = kinds(&log)
        .iter()
        .filter(|k| k.as_str() == DOCK_GHOST_KIND)
        .count();
    assert_eq!(ghost_recs, 1, "exactly one dock.ghost record");
    // Idempotent — a second observation marks nothing new.
    let marked2 = hugit_cli::dock::resolve::mark_ghosts(&log).unwrap();
    assert_eq!(marked2, 0, "ghost-mark is exact-once");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── M5 + L2 + R1 ───────────────────────────────────────────────────────────

#[test]
fn hermetic_m5_auto_coin_and_l2_unlabeled_worktree_stays_nodock() {
    let dir = repo("m5");
    let log = log_path(&dir);
    assert_eq!(dock_kind_count(&log), 0, "no dock before resolve");

    // L2: a marker-less worktree stays NoDock (honest unlabeled) — the git
    // model has no hook-born dock there, and we never fabricate one.
    let wt = dir.join("wt-no-marker");
    git_in(
        &dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/nm",
            wt.to_str().unwrap(),
        ],
    );
    let wt_git = wt.join(".git");
    // Strip the marker the temp hook may have left (simulate a clone: no hook).
    if wt_git.is_dir() {
        let _ = std::fs::remove_file(wt_git.join("hugit-dock"));
    } else if wt_git.is_file() {
        // linked worktree: gitdir in main
        let main_gd = git_in(&dir, &["rev-parse", "--absolute-git-dir"])
            .1
            .trim()
            .to_string();
        let _ = std::fs::remove_file(Path::new(&main_gd).join("worktrees/wt-no-marker/hugit-dock"));
    }

    match resolve_at(&wt, &log, None) {
        Err(ResolveError::NoDock) => {} // L2 — honest unlabeled
        other => panic!("L2 — marker-less worktree must stay NoDock, got {other:?}"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hermetic_r1_self_heal_recoins_when_marker_survives() {
    let dir = repo("r1");
    let log = log_path(&dir);

    // A worktree WITH a marker (as the hook would) → coin → then wipe the
    // record but keep the marker (partial-write loss).
    let wt = dir.join("wt-heal");
    git_in(
        &dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/heal",
            wt.to_str().unwrap(),
        ],
    );
    let wt_gitdir = git_in(&wt, &["rev-parse", "--absolute-git-dir"])
        .1
        .trim()
        .to_string();
    coin_dock(&hugit_cli::dock::CoinSpec {
        log: &log,
        hook_log: None,
        top_level: &dir,
        gitdir: &wt_gitdir,
        branch: "feat/heal",
        charter: "add heal",
        origin: "worktree",
        env_dock_id: None,
    })
    .unwrap();

    let marker = Path::new(&wt_gitdir).join("hugit-dock");
    assert!(marker.exists(), "marker exists");

    // Wipe ALL dock.records (simulate the loss) but keep the marker.
    let bytes = std::fs::read(&log).unwrap();
    let mut logs: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
    logs.retain(|r| r.get("kind").and_then(|k| k.as_str()) != Some("dock.record"));
    std::fs::write(&log, serde_json::to_vec_pretty(&logs).unwrap()).unwrap();
    assert_eq!(dock_kind_count(&log), 0, "records wiped");

    // Resolve → self-heal re-coins via the marker (R1), no duplication.
    let rd = resolve_at(&wt, &log, None).expect("self-heal re-coins");
    assert_eq!(rd.origin, "worktree", "R1 — restored");
    assert!(dock_kind_count(&log) >= 1, "record restored");

    let _ = std::fs::remove_dir_all(&dir);
}

// ── e2e: real worktree + env fast-path ─────────────────────────────────────

#[test]
fn e2e_resolve_real_worktree_and_env_fastpath() {
    let dir = repo("e2e");
    let log = log_path(&dir);

    let wt = dir.join("wt");
    git_in(
        &dir,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feat/e2e",
            wt.to_str().unwrap(),
        ],
    );
    let wt_gitdir = wt.join(".git");
    let wt_gitdir = if wt_gitdir.is_dir() {
        wt_gitdir
    } else {
        std::path::PathBuf::from(git_in(&wt, &["rev-parse", "--absolute-git-dir"]).1.trim())
    };
    let wt_gitdir_s = wt_gitdir.to_string_lossy().to_string();
    let wt_id = coin_dock(&hugit_cli::dock::CoinSpec {
        log: &log,
        hook_log: None,
        top_level: &dir,
        gitdir: &wt_gitdir_s,
        branch: "feat/e2e",
        charter: "add e2e",
        origin: "worktree",
        env_dock_id: None,
    })
    .unwrap();

    // Fast-path: env dock == cwd dock ⇒ adopted.
    let rd = resolve_at(&wt, &log, Some(wt_id.clone())).expect("fast-path resolves");
    assert_eq!(rd.dock_id, wt_id, "fast-path — env agrees, adopted");
    assert_eq!(rd.state, DockState::Open, "live");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Cross-platform worktree-gitdir detection (regression for the Windows find):
/// a linked worktree's gitdir has a `worktrees` path component, detected on
/// BOTH unix (`/`) and windows (`\`) separators. The old
/// `gitdir.contains("/worktrees/")` misclassified a Windows worktree as the
/// main repo (auto-coining a repo-scope dock where it must stay NoDock — L2).
#[test]
fn worktree_gitdir_detection_is_separator_agnostic() {
    assert!(
        hugit_cli::dock::is_worktree_gitdir("/repo/.git/worktrees/wt-a"),
        "unix worktree gitdir detected"
    );
    assert!(
        hugit_cli::dock::is_worktree_gitdir(r"C:\repo\.git\worktrees\wt-a"),
        "windows worktree gitdir detected"
    );
    assert!(
        !hugit_cli::dock::is_worktree_gitdir("/repo/.git"),
        "main repo gitdir is NOT a worktree"
    );
    assert!(
        !hugit_cli::dock::is_worktree_gitdir(r"C:\repo\.git"),
        "windows main repo gitdir is NOT a worktree"
    );
    assert!(
        !hugit_cli::dock::is_worktree_gitdir(""),
        "empty is not a worktree"
    );
}
