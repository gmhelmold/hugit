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

    /// The marker line identifying a hugit-managed hook. Used for idempotent
    /// install and for clean removal — a pre-existing non-hugit hook with this
    /// marker means hugit already manages it (no rewrite).
    const HUGIT_HOOK_MARKER: &str = "# hugit-hook (managed by hugit init)";

    /// The 4 git hooks hugit installs + the capture invocation each runs.
    const HOOK_KINDS: [&str; 4] = ["post-commit", "post-checkout", "pre-push", "post-merge"];

    /// Resolve the hooks dir via git itself (worktree-safe): `git rev-parse
    /// --git-path hooks` from the git dir. Returns `Err` if git is missing.
    fn resolve_hooks_dir(root: &std::path::Path) -> Result<std::path::PathBuf, PorcelainError> {
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
        // `git rev-parse --git-path hooks` is RELATIVE to the git dir; resolve it
        // against the repo root so the writer lands in the right place regardless
        // of the process cwd (hugit init may run from anywhere).
        Ok(if rel.is_absolute() {
            rel
        } else {
            root.join(rel)
        })
    }

    /// The shell body for each hook, generated deterministically. Every hook:
    /// - resolves the hugit binary ($HUGIT_BIN then `hugit`),
    /// - resolves the repo root via git itself (worktree-safe),
    /// - detaches the capture child (`nohup ... &` + re-direct) then exits 0,
    ///   so a hugit failure can NEVER fail/block the git operation.
    fn hook_script(kind: &str) -> String {
        // The capture invocation for each kind (post-commit takes the new HEAD +
        // branch; post-checkout passes from/to/branch when flag==1; pre-push reads
        // refspecs/shas from stdin into the child; post-merge passes the merged tip).
        match kind {
        "post-commit" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: the LLM used `git commit`; hugit records ref.update async.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
LOG="$ROOT/.hugit/log.json"
[ -f "$LOG" ] || exit 0
(
  FILES="$(git diff-tree --root --name-only -r --no-commit-id HEAD 2>/dev/null)"
  FILE_ARGS=""
  for f in $FILES; do
    FILE_ARGS="$FILE_ARGS --files $f"
  done
  "$HUGIT_BIN" capture --kind commit --top-level "$ROOT" --log "$LOG" --hook-log "$ROOT/.hugit/hooks.log"     --oid "$(git rev-parse HEAD 2>/dev/null)"     --branch "$(git branch --show-current 2>/dev/null)"     --recorded-at "$(git log -1 --format=%ct 2>/dev/null)" $FILE_ARGS
) >>"$ROOT/.hugit/hooks.log" 2>&1 &
exit 0
"#.to_string(),
        "post-checkout" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: branch checkout (flag=1); records ref.update {checkout:true}.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
LOG="$ROOT/.hugit/log.json"
[ -f "$LOG" ] || exit 0
[ "$3" = "1" ] || exit 0   # only branch checkouts (flag=1), not file checkouts
(
  "$HUGIT_BIN" capture --kind checkout --top-level "$ROOT" --log "$LOG" --hook-log "$ROOT/.hugit/hooks.log"     --from "$1" --oid "$2" --branch "$(git branch --show-current 2>/dev/null)"
) >>"$ROOT/.hugit/hooks.log" 2>&1 &
exit 0
"#.to_string(),
        "pre-push" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: a push is attempted; records ref.update {attempt:true}.
# ALWAYS exits 0 — this is a PRE hook; a non-zero exit would BLOCK the push.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
LOG="$ROOT/.hugit/log.json"
[ -f "$LOG" ] || exit 0
STDIN_REFS="$(cat)"   # remote-name + url, then <local-ref> <local-sha> <remote-ref> <remote-sha> per line
(
  "$HUGIT_BIN" capture --kind push-attempt --top-level "$ROOT" --log "$LOG" --hook-log "$ROOT/.hugit/hooks.log"     --refspecs "$STDIN_REFS"
) >>"$ROOT/.hugit/hooks.log" 2>&1 &
exit 0
"#.to_string(),
        "post-merge" => r#"#!/bin/sh
# hugit-hook (managed by hugit init)
# Silent capture: a local merge landed; records ref.update {merged_from}.
HUGIT_BIN="${HUGIT_BIN:-hugit}"
ROOT=$(git rev-parse --show-toplevel 2>/dev/null) || exit 0
LOG="$ROOT/.hugit/log.json"
[ -f "$LOG" ] || exit 0
(
  "$HUGIT_BIN" capture --kind merge --top-level "$ROOT" --log "$LOG" --hook-log "$ROOT/.hugit/hooks.log"     --from "$(git rev-parse HEAD~1 2>/dev/null)"     --oid "$(git rev-parse HEAD 2>/dev/null)"     --recorded-at "$(git log -1 --format=%ct 2>/dev/null)"
) >>"$ROOT/.hugit/hooks.log" 2>&1 &
exit 0
"#.to_string(),
        _ => unreachable!("known hook kind"),
    }
    }

    /// Install the 4 hugit hooks into the repo's hooks dir. Idempotent: a hook
    /// file already containing the hugit marker is left untouched; a pre-existing
    /// NON-hugit hook (user's own) is preserved (we append the marker + our body
    /// only when the file does not yet reference hugit — actually we create/append
    /// carefully: if the file exists and has no marker, we leave it alone and
    /// report the conflict so a human resolves it; we never clobber a user hook).
    fn install_hooks(root: &std::path::Path) -> Result<Vec<String>, PorcelainError> {
        let hooks_dir = resolve_hooks_dir(root)?;
        std::fs::create_dir_all(&hooks_dir)
            .map_err(|e| PorcelainError::io("create hooks dir", &hooks_dir, &e))?;

        let mut installed = Vec::new();
        for kind in HOOK_KINDS {
            let path = hooks_dir.join(kind);
            if path.exists() {
                // A hook already exists. If it's ours (marker present) → no-op
                // (idempotent). If it's someone else's → DO NOT clobber: report
                // the conflict, leave it alone (a human resolves).
                let existing = std::fs::read_to_string(&path)
                    .map_err(|e| PorcelainError::io("read hook", &path, &e))?;
                if existing.contains(HUGIT_HOOK_MARKER) {
                    continue; // already ours, idempotent no-op
                }
                // Not ours: preserve + record the conflict honestly.
                installed.push(format!("{kind}:conflict"));
                continue;
            }
            std::fs::write(&path, hook_script(kind))
                .map_err(|e| PorcelainError::io("write hook", &path, &e))?;
            // hooks must be executable for git to run them.
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
            installed.push(kind.to_string());
        }
        Ok(installed)
    }

    // Install the silent git hooks (post-commit/checkout/push/merge) so the
    // LLM using git normally is captured into the log asynchronously. A
    // conflict (a pre-existing non-hugit hook) is reported, never clobbered.
    let hooks = install_hooks(&root)?;
    let hooks_conflict: Vec<&String> = hooks.iter().filter(|h| h.ends_with(":conflict")).collect();

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
        "hooks_installed": hooks.iter().map(|h| h.trim_end_matches(":conflict")).collect::<Vec<_>>(),
        "hooks_conflict": hooks_conflict.iter().map(|h| h.trim_end_matches(":conflict")).collect::<Vec<_>>(),
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
}
