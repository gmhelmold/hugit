//! Regen driver framework — regenerate, never merge (WP-C4).
//!
//! Derived-file regeneration drivers v0.  Any file classified as
//! derived (lockfile, codegen, snapshot) is **always** regenerated from
//! sources; the text-merge code-path is structurally unreachable for those
//! paths (see `RegenDriver::regenerate` + item ⑥ method-proof fixture).
//!
//! ## Fail-CLOSED contract (item ⑤)
//! If the regen command itself fails the `run` path returns
//! `Err(RegenError::FailClosed { … })`.  The caller must never
//! fall back to a text-merge for a derived-classified path.

mod cargo_lock;
mod pnpm_lock;

pub use cargo_lock::CargoLockDriver;
pub use pnpm_lock::{PNPM_REGEN_ARGS, PnpmLockDriver};

use std::path::{Path, PathBuf};
use std::process::Command;

// ── Derived-file classification ───────────────────────────────────────────────

/// The three derived-file classes exercised by the v0 regen pipeline.
///
/// Every variant is regenerated from sources; none is text-merged (item ④).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DerivedClass {
    /// A language-ecosystem lockfile (Cargo.lock, pnpm-lock.yaml, …).
    Lockfile,
    /// A file produced by a codegen step (schema → code, proto → stubs, …).
    Codegen,
    /// A snapshot file (insta snapshots, golden files, …).
    Snapshot,
}

/// Classify a file path into its derived class, if any.
///
/// Returns `None` for ordinary (non-derived) source files.
pub fn classify(path: &Path) -> Option<DerivedClass> {
    let name = path.file_name()?.to_string_lossy();
    let name_lc = name.to_lowercase();

    // Lockfiles
    if name_lc == "cargo.lock"
        || name_lc == "pnpm-lock.yaml"
        || name_lc == "package-lock.json"
        || name_lc == "yarn.lock"
        || name_lc == "poetry.lock"
        || name_lc == "gemfile.lock"
    {
        return Some(DerivedClass::Lockfile);
    }

    // Snapshots — insta / proptest / golden file conventions
    if path
        .components()
        .any(|c| c.as_os_str() == "snapshots" || c.as_os_str() == "__snapshots__")
    {
        return Some(DerivedClass::Snapshot);
    }
    if name_lc.ends_with(".snap") || name_lc.ends_with(".snap.new") {
        return Some(DerivedClass::Snapshot);
    }

    // Codegen — common generated-file markers
    if name_lc.ends_with(".pb.rs")
        || name_lc.ends_with(".pb.go")
        || name_lc.ends_with("_generated.rs")
        || name_lc.ends_with(".generated.ts")
        || name_lc.ends_with(".schema.json")
    {
        return Some(DerivedClass::Codegen);
    }

    None
}

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors returned by regen drivers.
///
/// The `FailClosed` variant enforces item ⑤: any regen-command failure
/// surfaces as a typed error so callers **cannot** silently fall back to
/// a text-merge.
#[derive(Debug)]
pub enum RegenError {
    /// The regeneration command exited non-zero or could not be spawned.
    ///
    /// The union build must propagate this as a hard failure — never a
    /// partial/silent merge of the derived file (item ⑤ fail-closed).
    FailClosed {
        /// Human-readable description of the failure.
        message: String,
        /// The exit code from the regen command (if available).
        exit_code: Option<i32>,
    },
    /// A required tool was not found in PATH.
    ToolNotFound { tool: String },
    /// I/O error during regen (e.g. workspace path not accessible).
    Io(std::io::Error),
}

impl std::fmt::Display for RegenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RegenError::FailClosed { message, exit_code } => {
                write!(f, "regen_failed (fail-closed): {message}")?;
                if let Some(code) = exit_code {
                    write!(f, " (exit {code})")?;
                }
                Ok(())
            }
            RegenError::ToolNotFound { tool } => {
                write!(f, "regen tool not found: {tool}")
            }
            RegenError::Io(e) => write!(f, "regen I/O error: {e}"),
        }
    }
}

impl std::error::Error for RegenError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            RegenError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for RegenError {
    fn from(e: std::io::Error) -> Self {
        RegenError::Io(e)
    }
}

// ── Driver trait ──────────────────────────────────────────────────────────────

/// The regen driver interface.
///
/// A driver is responsible for **one** kind of derived file.  It runs the
/// canonical regeneration command from sources; it never performs a text-merge.
///
/// # Fail-closed contract
/// If `regenerate` returns `Err(RegenError::FailClosed { … })` the caller
/// must surface a hard failure.  It must **never** fall back to a text-merge
/// for any path that `classify` returns `Some(_)` for (item ⑤ + ⑥).
pub trait RegenDriver: Send + Sync {
    /// The derived-file class this driver handles.
    fn class(&self) -> DerivedClass;

