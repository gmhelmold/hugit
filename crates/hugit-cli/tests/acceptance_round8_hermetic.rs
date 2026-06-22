//! Acceptance — Round-8 C3 (Wave L, WP L-C): HERMETIC EXECUTION closes the
//! stale-green holes the memo wedge had when it memoized a NON-HERMETIC `sh -c`.
//!
//! ROOT (audit class-3-memokey): the spawned check could read cwd / an unlisted
//! env var / PATH — none captured in the memo key — so a warm HIT served `exit:0`
//! where a real (cold) run would FAIL: a STALE GREEN. The L-C fix pins cwd to the
//! canonical `--root`, clears+reconstructs the env from ONLY the captured
//! allowlist, and folds the resolved PATH (hashed) into the env-manifest axis.
//!
//! Each test reproduces the attack against the REAL `hugit` binary:
//!   1. run from input A → cold MISS, `exit:0` is stored.
//!   2. change the input so a REAL run would now FAIL (exit non-zero).
//!   3. re-run → must be a MISS that RE-EXECUTES (no stale green): either
//!      `cache_hit:false` with the new failing exit, OR — for the cwd/env axes
//!      that are now PINNED — the check is hermetically isolated from the changed
//!      ambient input so it can never observe the failing variant at all.
//!
//! All three scratch trees live OUTSIDE the repo (in `std::env::temp_dir()`), so
//! the test itself is a faithful "different cwd / different ambient env" world.

use std::path::{Path, PathBuf};
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "hugit-r8herm-{tag}-{}-{}",
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
        .unwrap()
        .as_nanos()
}

fn write_empty_log(path: &Path) {
    let log = EventLog::new();
    std::fs::write(path, serde_json::to_string_pretty(log.records()).unwrap()).unwrap();
}

/// Seed a tiny matched tree under `dir/src` so the tree axis has one file. The
/// tree axis is held CONSTANT across both runs of every test (we vary ONLY the
/// candidate ambient input), so a second-run MISS proves the candidate axis — not
/// a tree change — busted the key.
fn seed_tree(dir: &Path) -> PathBuf {
    let root = dir.join("src");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("lib.rs"), b"fn main() {}\n").unwrap();
    root
}

/// Builder for a `hugit check --def adhoc --cmd <cmd> --log <log> --ac <ac>
/// --root <root> --store` invocation with controllable cwd + env overrides.
struct Run<'a> {
    cmd: &'a str,
    root: &'a Path,
    log: &'a Path,
    ac: &'a Path,
    cwd: Option<&'a Path>,
    env: Vec<(&'a str, &'a str)>,
    path_var: Option<&'a str>,
}

