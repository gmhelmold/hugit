//! `hugit init` — bootstrap shared Git-common-dir hugit runtime state.
//!
//! Git-proximate: just as `git init` creates `.git/`, `hugit init` creates
//! `.git/hugit/event-log.json` outside tracked worktree state. Idempotent: an
//! existing log is never clobbered — re-running `init` leaves bytes untouched.
//!
//! Output is the same stable-JSON-on-stdout / one-exit-code law as every other
//! verb (`0` success, `2` structured domain error).

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::json;

use crate::porcelain::PorcelainError;

/// Marker identifying a hook wholly owned by hugit.
pub const HUGIT_HOOK_MARKER: &str = "# hugit-hook (managed by hugit init)";

/// Git hook names currently installed by hugit.
pub const HOOK_KINDS: [&str; 4] = ["post-commit", "post-checkout", "pre-push", "post-merge"];

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

/// Arguments for `hugit attach`.
#[derive(clap::Args, Debug)]
pub struct AttachArgs {
    /// Existing repository to attach (defaults to the current directory).
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Inspect current attachment without mutating repository state.
    #[arg(long)]
    pub status: bool,
}

/// Run `hugit init` — create runtime state and an empty canonical event log.
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

/// Attach hugit to an existing Git repository without creating a repository.
pub fn attach_run(args: AttachArgs) -> ExitCode {
    if args.status {
        return crate::health::run(crate::health::HealthArgs { dir: args.dir });
    }
    let requested_root = args.dir.clone().unwrap_or_else(|| PathBuf::from("."));
    let root = match git_top_level(&requested_root) {
        Ok(root) => root,
        Err(_) => {
            let err = PorcelainError::new(
                "not_a_git_repo",
                format!("{} is not a Git repository", requested_root.display()),
                "run `git init` first, then `hugit attach`",
            );
            println!("{}", err.to_json());
            return err.exit_code();
        }
    };
    match configured_hooks_path(&root) {
        Ok(Some(path)) => {
            let err = PorcelainError::new(
                "hooks_path_unsupported",
                format!("effective core.hooksPath is {path:?}"),
                "remove core.hooksPath or configure hugit through that hook manager before attach",
            );
            println!("{}", err.to_json());
            return err.exit_code();
        }
        Ok(None) => {}
        Err(err) => {
            println!("{}", err.to_json());
            return err.exit_code();
        }
    }
    let log_root = match legacy_log_root(&root) {
        Ok(root) => root,
        Err(err) => {
            println!("{}", err.to_json());
            return err.exit_code();
        }
    };
    run(InitArgs {
        dir: Some(log_root),
    })
}

/// Remove only hooks wholly owned by hugit, retaining all captured evidence.
pub fn detach_run(args: InitArgs) -> ExitCode {
    let requested_root = args.dir.unwrap_or_else(|| PathBuf::from("."));
    let root = match git_top_level(&requested_root) {
        Ok(root) => root,
        Err(_) => {
            let err = PorcelainError::new(
                "not_a_git_repo",
                format!("{} is not a Git repository", requested_root.display()),
                "run `hugit detach` inside an existing Git repository",
            );
            println!("{}", err.to_json());
            return err.exit_code();
        }
    };
    let result = (|| {
        let hooks_dir = resolve_hooks_dir(&root)?;
        let mut removed = Vec::new();
        let mut preserved = Vec::new();
        for kind in HOOK_KINDS {
            let path = hooks_dir.join(kind);
            match std::fs::read_to_string(&path) {
                Ok(contents) if is_managed_hook(kind, &contents) => {
                    std::fs::remove_file(&path)
                        .map_err(|e| PorcelainError::io("remove hugit hook", &path, &e))?;
                    removed.push(kind);
                }
                Ok(_) => preserved.push(kind),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(PorcelainError::io("read hook", &path, &error)),
            }
        }
        Ok(serde_json::json!({
            "detached": true,
            "removed": removed,
            "preserved": preserved,
            "evidence_retained": true,
        }))
    })();
    match result {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            println!("{}", error.to_json());
            error.exit_code()
        }
    }
}

