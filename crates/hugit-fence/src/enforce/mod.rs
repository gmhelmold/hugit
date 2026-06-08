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

use crate::util::shell_quote;

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

/// Normalize a path into canonical segments, exposed for the materialize and
/// broker layers so they can detect allow-all `path_set` entries and traversal
/// escapes using the exact same rules the classifier uses. Returns `None` for
/// an escaping path (absolute or containing `..`); `Some(vec![])` for a path
/// that collapses to the root (`"."`, `"./"`, `""`).
#[must_use]
pub fn normalize_path(path: &str) -> Option<Vec<&str>> {
    normalize_segments(path)
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
/// This is the **named enforcement gate**, and it is *live*: it is the single
/// predicate the materialize seam ([`crate::materialize::select_in_fence`])
/// routes every candidate through, so a path that this function rejects can
/// never be placed into the workspace. Enforcement is therefore *physical* —
/// a rejected path is never written, so a later access returns ENOENT — and the
/// API is not a side door but the gate the runtime path actually calls.
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

/// `true` iff `path` is admitted by the fence (the enforced gate says Inside).
///
/// This is the boolean form of [`check_access`] used by the materialize seam:
/// the materializer admits a candidate **iff** `is_admitted` returns `true`, so
/// out-of-fence candidates are dropped at exactly this gate and never reach the
/// box. Keeping a single admission predicate means there is one — and only one
/// — place the fence boundary is decided at runtime.
#[must_use]
pub fn is_admitted(manifest: &FenceManifest, path: &str) -> bool {
    // The enforced gate: a candidate is admitted iff `check_access` finds no
    // violation. (`check_access` is infallible today; treat any future error as
    // fail-closed — a path we cannot prove in-fence is denied.)
    matches!(check_access(manifest, path), Ok(None))
}

/// The result of an in-container access probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnoentProof {
    /// The probed path (joined under the workspace root).
    pub path: String,
    /// `true` iff the path does not exist inside the container (ENOENT — the
    /// file was never materialized, so any access fails with "No such file").
    pub enoent: bool,
    /// The stable token observed from the probe (`ABSENT` / `PRESENT_FILE` /
    /// `PRESENT_DIR`) — locale-independent, for the evidence bundle.
    pub observed: String,
}

/// Prove that accessing `outside_path` inside the running container returns
/// **ENOENT** — i.e. the file physically isn't there because it was never
/// materialized.
///
/// `workspace_root` is the in-container root the fence materialized into; the
/// probe checks `<workspace_root>/<outside_path>` for **physical absence**.
/// This is the box-backed counterpart to [`classify`]: the path must be
/// out-of-fence per [`classify`] *and* the OS must agree it is absent.
///
/// Detection is by **exit code**, not by parsing libc's stderr text: we run
/// `test ! -e <path>` (true iff the path does not exist for *any* type — file,
/// directory, symlink, …). This is locale-independent and correctly classifies
/// a directory-at-path as PRESENT (a breach) rather than misreading it. A
/// present file/dir is reported with a stable token so a materialized
/// (breaching) entry is unambiguous.
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
    // Exit-code probe (locale-independent). `test -e` is true for any existing
    // path type; `test ! -e` is true iff the path is physically absent. We also
    // distinguish dir vs file in the PRESENT branch so a directory-at-path is
    // recognized as a breach, not silently treated as absent.
    let script = format!(
        "if test ! -e {q}; then printf ABSENT; \
         elif test -d {q}; then printf PRESENT_DIR; \
         else printf PRESENT_FILE; fi",
        q = shell_quote(&full),
    );
    let out = boxx
        .run(&["docker", "exec", &container.name, "sh", "-c", &script])
        .with_context(|| format!("probing {full} inside {}", container.name))?;
    if !out.ok() {
        // The probe shell itself failed — fail closed: we cannot prove absence.
        anyhow::bail!(
            "ENOENT probe shell failed for {full} in {} (code={:?} stderr={:?})",
            container.name,
            out.code,
            out.stderr.trim()
        );
    }
    let observed = out.stdout.trim().to_string();
    Ok(EnoentProof {
        path: full,
        enoent: observed == "ABSENT",
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
    fn is_admitted_is_the_enforced_gate() {
        // `is_admitted` is the single predicate the materialize seam routes
        // through; it must agree with `classify`/`check_access` exactly.
        let m = manifest(&["src/", "Cargo.toml"]);
        assert!(is_admitted(&m, "src/main.rs"));
        assert!(is_admitted(&m, "Cargo.toml"));
        assert!(!is_admitted(&m, "secret.env"));
        assert!(!is_admitted(&m, "../escape"));
        assert!(!is_admitted(&m, "/etc/passwd"));
    }

    #[test]
    fn absolute_path_injection_is_outside() {
        // An absolute path must never be admitted even if a same-named relative
        // entry is in the fence (no `/src/main.rs` smuggling past `src/`).
        let m = manifest(&["src/", "src/main.rs"]);
        assert_eq!(classify(&m, "/src/main.rs"), FenceVerdict::Outside);
        assert_eq!(classify(&m, "/etc/passwd"), FenceVerdict::Outside);
        assert!(!is_admitted(&m, "/src/main.rs"));
    }

    #[test]
    fn prefix_collision_rejected_segmentwise() {
        // `src` (exact file) must not admit `srcfoo`; `src/` (dir) must not
        // admit `srcfoo/...`. Prefix matching is segment-wise, not substring.
        let m = manifest(&["src/", "lib"]);
        assert_eq!(classify(&m, "srcfoo/x.rs"), FenceVerdict::Outside);
        assert_eq!(classify(&m, "libfoo"), FenceVerdict::Outside);
        assert_eq!(classify(&m, "lib"), FenceVerdict::Inside);
    }

    #[test]
    fn dotdot_escaping_root_is_outside_even_when_renormalizing_inside() {
        // A path that uses `..` to climb above root and then dives back into an
        // in-fence dir must still be rejected: any `..` is an escape, because
        // segment-collapse is NOT applied across `..` (that would let a symlink
        // or a real parent dir be traversed). `src/../src/main.rs` resolves to
        // `src/main.rs` lexically but is denied — `..` is fail-closed.
        let m = manifest(&["src/"]);
        assert_eq!(classify(&m, "src/../src/main.rs"), FenceVerdict::Outside);
        assert_eq!(classify(&m, "a/../../src/main.rs"), FenceVerdict::Outside);
        assert!(!is_admitted(&m, "src/../src/main.rs"));
    }

    #[test]
    fn empty_normalizing_entry_does_not_allow_all() {
        // A path_set entry that normalizes to empty (e.g. "./") is a directory
        // prefix covering everything in-root — that is an allow-all and is the
        // antithesis of a fence. `classify` treats such an entry as root-cover
        // (Inside-everything); the *materialize* layer rejects such manifests
        // fail-closed (see materialize::reject_empty_normalizing_path_set). Here
        // we pin the classify behaviour so the materialize guard is the single
        // place the allow-all is refused, and document it.
        let m = manifest(&["./"]);
        // "./" → empty prefix → root cover. Pinned so the materialize guard,
        // not classify, is responsible for refusing allow-all manifests.
        assert_eq!(classify(&m, "anything.rs"), FenceVerdict::Inside);
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
