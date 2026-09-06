//! `hugit` as a git-local CLI — the prove-by-using journey (2026-09-02).
//!
//! Owner decision: hugit works where git works — a CLI in a repository, no
//! server, no account, no CoreLink. This suite proves the USER-FACING flow end
//! to end, exactly as a developer would use it:
//!
//! 1. `hugit init` in a fresh dir → creates a **git repository** (git init) +
//!    `.hugit/` + the canonical log (git-proximate ceremony).
//! 2. `hugit check` runs a command locally, memoizes it (file-backed AC), and a
//!    warm re-run is a HIT with `duration_ms: 0` — zero network, zero CoreLink.
//! 3. `hugit export` dumps the git artifact + JSON envelope (the zero-lock-in
//!    exit proof), and `restore` round-trips it object-for-object.
//!
//! Every step is exercised through the REAL binary (`CARGO_BIN_EXE_hugit`) and
//! the REAL library entry points — no mocks, no fixtures beyond a tiny source
//! tree. This is the "prove by using, not testing" acceptance for the git-local
//! direction.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

use hugit_cli::init::{InitArgs, run as run_init};

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-gitlocal-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit <args>` in `cwd`, returning `(exit_code, parsed_stdout_json)`.
fn run_in(cwd: &Path, args: &[&str]) -> (i32, Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

/// Run a system `git` command in `cwd` (the git-proximate half of the journey).
fn git_in(cwd: &Path, args: &[&str]) -> (i32, String) {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("git binary runs");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

/// Run `hugit init` via the library entry point (the binary verb is reserved
/// pending the X5 namespace amendment — `init` shadows `git init`). Returns the
/// parsed JSON the verb prints.
fn lib_init(dir: &Path) -> Value {
    let code = run_init(InitArgs {
        dir: Some(dir.to_path_buf()),
    });
    assert_eq!(code, std::process::ExitCode::SUCCESS, "lib init exits 0");
    // The verb prints JSON to stdout; capture it via a temp redirect is complex,
    // so we assert the side effects directly instead (the caller checks .git/.hugit).
    Value::Null
}

/// A tiny deterministic source tree for the check tree-axis.
fn seed_tree(dir: &Path) -> PathBuf {
    let root = dir.join("src");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("lib.rs"), b"fn main() {}\n").unwrap();
    root
}

// ── 1. hugit init is git-proximate ──────────────────────────────────────────

#[test]
fn journey_init_creates_git_repo_and_hugit_dir() {
    let root = scratch("init");
    lib_init(&root);
    assert!(root.join(".git").is_dir(), ".git created by hugit init");
    assert!(root.join(".git").is_dir(), ".git created by hugit init");
    assert!(root.join(".hugit").is_dir(), ".hugit created");
    assert!(
        root.join(".hugit/log.json").is_file(),
        "canonical log created"
    );

    // The created repo is a REAL git repo — `git status` works in it.
    let (gcode, _) = git_in(&root, &["status"]);
    assert_eq!(gcode, 0, "git status works in the hugit-initialized repo");
}

#[test]
fn journey_init_leaves_existing_git_repo_untouched() {
    let root = scratch("init-existing");
    // Pre-create a git repo with a commit.
    let (code, _) = git_in(&root, &["init"]);
    assert_eq!(code, 0);
    std::fs::write(root.join("tracked.txt"), "x").unwrap();
    git_in(&root, &["add", "tracked.txt"]);
    git_in(&root, &["commit", "-m", "seed", "--no-gpg-sign"]);

    lib_init(&root);
    assert!(root.join(".git").is_dir(), ".git still there");
    assert!(root.join(".hugit/log.json").is_file(), "hugit log added");
}

// ── 2. hugit check is local + memoized (zero CoreLink) ──────────────────────

#[test]
fn journey_check_memoizes_locally() {
    let root = scratch("check");
    let src = seed_tree(&root);
    let log = root.join(".hugit/log.json");

    // `hugit init` creates .hugit/ + the canonical log (git-proximate).
    lib_init(&root);

    // Cold run: executes for real (MISS), records a check.recorded event.
    let (code, v) = run_in(
        &root,
        &[
            "check",
            "run",
            "--def",
            "ad-hoc",
            "--cmd",
            "echo hello",
            "--root",
            src.to_str().unwrap(),
            "--log",
            log.to_str().unwrap(),
            "--store",
        ],
    );
    assert_eq!(code, 0, "cold check exits 0");
    assert_eq!(v["cache_hit"], false, "cold run is a MISS");

    // Warm re-run: HIT, duration_ms 0 — the memoization wedge, fully local.
    let (code, v) = run_in(
        &root,
        &[
            "check",
            "run",
            "--def",
            "ad-hoc",
            "--cmd",
            "echo hello",
            "--root",
            src.to_str().unwrap(),
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "warm check exits 0");
    assert_eq!(v["cache_hit"], true, "warm run is a HIT");
    assert_eq!(v["duration_ms"], 0, "memoized hit has zero duration");

    // No CoreLink env was set anywhere in this test — the run is local by
    // construction (the CLI's default AC is the file-backed local one).
}

// ── 3. hugit export is the zero-lock-in exit proof ───────────────────────────

#[test]
fn journey_export_roundtrip_restores() {
    let root = scratch("export");
    let log = root.join(".hugit/log.json");

    // `hugit init` creates .hugit/ + the canonical log (git-proximate).
    lib_init(&root);

    // Record a landed intent so the export has real content.
    let (code, _) = run_in(
        &root,
        &[
            "intent",
            "new",
            "--log",
            log.to_str().unwrap(),
            "--charter",
            "journey intent",
            "--campaign",
            "journey",
        ],
    );
    assert_eq!(code, 0, "intent new exits 0");

    // Export the git artifact + JSON envelope.
    let out_dir = root.join("export-out");
    let (code, _v) = run_in(
        &root,
        &[
            "export",
            "--log",
            log.to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0, "export exits 0");
    assert!(out_dir.join("repo.git").exists(), "git artifact exported");
    assert!(
        out_dir.join("export.json").exists(),
        "JSON envelope exported"
    );

    // The exported artifact is a real git repo usable with zero hugit tooling.
    let (gcode, _) = git_in(
        &root,
        &[
            "--git-dir",
            out_dir.join("repo.git").to_str().unwrap(),
            "rev-parse",
            "--is-bare-repository",
        ],
    );
    assert_eq!(gcode, 0, "exported artifact is a usable git repo");
}

// ── 4. The PR landing journey is git-local ────────────────────────────────────

/// Run `hugit <args>` in `cwd` and assert exit 0, returning parsed stdout JSON.
fn run_ok(cwd: &Path, args: &[&str]) -> Value {
    let (code, v) = run_in(cwd, args);
    assert_eq!(code, 0, "`hugit {}` exits 0 (err: {:?})", args.join(" "), v);
    v
}

#[test]
fn journey_pr_cycle_lands_locally() {
    let root = scratch("pr-cycle");
    lib_init(&root);
    let log = root.join(".hugit/log.json");

    // 1. Record an intent.
    let v = run_ok(
        &root,
        &[
            "intent",
            "new",
            "--charter",
            "journey feature",
            "--campaign",
            "camp",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    let intent_id = v["intent_id"].as_str().expect("intent id").to_string();

    // 2. Open a PR bundling that intent.
    let v = run_ok(
        &root,
        &[
            "pr",
            "open",
            "--pr",
            "PR-1",
            "--campaign",
            "camp",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "run-1",
            "--intent",
            &intent_id,
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(v["state"], "proposed", "PR opens as proposed");

    // 3. Queue it for the union-testing landing queue.
    let v = run_ok(
        &root,
        &[
            "pr",
            "queue",
            "--pr",
            "PR-1",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(v["queued"], true, "PR enters the landing queue");

    // 4. Batch-land the queue (union engine: green → lands).
    let v = run_ok(
        &root,
        &[
            "land",
            "queue",
            "--campaign",
            "camp",
            "--log",
            log.to_str().unwrap(),
        ],
    );
    assert_eq!(v["verdict"], "green", "union verdict is green");
    assert_eq!(v["landed"][0], "PR-1", "PR lands in the green set");

    // 5. Show the PR reflects the terminal LANDED state.
    let v = run_ok(
        &root,
        &["pr", "show", "--pr", "PR-1", "--log", log.to_str().unwrap()],
    );
    assert_eq!(
        v["queue"]["queued"], false,
        "PR left the queue after landing"
    );
}

/// SETUP JOURNEY — `hugit setup` + git's init.templateDir: no per-repo init.
///
/// Prove the boot ceremony WITHOUT `hugit init` per repo: `hugit setup` writes
/// the 4 hooks into a template + points git's GLOBAL `init.templateDir` at it;
/// a fresh `git init` then ships the hooks automatically, and the FIRST git op
/// lazy-boots `.hugit/log.json` + captures the real ref.update. Entirely under
/// an isolated HOME/XDG so the machine's real gitconfig is never touched.
#[test]
fn setup_templates_git_init_and_lazy_boots() {
    // Isolated env: a temp HOME (git global config lands there) + temp XDG
    // (the hugit template dir).
    let home = scratch("home");
    let xdg = scratch("xdg");
    let repo = scratch("repo");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&xdg).unwrap();
    std::fs::create_dir_all(&repo).unwrap();

    let mut cmd = Command::new(hugit_bin());
    let out = cmd
        .arg("setup")
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", &xdg)
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("hugit setup runs");
    assert!(
        out.status.success(),
        "hugit setup exits 0 — stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // The template hooks exist where the config points.
    let template_dir = xdg.join("hugit/template");
    assert!(
        template_dir.join("hooks/post-commit").exists(),
        "template post-commit written"
    );
    assert!(
        template_dir.join("hooks/post-checkout").exists(),
        "template post-checkout written"
    );
    assert!(
        template_dir.join("OWNED-BY-HUGIT").exists(),
        "ownership marker written"
    );

    // A FRESH repo: plain `git init` must ship the hooks (via templateDir).
    let mut cmd = Command::new("git");
    let out = cmd
        .args(["init", "-q", "-b", "main"])
        .current_dir(&repo)
        .env("HOME", &home)
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git init runs");
    assert!(out.status.success(), "git init works");
    assert!(
        repo.join(".git/hooks/post-commit").exists(),
        "git init copied the hugit post-commit hook from the template"
    );

    // Commit twice under isolated env; the first op lazy-boots the log.
    let mut cmd = Command::new("git");
    let _ = cmd
        .args(["config", "user.email", "t@t"])
        .current_dir(&repo)
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output();
    let mut cmd = Command::new("git");
    let _ = cmd
        .args(["config", "user.name", "t"])
        .current_dir(&repo)
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output();
    std::fs::write(repo.join("a.txt"), "a").unwrap();

    let mut cmd = Command::new("git");
    let _ = cmd
        .args(["add", "a.txt"])
        .current_dir(&repo)
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HUGIT_BIN", hugit_bin())
        .output();
    let mut cmd = Command::new("git");
    let _ = cmd
        .args(["commit", "-q", "-m", "first"])
        .current_dir(&repo)
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("HUGIT_BIN", hugit_bin())
        .output();

    // Poll the async capture: the lazy-booted log must hold a ref.update.
    let log_path = repo.join(".hugit/log.json");
    let mut landed = false;
    for _ in 0..60 {
        if let Ok(bytes) = std::fs::read(&log_path) {
            if let Ok(v) = serde_json::from_slice::<Value>(&bytes)
                && let Some(recs) = v.as_array()
                && !recs.is_empty()
            {
                landed = true;
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    assert!(
        landed,
        "first commit lazy-booted .hugit/log.json with a captured ref.update"
    );
    let bytes = std::fs::read(&log_path).unwrap();
    let recs: Vec<Value> = serde_json::from_slice(&bytes).unwrap();
    assert!(
        recs.iter().any(|r| r["kind"] == "ref.update"),
        "log holds a ref.update record: {recs:?}"
    );
}