/// Resolve effective hook path through Git, including worktree-aware paths.
pub fn resolve_hooks_dir(root: &std::path::Path) -> Result<std::path::PathBuf, PorcelainError> {
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
            "is `git` on PATH?",
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

/// Return effective `core.hooksPath` when Git configuration overrides default hooks.
pub fn configured_hooks_path(root: &std::path::Path) -> Result<Option<String>, PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["config", "--get", "core.hooksPath"])
        .output()
        .map_err(|e| PorcelainError::io("read core.hooksPath", root, &e))?;
    if !output.status.success() {
        return Ok(None);
    }
    let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok((!path.is_empty()).then_some(path))
}

pub(crate) struct HookInstallResult {
    pub installed: Vec<String>,
    pub noop: Vec<String>,
    pub conflict: Vec<String>,
}

/// Install managed hooks only into Git's active default hook directory.
pub(crate) fn install_hooks(root: &std::path::Path) -> Result<HookInstallResult, PorcelainError> {
    if configured_hooks_path(root)?.is_some() {
        return Err(PorcelainError::new(
            "custom_hooks_path",
            "refusing to install hooks while core.hooksPath is configured",
            "unset core.hooksPath or install hugit hooks in that path yourself",
        ));
    }
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
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(PorcelainError::new(
                    "unsafe_hook_path",
                    format!("refusing non-regular hook path {path:?}"),
                    "replace the hook path with a regular file, then rerun setup",
                ));
            }
            Ok(_) => {
                let contents = std::fs::read_to_string(&path)
                    .map_err(|e| PorcelainError::io("read hook", &path, &e))?;
                if is_managed_hook(kind, &contents) {
                    result.noop.push(kind.to_string());
                } else {
                    result.conflict.push(kind.to_string());
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::write(&path, hook_script(kind))
                    .map_err(|e| PorcelainError::io("write hook", &path, &e))?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut permissions = std::fs::metadata(&path)
                        .map_err(|e| PorcelainError::io("stat hook", &path, &e))?
                        .permissions();
                    permissions.set_mode(0o755);
                    std::fs::set_permissions(&path, permissions)
                        .map_err(|e| PorcelainError::io("chmod hook", &path, &e))?;
                }
                result.installed.push(kind.to_string());
            }
            Err(error) => return Err(PorcelainError::io("stat hook", &path, &error)),
        }
    }
    Ok(result)
}

/// Resolve repository root through Git, including nested directories and worktrees.
pub fn git_top_level(path: &std::path::Path) -> Result<PathBuf, PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| PorcelainError::io("resolve repository root", path, &e))?;
    if !output.status.success() {
        return Err(PorcelainError::new(
            "not_a_git_repo",
            format!("{} is not a Git repository", path.display()),
            "run this command inside a Git repository",
        ));
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if root.is_empty() {
        return Err(PorcelainError::new(
            "not_a_git_repo",
            format!("{} has no Git top-level", path.display()),
            "run this command inside a Git repository",
        ));
    }
    Ok(PathBuf::from(root))
}

/// A hook is hugit-owned only when it exactly matches deterministic renderer output.
pub fn is_managed_hook(kind: &str, contents: &str) -> bool {
    contents == hook_script(kind)
}

/// Legacy hook storage is shared by linked worktrees through their main worktree.
pub fn legacy_log_root(path: &std::path::Path) -> Result<PathBuf, PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(path)
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|e| PorcelainError::io("resolve main worktree", path, &e))?;
    if !output.status.success() {
        return Err(PorcelainError::new(
            "git_worktree_list_failed",
            format!("`git worktree list --porcelain` in {:?} failed", path),
            "use a non-bare Git working tree",
        ));
    }
    let main = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from);
    main.ok_or_else(|| {
        PorcelainError::new(
            "git_worktree_list_invalid",
            "Git returned no main working tree",
            "use a non-bare Git working tree",
        )
    })
}

