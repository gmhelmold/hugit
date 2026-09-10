//! `hugit init` — bootstrap the conventional repo-local hugit directory.
//!
//! Git-proximate: just as `git init` creates `.git/`, `hugit init` creates
//! `.hugit/` and an empty canonical event log `.hugit/log.json` (an empty JSON
//! array), so every other verb's default `--log` resolves to a real file with
//! zero ceremony. Idempotent: an existing log is never clobbered — re-running
//! `init` reports it already exists and leaves the bytes untouched.
//!
//! Output is the same stable-JSON-on-stdout / one-exit-code law as every other
//! verb (`0` success, `2` structured domain error).

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use crate::log_resolve::DEFAULT_LOG_PATH;
use crate::porcelain::PorcelainError;

/// True if `root` already contains a git repository (`.git` dir or worktree).
fn is_git_repo(root: &std::path::Path) -> bool {
    let dotgit = root.join(".git");
    dotgit.is_dir() || dotgit.is_file() // worktree: .git is a file pointing at the gitdir
}

/// Run `git init` in `root` (git-proximate ceremony), leaving errors as a
/// structured `PorcelainError`.
fn git_init(root: &std::path::Path) -> Result<(), PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("init")
        .arg(root)
        .output()
        .map_err(|e| PorcelainError::io("run git init", root, &e))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(PorcelainError::new(
            "git_init_failed",
            format!("`git init` in {:?} failed: {}", root, stderr.trim()),
            "install git and add it to PATH, then re-run `hugit init`",
        ));
    }
    Ok(())
}

/// Arguments for `hugit init`.
#[derive(clap::Args, Debug)]
pub struct InitArgs {
    /// Where to create the hugit directory (defaults to the current directory).
    #[arg(long)]
    pub dir: Option<PathBuf>,
}

/// Run `hugit init` — create `.hugit/` + an empty `.hugit/log.json`, idempotently.
pub fn run(args: InitArgs) -> ExitCode {
    match do_run(&args) {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("{}", err.to_json());
            err.exit_code()
        }
    }
}