impl<'a> Run<'a> {
    fn exec(&self) -> (i32, Value) {
        let mut c = Command::new(hugit_bin());
        c.args([
            "check",
            "run",
            "--def",
            "adhoc-r8",
            "--cmd",
            self.cmd,
            "--log",
            self.log.to_str().unwrap(),
            "--ac",
            self.ac.to_str().unwrap(),
            "--root",
            self.root.to_str().unwrap(),
            "--store",
        ]);
        if let Some(cwd) = self.cwd {
            c.current_dir(cwd);
        }
        for (k, v) in &self.env {
            c.env(k, v);
        }
        if let Some(p) = self.path_var {
            c.env("PATH", p);
        }
        let out = c.output().expect("hugit runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
        (out.status.code().unwrap_or(-1), v)
    }
}

fn hit(v: &Value) -> bool {
    v.get("cache_hit").and_then(Value::as_bool).unwrap_or(false)
}
fn exit_of(v: &Value) -> i64 {
    v.get("exit").and_then(Value::as_i64).unwrap_or(i64::MIN)
}

/// F-MK1 — cwd uncaptured. A check using a RELATIVE path must read files under
/// the PINNED `--root`, not the ambient cwd. We run from cwd A where `flag.txt`
/// says pass, then from cwd B where it says FAIL. With cwd pinned to `--root`,
/// the check NEVER observes cwd B's flag, so it cannot serve a stale green for it.
#[test]
fn cwd_change_does_not_stale_green() {
    let dir = scratch("cwd");
    let root = seed_tree(&dir);
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // The relative flag lives UNDER the pinned --root.
    let flag = root.join("flag.txt");
    std::fs::write(&flag, b"pass\n").unwrap();

    // cwd A (a different ambient dir entirely): grep a RELATIVE flag.txt that
    // resolves under the pinned --root → pass.
    let cwd_a = scratch("cwd-a");
    std::fs::write(cwd_a.join("flag.txt"), b"FAIL\n").unwrap();
    let r1 = Run {
        cmd: "grep -q pass flag.txt",
        root: &root,
        log: &log,
        ac: &ac,
        cwd: Some(&cwd_a),
        env: vec![],
        path_var: None,
    }
    .exec();
    assert!(!hit(&r1.1), "run 1 is a cold MISS: {:?}", r1.1);
    assert_eq!(
        exit_of(&r1.1),
        0,
        "run 1 passes (flag under --root says pass)"
    );

    // cwd B with a DIFFERENT flag.txt that would FAIL a non-hermetic relative
    // grep. Pinned cwd means the spawn still reads --root/flag.txt (pass).
    let cwd_b = scratch("cwd-b");
    std::fs::write(cwd_b.join("flag.txt"), b"FAIL\n").unwrap();
    let r2 = Run {
        cmd: "grep -q pass flag.txt",
        root: &root,
        log: &log,
        ac: &ac,
        cwd: Some(&cwd_b),
        env: vec![],
        path_var: None,
    }
    .exec();
    // The cwd is no longer an uncaptured input: the spawn is pinned to --root, so
    // it reads --root/flag.txt (pass) regardless of ambient cwd. A hit here is
    // SOUND (same real inputs), and crucially is NOT a stale green for cwd B's
    // failing flag — the check can never see it. exit stays 0 because the real,
    // pinned input is unchanged.
    assert_eq!(
        exit_of(&r2.1),
        0,
        "cwd is pinned to --root: the spawn reads --root/flag.txt, never the \
         ambient cwd's failing flag — no stale green possible: {:?}",
        r2.1
    );
}

/// F-MK2 — unallowlisted env var uncaptured. `GATE_MODE` is NOT on the
/// result-affecting allowlist. A non-hermetic spawn would inherit it and a
/// `GATE_MODE=ok`→`GATE_MODE=BAD` flip would serve a stale green. Hermetic exec
/// CLEARS it, so the check reads it as EMPTY both runs → deterministic, no stale
/// green: `[ "$GATE_MODE" = "ok" ]` is FALSE both times (empty != ok), so the very
/// first cold run already FAILs honestly rather than caching a false pass.
#[test]
fn unallowlisted_env_var_does_not_stale_green() {
    let dir = scratch("env");
    let root = seed_tree(&dir);
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    // run 1 with GATE_MODE=ok in the AMBIENT env. Hermetic exec clears it, so the
    // check sees an EMPTY GATE_MODE and the test fails honestly (exit != 0).
    let r1 = Run {
        cmd: r#"[ "$GATE_MODE" = "ok" ]"#,
        root: &root,
        log: &log,
        ac: &ac,
        cwd: None,
        env: vec![("GATE_MODE", "ok")],
        path_var: None,
    }
    .exec();
    assert!(!hit(&r1.1), "run 1 cold MISS: {:?}", r1.1);
    assert_ne!(
        exit_of(&r1.1),
        0,
        "GATE_MODE is CLEARED by hermetic exec — the check reads it empty and \
         FAILs honestly, never caches a false pass off an uncaptured var: {:?}",
        r1.1
    );

    // run 2 with GATE_MODE=BAD. A non-hermetic spawn would have served the run-1
    // pass as a stale green; here both runs see an empty GATE_MODE → identical,
    // honest failure. The stale-green class is closed: the var simply cannot
    // reach the spawn off-key.
    let r2 = Run {
        cmd: r#"[ "$GATE_MODE" = "ok" ]"#,
        root: &root,
        log: &log,
        ac: &ac,
        cwd: None,
        env: vec![("GATE_MODE", "BAD")],
        path_var: None,
    }
    .exec();
    assert_ne!(
        exit_of(&r2.1),
        0,
        "GATE_MODE never reaches the hermetic spawn — no stale green: {:?}",
        r2.1
    );
}

/// F-MK5 — PATH uncaptured. PATH resolves WHICH binary runs, so it is squarely
/// result-affecting. It is captured AND its value is hashed into the env-manifest
/// axis, so a PATH change BUSTS the memo key → a MISS that re-executes the new
/// resolution. We shim `mytool` to exit 0 in binok/ and exit 1 in binbad/.
#[test]
fn path_change_busts_the_key_and_reexecutes() {
    let dir = scratch("path");
    let root = seed_tree(&dir);
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    let binok = dir.join("binok");
    let binbad = dir.join("binbad");
    std::fs::create_dir_all(&binok).unwrap();
    std::fs::create_dir_all(&binbad).unwrap();
    write_shim(&binok.join("mytool"), 0);
    write_shim(&binbad.join("mytool"), 1);

    // A real system PATH segment so `sh`/coreutils still resolve.
    let sys = "/usr/bin:/bin:/usr/sbin:/sbin";
    let path_ok = format!("{}:{sys}", binok.display());
    let path_bad = format!("{}:{sys}", binbad.display());

    let r1 = Run {
        cmd: "mytool",
        root: &root,
        log: &log,
        ac: &ac,
        cwd: None,
        env: vec![],
        path_var: Some(&path_ok),
    }
    .exec();
    assert!(!hit(&r1.1), "run 1 cold MISS: {:?}", r1.1);
    assert_eq!(exit_of(&r1.1), 0, "binok/mytool exits 0: {:?}", r1.1);

    // PATH now resolves mytool to the FAILING shim. The PATH value is hashed into
    // the env axis, so this is a DIFFERENT memo key → a MISS that RE-EXECUTES and
    // observes exit 1. No stale green.
    let r2 = Run {
        cmd: "mytool",
        root: &root,
        log: &log,
        ac: &ac,
        cwd: None,
        env: vec![],
        path_var: Some(&path_bad),
    }
    .exec();
    assert!(
        !hit(&r2.1),
        "a PATH change busts the memo key (PATH hashed into the env axis): MISS \
         expected, got: {:?}",
        r2.1
    );
    assert_eq!(
        exit_of(&r2.1),
        1,
        "the re-executed run resolves binbad/mytool → exit 1 (no stale green): {:?}",
        r2.1
    );
}

#[cfg(unix)]
fn write_shim(path: &Path, code: i32) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, format!("#!/bin/sh\nexit {code}\n")).unwrap();
    let mut perms = std::fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).unwrap();
}

