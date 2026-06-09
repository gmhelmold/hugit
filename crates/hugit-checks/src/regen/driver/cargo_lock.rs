//! Cargo.lock regeneration driver (WP-C4 v0, lockfile class).
//!
//! Regenerates `Cargo.lock` by running `cargo generate-lockfile` in the
//! workspace root.  Hand-edits to `Cargo.lock` are discarded — the file is
//! always regenerated from `Cargo.toml` / `[dependencies]` sources (item ④).
//!
//! Fail-CLOSED: any non-zero exit from `cargo generate-lockfile` is surfaced
//! as `RegenError::FailClosed`; the text-merge code-path is never entered.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::{DerivedClass, RegenDriver, RegenError, which_tool};

/// Driver for `Cargo.lock`.
///
/// Regenerates by running:
/// ```text
/// cargo generate-lockfile --manifest-path <workspace_root>/Cargo.toml
/// ```
///
/// Deterministic: given the same `Cargo.toml` dependency tree and the same
/// Cargo registry snapshot, `cargo generate-lockfile` is deterministic.
#[derive(Debug, Default)]
pub struct CargoLockDriver {
    /// Optional override for the cargo binary path (used in tests).
    cargo_bin: Option<PathBuf>,
}

impl CargoLockDriver {
    /// Construct with the `cargo` binary found in PATH.
    pub fn new() -> Self {
        Self { cargo_bin: None }
    }

    /// Construct with an explicit cargo binary path (test helper).
    pub fn with_bin(bin: PathBuf) -> Self {
        Self {
            cargo_bin: Some(bin),
        }
    }

    fn cargo(&self) -> PathBuf {
        self.cargo_bin
            .clone()
            .unwrap_or_else(|| PathBuf::from("cargo"))
    }
}

impl RegenDriver for CargoLockDriver {
    fn class(&self) -> DerivedClass {
        DerivedClass::Lockfile
    }

    fn owns(&self, path: &Path) -> bool {
        path.file_name()
            .map(|n| n.eq_ignore_ascii_case("Cargo.lock"))
            .unwrap_or(false)
    }

    fn regenerate(&self, workspace_root: &Path, path: &Path) -> Result<PathBuf, RegenError> {
        let manifest = workspace_root.join("Cargo.toml");

        let status = Command::new(self.cargo())
            .args(["generate-lockfile", "--manifest-path"])
            .arg(&manifest)
            .status()
            .map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    RegenError::ToolNotFound {
                        tool: "cargo".to_owned(),
                    }
                } else {
                    RegenError::Io(e)
                }
            })?;

        if status.success() {
            Ok(workspace_root.join(path.file_name().unwrap_or(path.as_os_str())))
        } else {
            Err(RegenError::FailClosed {
                message: "cargo generate-lockfile failed — constraints unsatisfiable or \
                          registry unavailable; refusing to merge derived file"
                    .to_owned(),
                exit_code: status.code(),
            })
        }
    }

    fn tool_available(&self) -> bool {
        which_tool(&self.cargo())
    }
}
