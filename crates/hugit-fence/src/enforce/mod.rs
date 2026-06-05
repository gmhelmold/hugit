//! Fence enforcement: classify a path against a [`FenceManifest`] path-set and
//! prove the ENOENT guarantee on the live box.
//!
//! The fence is *physical*: outside the `path_set` the file was never
//! materialized, so an access returns ENOENT. [`classify`] is the pure
//! decision (in-fence vs out-of-fence); [`probe_outside_enoent`] is the
//! box-backed proof that an out-of-fence access really fails with ENOENT inside
//! a running per-job container.

use anyhow::{Context, Result};
use hugit_contracts::FenceManifest;
use hugit_runner::isolation::RunningContainer;
use hugit_runner::lease::BoxExec;

/// Whether a path is inside or outside the fence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FenceVerdict {
    /// The path is covered by the manifest `path_set` (in-fence; materialized).
    Inside,
    /// The path is not covered by the manifest `path_set` (out-of-fence;
    /// physically absent → ENOENT on access).
    Outside,
}

impl FenceVerdict {
    /// `true` iff the path is inside the fence.
    #[must_use]
    pub fn is_inside(self) -> bool {
        matches!(self, FenceVerdict::Inside)
    }
}

/// A detected fence violation: an access that resolved outside the fence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceViolation {
    /// The offending path, as requested.
    pub path: String,
    /// Human-readable reason the path is out-of-fence.
    pub reason: String,
}

/// Normalize a relative path into canonical, slash-separated segments.
///
/// Returns `None` if the path escapes the workspace root — an absolute path or
/// any `..` component is treated as an escape (and therefore out-of-fence),
/// because such a path can resolve outside the materialized set. `.` and empty
/// segments are dropped.
fn normalize_segments(path: &str) -> Option<Vec<&str>> {
    if path.starts_with('/') {
        return None; // absolute → escapes the workspace root
    }
    let mut out = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}       // drop empty / current-dir segments
            ".." => return None, // parent traversal → escape
            s => out.push(s),
        }
    }
    Some(out)
}

/// Classify `path` against the fence `manifest`.
///
/// A path is **inside** iff, after normalization, it is covered by some
/// `path_set` entry:
/// - an entry ending in `/` is a directory prefix: the path is inside iff it is
///   the directory itself or any descendant;
/// - any other entry is an exact file: the path is inside iff it matches
///   exactly.
///
/// Any path that escapes the workspace root (absolute, or containing `..`) is
/// **outside** by construction. When `deny_default` is false the classifier
/// still uses the explicit allowlist — `deny_default = true` is the contract
/// for production manifests, but `classify` does not weaken the allowlist when
/// it is false (it never *adds* paths).
#[must_use]
pub fn classify(manifest: &FenceManifest, path: &str) -> FenceVerdict {
    let Some(segs) = normalize_segments(path) else {
        return FenceVerdict::Outside;
    };

    for entry in &manifest.path_set {
        if entry.ends_with('/') {
            // Directory prefix. Normalize the prefix the same way; an escaping
            // or empty prefix matches nothing.
            let Some(prefix) = normalize_segments(entry) else {
                continue;
            };
            if prefix.is_empty() {
                // "/" or "./" — a root entry covers everything in-root.
                return FenceVerdict::Inside;
            }
            if segs.len() >= prefix.len() && segs[..prefix.len()] == prefix[..] {
                return FenceVerdict::Inside;
            }
        } else {
            // Exact file entry.
            let Some(exact) = normalize_segments(entry) else {
                continue;
            };
            if segs == exact {
                return FenceVerdict::Inside;
            }
        }
    }
    FenceVerdict::Outside
}

/// Validate an access request against the fence, returning the violation if the
/// path is out-of-fence.
///
/// # Errors
/// Never errors; returns `Ok(None)` when in-fence, `Ok(Some(_))` otherwise.
/// (Result-typed for symmetry with the box-backed probe and forward
/// compatibility.)
pub fn check_access(manifest: &FenceManifest, path: &str) -> Result<Option<FenceViolation>> {
    match classify(manifest, path) {
        FenceVerdict::Inside => Ok(None),
        FenceVerdict::Outside => Ok(Some(FenceViolation {
            path: path.to_string(),
            reason: "path is outside the fence path_set (not materialized)".to_string(),
        })),
    }
}

/// The result of an in-container access probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnoentProof {
    /// The probed path (joined under the workspace root).
    pub path: String,
    /// `true` iff opening the path inside the container failed with ENOENT.
    pub enoent: bool,
    /// The raw stderr/stdout token observed from the probe (for the evidence
    /// bundle).
    pub observed: String,
}