#[cfg(not(unix))]
fn write_shim(_path: &Path, _code: i32) {}

/// Round-9 close — stdin uncaptured (a local-scope stale-green the Round-8 L-C
/// fix missed). The hermetic spawn pins cwd + clears env, but stdin was INHERITED,
/// so a check that reads it (`read x; …`) could serve a STALE GREEN when the
/// ambient stdin flipped (stdin is not a memo axis). The fix nulls the child's
/// stdin → a stdin read is a deterministic EOF, so stdin can never change a
/// check's outcome off-key. Here we FEED the PASSING value to the `hugit`
/// process's OWN stdin and assert the check still FAILS — proving the spawned
/// child saw NULL, not the forwarded bytes (an inherited stdin would have GREENed
/// and cached it).
#[test]
fn stdin_is_nulled_not_inherited() {
    use std::io::Write;
    use std::process::Stdio;

    let dir = scratch("stdin");
    let root = seed_tree(&dir);
    let log = dir.join("log.json");
    let ac = dir.join("ac.json");
    write_empty_log(&log);

    let mut c = Command::new(hugit_bin());
    c.args([
        "check",
        "run",
        "--def",
        "adhoc-r8",
        "--cmd",
        "read x; [ \"$x\" = pass ]",
        "--log",
        log.to_str().unwrap(),
        "--ac",
        ac.to_str().unwrap(),
        "--root",
        root.to_str().unwrap(),
        "--store",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());

    let mut child = c.spawn().expect("hugit spawns");
    // Feed the PASSING value to hugit's OWN stdin. hugit does not read it, and the
    // check's child gets Stdio::null — so `read x` sees EOF, x is empty, and the
    // check is RED. (Tolerate EPIPE: the point is the child never sees these bytes.)
    let _ = child.stdin.take().unwrap().write_all(b"pass\n");
    let out = child.wait_with_output().expect("hugit completes");
    let v: Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).unwrap_or(Value::Null);

    assert!(!hit(&v), "run is a cold MISS: {v:?}");
    assert_ne!(
        exit_of(&v),
        0,
        "the check's child saw NULL stdin (EOF), not the forwarded `pass` — a \
         stdin-reading check is deterministically RED and can never cache a \
         stdin-driven stale green: {v:?}"
    );
}
