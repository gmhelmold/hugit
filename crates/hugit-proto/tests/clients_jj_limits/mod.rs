//! Shared fixtures for the WP-D2b owned acceptance suite (client matrix, jj
//! stacks, CPU/chunked fallback, scale ceilings, degradation kill-test).
//!
//! Included by `tests/acceptance_d2b.rs` (`#[path]`); the directory is the
//! contract-owned test home `crates/hugit-proto/tests/clients_jj_limits/`.
//!
//! Fixtures build a genuine git object graph in a content-addressed source so
//! every assertion rides real git oids and the real D2a serve path — never a
//! mock of pack assembly.

use std::collections::BTreeMap;

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
