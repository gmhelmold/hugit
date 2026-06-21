//! Lazy git-dir-backed [`ObjectSource`] for the `hugit symbol --ref` CLI path.
//!
//! The serve path (`hugit-serve/src/state.rs`) loads ALL reachable objects eagerly
//! into a `CasObjectSource` because it needs the full closure for `git clone/fetch`.
//! The CLI only needs the handful of tree + blob objects on the path from a root
//! tree to one file, so eagerly loading thousands of objects would be wasteful.
//!
//! [`GitCatFileSource`] is an [`ObjectSource`] that calls
//! `git -C <dir> cat-file <type> <oid>` on demand — one subprocess per object get.
//! For a depth-N tree walk this costs N+1 git subprocesses, which is negligible
//! in a CLI context (typical file depths are 1–4).
//!
//! Two entry points are exposed:
//! - [`open_git_dir`] — discover the `.git` dir by walking upward from cwd (mirrors
//!   `git rev-parse --show-toplevel`).
//! - [`resolve_ref_root_tree`] — resolve a ref name (or any revspec) to the root
//!   tree oid of the commit it points to.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gix_hash::ObjectId;
use hugit_proto::{GitObject, ObjectKind, ObjectSource, PackError};

/// Maximum blob size the CLI will buffer from `git cat-file`.
///
/// Matches the serve-path cap (`MAX_BLOB_BYTES` in `hugit-serve/src/blob.rs`).
/// An attacker who controls a ref (e.g. via a crafted repo) could otherwise
/// trigger an OOM by pointing `--ref` at a commit whose tree contains a
/// multi-gigabyte blob. Reads beyond this cap produce a [`PackError::Source`]
/// rather than buffering the whole blob.
pub const MAX_BLOB_BYTES: usize = 10 * 1024 * 1024; // 10 MiB

/// A lazy, git-subprocess-backed [`ObjectSource`].
///
/// Every `get` call shells out to `git cat-file <type> <oid>` to fetch the
/// raw object bytes. This is correct and safe for CLI use where only a handful
/// of objects are needed (the tree-walk to one blob). It must NOT be used in a
/// hot-loop / server context — use `CasObjectSource` with bulk loading there.
pub struct GitCatFileSource {
    /// The path to the git working tree (or bare git dir). Passed as `-C <git_dir>`.
    git_dir: PathBuf,
}

impl GitCatFileSource {
    /// Create a source anchored to `git_dir`.
    pub fn new(git_dir: PathBuf) -> Self {
        Self { git_dir }
    }
}

impl ObjectSource for GitCatFileSource {
    /// Fetch one object by oid.
    ///
    /// Shells out to `git -C <dir> cat-file <type> <oid>`. Returns:
    /// - `Ok(Some(obj))` — found.
    /// - `Ok(None)`      — the oid is absent (git reports `missing` or exits 128).
    /// - `Err(PackError::Source(_))` — a real I/O or parse error.
    fn get(&self, oid: &ObjectId) -> Result<Option<GitObject>, PackError> {
        let oid_hex = oid.to_hex().to_string();

        // First, get the type of the object.
        let type_out = Command::new("git")
            .arg("-C")
            .arg(&self.git_dir)
            .args(["cat-file", "-t", &oid_hex])
            .output()
            .map_err(|e| PackError::Source(format!("git cat-file -t {oid_hex}: {e}")))?;

        if !type_out.status.success() {
            // `git cat-file -t` exits non-zero for a missing object — Ok(None).
            return Ok(None);
        }

        let kind_str = std::str::from_utf8(&type_out.stdout)
            .map_err(|e| PackError::Source(format!("cat-file -t output not UTF-8: {e}")))?
            .trim();

        let kind = match kind_str {
            "blob" => ObjectKind::Blob,
            "tree" => ObjectKind::Tree,
            "commit" => ObjectKind::Commit,
            "tag" => ObjectKind::Tag,
            other => {
                return Err(PackError::Source(format!(
                    "unknown object kind from git: {other}"
                )));
            }
        };

        // Fetch the raw body bytes — capped at MAX_BLOB_BYTES to prevent OOM.
        //
        // We spawn with piped stdout and use `Read::take(MAX_BLOB_BYTES + 1)` so
        // we can detect an over-cap object (read == MAX_BLOB_BYTES + 1 means there
        // is at least one more byte) without buffering the whole blob.  Any object
        // larger than the cap is rejected with a clear error; this matches the
        // serve-path blob cap.
        let mut child = Command::new("git")
            .arg("-C")
            .arg(&self.git_dir)
            .args(["cat-file", kind_str, &oid_hex])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| PackError::Source(format!("git cat-file {kind_str} {oid_hex}: {e}")))?;

