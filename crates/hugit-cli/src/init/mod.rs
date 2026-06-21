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

    Ok(json!({
        "initialized": created,
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
    fn init_creates_dir_and_empty_log_array() {
        let root = scratch("create");
        let v = do_run(&InitArgs {
            dir: Some(root.clone()),
        })
        .expect("init ok");
        assert_eq!(v["initialized"], true);
        let log = root.join(".hugit/log.json");
        assert!(log.is_file(), "log file created");
        let bytes = std::fs::read(&log).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(
            parsed.as_array().is_some_and(|a| a.is_empty()),
            "log is an empty JSON array"
        );
        assert!(root.join(".hugit").is_dir(), ".hugit dir created");
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