/// The shell body for each hook, generated deterministically. Every hook:
/// - resolves the hugit binary ($HUGIT_BIN then `hugit`),
/// - resolves the repo root via git itself (worktree-safe),
/// - detaches the capture child (`nohup ... &` + re-direct) then exits 0,
///   so a hugit failure can NEVER fail/block the git operation.
pub(crate) fn hook_script(kind: &str) -> String {
    // The capture invocation for each kind (post-commit takes immutable commit
    // facts only; capture itself discovers paths from that commit with Git;
    // post-checkout passes from/to/branch when flag==1; pre-push reads
    // refspecs/shas from stdin into the child; post-merge passes the merged tip).
    match kind {
    "post-commit" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: the LLM used `git commit`; hugit records ref.update async.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# The canonical log lives at the MAIN repo's .hugit (shared across linked
# worktrees). In a worktree, `--show-toplevel` is the WORKTREE root, so resolve
# the shared log from the common git dir (the main .git) instead.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
LOG="$COMMON/../.hugit/log.json"
HL="$COMMON/../.hugit/hooks.log"
[ -f "$LOG" ] || {
  # LAZY BOOT: no hugit log yet — create one (a template-dir repo that never
  # ran `hugit init` still becomes active on the FIRST git op). The log is an
  # empty JSON array; hooks must never block git, so failures are best-effort.
  mkdir -p "$(dirname "$LOG")" 2>/dev/null
  printf '[]\n' > "$LOG" 2>/dev/null
}
OID="$(git rev-parse HEAD 2>/dev/null)" || exit 0
BRANCH="$(git branch --show-current 2>/dev/null)"
RECORDED_AT="$(git log -1 --format=%ct 2>/dev/null)"
(
  "$HUGIT_BIN" capture --kind commit --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --oid "$OID"     --branch "$BRANCH"     --recorded-at "$RECORDED_AT"
) >>"$HL" 2>&1 &
exit 0
"#.to_string(),
    "post-checkout" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: branch checkout (flag=1); records ref.update {checkout:true}.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# The canonical log lives at the MAIN repo's .hugit (shared across linked
# worktrees). In a worktree, `--show-toplevel` is the WORKTREE root, so resolve
# the shared log from the common git dir (the main .git) instead.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
LOG="$COMMON/../.hugit/log.json"
HL="$COMMON/../.hugit/hooks.log"
[ -f "$LOG" ] || {
  # LAZY BOOT: no hugit log yet — create one (a template-dir repo that never
  # ran `hugit init` still becomes active on the FIRST git op). The log is an
  # empty JSON array; hooks must never block git, so failures are best-effort.
  mkdir -p "$(dirname "$LOG")" 2>/dev/null
  printf '[]\n' > "$LOG" 2>/dev/null
}
[ "$3" = "1" ] || exit 0   # only branch checkouts (flag=1), not file checkouts
GITDIR=$(git rev-parse --absolute-git-dir 2>/dev/null) || exit 0
(
  "$HUGIT_BIN" capture --kind checkout --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --from "$1" --oid "$2" --branch "$(git branch --show-current 2>/dev/null)"
) >>"$HL" 2>&1 &
# Worktree-dock (ADR-0005, WP-DOCK-1): coin the physical binding at checkout
# time (idempotent — marker present ⇒ no-op; never blocks git).
(
  "$HUGIT_BIN" dock coin --top-level "$ROOT" --gitdir "$GITDIR" --log "$LOG" --hook-log "$HL" --branch "$(git branch --show-current 2>/dev/null)"
) >>"$HL" 2>&1 &
exit 0
"#.to_string(),
    "pre-push" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: a push is attempted; records ref.update {attempt:true} with
# the LOCAL shas being pushed (the 2nd field of each refspec stdin line), so a
# `git push` is a captured-commit proof too (pr open --commit <pushed-sha>).
# ALWAYS exits 0 — this is a PRE hook; a non-zero exit would BLOCK the push.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# The canonical log lives at the MAIN repo's .hugit (shared across linked
# worktrees). In a worktree, `--show-toplevel` is the WORKTREE root, so resolve
# the shared log from the common git dir (the main .git) instead.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
LOG="$COMMON/../.hugit/log.json"
HL="$COMMON/../.hugit/hooks.log"
[ -f "$LOG" ] || {
  # LAZY BOOT: no hugit log yet — create one (a template-dir repo that never
  # ran `hugit init` still becomes active on the FIRST git op). The log is an
  # empty JSON array; hooks must never block git, so failures are best-effort.
  mkdir -p "$(dirname "$LOG")" 2>/dev/null
  printf '[]\n' > "$LOG" 2>/dev/null
}
STDIN_REFS="$(if [ -n "$HUGIT_PRE_PUSH_FILE" ]; then cat "$HUGIT_PRE_PUSH_FILE"; else cat; fi)"   # dispatcher may preserve stdin for an adopted foreign hook
# Extract the LOCAL sha (2nd field) from each refspec line that has 4 fields.
SHAS="$(echo "$STDIN_REFS" | awk 'NF>=4 {print $2}')"
(
  "$HUGIT_BIN" capture --kind push-attempt --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --refspecs "$STDIN_REFS" --shas "$SHAS"
) >>"$HL" 2>&1 &
exit 0
"#.to_string(),
    "post-merge" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: a local merge landed; records ref.update {merged_from}.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# The canonical log lives at the MAIN repo's .hugit (shared across linked
# worktrees). In a worktree, `--show-toplevel` is the WORKTREE root, so resolve
# the shared log from the common git dir (the main .git) instead.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
LOG="$COMMON/../.hugit/log.json"
HL="$COMMON/../.hugit/hooks.log"
[ -f "$LOG" ] || {
  # LAZY BOOT: no hugit log yet — create one (a template-dir repo that never
  # ran `hugit init` still becomes active on the FIRST git op). The log is an
  # empty JSON array; hooks must never block git, so failures are best-effort.
  mkdir -p "$(dirname "$LOG")" 2>/dev/null
  printf '[]\n' > "$LOG" 2>/dev/null
}
(
  "$HUGIT_BIN" capture --kind merge --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --from "$(git rev-parse HEAD~1 2>/dev/null)"     --oid "$(git rev-parse HEAD 2>/dev/null)"     --recorded-at "$(git log -1 --format=%ct 2>/dev/null)"
) >>"$HL" 2>&1 &
exit 0
"#.to_string(),
    _ => unreachable!("known hook kind"),
}
}