/// The shell body for each hook, generated deterministically. Every hook:
/// - resolves the hugit binary ($HUGIT_BIN then `hugit`),
/// - resolves the repo root via git itself (worktree-safe),
/// - detaches the capture child (`nohup ... &` + re-direct) then exits 0,
///   so a hugit failure can NEVER fail/block the git operation.
pub(crate) fn hook_script(kind: &str) -> String {
    // The capture invocation for each kind (post-commit takes the new HEAD +
    // branch; post-checkout passes from/to/branch when flag==1; pre-push reads
    // refspecs/shas from stdin into the child; post-merge passes the merged tip).
    match kind {
    "post-commit" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: the LLM used `git commit`; hugit records ref.update async.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# Runtime state lives in Git common dir, shared across all linked worktrees.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
RUNTIME="$COMMON/hugit"
LOG="$RUNTIME/event-log.json"
HL="$RUNTIME/hooks.log"
(
  FILES="$(git diff-tree --root --name-only -r --no-commit-id HEAD 2>/dev/null)"
  FILE_ARGS=""
  for f in $FILES; do
FILE_ARGS="$FILE_ARGS --files $f"
  done
  "$HUGIT_BIN" capture --kind commit --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --oid "$(git rev-parse HEAD 2>/dev/null)"     --branch "$(git branch --show-current 2>/dev/null)"     --recorded-at "$(git log -1 --format=%ct 2>/dev/null)" $FILE_ARGS
) >/dev/null 2>&1 &
exit 0
"#.to_string(),
    "post-checkout" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: branch checkout (flag=1); records ref.update {checkout:true}.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# Runtime state lives in Git common dir, shared across all linked worktrees.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
RUNTIME="$COMMON/hugit"
LOG="$RUNTIME/event-log.json"
HL="$RUNTIME/hooks.log"
[ "$3" = "1" ] || exit 0   # only branch checkouts (flag=1), not file checkouts
GITDIR=$(git rev-parse --absolute-git-dir 2>/dev/null) || exit 0
(
  "$HUGIT_BIN" capture --kind checkout --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --from "$1" --oid "$2" --branch "$(git branch --show-current 2>/dev/null)"
) >/dev/null 2>&1 &
# Worktree-dock (ADR-0005, WP-DOCK-1): coin the physical binding at checkout
# time (idempotent — marker present ⇒ no-op; never blocks git).
(
  "$HUGIT_BIN" dock coin --top-level "$ROOT" --gitdir "$GITDIR" --log "$LOG" --hook-log "$HL" --branch "$(git branch --show-current 2>/dev/null)"
) >/dev/null 2>&1 &
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
# Runtime state lives in Git common dir, shared across all linked worktrees.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
RUNTIME="$COMMON/hugit"
LOG="$RUNTIME/event-log.json"
HL="$RUNTIME/hooks.log"
STDIN_REFS="$(cat)"   # first line: remote-name + url; then <local-ref> <local-sha> <remote-ref> <remote-sha> per line
# Extract the LOCAL sha (2nd field) from each refspec line that has 4 fields.
SHAS="$(echo "$STDIN_REFS" | awk 'NF>=4 {print $2}')"
(
  "$HUGIT_BIN" capture --kind push-attempt --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --refspecs "$STDIN_REFS" --shas "$SHAS"
) >/dev/null 2>&1 &
exit 0
"#.to_string(),
    "post-merge" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: a local merge landed; records ref.update {merged_from}.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
# Runtime state lives in Git common dir, shared across all linked worktrees.
COMMON=$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null) || exit 0
RUNTIME="$COMMON/hugit"
LOG="$RUNTIME/event-log.json"
HL="$RUNTIME/hooks.log"
(
  "$HUGIT_BIN" capture --kind merge --top-level "$ROOT" --log "$LOG" --hook-log "$HL"     --from "$(git rev-parse HEAD~1 2>/dev/null)"     --oid "$(git rev-parse HEAD 2>/dev/null)"     --recorded-at "$(git log -1 --format=%ct 2>/dev/null)"
) >/dev/null 2>&1 &
exit 0
"#.to_string(),
    _ => unreachable!("known hook kind"),
}
}

