//! `hugit health` reports local hook and log health without mutating Git state.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use serde_json::{Map, Value, json};

use crate::init::{
    HOOK_KINDS, configured_hooks_path, git_top_level, is_managed_hook, resolve_hooks_dir,
};

/// Arguments for `hugit health`.
#[derive(clap::Args, Debug)]
pub struct HealthArgs {
    /// Repository directory to inspect (defaults to current directory).
    #[arg(long)]
    pub dir: Option<PathBuf>,
}

pub fn run(args: HealthArgs) -> ExitCode {
    let root = args.dir.unwrap_or_else(|| PathBuf::from("."));
    let value = health(&root);
    println!("{value}");
    ExitCode::SUCCESS
}

fn health(root: &Path) -> Value {
    let root = match git_top_level(root) {
        Ok(root) => root,
        Err(_) => {
            return json!({
                "mode": "inactive",
                "repo": root.display().to_string(),
                "next": "run `hugit attach` inside an existing Git repository",
            });
        }
    };

    match configured_hooks_path(&root) {
        Ok(Some(path)) => {
            return json!({
                "mode": "partial",
                "repo": root.display().to_string(),
                "hooks_path": {"state": "external", "path": path},
                "next": "hugit attach refuses core.hooksPath until explicit hook-manager support exists",
            });
        }
        Ok(None) => {}
        Err(error) => {
            return json!({
                "mode": "partial",
                "repo": root.display().to_string(),
                "hooks_error": error.to_json(),
            });
        }
    }

    let hooks_dir = match resolve_hooks_dir(&root) {
        Ok(path) => path,
        Err(error) => {
            return json!({
                "mode": "partial",
                "repo": root.display().to_string(),
                "hooks_error": error.to_json(),
            });
        }
    };

    let mut hooks = Map::new();
    let mut managed = 0usize;
    for kind in HOOK_KINDS {
        let path = hooks_dir.join(kind);
        let state = match std::fs::read_to_string(&path) {
            Ok(contents) if is_managed_hook(kind, &contents) && hook_is_executable(&path) => {
                managed += 1;
                "managed"
            }
            Ok(contents) if is_managed_hook(kind, &contents) => "non_executable",
            Ok(contents) if contents.contains("# hugit-hook (managed by hugit init)") => "modified",
            Ok(_) => "foreign",
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "missing",
            Err(_) => "unreadable",
        };
        hooks.insert(
            kind.replace('-', "_"),
            json!({"state": state, "path": path.display().to_string()}),
        );
    }

    let runtime = match crate::runtime_store::for_repo(&root) {
        Ok(runtime) => runtime,
        Err(error) => {
            return json!({
                "mode": "partial",
                "repo": root.display().to_string(),
                "log_error": error.to_json(),
            });
        }
    };
    let runtime_log = runtime.canonical_log();
    let legacy = crate::runtime_store::legacy_path(&root);
    let (log, log_state) = if runtime_log.exists() {
        let state = if crate::checks::load_event_log(&runtime_log).is_ok() {
            "valid"
        } else {
            "invalid"
        };
        (runtime_log, state)
    } else if legacy.exists() {
        let state = if crate::checks::load_event_log(&legacy).is_ok() {
            "legacy_pending"
        } else {
            "blocked"
        };
        (legacy, state)
    } else {
        (runtime_log, "missing")
    };
    let mode = if log_state == "blocked" {
        "blocked"
    } else if managed == HOOK_KINDS.len() && log_state == "valid" {
        "active"
    } else if managed == 0 && log_state == "missing" {
        "inactive"
    } else {
        "partial"
    };

    json!({
        "mode": mode,
        "repo": root.display().to_string(),
        "hooks": hooks,
        "log": {"path": log.display().to_string(), "state": log_state, "runtime_dir": runtime.root.display().to_string()},
        "capture": {"state": "best_effort", "receipts": "not_available_yet", "dead_letters": "not_available_yet"},
        "next": if mode == "active" { "hooks are installed; capture failures remain observable only in hooks.log until receipt spool ships" } else { "run `hugit attach` to install missing hooks; foreign hooks are preserved" },
    })
}

#[cfg(unix)]
fn hook_is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

#[cfg(not(unix))]
fn hook_is_executable(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inactive_outside_git_repo() {
        let root = std::env::temp_dir().join(format!("hugit-health-{}", std::process::id()));
        let value = health(&root);
        assert_eq!(value["mode"], "inactive");
    }

    #[test]
    fn active_from_nested_directory() {
        let unique = format!(
            "hugit-health-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        );
        let repo = std::env::temp_dir().join(unique);
        std::fs::create_dir(&repo).expect("create temp repo");
        let output = std::process::Command::new("git")
            .arg("init")
            .arg(&repo)
            .output()
            .expect("git init");
        assert!(output.status.success());
        let hooks = repo.join(".git/hooks");
        for kind in HOOK_KINDS {
            let hook = hooks.join(kind);
            std::fs::write(&hook, crate::init::hook_script(kind)).expect("write managed hook");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755))
                    .expect("make hook executable");
            }
        }
        let runtime = crate::runtime_store::for_repo(&repo).expect("resolve runtime");
        std::fs::create_dir_all(&runtime.root).expect("create runtime");
        std::fs::write(runtime.canonical_log(), b"[]\n").expect("write log");
        let nested = repo.join("nested");
        std::fs::create_dir(&nested).expect("create nested directory");

        let value = health(&nested);
        assert_eq!(value["mode"], "active");
        assert_eq!(value["log"]["state"], "valid");
        std::fs::remove_dir_all(repo).expect("remove temp repo");
    }
}