    /// Returns `true` if this driver owns `path`.
    fn owns(&self, path: &Path) -> bool;

    /// Regenerate the derived file for the given workspace root.
    ///
    /// On success returns the absolute path of the regenerated file.
    /// On failure returns `Err(RegenError::FailClosed { … })` — the caller
    /// must propagate this as a hard build failure (item ⑤).
    ///
    /// **Important:** implementations MUST NOT read any pre-existing content of
    /// `path` when regenerating — hand-edits are discarded (item ④).
    fn regenerate(&self, workspace_root: &Path, path: &Path) -> Result<PathBuf, RegenError>;

    /// Returns `true` if the tool required by this driver is present in PATH.
    fn tool_available(&self) -> bool;
}

// ── Registry ─────────────────────────────────────────────────────────────────

/// A collection of drivers used to regenerate derived files.
///
/// Lookup is by path; the first driver whose `owns` returns `true` is used.
/// If no driver owns a path the path is treated as non-derived (ordinary
/// source) and must not be regenerated.
pub struct DriverRegistry {
    drivers: Vec<Box<dyn RegenDriver>>,
}

impl DriverRegistry {
    /// Create a new registry with the default v0 drivers
    /// (Cargo.lock + pnpm-lock).
    pub fn default_v0() -> Self {
        let mut r = Self::empty();
        r.register(Box::new(CargoLockDriver::new()));
        r.register(Box::new(PnpmLockDriver::new()));
        r
    }

    /// Create an empty registry (for testing with custom drivers).
    pub fn empty() -> Self {
        Self {
            drivers: Vec::new(),
        }
    }

    /// Register a driver.
    pub fn register(&mut self, driver: Box<dyn RegenDriver>) {
        self.drivers.push(driver);
    }

    /// Find the driver that owns `path`, if any.
    pub fn driver_for(&self, path: &Path) -> Option<&dyn RegenDriver> {
        self.drivers
            .iter()
            .find(|d| d.owns(path))
            .map(|d| d.as_ref())
    }

    /// Regenerate the derived file at `path` inside `workspace_root`.
    ///
    /// **Always** returns [`RegenError::FailClosed`] on any failure — the
    /// registry is the fail-closed boundary (item ⑤). A driver may surface a
    /// `ToolNotFound` or `Io` error from its own internals, but those MUST NOT
    /// escape the boundary as non-`FailClosed` variants: a caller that pattern-
    /// matches only `FailClosed` would otherwise treat a missing tool / I/O
    /// fault as non-fatal and fall back to a text-merge. They are therefore
    /// funnelled into `FailClosed` here (the original error is preserved in the
    /// message). Returns `FailClosed` if:
    /// - the regen command fails (item ⑤),
    /// - the regen tool is missing or an I/O fault occurs (funnelled), or
    /// - no driver owns the path (caller tried to regen a non-derived file).
    ///
    /// The text-merge code-path is **never** entered for any path handled by
    /// a registered driver (item ⑥).
    pub fn regenerate(&self, workspace_root: &Path, path: &Path) -> Result<PathBuf, RegenError> {
        match self.driver_for(path) {
            Some(driver) => driver
                .regenerate(workspace_root, path)
                .map_err(fail_closed_boundary),
            None => Err(RegenError::FailClosed {
                message: format!("no driver registered for derived path `{}`", path.display()),
                exit_code: None,
            }),
        }
    }
}

/// Check whether a binary exists in PATH by running `which <bin>`.
///
/// Used by [`RegenDriver::tool_available`] implementations. Returns `true` iff
/// `which` exits zero (the binary was found).
pub(super) fn which_tool(bin: &Path) -> bool {
    Command::new("which")
        .arg(bin)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Funnel any [`RegenError`] into a [`RegenError::FailClosed`] at the registry
/// boundary (item ⑤). `FailClosed` passes through unchanged; `ToolNotFound` and
/// `Io` — which a caller could otherwise misread as non-fatal — are wrapped so
/// the only error a [`DriverRegistry::regenerate`] caller can observe is the
/// hard-failure variant.
fn fail_closed_boundary(err: RegenError) -> RegenError {
    match err {
        fc @ RegenError::FailClosed { .. } => fc,
        RegenError::ToolNotFound { ref tool } => RegenError::FailClosed {
            message: format!(
                "regen tool `{tool}` not found — refusing to merge derived file (fail-closed)"
            ),
            exit_code: None,
        },
        RegenError::Io(e) => RegenError::FailClosed {
            message: format!("regen I/O error — refusing to merge derived file (fail-closed): {e}"),
            exit_code: None,
        },
    }
}