pub(crate) const HUGIT_HOOK_MARKER: &str = "# hugit-hook (managed by hugit init)";
pub(crate) const HOOK_KINDS: [&str; 4] = ["post-commit", "post-checkout", "pre-push", "post-merge"];

pub(crate) struct HookInstallResult {
    pub installed: Vec<String>,
    pub noop: Vec<String>,
    pub conflict: Vec<String>,
}

fn resolve_hooks_dir(root: &std::path::Path) -> Result<std::path::PathBuf, PorcelainError> {
    let hooks_path = std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap_or("."),
            "config",
            "--get",
            "core.hooksPath",
        ])
        .output()
        .map_err(|e| PorcelainError::io("check core.hooksPath", root, &e))?;
    if hooks_path.status.success()
        && !String::from_utf8_lossy(&hooks_path.stdout)
            .trim()
            .is_empty()
    {
        return Err(PorcelainError::new(
            "custom_hooks_path",
            "refusing to install hooks while core.hooksPath is configured",
            "unset core.hooksPath or install hugit hooks in that path yourself",
        ));
    }
    let out = std::process::Command::new("git")
        .args([
            "-C",
            root.to_str().unwrap_or("."),
            "rev-parse",
            "--git-path",
            "hooks",
        ])
        .output()
        .map_err(|e| PorcelainError::io("resolve hooks dir", root, &e))?;
    if !out.status.success() {
        return Err(PorcelainError::new(
            "git_hooks_dir_failed",
            format!("`git rev-parse --git-path hooks` in {:?} failed", root),
            "is this an existing git repository and is `git` on PATH?",
        ));
    }
    let path = String::from_utf8_lossy(&out.stdout);
    let rel = std::path::PathBuf::from(path.trim());
    Ok(if rel.is_absolute() {
        rel
    } else {
        root.join(rel)
    })
}

/// Install hugit hooks without creating or otherwise changing a git repository.
pub(crate) fn install_hooks(root: &std::path::Path) -> Result<HookInstallResult, PorcelainError> {
    let hooks_dir = resolve_hooks_dir(root)?;
    std::fs::create_dir_all(&hooks_dir)
        .map_err(|e| PorcelainError::io("create hooks dir", &hooks_dir, &e))?;

    let mut result = HookInstallResult {
        installed: Vec::new(),
        noop: Vec::new(),
        conflict: Vec::new(),
    };
    for kind in HOOK_KINDS {
        let path = hooks_dir.join(kind);
        let metadata = match std::fs::symlink_metadata(&path) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(PorcelainError::io("stat hook", &path, &error)),
        };
        if let Some(metadata) = metadata {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(PorcelainError::new(
                    "unsafe_hook_path",
                    format!("refusing non-regular hook path {:?}", path),
                    "replace the hook path with a regular file, then rerun setup",
                ));
            }
            let existing = std::fs::read_to_string(&path)
                .map_err(|e| PorcelainError::io("read hook", &path, &e))?;
            if existing.contains(HUGIT_HOOK_MARKER) {
                result.noop.push(kind.to_string());
            } else {
                result.conflict.push(kind.to_string());
            }
            continue;
        }
        std::fs::write(&path, hook_script(kind))
            .map_err(|e| PorcelainError::io("write hook", &path, &e))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path)
                .map_err(|e| PorcelainError::io("stat hook", &path, &e))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms)
                .map_err(|e| PorcelainError::io("chmod hook", &path, &e))?;
        }
        result.installed.push(kind.to_string());
    }
    Ok(result)
}

