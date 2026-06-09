//! Shared fixtures for the WP-D2b owned acceptance suite (client matrix, jj
//! stacks, CPU/chunked fallback, scale ceilings, degradation kill-test).
//!
//! Included by `tests/acceptance_d2b.rs` (`#[path]`); the directory is the
//! contract-owned test home `crates/hugit-proto/tests/clients_jj_limits/`.
//!
//! Fixtures build a genuine git object graph in a content-addressed source so
//! every assertion rides real git oids and the real D2a serve path — never a
//! mock of pack assembly.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use gix_hash::ObjectId;
use hugit_proto::read::pack::{CasObjectSource, GitObject, ObjectKind};

/// A blob holding `content`.
pub fn blob(content: &[u8]) -> GitObject {
    GitObject::new(ObjectKind::Blob, content.to_vec())
}

/// A tree with one regular-file entry `name -> blob_oid` (mode 100644).
pub fn tree_one(name: &str, blob_oid: &ObjectId) -> GitObject {
    let mut body = Vec::new();
    body.extend_from_slice(b"100644 ");
    body.extend_from_slice(name.as_bytes());
    body.push(0);
    body.extend_from_slice(blob_oid.as_slice());
    GitObject::new(ObjectKind::Tree, body)
}

/// A commit pointing at `tree`, with optional `parent`. Deterministic identity
/// (fixed author/committer + timestamp) so the oids are stable across runs.
pub fn commit(tree: &ObjectId, parent: Option<&ObjectId>, message: &str) -> GitObject {
    let mut body = String::new();
    body.push_str(&format!("tree {tree}\n"));
    if let Some(p) = parent {
        body.push_str(&format!("parent {p}\n"));
    }
    let ident = "hugit <bot@hugit.dev> 1717000000 +0000";
    body.push_str(&format!("author {ident}\n"));
    body.push_str(&format!("committer {ident}\n"));
    body.push('\n');
    body.push_str(message);
    body.push('\n');
    GitObject::new(ObjectKind::Commit, body.into_bytes())
}

/// A repository fixture: a CAS of objects, its ref tips, and the per-commit oids
/// the edge tests assert against.
pub struct Fixture {
    /// The content-addressed object store standing in for the CoreLink CAS.
    pub cas: CasObjectSource,
    /// Ref name → tip oid (the D1 derived view).
    pub refs: BTreeMap<String, String>,
    /// Named commit oids for assertions (c1, c2, cf).
    pub tips: BTreeMap<String, ObjectId>,
}

/// Build the same deterministic two-branch repo D2a uses: `main` = c1→c2,
/// `feature` = c1→cf reusing c1's tree. Seven distinct objects in the closure.
pub fn build_repo() -> Fixture {
    let mut cas = CasObjectSource::new();

    let b1 = blob(b"hello hugit\n");
    let b1_oid = cas.insert(b1);
    let t1 = tree_one("README", &b1_oid);
    let t1_oid = cas.insert(t1);
    let c1 = commit(&t1_oid, None, "init");
    let c1_oid = cas.insert(c1);

    let b2 = blob(b"hello hugit, again\n");
    let b2_oid = cas.insert(b2);
    let t2 = tree_one("README", &b2_oid);
    let t2_oid = cas.insert(t2);
    let c2 = commit(&t2_oid, Some(&c1_oid), "update readme");
    let c2_oid = cas.insert(c2);

    let cf = commit(&t1_oid, Some(&c1_oid), "feature work");
    let cf_oid = cas.insert(cf);

    let mut refs = BTreeMap::new();
    refs.insert("refs/heads/main".to_string(), c2_oid.to_string());
    refs.insert("refs/heads/feature".to_string(), cf_oid.to_string());

    let mut tips = BTreeMap::new();
    tips.insert("c1".into(), c1_oid);
    tips.insert("c2".into(), c2_oid);
    tips.insert("cf".into(), cf_oid);

    Fixture { cas, refs, tips }
}

/// Build a deterministic linear chain of `n` commits (each a new blob+tree on a
/// single ref), returning the CAS, the ref map, and the tip-commit oids in
/// order. Used by the fallback and scale-ceiling tests that need a larger,
/// size-tunable closure.
pub fn build_chain(n: usize) -> (CasObjectSource, BTreeMap<String, String>, Vec<ObjectId>) {
    let mut cas = CasObjectSource::new();
    let mut tips = Vec::with_capacity(n);
    let mut parent: Option<ObjectId> = None;
    for i in 0..n {
        let b = blob(format!("content for commit {i}\n").as_bytes());
        let b_oid = cas.insert(b);
        let t = tree_one("FILE", &b_oid);
        let t_oid = cas.insert(t);
        let c = commit(&t_oid, parent.as_ref(), &format!("commit {i}"));
        let c_oid = cas.insert(c);
        tips.push(c_oid);
        parent = Some(c_oid);
    }
    let mut refs = BTreeMap::new();
    if let Some(tip) = tips.last() {
        refs.insert("refs/heads/main".to_string(), tip.to_string());
    }
    (cas, refs, tips)
}

