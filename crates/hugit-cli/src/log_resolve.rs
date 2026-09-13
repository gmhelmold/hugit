//! The ONE shared `--log` resolver — git-proximate default-path ergonomics.
//!
//! Every verb that reads/writes the canonical event log takes an OPTIONAL
//! `--log <path>`. When it is omitted, the path is resolved here, in one place,
//! so the default is identical across the whole CLI surface (the `--log`
//! friction kill, PART A):
//!
//! 1. an explicit `--log <path>` flag wins (the caller is always in control);
//! 2. else the `HUGIT_LOG` environment variable, if set and non-empty;
//! 3. else the shared Git-common-dir runtime log, migrating verified legacy
//!    state before it can be returned to any writer.
//!
//! This mirrors git's own muscle memory: you run `git log` in a repo without
//! naming the object store, because the tool knows the convention. `hugit check`
//! / `hugit pr show` / … now work the same way.

use std::path::PathBuf;

use crate::porcelain::PorcelainError;

/// The environment variable that overrides the default log path (still beaten by
/// an explicit `--log`).
pub const HUGIT_LOG_ENV: &str = "HUGIT_LOG";

/// The conventional repo-local event log, created by `hugit init`.
pub const DEFAULT_LOG_PATH: &str = ".git/hugit/event-log.json";

/// The one-line `--log` flag help, documenting the resolution order so it shows
/// up in every verb's `--help`. Used as the `#[arg(long, help = LOG_FLAG_HELP)]`
/// text wherever `--log` is optional.
pub const LOG_FLAG_HELP: &str = "Path to the canonical JSON event log. Defaults to $HUGIT_LOG, else Git common-dir runtime state.";

/// Resolve the effective `--log` path from an optional explicit flag.
///
/// The single source of truth for the default: explicit flag → `$HUGIT_LOG` →
/// runtime state. Every verb funnels its `Option<PathBuf>` through this so
/// the default can never drift between verbs.
pub fn resolve_log(explicit: Option<PathBuf>) -> PathBuf {
    resolve_log_checked(explicit).unwrap_or_else(|_| PathBuf::from(DEFAULT_LOG_PATH))
}

/// Checked resolver for mutation paths. It never returns legacy storage: valid
/// legacy bytes migrate under runtime lock; corrupt/incomplete state is a stable
/// `migration_blocked` error before a writer can bootstrap an empty log.
pub fn resolve_log_checked(explicit: Option<PathBuf>) -> Result<PathBuf, PorcelainError> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    resolve_log_for_repo(explicit, &cwd)
}

/// Resolve a default log for a declared repository rather than process CWD.
/// Hook and MCP capture callers name their repository explicitly.
pub fn resolve_log_for_repo(
    explicit: Option<PathBuf>,
    repo: &std::path::Path,
) -> Result<PathBuf, PorcelainError> {
    if let Some(path) = explicit {
        crate::runtime_store::prepare_runtime_log(&path)?;
        return Ok(path);
    }
    if let Some(env) = std::env::var_os(HUGIT_LOG_ENV)
        && !env.is_empty()
    {
        let path = PathBuf::from(env);
        crate::runtime_store::prepare_runtime_log(&path)?;
        return Ok(path);
    }
    if let Ok(store) = crate::runtime_store::for_repo(repo) {
        let canonical = store.canonical_log();
        let legacy = crate::runtime_store::legacy_path(repo);
        if canonical.exists() || legacy.exists() {
            crate::runtime_store::migrate(&store, &legacy)?;
            return Ok(canonical);
        }
        return Ok(canonical);
    }
    let legacy = crate::runtime_store::legacy_path(repo);
    if legacy.exists() {
        return Ok(legacy);
    }
    Ok(PathBuf::from(DEFAULT_LOG_PATH))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_flag_wins() {
        let p = resolve_log(Some(PathBuf::from("/tmp/explicit.json")));
        assert_eq!(p, PathBuf::from("/tmp/explicit.json"));
    }

    #[test]
    fn default_constant_describes_runtime_fallback() {
        assert_eq!(DEFAULT_LOG_PATH, ".git/hugit/event-log.json");
    }

    #[test]
    fn legacy_log_follows_unavailable_runtime() {
        let dir = std::env::temp_dir().join(format!(
            "hugit-log-resolve-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let legacy = crate::runtime_store::legacy_path(&dir);
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, b"[]\n").unwrap();

        assert_eq!(resolve_log_for_repo(None, &dir).unwrap(), legacy);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