/// Prove that accessing `outside_path` inside the running container returns
/// **ENOENT** — i.e. the file physically isn't there because it was never
/// materialized.
///
/// `workspace_root` is the in-container root the fence materialized into; the
/// probe attempts to read `<workspace_root>/<outside_path>` and confirms the OS
/// reports "No such file or directory". This is the box-backed counterpart to
/// [`classify`]: the path must be out-of-fence per [`classify`] *and* the OS
/// must agree it is absent.
///
/// # Errors
/// Fails if the box itself is unreachable (the caller asserts on the returned
/// [`EnoentProof`] for a *present* file, which is a fence breach).
pub fn probe_outside_enoent<B: BoxExec>(
    boxx: &B,
    container: &RunningContainer,
    workspace_root: &str,
    outside_path: &str,
) -> Result<EnoentProof> {
    let full = join_under_root(workspace_root, outside_path);
    // Use `cat` and capture stderr: a missing file yields the libc ENOENT
    // string "No such file or directory". `printf` a stable token on the
    // success branch so a materialized (breaching) file is unambiguous.
    let script = format!(
        "if cat -- {q} >/dev/null 2>err.$$; then printf PRESENT; \
         else if grep -qi 'No such file' err.$$ 2>/dev/null; then printf ENOENT; \
         else printf 'OTHER:'; cat err.$$ 2>/dev/null; fi; fi; rm -f err.$$",
        q = shell_quote(&full),
    );
    let out = boxx
        .run(&["docker", "exec", &container.name, "sh", "-c", &script])
        .with_context(|| format!("probing {full} inside {}", container.name))?;
    let observed = out.stdout.trim().to_string();
    Ok(EnoentProof {
        path: full,
        enoent: observed == "ENOENT",
        observed,
    })
}

/// Join an in-container path under the workspace root, collapsing a trailing
/// slash on the root.
fn join_under_root(root: &str, path: &str) -> String {
    let root = root.trim_end_matches('/');
    let path = path.trim_start_matches('/');
    format!("{root}/{path}")
}

/// POSIX single-quote a path for safe interpolation into a remote `sh -c`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_contracts::FenceManifest;

    fn manifest(paths: &[&str]) -> FenceManifest {
        FenceManifest {
            path_set: paths.iter().map(|s| s.to_string()).collect(),
            deny_default: true,
            materialized: vec![],
        }
    }

    #[test]
    fn exact_file_inside() {
        let m = manifest(&["src/main.rs", "Cargo.toml"]);
        assert_eq!(classify(&m, "src/main.rs"), FenceVerdict::Inside);
        assert_eq!(classify(&m, "Cargo.toml"), FenceVerdict::Inside);
    }

    #[test]
    fn unlisted_file_outside() {
        let m = manifest(&["src/main.rs"]);
        assert_eq!(classify(&m, "src/secret.rs"), FenceVerdict::Outside);
        assert_eq!(classify(&m, "Cargo.toml"), FenceVerdict::Outside);
    }

    #[test]
    fn dir_prefix_covers_descendants() {
        let m = manifest(&["src/"]);
        assert_eq!(classify(&m, "src/main.rs"), FenceVerdict::Inside);
        assert_eq!(classify(&m, "src/inner/deep.rs"), FenceVerdict::Inside);
        assert_eq!(classify(&m, "tests/x.rs"), FenceVerdict::Outside);
    }

    #[test]
    fn dir_prefix_is_not_a_sibling_substring() {
        // "src/" must not match "srcfoo/x" — prefix is segment-wise.
        let m = manifest(&["src/"]);
        assert_eq!(classify(&m, "srcfoo/x.rs"), FenceVerdict::Outside);
    }

    #[test]
    fn parent_traversal_escapes_outside() {
        let m = manifest(&["src/"]);
        assert_eq!(classify(&m, "src/../etc/passwd"), FenceVerdict::Outside);
        assert_eq!(classify(&m, "../outside.rs"), FenceVerdict::Outside);
    }

    #[test]
    fn absolute_path_escapes_outside() {
        let m = manifest(&["src/"]);
        assert_eq!(classify(&m, "/etc/passwd"), FenceVerdict::Outside);
    }

    #[test]
    fn dot_segments_normalized() {
        let m = manifest(&["src/main.rs"]);
        assert_eq!(classify(&m, "./src/main.rs"), FenceVerdict::Inside);
        assert_eq!(classify(&m, "src/./main.rs"), FenceVerdict::Inside);
    }

    #[test]
    fn check_access_reports_violation_outside() {
        let m = manifest(&["src/"]);
        assert!(check_access(&m, "src/lib.rs").unwrap().is_none());
        let v = check_access(&m, "secret.env").unwrap().unwrap();
        assert_eq!(v.path, "secret.env");
    }

    #[test]
    fn join_under_root_collapses_slashes() {
        assert_eq!(join_under_root("/work/", "/a/b"), "/work/a/b");
        assert_eq!(join_under_root("/work", "a/b"), "/work/a/b");
    }

    #[test]
    fn shell_quote_escapes_quotes() {
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
    }
}