pub(crate) fn do_run(args: &InitArgs) -> Result<serde_json::Value, PorcelainError> {
    let root = args.dir.clone().unwrap_or_else(|| PathBuf::from("."));

    // Git-proximate ceremony: ensure a git repo exists. If `root` is not yet
    // a git repository, shell out to `git init` (same CLI a user would run);
    // if it already is (`.git` dir or worktree file) leave it untouched.
    let git_created = if is_git_repo(&root) {
        false
    } else {
        git_init(&root)?;
        true
    };

    // Validate source before runtime writes or hook installation. Re-entry
    // resumes each atomic file boundary without changing source evidence.
    let runtime = crate::runtime_store::for_repo(&root)?;
    let log_path = runtime.canonical_log();
    let created = !log_path.exists();
    let migration =
        crate::runtime_store::migrate(&runtime, &crate::runtime_store::legacy_path(&root))?;

    // Install the silent git hooks (post-commit/checkout/push/merge) so the
    // LLM using git normally is captured into the log asynchronously. A
    // conflict (a pre-existing non-hugit hook) is reported, never clobbered.
    let hooks = install_hooks(&root)?;

    let hint = if created {
        "initialized empty runtime event log; next: `hugit campaign open --campaign <key> \
         --charter <text> --owner <you>` then `hugit intent new …`"
    } else {
        "runtime event log already exists; left untouched (init is idempotent)"
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
        "runtime_dir": runtime.root.display().to_string(),
        "log": log_path.display().to_string(),
        "migration": migration,
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
        let log = crate::runtime_store::for_repo(&root)
            .unwrap()
            .canonical_log();
        assert!(log.is_file(), "log file created");
        let bytes = std::fs::read(&log).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            parsed.as_array().is_some_and(|a| a.is_empty()),
            "log is an empty JSON array"
        );
        assert!(log.parent().unwrap().is_dir(), "runtime directory created");
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
        assert!(
            crate::runtime_store::for_repo(&root)
                .unwrap()
                .canonical_log()
                .is_file(),
            "log created"
        );
    }

    #[test]
    fn init_migrates_valid_legacy_log_into_git_runtime() {
        let root = scratch("migrate-legacy");
        git_init(&root).expect("git init");
        let legacy = crate::runtime_store::legacy_path(&root);
        std::fs::create_dir_all(legacy.parent().expect("legacy parent")).unwrap();
        let legacy_bytes = b"[]\n";
        std::fs::write(&legacy, legacy_bytes).unwrap();

        let value = do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("migrate valid legacy log");
        let runtime = crate::runtime_store::for_repo(&root).unwrap();
        assert_eq!(std::fs::read(runtime.legacy_log()).unwrap(), legacy_bytes);
        let records: Vec<serde_json::Value> =
            serde_json::from_slice(&std::fs::read(runtime.canonical_log()).unwrap()).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["kind"], crate::runtime_store::LEGACY_PREFIX_KIND);
        assert_eq!(
            value["migration"]["migration"]["legacy_prefix_sha256"],
            crate::runtime_store::sha256_hex(legacy_bytes)
        );
    }

    #[test]
    fn init_installs_managed_hooks_into_git_hooks_dir() {
        let root = scratch("install-hooks");
        do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("init installs hooks");
        let hooks_dir = resolve_hooks_dir(&root).unwrap();

        for kind in HOOK_KINDS {
            let contents = std::fs::read_to_string(hooks_dir.join(kind)).unwrap();
            assert!(is_managed_hook(kind, &contents), "{kind} is managed");
            assert!(
                contents.contains("$COMMON/hugit"),
                "{kind} uses Git runtime"
            );
        }
    }

    #[test]
    fn init_is_idempotent_and_never_clobbers() {
        let root = scratch("idem");
        do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("first init");
        // Seed valid canonical bytes so re-init proves it does not clobber state.
        let log = crate::runtime_store::for_repo(&root)
            .unwrap()
            .canonical_log();
        let seeded = b"[]\n";
        std::fs::write(&log, seeded).unwrap();

        let v = do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("second init ok");
        assert_eq!(v["initialized"], false, "re-init reports not-created");
        // The seeded bytes survive — init never clobbers an existing log.
        assert_eq!(
            std::fs::read(&log).unwrap(),
            seeded,
            "existing log left untouched"
        );
    }
}
