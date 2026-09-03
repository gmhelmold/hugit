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
