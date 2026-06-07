//! Sparse fence materialization: hydrate a workspace by path-set.
//!
//! Materialization is the fence. Given a frozen [`FenceManifest`] and a set of
//! candidate workspace entries, this module writes **only** the entries whose
//! path is in-fence (per [`crate::enforce::classify`]) into the runner
//! workspace on the box, and records the materialized `(path, digest)` pairs.
//! Out-of-fence candidates are dropped — never written — so a later access to
//! such a path returns ENOENT.
//!
//! C5a does **not** re-implement the snapshot/hydrate client; it *filters* the
//! materialized view by the manifest and drives the C2a runner's
//! [`BoxExec`](hugit_runner::lease::BoxExec) seam to place the in-fence files.

use std::fmt;

use anyhow::{Context, Result, bail};
use hugit_contracts::FenceManifest;
use hugit_contracts::fence_manifest::MaterializedEntry;
use hugit_runner::isolation::RunningContainer;
use hugit_runner::lease::BoxExec;

use crate::enforce::is_admitted;

/// A candidate workspace entry offered to the fence for materialization.
///
/// Models one (path, content) pair a hydrate client *could* place. The fence
/// admits it only if its path is in-fence; otherwise it is dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateEntry {
    /// Workspace-relative path of the candidate file.
    pub path: String,
    /// File content bytes.
    pub content: Vec<u8>,
}

impl CandidateEntry {
    /// Construct a candidate from a path and content.
    pub fn new(path: impl Into<String>, content: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.into(),
            content: content.into(),
        }
    }
}

/// Errors raised while materializing the sparse fence.
#[derive(Debug)]
pub enum MaterializeError {
    /// The manifest is not production-safe: `deny_default` must be `true`, else
    /// unlisted paths would not be denied and the fence would not hold.
    DenyDefaultRequired,
    /// The manifest carries a `path_set` entry that normalizes to empty (e.g.
    /// `"."`, `"./"`, or `""`) — a root-cover allow-all that admits every
    /// candidate and defeats the fence. Rejected **fail-closed**: an allow-all
    /// manifest may never materialize.
    AllowAllEntry {
        /// The offending entry as written in the manifest.
        entry: String,
    },
    /// A box command failed (placement of an in-fence file or root setup).
    Box(anyhow::Error),
}

impl fmt::Display for MaterializeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MaterializeError::DenyDefaultRequired => write!(
                f,
                "FenceManifest.deny_default must be true; a fence with default-allow \
                 cannot enforce ENOENT outside the path_set"
            ),
            MaterializeError::AllowAllEntry { entry } => write!(
                f,
                "FenceManifest path_set entry {entry:?} normalizes to root-cover \
                 (allow-all); refusing to materialize an allow-all fence (fail-closed)"
            ),
            MaterializeError::Box(e) => write!(f, "box command failed: {e}"),
        }
    }
}

impl std::error::Error for MaterializeError {}