        let mut buf = Vec::new();
        if let Some(stdout) = child.stdout.take() {
            stdout
                .take((MAX_BLOB_BYTES as u64) + 1)
                .read_to_end(&mut buf)
                .map_err(|e| {
                    PackError::Source(format!("git cat-file read {oid_hex}: {e}"))
                })?;
        }
        // Wait for the child so it doesn't become a zombie.
        let status = child
            .wait()
            .map_err(|e| PackError::Source(format!("git cat-file wait {oid_hex}: {e}")))?;

        if !status.success() {
            // Object disappeared between the -t and the body fetch (very unlikely,
            // but handle it cleanly).
            return Ok(None);
        }
        if buf.len() > MAX_BLOB_BYTES {
            return Err(PackError::Source(format!(
                "object {oid_hex} exceeds MAX_BLOB_BYTES ({MAX_BLOB_BYTES} bytes) — refusing to buffer"
            )));
        }

        Ok(Some(GitObject::new(kind, buf)))
    }
}

/// Discover the git working-tree root by walking upward from `start` until a `.git`
/// entry (directory or file) is found. Returns the directory that CONTAINS `.git`
/// (the working-tree root), not the `.git` path itself.
///
/// Returns an error string if no `.git` is found from `start` to the filesystem root.
pub fn open_git_dir(start: &Path) -> Result<PathBuf, String> {
    let mut current = start.to_path_buf();
    loop {
        if current.join(".git").exists() {
            return Ok(current);
        }
        match current.parent() {
            Some(p) => current = p.to_path_buf(),
            None => {
                return Err(format!(
                    "not a git repository (or any parent up to the filesystem root): {}",
                    start.display()
                ));
            }
        }
    }
}