fn do_run(args: &InitArgs) -> Result<serde_json::Value, PorcelainError> {
    let root = args.dir.clone().unwrap_or_else(|| PathBuf::from("."));
    let hugit_dir = root.join(".hugit");
    let log_path = root.join(DEFAULT_LOG_PATH);

    std::fs::create_dir_all(&hugit_dir)
        .map_err(|e| PorcelainError::io("create .hugit directory", &hugit_dir, &e))?;

    // Git-proximate ceremony: ensure a git repo exists. If `root` is not yet
    // a git repository, shell out to `git init` (same CLI a user would run);
    // if it already is (`.git` dir or worktree file) leave it untouched.
    let git_created = if is_git_repo(&root) {
        false
    } else {
        git_init(&root)?;
        true
    };

    // Install the silent git hooks (post-commit/checkout/push/merge) so the
    // LLM using git normally is captured into the log asynchronously. A
    // conflict (a pre-existing non-hugit hook) is reported, never clobbered.
    let hooks = install_hooks(&root)?;

    // Idempotent: never clobber an existing log (it carries the hash-chained,
    // append-only history — re-init must be safe to run in a live repo).
    let created = if log_path.exists() {
        false
    } else {
        // An empty canonical event log is an empty JSON array `[]` — the shape
        // every porcelain read verb rehydrates from.
        std::fs::write(&log_path, b"[]\n")
            .map_err(|e| PorcelainError::io("write log", &log_path, &e))?;
        true
    };

    let hint = if created {
        "initialized empty hugit log; next: `hugit campaign open --campaign <key> \
         --charter <text> --owner <you>` then `hugit intent new …` (every verb's \
         --log now defaults to .hugit/log.json)"
    } else {
        "hugit log already exists; left untouched (init is idempotent). Every \
         verb's --log defaults to .hugit/log.json"
    };

    let git_hint = if git_created {
        "git repository created (`git init` ran)"
    } else {
        "git repository already present"
    };

    Ok(json!({
        "initialized": created,
        "hooks_installed": hooks.installed,
        "hooks_noop": hooks.noop,
        "hooks_conflict": hooks.conflict,
        "git_created": git_created,
        "git_hint": git_hint,
        "hugit_dir": hugit_dir.display().to_string(),
        "log": log_path.display().to_string(),
        "hint": hint,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-init-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn init_creates_dir_and_empty_log_array_and_git_repo() {
        let root = scratch("create");
        let v = do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("init ok");
        assert_eq!(v["initialized"], true);
        assert_eq!(
            v["git_created"], true,
            "git init should have run on a bare dir"
        );
        let log = root.join(".hugit/log.json");
        assert!(log.is_file(), "log file created");
        let bytes = std::fs::read(&log).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            parsed.as_array().is_some_and(|a| a.is_empty()),
            "log is an empty JSON array"
        );
        assert!(root.join(".hugit").is_dir(), ".hugit dir created");
        assert!(
            root.join(".git").is_dir(),
            "git repo created by `hugit init`"
        );
    }

    #[test]
    fn init_leaves_existing_git_repo_untouched() {
        let root = scratch("existing-git");
        // Pre-create a git repo (git init).
        let status = std::process::Command::new("git")
            .arg("init")
            .arg(&root)
            .status();
        if status.map(|s| !s.success()).unwrap_or(true) {
            eprintln!("SKIP: system `git` unavailable");
            return;
        }
        // Seed a git commit marker file so we can prove git state was not clobbered.
        std::fs::write(root.join("tracked.txt"), "x").unwrap();
        std::process::Command::new("git")
            .args(["-C", root.to_str().unwrap(), "add", "tracked.txt"])
            .status()
            .unwrap();

        let v = do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("init ok on existing git repo");
        assert_eq!(v["git_created"], false, "existing repo left alone");
        assert!(root.join(".git").is_dir(), ".git still there");
        assert!(root.join(".hugit").is_dir(), ".hugit added");
        assert!(root.join(".hugit/log.json").is_file(), "log created");
    }

    #[test]
    fn init_is_idempotent_and_never_clobbers() {
        let root = scratch("idem");
        do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("first init");
        // Seed a record so we can prove re-init does NOT clobber the history.
        let log = root.join(".hugit/log.json");
        std::fs::write(&log, br#"[{"seq":0}]"#).unwrap();

        let v = do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("second init ok");
        assert_eq!(v["initialized"], false, "re-init reports not-created");
        // The seeded bytes survive — init never clobbers an existing log.
        let bytes = std::fs::read_to_string(&log).unwrap();
        assert!(bytes.contains("\"seq\":0"), "existing log left untouched");
    }

    #[test]
    fn post_commit_snapshots_head_before_detaching_capture() {
        let script = hook_script("post-commit");
        let snapshot = script.find("OID=\"$(git rev-parse HEAD").unwrap();
        let child = script.find("(\n  \"$HUGIT_BIN\" capture").unwrap();
        assert!(snapshot < child);
        assert!(script.contains("--oid \"$OID\""));
    }
}