/// SHA-256 content digest (lowercase hex).
///
/// The contract's [`MaterializedEntry::digest`] follows the frozen WP-00
/// scalar convention: digests are SHA-256 lowercase hex. (Lead FIX-FIRST at
/// integration: the initial FNV-1a placeholder violated the frozen digest
/// convention — contract scalars are not deferrable.)
fn content_digest(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A `path_set` entry that normalizes to **empty** (e.g. `"."`, `"./"`, `"/"`,
/// or `""`) is a root-cover allow-all — it would admit every candidate and
/// defeat the fence. This detects such an entry so the materializer can refuse
/// it fail-closed.
fn entry_is_allow_all(entry: &str) -> bool {
    // Reuse the same normalization the classifier uses. An entry that escapes
    // (absolute / `..`) is *not* allow-all (it simply matches nothing); only an
    // entry whose normalized segment list is empty covers the whole root.
    crate::enforce::normalize_segments_pub(entry).is_some_and(|segs| segs.is_empty())
}

/// Validate a manifest before any materialization. **Fail-closed:** rejects an
/// allow-all `path_set` entry (root-cover) that would defeat the fence.
fn validate_manifest(manifest: &FenceManifest) -> Result<(), MaterializeError> {
    if !manifest.deny_default {
        return Err(MaterializeError::DenyDefaultRequired);
    }
    for entry in &manifest.path_set {
        if entry_is_allow_all(entry) {
            return Err(MaterializeError::AllowAllEntry {
                entry: entry.clone(),
            });
        }
    }
    Ok(())
}

/// Filter `candidates` by the fence, returning only the in-fence entries paired
/// with their content digest. Pure; no box access.
///
/// This is the heart of "sparse materialization = the fence": out-of-fence
/// candidates are dropped here and never reach the box. Admission routes through
/// the **named enforcement gate** [`is_admitted`] — the *same* predicate
/// [`crate::enforce::check_access`] uses — so there is exactly one place the
/// fence boundary is decided, and the materialize filter cannot diverge from
/// the enforcement API.
#[must_use]
pub fn select_in_fence<'a>(
    manifest: &FenceManifest,
    candidates: &'a [CandidateEntry],
) -> Vec<(&'a CandidateEntry, MaterializedEntry)> {
    candidates
        .iter()
        .filter(|c| is_admitted(manifest, &c.path))
        .map(|c| {
            let entry = MaterializedEntry {
                path: c.path.clone(),
                digest: content_digest(&c.content),
            };
            (c, entry)
        })
        .collect()
}

/// Sparse-hydrate the fence into the running container's `workspace_root`.
///
/// Writes **only** the in-fence candidates (per [`select_in_fence`]) under
/// `workspace_root` on the box, then returns the manifest with its
/// `materialized` field populated by exactly those entries. Out-of-fence
/// candidates are dropped — guaranteeing a later access to such a path returns
/// ENOENT.
///
/// # Errors
/// - [`MaterializeError::DenyDefaultRequired`] if the manifest is not
///   default-deny.
/// - [`MaterializeError::AllowAllEntry`] if any `path_set` entry is a root-cover
///   allow-all (fail-closed).
/// - [`MaterializeError::Box`] if any box command fails.
pub fn materialize_sparse<B: BoxExec>(
    boxx: &B,
    container: &RunningContainer,
    workspace_root: &str,
    manifest: &FenceManifest,
    candidates: &[CandidateEntry],
) -> Result<FenceManifest, MaterializeError> {
    validate_manifest(manifest)?;

    let root = workspace_root.trim_end_matches('/');
    let selected = select_in_fence(manifest, candidates);

    let mut materialized = Vec::with_capacity(selected.len());
    for (cand, entry) in &selected {
        place_file(boxx, container, root, &cand.path, &cand.content)
            .map_err(MaterializeError::Box)?;
        materialized.push((*entry).clone());
    }

    Ok(FenceManifest {
        path_set: manifest.path_set.clone(),
        deny_default: manifest.deny_default,
        materialized,
    })
}

/// Place one in-fence file under `root` inside the container.
///
/// Creates the parent directory then writes the content via base64 so arbitrary
/// bytes survive the shell transport. The in-container path is computed
/// segment-safely from the (already in-fence, traversal-free) relative path.
fn place_file<B: BoxExec>(
    boxx: &B,
    container: &RunningContainer,
    root: &str,
    rel_path: &str,
    content: &[u8],
) -> Result<()> {
    // Defense-in-depth re-guard: an in-fence candidate must already be
    // traversal-free (the gate rejects any `..` / absolute path), but re-assert
    // here so a future caller that bypasses `select_in_fence` cannot place a
    // path that escapes the workspace root. Fail-closed.
    if path_escapes_root(rel_path) {
        bail!("refusing to place candidate with traversal/absolute path: {rel_path:?}");
    }
    let rel = rel_path.trim_start_matches("./").trim_start_matches('/');
    if rel.is_empty() {
        bail!("in-fence candidate has empty path");
    }
    let full = format!("{root}/{rel}");
    let dir = full.rsplit_once('/').map_or(root, |(d, _)| d).to_string();
    let b64 = base64_encode(content);
    let script = format!(
        "mkdir -p {dq} && printf %s {bq} | base64 -d > {fq}",
        dq = shell_quote(&dir),
        bq = shell_quote(&b64),
        fq = shell_quote(&full),
    );
    let out = boxx
        .run(&["docker", "exec", &container.name, "sh", "-c", &script])
        .with_context(|| format!("materializing {full} into {}", container.name))?;
    if !out.ok() {
        bail!("placing {full} failed: {}", out.stderr.trim());
    }
    Ok(())
}

/// `true` iff `path` escapes the workspace root — absolute, or containing any
/// `..` component. Uses the same normalization as the classifier (a path that
/// cannot be normalized is an escape). The materialize re-guard refuses such a
/// path even if it somehow reached `place_file`.
fn path_escapes_root(path: &str) -> bool {
    crate::enforce::normalize_segments_pub(path).is_none()
}

/// POSIX single-quote for safe interpolation into a remote `sh -c`.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Minimal, dependency-free base64 (standard alphabet, padded).
fn base64_encode(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(A[(n >> 18) & 63] as char);
        out.push(A[(n >> 12) & 63] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6) & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[n & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(paths: &[&str], deny_default: bool) -> FenceManifest {
        FenceManifest {
            path_set: paths.iter().map(|s| s.to_string()).collect(),
            deny_default,
            materialized: vec![],
        }
    }

    #[test]
    fn select_drops_out_of_fence() {
        let m = manifest(&["src/"], true);
        let cands = vec![
            CandidateEntry::new("src/main.rs", b"fn main(){}".to_vec()),
            CandidateEntry::new("secret.env", b"TOKEN=abc".to_vec()),
            CandidateEntry::new("src/inner/x.rs", b"// x".to_vec()),
        ];
        let sel = select_in_fence(&m, &cands);
        let paths: Vec<_> = sel.iter().map(|(c, _)| c.path.as_str()).collect();
        assert_eq!(paths, vec!["src/main.rs", "src/inner/x.rs"]);
        // The dropped secret is never represented in the materialized set.
        assert!(sel.iter().all(|(_, e)| e.path != "secret.env"));
    }

    #[test]
    fn digest_is_deterministic_and_content_sensitive() {
        assert_eq!(content_digest(b"abc"), content_digest(b"abc"));
        assert_ne!(content_digest(b"abc"), content_digest(b"abd"));
    }

    #[test]
    fn base64_roundtrip_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn validate_rejects_allow_all_entries_fail_closed() {
        for bad in ["./", ".", ""] {
            let m = manifest(&[bad], true);
            let err = validate_manifest(&m).unwrap_err();
            assert!(
                matches!(err, MaterializeError::AllowAllEntry { .. }),
                "entry {bad:?} must be rejected as allow-all, got {err:?}"
            );
        }
    }

    #[test]
    fn validate_rejects_default_allow() {
        let m = manifest(&["src/"], false);
        assert!(matches!(
            validate_manifest(&m).unwrap_err(),
            MaterializeError::DenyDefaultRequired
        ));
    }

    #[test]
    fn validate_accepts_a_real_fence() {
        let m = manifest(&["src/", "Cargo.toml"], true);
        assert!(validate_manifest(&m).is_ok());
    }

    #[test]
    fn allow_all_entry_detection() {
        assert!(entry_is_allow_all("./"));
        assert!(entry_is_allow_all("."));
        assert!(entry_is_allow_all(""));
        assert!(!entry_is_allow_all("src/"));
        assert!(!entry_is_allow_all("/")); // absolute → matches nothing, not allow-all
        assert!(!entry_is_allow_all("Cargo.toml"));
    }

    #[test]
    fn path_escapes_root_catches_traversal_and_absolute() {
        assert!(path_escapes_root("../x"));
        assert!(path_escapes_root("src/../../x"));
        assert!(path_escapes_root("/etc/passwd"));
        assert!(!path_escapes_root("src/main.rs"));
        assert!(!path_escapes_root("./src/main.rs"));
    }

    #[test]
    fn select_routes_through_named_gate() {
        // The materialize filter must agree with the enforcement gate exactly:
        // anything `is_admitted` denies is dropped, anything it admits is kept.
        let m = manifest(&["src/"], true);
        let cands = vec![
            CandidateEntry::new("src/a.rs", b"a".to_vec()),
            CandidateEntry::new("../escape", b"e".to_vec()),
            CandidateEntry::new("/abs", b"x".to_vec()),
            CandidateEntry::new("secret.env", b"s".to_vec()),
        ];
        let sel = select_in_fence(&m, &cands);
        let paths: Vec<_> = sel.iter().map(|(c, _)| c.path.as_str()).collect();
        assert_eq!(paths, vec!["src/a.rs"]);
    }
}