// ─── DEFECT-8: REAL client round-trip (clone via real git, diff vs source) ────
//
// The client-matrix conformance must not compare the serve to itself. These
// helpers feed the ACTUAL assembled pack to a real client (`git`), clone it back,
// and return the object closure the client reconstructed — so the oracle can diff
// the cloned set against the served set on a genuine wire round-trip.

/// Whether a local binary is on PATH.
pub fn have_binary(name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Reconstruct, with REAL git, the object closure a client gets from `pack_bytes`
/// (the actual assembled serve pack) when the ref `ref_name` points at `tip`:
/// unpack the pack into a bare repo, set the ref, then `git clone` it and read the
/// cloned repo's full object set. Returns the cloned object oids — what a real
/// client actually reconstructed off the wire, for diffing against the served set.
///
/// Panics (FAIL-not-skip) if `git` is missing — `git` is a contracted client.
/// Real **libgit2** clone of the served pack — proves item ③'s libgit2 client
/// for real (a genuine `git2`/libgit2 round-trip, not construction-equivalence).
/// Builds the same bare server as [`clone_object_set_via_git`], then clones it
/// with libgit2 and enumerates the cloned object closure via the object database.
/// Returns the set of object names, to be compared byte-for-byte with the git
/// client's set and the served closure.
pub fn clone_object_set_via_libgit2(
    pack_bytes: &[u8],
    ref_name: &str,
    tip: &ObjectId,
) -> BTreeSet<String> {
    let server = ScratchDir::new("hugit-d2b-srv-lg2");
    git(server.path(), &["init", "-q", "--bare", "."]);
    // Same genuine V2 pack fed to real git, so libgit2 clones the ACTUAL served bytes.
    git_stdin(server.path(), &["unpack-objects", "-q"], pack_bytes);
    git(server.path(), &["update-ref", ref_name, &tip.to_string()]);
    git(server.path(), &["symbolic-ref", "HEAD", ref_name]);

    let dst = ScratchDir::new("hugit-d2b-clone-lg2");
    let work = dst.path().join("work");
    let server_url = server.path().to_str().expect("utf8 server path");
    // Real libgit2 clone (the C library via the git2 crate).
    let repo = git2::Repository::clone(server_url, &work)
        .expect("real libgit2 clone of the served pack failed");

    // Enumerate the cloned repo's full object closure via libgit2's odb.
    let odb = repo.odb().expect("open cloned odb");
    let mut set = BTreeSet::new();
    odb.foreach(|oid| {
        set.insert(oid.to_string());
        true
    })
    .expect("iterate cloned odb");
    set
}

pub fn clone_object_set_via_git(
    pack_bytes: &[u8],
    ref_name: &str,
    tip: &ObjectId,
) -> BTreeSet<String> {
    assert!(
        have_binary("git"),
        "git is a CONTRACTED client (item ③); refusing to skip — install git"
    );
    let server = ScratchDir::new("hugit-d2b-srv");
    git(server.path(), &["init", "-q", "--bare", "."]);
    // Feed the ACTUAL served pack to real git (unpack-objects accepts a real V2
    // pack on stdin), proving the assembled pack is genuine git on the wire.
    git_stdin(server.path(), &["unpack-objects", "-q"], pack_bytes);
    git(server.path(), &["update-ref", ref_name, &tip.to_string()]);
    git(server.path(), &["symbolic-ref", "HEAD", ref_name]);

    // Real client clone.
    let dst = ScratchDir::new("hugit-d2b-clone");
    let work = dst.path().join("work");
    let status = Command::new("git")
        .arg("clone")
        .arg("-q")
        .arg(server.path())
        .arg(&work)
        .status()
        .expect("spawn git clone");
    assert!(status.success(), "real git clone of the served pack failed");

    // Read the cloned repo's full object set.
    let listing = git(
        &work,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname)",
        ],
    );
    listing.split_whitespace().map(|s| s.to_string()).collect()
}

fn git(cwd: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("spawn git {args:?}: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn git_stdin(cwd: &Path, args: &[&str], stdin: &[u8]) {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("spawn git {args:?}: {e}"));
    child
        .stdin
        .take()
        .expect("git stdin")
        .write_all(stdin)
        .expect("write git stdin");
    let out = child.wait_with_output().expect("git wait");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// A unique self-cleaning scratch directory.
struct ScratchDir {
    path: PathBuf,
}

impl ScratchDir {
    fn new(prefix: &str) -> Self {
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path =
            std::env::temp_dir().join(format!("{prefix}-{}-{n}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).expect("create scratch dir");
        ScratchDir { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}
