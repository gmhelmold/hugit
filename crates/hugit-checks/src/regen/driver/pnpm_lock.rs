//! pnpm-lock.yaml regeneration driver (WP-C4 v0, lockfile class).
//!
//! Regenerates `pnpm-lock.yaml` deterministically by running
//! `pnpm install --lockfile-only --dir <workspace_root>` — pnpm resolves the
//! dependency graph from the `package.json` manifests and rewrites the lockfile
//! WITHOUT touching `node_modules`. `--lockfile-only` is the correct
//! regenerate-from-sources mode (the previous bare `pnpm install --dir` could
//! mutate the lockfile against an already-installed store, i.e. nondeterministic
//! regen; brutal-review §hugit-checks pnpm nondeterminism).
//!
//! Hand-edits to `pnpm-lock.yaml` are discarded — the file is always
//! regenerated from `package.json` sources (item ④).
//!
//! Fail-CLOSED: any non-zero exit is surfaced as `RegenError::FailClosed`;
//! the text-merge code-path is never entered.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{DerivedClass, RegenDriver, RegenError};

/// Driver for `pnpm-lock.yaml`.
///
/// Regenerates by running:
/// ```text
/// pnpm install --lockfile-only --dir <workspace_root>
/// ```
///
/// Deterministic: `--lockfile-only` resolves the graph from the manifests and
/// rewrites ONLY the lockfile (no store/`node_modules` side effects), so the
/// same `package.json` sources + registry snapshot always reproduce the same
/// lockfile bytes.
#[derive(Debug, Default)]
pub struct PnpmLockDriver {
    /// Optional override for the pnpm binary path (used in tests).
    pnpm_bin: Option<PathBuf>,
}

/// The fixed regen sub-command (everything after the workspace-root value).
///
/// Exposed as a pure function so the determinism contract — that the regen runs
/// `pnpm install --lockfile-only` (lockfile-only, never a bare install that can
/// drift against an installed store) — is assertable WITHOUT spawning pnpm.
/// The `--dir <workspace_root>` value is appended by [`PnpmLockDriver::regenerate`].
pub const PNPM_REGEN_ARGS: &[&str] = &["install", "--lockfile-only", "--dir"];

impl PnpmLockDriver {
    /// Construct with the `pnpm` binary found in PATH.
    pub fn new() -> Self {
        Self { pnpm_bin: None }
    }

    /// Construct with an explicit pnpm binary path (test helper).
    pub fn with_bin(bin: PathBuf) -> Self {
        Self {
            pnpm_bin: Some(bin),
        }
    }

    fn pnpm(&self) -> PathBuf {
        self.pnpm_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from("pnpm"))
    }
}

impl RegenDriver for PnpmLockDriver {
    fn class(&self) -> DerivedClass {
        DerivedClass::Lockfile
    }

    fn owns(&self, path: &Path) -> bool {
        path.file_name()
            .map(|n| n.eq_ignore_ascii_case("pnpm-lock.yaml"))
            .unwrap_or(false)
    }

    fn regenerate(&self, workspace_root: &Path, path: &Path) -> Result<PathBuf, RegenError> {
        let status = Command::new(self.pnpm())
            .args(PNPM_REGEN_ARGS)
            .arg(workspace_root)
            .status()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    RegenError::ToolNotFound {
                        tool: "pnpm".to_owned(),
                    }
                } else {
                    RegenError::Io(e)
                }
            })?;

        if status.success() {
            Ok(workspace_root.join(path.file_name().unwrap_or(path.as_os_str())))
        } else {
            Err(RegenError::FailClosed {
                message: "pnpm install failed — constraints unsatisfiable or \
                          registry unavailable; refusing to merge derived file"
                    .to_owned(),
                exit_code: status.code(),
            })
        }
    }

    fn tool_available(&self) -> bool {
        Command::new("which")
            .arg(self.pnpm())
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}