/// Resolve a git revspec (e.g. `HEAD`, `main`, `abc1234`) to the root TREE oid
/// of the commit it names, using `git -C <dir> rev-parse <ref>^{{tree}}`.
///
/// Returns the parsed [`ObjectId`] on success, or an error string if the ref
/// is absent, ambiguous, or the output cannot be parsed.
///
/// SECURITY: `git rev-parse` uses `--` to delimit revisions from pathspecs
/// (not to stop flag parsing the way most git sub-commands do) — so inserting
/// `--` before the revspec would print `--` as a literal line and break the
/// output parse. Instead, the refspec is validated up-front: any refspec that
/// begins with `-` is rejected before spawning any subprocess. In legitimate
/// usage a refspec is always a SHA-1 hex, a branch name, a tag, or a keyword
/// like `HEAD` / `FETCH_HEAD` — none of these start with `-`.
pub fn resolve_ref_root_tree(git_dir: &Path, refspec: &str) -> Result<ObjectId, String> {
    // SECURITY: reject any refspec that looks like a flag. A leading `-` is
    // never valid in a refspec; refusing it up-front prevents argument injection
    // even on git versions that do not honour `--` in this position.
    if refspec.starts_with('-') {
        return Err(format!(
            "invalid refspec `{refspec}`: revspecs must not start with `-`"
        ));
    }

    let tree_spec = format!("{refspec}^{{tree}}");
    let out = Command::new("git")
        .arg("-C")
        .arg(git_dir)
        .args(["rev-parse", &tree_spec])
        .output()
        .map_err(|e| format!("failed to spawn `git rev-parse`: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!(
            "ref `{refspec}` not found in git repo `{}`: {}",
            git_dir.display(),
            stderr.trim()
        ));
    }

    let hex = std::str::from_utf8(&out.stdout)
        .map_err(|e| format!("git rev-parse output is not UTF-8: {e}"))?
        .trim();

    ObjectId::from_hex(hex.as_bytes())
        .map_err(|e| format!("git rev-parse returned an invalid oid `{hex}`: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    /// `open_git_dir` from the crate root must resolve to the hugit repo root.
    #[test]
    fn open_git_dir_finds_repo_from_cwd() {
        let cwd = env::current_dir().expect("cwd");
        let root = open_git_dir(&cwd).expect("should find .git");
        assert!(
            root.join(".git").exists(),
            ".git must exist at {}",
            root.display()
        );
    }

    /// `open_git_dir` from a non-git temp dir returns an error.
    #[test]
    fn open_git_dir_fails_outside_repo() {
        // Use the filesystem root — guaranteed to have no .git.
        let root = Path::new("/");
        let result = open_git_dir(root);
        assert!(result.is_err(), "expected error outside a git repo");
    }

    /// Resolving HEAD yields a valid 40-hex oid string (basic smoke test).
    #[test]
    fn resolve_ref_root_tree_head_smoke() {
        let cwd = env::current_dir().expect("cwd");
        let root = open_git_dir(&cwd).expect("find repo");
        let oid = resolve_ref_root_tree(&root, "HEAD").expect("HEAD must exist");
        // A SHA-1 ObjectId is 20 bytes = 40 hex chars.
        assert_eq!(oid.to_hex().to_string().len(), 40);
    }

    /// `resolve_ref_root_tree` with a bogus ref returns an error, not a panic.
    #[test]
    fn resolve_ref_bad_ref_returns_error() {
        let cwd = env::current_dir().expect("cwd");
        let root = open_git_dir(&cwd).expect("find repo");
        let result = resolve_ref_root_tree(&root, "refs/heads/this-branch-does-not-exist-ever");
        assert!(result.is_err(), "expected error for unknown ref");
    }

    /// A refspec starting with `-` is rejected before spawning any subprocess
    /// (argument-injection guard).
    #[test]
    fn resolve_ref_leading_dash_is_rejected() {
        let cwd = env::current_dir().expect("cwd");
        let root = open_git_dir(&cwd).expect("find repo");
        for bad in ["--format=bad", "-exec", "--upload-pack=evil"] {
            let result = resolve_ref_root_tree(&root, bad);
            assert!(
                result.is_err(),
                "refspec starting with `-` must be rejected, got Ok for `{bad}`"
            );
            let err_msg = result.unwrap_err();
            assert!(
                err_msg.contains("must not start with `-`"),
                "error message should explain the rejection, got: {err_msg}"
            );
        }
    }

    /// `GitCatFileSource::get` can fetch a known object (the HEAD tree). This proves
    /// the subprocess path works end-to-end: resolve HEAD tree oid → get that tree
    /// object → it is `ObjectKind::Tree`.
    #[test]
    fn git_cat_file_source_fetches_head_tree() {
        let cwd = env::current_dir().expect("cwd");
        let root = open_git_dir(&cwd).expect("find repo");
        let oid = resolve_ref_root_tree(&root, "HEAD").expect("HEAD tree");
        let src = GitCatFileSource::new(root);
        let obj = src
            .get(&oid)
            .expect("no error")
            .expect("object must be present");
        assert_eq!(
            obj.kind,
            ObjectKind::Tree,
            "HEAD tree must be a Tree object"
        );
        assert!(!obj.data.is_empty(), "tree body must be non-empty");
    }

    /// A completely made-up oid returns `Ok(None)`, not an error or a panic.
    #[test]
    fn git_cat_file_source_missing_oid_returns_none() {
        let cwd = env::current_dir().expect("cwd");
        let root = open_git_dir(&cwd).expect("find repo");
        // Craft a well-formed but non-existent oid (all-zeros via from_hex).
        let zero_oid = ObjectId::from_hex(b"0000000000000000000000000000000000000001")
            .expect("valid zero oid");
        let src = GitCatFileSource::new(root);
        let result = src.get(&zero_oid).expect("no error for missing oid");
        assert!(result.is_none(), "non-existent oid must return None");
    }
}
