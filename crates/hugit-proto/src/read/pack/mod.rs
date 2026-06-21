//! Pack assembly from the CoreLink CAS.
//!
//! Objects live in the **content-addressed store** (whitepaper §4: "all objects
//! are content-addressed, immutable, and stored in the CoreLink CAS"). This
//! module fetches the negotiated objects from that store and encodes them into a
//! real git packfile using a libgit2-class library (`gix-pack`) — the git pack
//! format is NOT reimplemented here, only wired to CAS-backed object access.
//!
//! # Content addressing is the byte-identity guarantee
//!
//! A git object id is the SHA-1 over `"<kind> <len>\0<body>"`. Because the
//! [`ObjectSource`] is keyed by that id and every stored object is verified
//! against its id on insert, the bytes served for a given oid are *the* bytes
//! git itself would store for that oid. A clone is therefore byte-identical to
//! the GitHub mirror, which holds the same content-addressed objects (D2 item ①).

use std::collections::BTreeMap;

use gix_hash::{Kind as HashKind, ObjectId};
use gix_object::{Data, Kind as GixKind};
use gix_pack::data::{
    Version,
    output::{Count, Entry, bytes::FromEntriesIter},
};

/// The kind of a git object — the four base object types of git's model
/// (whitepaper §4: "git's objects, byte-exact — the projection").
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    /// A blob (file content).
    Blob,
    /// A tree (directory listing).
    Tree,
    /// A commit.
    Commit,
    /// An annotated tag.
    Tag,
}

impl ObjectKind {
    fn to_gix(self) -> GixKind {
        match self {
            ObjectKind::Blob => GixKind::Blob,
            ObjectKind::Tree => GixKind::Tree,
            ObjectKind::Commit => GixKind::Commit,
            ObjectKind::Tag => GixKind::Tag,
        }
    }
}

/// A single git object as held in the CAS: its kind plus its raw, uncompressed
/// body (NOT including the `"<kind> <len>\0"` loose header — that header is part
/// of the hash pre-image, computed on demand).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitObject {
    /// The object's kind.
    pub kind: ObjectKind,
    /// The object's raw body bytes (canonical git encoding for the kind).
    pub data: Vec<u8>,
}

impl GitObject {
    /// Construct an object from its kind and body.
    pub fn new(kind: ObjectKind, data: impl Into<Vec<u8>>) -> Self {
        Self {
            kind,
            data: data.into(),
        }
    }

    /// The git object id (SHA-1 over the loose-object pre-image) for this object,
    /// propagating any hashing failure as a [`PackError`].
    ///
    /// Delegates to the git library's canonical hash so the id matches git
    /// byte-for-byte; never reimplemented here. The internal CAS paths (insert /
    /// content-address check / pack assembly) use this fallible form so a hashing
    /// failure surfaces as a real error rather than a panic.
    pub fn try_oid(&self) -> Result<ObjectId, PackError> {
        gix_object::compute_hash(HashKind::Sha1, self.kind.to_gix(), &self.data)
            .map_err(|e| PackError::Hash(e.to_string()))
    }

    /// The git object id, panicking only on the (in practice unreachable) hashing
    /// failure. Convenience for callers that already hold a well-formed object;
    /// prefer [`GitObject::try_oid`] on any path that can return a [`PackError`].
    pub fn oid(&self) -> ObjectId {
        self.try_oid()
            .expect("sha1 hashing of an in-memory object is infallible")
    }
}

/// Errors raised while reading objects from a source or assembling a pack.
#[derive(Debug, thiserror::Error)]
pub enum PackError {
    /// A requested object id was not present in the CAS-backed source.
    #[error("object {0} not found in CAS")]
    MissingObject(ObjectId),
    /// An object id string could not be parsed as a git oid.
    #[error("invalid object id: {0}")]
    InvalidOid(String),
    /// An object stored under an id whose bytes do not hash to that id — a
    /// content-addressing violation. Fail-closed: a CAS that hands back
    /// mislabeled bytes can never be trusted to serve a byte-identical clone.
    #[error("content-address mismatch: stored under {stored}, hashes to {actual}")]
    AddressMismatch {
        /// The id the object was filed under.
        stored: ObjectId,
        /// The id its bytes actually hash to.
        actual: ObjectId,
    },
    /// Computing an object's content address (SHA-1) failed.
    #[error("object hashing failed: {0}")]
    Hash(String),
    /// The git library failed to encode an object into a pack entry.
    #[error("pack entry encoding failed: {0}")]
    Encode(String),
    /// The git library failed while writing pack bytes.
    #[error("pack write failed: {0}")]
    Write(String),
    /// The backing object source (e.g. a remote CAS) failed to fetch or decode an
    /// object it should hold. Distinct from [`PackError::MissingObject`] (a clean
    /// absence): this is a transport/decode error or a fail-closed "indexed object
    /// is gone" — a lazy source surfaces it here so a broken seam never serves
    /// partial/empty bytes into a clone.
    #[error("object source error: {0}")]
    Source(String),
}

/// The CAS object-get surface the read path consumes.
///
/// This is the frozen CoreLink CAS contract as seen by the projection layer:
/// content-addressed `get(oid) -> object`. The production implementation is a
/// CoreLink tenant (chunked SplitBlob/SpliceBlob Merkle manifests, whitepaper
/// §5); the read path only needs the get-by-oid surface, so it depends on this
/// trait, never on a concrete CAS client.
pub trait ObjectSource {
    /// Fetch an object by its git oid. `Ok(None)` if absent.
    fn get(&self, oid: &ObjectId) -> Result<Option<GitObject>, PackError>;

    /// Whether the object is present.
    fn contains(&self, oid: &ObjectId) -> bool {
        matches!(self.get(oid), Ok(Some(_)))
    }
}

/// An in-process, content-addressed [`ObjectSource`].
///
/// Models the CoreLink CAS for the projection layer: objects are keyed by their
/// git oid and **verified against that oid on insert** — exactly the invariant a
/// content-addressed store enforces. This is what makes a served clone provably
/// byte-identical to the mirror: both stores key the same bytes by the same id.
#[derive(Debug, Clone, Default)]
pub struct CasObjectSource {
    objects: BTreeMap<ObjectId, GitObject>,
}

impl CasObjectSource {
    /// An empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert an object, computing its content address. Returns the oid it was
    /// filed under.
    pub fn insert(&mut self, object: GitObject) -> ObjectId {
        let oid = object.oid();
        self.objects.insert(oid, object);
        oid
    }

    /// Insert raw bytes already known to be a git object of `kind`.
    pub fn insert_raw(&mut self, kind: ObjectKind, data: impl Into<Vec<u8>>) -> ObjectId {
        self.insert(GitObject::new(kind, data))
    }

    /// Number of distinct objects held.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether the store is empty.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

impl ObjectSource for CasObjectSource {
    fn get(&self, oid: &ObjectId) -> Result<Option<GitObject>, PackError> {
        match self.objects.get(oid) {
            None => Ok(None),
            Some(obj) => {
                // Fail-closed content-addressing check: the bytes filed under
                // `oid` must hash to `oid`. A mismatch means the store is
                // corrupt; never serve mislabeled bytes into a clone. A hashing
                // failure propagates as a PackError rather than panicking.
                let actual = obj.try_oid()?;
                if &actual != oid {
                    return Err(PackError::AddressMismatch {
                        stored: *oid,
                        actual,
                    });
                }
                Ok(Some(obj.clone()))
            }
        }
    }

    fn contains(&self, oid: &ObjectId) -> bool {
        self.objects.contains_key(oid)
    }
}

/// The result of assembling a packfile: the wire bytes plus the metadata a
/// negotiation needs to report and a delta-fetch proof needs to assert against.
#[derive(Debug, Clone)]
pub struct PackAssembly {
    /// The complete packfile bytes (`PACK` header, V2 entries, trailing SHA-1).
    pub bytes: Vec<u8>,
    /// The object ids included, in the order they were packed.
    pub object_ids: Vec<ObjectId>,
}

impl PackAssembly {
    /// The number of objects in the pack (also encoded in the pack header).
    pub fn object_count(&self) -> usize {
        self.object_ids.len()
    }
}

/// Assemble a git packfile containing exactly `oids`, fetching each object from
/// the CAS-backed `source`.
///
/// Objects are emitted as base (non-delta) entries — correct and byte-identical
/// for any client; delta compression at scale is D2b, not this core. The pack is
/// a real V2 packfile written by `gix-pack`: header, zlib-deflated entries, and
/// a trailing SHA-1 over the stream. `oids` is packed in the given order; the
/// caller (serve) supplies a deterministic order.
pub fn assemble_pack(
    source: &dyn ObjectSource,
    oids: &[ObjectId],
) -> Result<PackAssembly, PackError> {
    // Build a pack-output Entry for each requested object, fetched from CAS.
    let mut entries: Vec<Entry> = Vec::with_capacity(oids.len());
    for oid in oids {
        let obj = source.get(oid)?.ok_or(PackError::MissingObject(*oid))?;
        let count = Count::from_data(*oid, None);
        let data = Data::new(obj.kind.to_gix(), &obj.data);
        let entry =
            Entry::from_data(&count, &data).map_err(|e| PackError::Encode(e.to_string()))?;
        entries.push(entry);
    }

    // `FromEntriesIter` writes the V2 pack header, every entry, and the trailing
    // hash. It pulls chunks of entries from the input iterator; one chunk holds
    // all entries (sorted by the caller's deterministic order).
    let num_entries = u32::try_from(entries.len())
        .map_err(|_| PackError::Write("too many objects for a single pack".into()))?;
    let input = std::iter::once(Ok::<_, std::convert::Infallible>(entries));
    let mut writer = FromEntriesIter::new(
        input,
        Vec::<u8>::new(),
        num_entries,
        Version::V2,
        HashKind::Sha1,
    );
    // Drive the iterator to completion; each `next()` returns bytes-written or an
    // error. The final `None`-driven step appends the trailing checksum.
    for step in writer.by_ref() {
        step.map_err(|e| PackError::Write(e.to_string()))?;
    }
    let bytes = writer.into_write();

    Ok(PackAssembly {
        bytes,
        object_ids: oids.to_vec(),
    })
}

/// Parse a 40-char hex git oid.
pub fn parse_oid(hex: &str) -> Result<ObjectId, PackError> {
    ObjectId::from_hex(hex.as_bytes()).map_err(|_| PackError::InvalidOid(hex.to_string()))
}

/// One entry returned by [`list_tree_at_dir`]: the name and kind of a single
/// node in the containing directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    /// The entry's filename (not a full path — just the leaf name).
    pub name: String,
    /// `true` if this entry is a subtree (directory); `false` if it is a blob
    /// (regular file or executable). Symlinks and gitlinks are excluded.
    pub is_dir: bool,
}

/// List the direct children of the directory that *contains* `path`.
///
/// For `path = "src/lib.rs"` this lists the entries of the `src/` directory.
/// For `path = "README.md"` (a root-level file) this lists the root tree.
///
/// Only regular blobs (mode 100644/100755) and subtrees (mode 40000) are
/// returned — symlinks (mode 120000) and gitlinks (mode 160000) are **excluded**
/// (symlinks expose filesystem paths and bypass secret-scrub; gitlinks are
/// submodule pointers, not useful as sidebar file-tree nodes).
///
/// Fail-closed: any missing object, malformed tree, or traversal-unsafe path
/// segment yields an empty `Vec` rather than an error — the caller renders a
/// sidebar with no entries, never a broken response.
pub fn list_tree_at_dir(
    src: &dyn ObjectSource,
    root_tree: &ObjectId,
    path: &str,
) -> Vec<TreeEntry> {
    list_tree_at_dir_inner(src, root_tree, path).unwrap_or_default()
}

fn list_tree_at_dir_inner(
    src: &dyn ObjectSource,
    root_tree: &ObjectId,
    path: &str,
) -> Option<Vec<TreeEntry>> {
    // Compute the *parent directory* of `path`.  For "src/lib.rs" the parent is
    // "src"; for "README.md" (no '/') the parent is the root tree itself.
    let parent_segments: Vec<&str> = {
        let segments: Vec<&str> = path.split('/').collect();
        // SECURITY: reject over-deep paths before doing any I/O — each segment
        // costs one CAS/disk get, so an unbounded path is a per-request DoS.
        if segments.len() > MAX_PATH_DEPTH {
            return None;
        }
        // All segments except the final (the file leaf).  May be empty for
        // root-level files.
        segments[..segments.len().saturating_sub(1)].to_vec()
    };

    // Walk from root_tree through each parent segment.
    let mut current_tree = *root_tree;
    for segment in &parent_segments {
        // Reject traversal-unsafe segments (same defence-in-depth as
        // `resolve_blob_at_path`).
        if segment.is_empty() || *segment == "." || *segment == ".." {
            return None;
        }

        let obj = src.get(&current_tree).ok()??;
        if obj.kind != ObjectKind::Tree {
            return None;
        }

        let entries = gix_object::TreeRefIter::from_bytes(&obj.data)
            .entries()
            .ok()?;
        let entry = entries
            .into_iter()
            .find(|e| e.filename == segment.as_bytes())?;

        // The intermediate must be a tree; fail-closed on any other kind.
        if !entry.mode.is_tree() {
            return None;
        }
        current_tree = entry.oid.to_owned();
    }

    // Now `current_tree` is the OID of the directory we want to list.
    let obj = src.get(&current_tree).ok()??;
    if obj.kind != ObjectKind::Tree {
        return None;
    }

    let raw_entries = gix_object::TreeRefIter::from_bytes(&obj.data)
        .entries()
        .ok()?;

    // SECURITY: cap the result to MAX_TREE_ENTRIES. A git tree in a pathological
    // repo could contain millions of entries; the sidebar only needs a bounded
    // listing, and an unbounded collect is an OOM/DoS vector.
    let mut result = Vec::with_capacity(raw_entries.len().min(MAX_TREE_ENTRIES));
    for e in raw_entries {
        if result.len() >= MAX_TREE_ENTRIES {
            break;
        }
        let mode = e.mode;
        // Exclude symlinks (is_link) and gitlinks/commits (is_commit). Only
        // blobs and subtrees are useful sidebar entries.
        if mode.is_link() || mode.is_commit() {
            continue;
        }
        let name = String::from_utf8_lossy(e.filename).into_owned();
        let is_dir = mode.is_tree();
        result.push(TreeEntry { name, is_dir });
    }

    // Return entries in the order the tree stores them (name-sorted, git
    // canonical).  The caller may re-sort if needed; we stay faithful to the
    // git object.
    Some(result)
}

/// Maximum number of path segments (`/`-separated) accepted by
/// [`resolve_blob_at_path`] and [`list_tree_at_dir`].
///
/// A depth-N walk issues N+1 CAS gets. Limiting depth bounds the per-request
/// CAS budget and prevents an attacker from chaining thousands of sub-tree
/// fetches via a crafted path (path-depth DoS). Real file trees are rarely
/// deeper than ~15 levels; 64 is generous and still safe.
pub const MAX_PATH_DEPTH: usize = 64;

/// Maximum number of entries returned by [`list_tree_at_dir`].
///
/// A git tree in a pathological repo could contain millions of entries.
/// The sidebar only needs a bounded listing; stopping at 2 000 entries is
/// safe for the UI and prevents an OOM from a large tree. Entries beyond the
/// cap are silently dropped (the sidebar is informational, not authoritative).
pub const MAX_TREE_ENTRIES: usize = 2_000;

/// Walk a git tree to resolve a repo-relative path to its blob (oid + raw bytes).
///
/// Splits `path` on '/', descends subtree-by-subtree from `root_tree`, and on the
/// final segment returns the blob's (ObjectId, bytes). Returns Ok(None) if any
/// segment is absent, an intermediate segment is not a tree, or the final entry
/// is not a blob.
///
/// SECURITY: empty segments, "." and ".." are REJECTED (return Ok(None)) — the
/// walk can never escape the tree. (Git trees can't contain these names anyway,
/// but reject explicitly as defence-in-depth.)
///
/// SECURITY: paths deeper than [`MAX_PATH_DEPTH`] segments are rejected
/// (return Ok(None)) — each segment requires a CAS fetch, so an unbounded
/// path depth is a per-request CAS-budget DoS.
///
/// The tree object parse reuses `gix_object::TreeRefIter` — the same libgit2-class
/// decoder the closure walk in [`crate::read::serve`] uses — so the byte-level git
/// tree format is never reimplemented here, only walked.
pub fn resolve_blob_at_path(
    src: &dyn ObjectSource,
    root_tree: &ObjectId,
    path: &str,
) -> Result<Option<(ObjectId, Vec<u8>)>, PackError> {
    let segments: Vec<&str> = path.split('/').collect();
    // SECURITY: reject paths with too many segments before doing any CAS I/O.
    if segments.len() > MAX_PATH_DEPTH {
        return Ok(None);
    }
    // An empty `path` splits to a single empty segment; the loop's reject below
    // catches it. Defence-in-depth: reject any traversal-unsafe segment up front.
    let last = segments.len().saturating_sub(1);

    let mut current_tree = *root_tree;
    for (idx, segment) in segments.iter().enumerate() {
        // SECURITY: never walk through an empty / "." / ".." segment. A git tree
        // cannot legally contain these names, but reject explicitly so a crafted
        // or corrupt tree can never be coaxed into escaping the root.
        if segment.is_empty() || *segment == "." || *segment == ".." {
            return Ok(None);
        }

        // Load + confirm the current object is a tree. (A non-tree intermediate
        // means the path descends through a file — no such entry.)
        let object = match src.get(&current_tree)? {
            Some(o) => o,
            None => return Ok(None),
        };
        if object.kind != ObjectKind::Tree {
            return Ok(None);
        }

        // Parse the tree's entries with the canonical decoder and find this
        // segment by exact filename match (git tree names are raw bytes).
        let entries = gix_object::TreeRefIter::from_bytes(&object.data)
            .entries()
            .map_err(|e| PackError::InvalidOid(format!("malformed tree {current_tree}: {e}")))?;
        let entry = entries
            .into_iter()
            .find(|e| e.filename == segment.as_bytes());
        let entry = match entry {
            Some(e) => e,
            None => return Ok(None),
        };
        let entry_oid = entry.oid.to_owned();

        if idx == last {
            // Final segment: it must be a regular or executable blob (git modes
            // 100644 / 100755). Symlinks (120000) are NOT served — they expose
            // filesystem paths and bypass secret-scrub (DoS/info-leak fix). A
            // tree (40000) or gitlink (160000) here is not a file at this path.
            if !entry.mode.is_blob() {
                return Ok(None);
            }
            let blob = match src.get(&entry_oid)? {
                Some(o) => o,
                None => return Ok(None),
            };
            if blob.kind != ObjectKind::Blob {
                return Ok(None);
            }
            return Ok(Some((entry_oid, blob.data)));
        }

        // Intermediate segment: it must be a tree to descend into.
        if !entry.mode.is_tree() {
            return Ok(None);
        }
        current_tree = entry_oid;
    }

    // Unreachable for any non-empty `path`: the final segment always returns. An
    // empty `path` is rejected by the empty-segment guard on the first iteration.
    Ok(None)
}

#[cfg(test)]
mod resolve_tests {
    use super::*;

    /// Git tree mode for a regular file blob.
    const MODE_BLOB: &str = "100644";
    /// Git tree mode for an executable blob.
    const MODE_EXE: &str = "100755";
    /// Git tree mode for a symlink blob.
    const MODE_LINK: &str = "120000";
    /// Git tree mode for a subtree.
    const MODE_TREE: &str = "40000";

    /// One entry to encode into a tree object.
    struct TreeEntry<'a> {
        mode: &'a str,
        name: &'a str,
        oid: ObjectId,
    }

    /// Build the raw bytes of a git tree object in the on-the-wire format:
    /// a concatenation of `<ascii-octal-mode> <name>\0<20-byte-binary-oid>` with
    /// NO separators. Entries are sorted by name, as git canonically requires, so
    /// the bytes round-trip through `gix_object::TreeRefIter`.
    fn build_tree_bytes(mut entries: Vec<TreeEntry<'_>>) -> Vec<u8> {
        entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        let mut out = Vec::new();
        for e in &entries {
            out.extend_from_slice(e.mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(e.name.as_bytes());
            out.push(0);
            out.extend_from_slice(e.oid.as_bytes());
        }
        out
    }

    /// Build + insert a tree object; return its oid.
    fn insert_tree(src: &mut CasObjectSource, entries: Vec<TreeEntry<'_>>) -> ObjectId {
        src.insert_raw(ObjectKind::Tree, build_tree_bytes(entries))
    }

    #[test]
    fn round_trip_tree_bytes_parse_back() {
        // Prove the hand-built wire bytes decode into the entries we put in.
        let mut src = CasObjectSource::new();
        let blob_a = src.insert_raw(ObjectKind::Blob, b"alpha".to_vec());
        let blob_b = src.insert_raw(ObjectKind::Blob, b"beta".to_vec());
        let raw = build_tree_bytes(vec![
            TreeEntry {
                mode: MODE_BLOB,
                name: "b.txt",
                oid: blob_b,
            },
            TreeEntry {
                mode: MODE_BLOB,
                name: "a.txt",
                oid: blob_a,
            },
        ]);
        let entries = gix_object::TreeRefIter::from_bytes(&raw)
            .entries()
            .expect("hand-built tree bytes must decode");
        assert_eq!(entries.len(), 2);
        // Sorted by name on build: a.txt then b.txt.
        assert_eq!(entries[0].filename, "a.txt".as_bytes());
        assert_eq!(entries[0].oid.to_owned(), blob_a);
        assert!(entries[0].mode.is_blob());
        assert_eq!(entries[1].filename, "b.txt".as_bytes());
        assert_eq!(entries[1].oid.to_owned(), blob_b);
    }

    #[test]
    fn resolves_top_level_file() {
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"hello world".to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "README.md",
                oid: blob,
            }],
        );

        let got = resolve_blob_at_path(&src, &root, "README.md").unwrap();
        let (oid, bytes) = got.expect("top-level file resolves");
        assert_eq!(oid, blob);
        assert_eq!(bytes, b"hello world");
    }

    #[test]
    fn resolves_nested_file() {
        // a/b/c.txt — two levels of subtree.
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"deep content".to_vec());
        let tree_b = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "c.txt",
                oid: blob,
            }],
        );
        let tree_a = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_TREE,
                name: "b",
                oid: tree_b,
            }],
        );
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_TREE,
                name: "a",
                oid: tree_a,
            }],
        );

        let (oid, bytes) = resolve_blob_at_path(&src, &root, "a/b/c.txt")
            .unwrap()
            .expect("nested file resolves");
        assert_eq!(oid, blob);
        assert_eq!(bytes, b"deep content");
    }

    #[test]
    fn resolves_executable_blob() {
        // 100755 is a regular executable blob — must resolve normally.
        let mut src = CasObjectSource::new();
        let exe = src.insert_raw(ObjectKind::Blob, b"#!/bin/sh\n".to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_EXE,
                name: "run.sh",
                oid: exe,
            }],
        );

        let (oid_e, _) = resolve_blob_at_path(&src, &root, "run.sh")
            .unwrap()
            .unwrap();
        assert_eq!(oid_e, exe);
    }

    #[test]
    fn symlink_entry_is_not_served() {
        // Mode 120000 (symlink) must NOT be served — it leaks fs paths and
        // bypasses secret scrub. resolve_blob_at_path must return Ok(None).
        let mut src = CasObjectSource::new();
        let link = src.insert_raw(ObjectKind::Blob, b"target/path".to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_LINK,
                name: "ln",
                oid: link,
            }],
        );

        assert!(
            resolve_blob_at_path(&src, &root, "ln").unwrap().is_none(),
            "a symlink tree entry must resolve to None, not its target bytes"
        );
    }

    #[test]
    fn missing_path_is_none() {
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"x".to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "present.txt",
                oid: blob,
            }],
        );

        assert!(
            resolve_blob_at_path(&src, &root, "absent.txt")
                .unwrap()
                .is_none()
        );
        assert!(
            resolve_blob_at_path(&src, &root, "no/such/path.txt")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn path_through_a_non_tree_is_none() {
        // present.txt is a file; descending "into" it must be None, not an error.
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"x".to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "present.txt",
                oid: blob,
            }],
        );

        assert!(
            resolve_blob_at_path(&src, &root, "present.txt/inner")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn final_entry_being_a_tree_is_none() {
        // A directory at the final segment is not a blob.
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"x".to_vec());
        let sub = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "f.txt",
                oid: blob,
            }],
        );
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_TREE,
                name: "dir",
                oid: sub,
            }],
        );

        assert!(resolve_blob_at_path(&src, &root, "dir").unwrap().is_none());
        // But the file inside it still resolves.
        assert!(
            resolve_blob_at_path(&src, &root, "dir/f.txt")
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn rejects_traversal_segments() {
        // SECURITY: empty / "." / ".." segments are rejected → Ok(None), never an
        // escape and never an error.
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"x".to_vec());
        let sub = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "b",
                oid: blob,
            }],
        );
        let root = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_TREE,
                    name: "a",
                    oid: sub,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "x",
                    oid: blob,
                },
            ],
        );

        for bad in ["../x", "a/../b", "./x", "", "a//b"] {
            assert!(
                resolve_blob_at_path(&src, &root, bad).unwrap().is_none(),
                "traversal-unsafe path {bad:?} must resolve to None"
            );
        }
    }

    #[test]
    fn missing_root_tree_is_none() {
        // A root oid absent from the store → Ok(None), not an error.
        let src = CasObjectSource::new();
        let fake = parse_oid("0000000000000000000000000000000000000001").unwrap();
        assert!(
            resolve_blob_at_path(&src, &fake, "anything")
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(test)]
mod list_tree_tests {
    use super::*;

    const MODE_BLOB: &str = "100644";
    const MODE_EXE: &str = "100755";
    const MODE_LINK: &str = "120000";
    const MODE_TREE: &str = "40000";

    struct TreeEntry<'a> {
        mode: &'a str,
        name: &'a str,
        oid: ObjectId,
    }

    fn build_tree_bytes(mut entries: Vec<TreeEntry<'_>>) -> Vec<u8> {
        entries.sort_by(|a, b| a.name.as_bytes().cmp(b.name.as_bytes()));
        let mut out = Vec::new();
        for e in &entries {
            out.extend_from_slice(e.mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(e.name.as_bytes());
            out.push(0);
            out.extend_from_slice(e.oid.as_bytes());
        }
        out
    }

    fn insert_tree(src: &mut CasObjectSource, entries: Vec<TreeEntry<'_>>) -> ObjectId {
        src.insert_raw(ObjectKind::Tree, build_tree_bytes(entries))
    }

    /// A root-level file → list the root tree's entries.
    #[test]
    fn list_root_level_file_returns_root_entries() {
        let mut src = CasObjectSource::new();
        let a = src.insert_raw(ObjectKind::Blob, b"a".to_vec());
        let b = src.insert_raw(ObjectKind::Blob, b"b".to_vec());
        let sub = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "inner.rs",
                oid: a,
            }],
        );
        let root = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "lib.rs",
                    oid: a,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "main.rs",
                    oid: b,
                },
                TreeEntry {
                    mode: MODE_TREE,
                    name: "src",
                    oid: sub,
                },
            ],
        );

        let entries = list_tree_at_dir(&src, &root, "lib.rs");
        assert_eq!(entries.len(), 3, "entries: {entries:?}");

        let lib = entries
            .iter()
            .find(|e| e.name == "lib.rs")
            .expect("lib.rs present");
        assert!(!lib.is_dir);
        let main = entries
            .iter()
            .find(|e| e.name == "main.rs")
            .expect("main.rs present");
        assert!(!main.is_dir);
        let src_entry = entries
            .iter()
            .find(|e| e.name == "src")
            .expect("src present");
        assert!(src_entry.is_dir);
    }

    /// A nested file → list its parent directory's entries.
    #[test]
    fn list_nested_file_returns_parent_entries() {
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"x".to_vec());
        let sub = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "foo.rs",
                    oid: blob,
                },
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "bar.rs",
                    oid: blob,
                },
            ],
        );
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_TREE,
                name: "crate",
                oid: sub,
            }],
        );

        let entries = list_tree_at_dir(&src, &root, "crate/foo.rs");
        assert_eq!(entries.len(), 2, "entries: {entries:?}");
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"foo.rs"), "foo.rs in {names:?}");
        assert!(names.contains(&"bar.rs"), "bar.rs in {names:?}");
    }

    /// Symlinks and gitlinks are excluded; regular blobs + executable blobs pass.
    #[test]
    fn symlinks_and_gitlinks_excluded() {
        let mut src = CasObjectSource::new();
        let real = src.insert_raw(ObjectKind::Blob, b"real".to_vec());
        let link_blob = src.insert_raw(ObjectKind::Blob, b"target".to_vec());
        // A gitlink (submodule) uses a Commit oid — use a dummy blob here since
        // mode alone determines gitlink exclusion (mode 160000).
        let commit_dummy = src.insert_raw(ObjectKind::Blob, b"".to_vec());
        let root = insert_tree(
            &mut src,
            vec![
                TreeEntry {
                    mode: MODE_BLOB,
                    name: "real.rs",
                    oid: real,
                },
                TreeEntry {
                    mode: MODE_EXE,
                    name: "run.sh",
                    oid: real,
                },
                TreeEntry {
                    mode: MODE_LINK,
                    name: "link.rs",
                    oid: link_blob,
                },
                TreeEntry {
                    mode: "160000", // gitlink / submodule
                    name: "submod",
                    oid: commit_dummy,
                },
            ],
        );

        let entries = list_tree_at_dir(&src, &root, "real.rs");
        assert_eq!(
            entries.len(),
            2,
            "only blobs/exe should be listed: {entries:?}"
        );
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"real.rs"));
        assert!(names.contains(&"run.sh"));
        assert!(!names.contains(&"link.rs"), "symlink must be excluded");
        assert!(!names.contains(&"submod"), "gitlink must be excluded");
    }

    /// A missing root oid → empty list, not a panic/error.
    #[test]
    fn missing_root_tree_returns_empty() {
        let src = CasObjectSource::new();
        let fake = parse_oid("0000000000000000000000000000000000000042").unwrap();
        let entries = list_tree_at_dir(&src, &fake, "anything.rs");
        assert!(entries.is_empty(), "missing root → empty, not error");
    }

    /// Traversal-unsafe path segments yield an empty list.
    #[test]
    fn traversal_unsafe_paths_return_empty() {
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"x".to_vec());
        let root = insert_tree(
            &mut src,
            vec![TreeEntry {
                mode: MODE_BLOB,
                name: "safe.rs",
                oid: blob,
            }],
        );

        for bad in ["../safe.rs", "./safe.rs", "a//b.rs"] {
            let entries = list_tree_at_dir(&src, &root, bad);
            assert!(
                entries.is_empty(),
                "traversal-unsafe path {bad:?} must return empty, got {entries:?}"
            );
        }
    }

    /// A path deeper than MAX_PATH_DEPTH is rejected before doing any I/O.
    #[test]
    fn over_depth_path_returns_empty() {
        let src = CasObjectSource::new();
        let fake_root = parse_oid("0000000000000000000000000000000000000001").unwrap();
        // Build a path with MAX_PATH_DEPTH + 1 segments.
        let deep: String = (0..=MAX_PATH_DEPTH)
            .map(|i| format!("dir{i}"))
            .collect::<Vec<_>>()
            .join("/");
        let entries = list_tree_at_dir(&src, &fake_root, &deep);
        assert!(
            entries.is_empty(),
            "path deeper than MAX_PATH_DEPTH must return empty (got {entries:?})"
        );
    }

    /// A tree with more than MAX_TREE_ENTRIES entries is capped at MAX_TREE_ENTRIES.
    #[test]
    fn over_cap_tree_entry_list_is_bounded() {
        let mut src = CasObjectSource::new();
        let blob = src.insert_raw(ObjectKind::Blob, b"x".to_vec());
        // Build MAX_TREE_ENTRIES + 10 entries.
        let n = MAX_TREE_ENTRIES + 10;
        let raw_entries: Vec<TreeEntry<'_>> = (0..n)
            .map(|i| TreeEntry {
                mode: MODE_BLOB,
                // We can't use temporary strings directly, so box them.
                name: Box::leak(format!("file{i:05}.rs").into_boxed_str()),
                oid: blob,
            })
            .collect();
        let root = insert_tree(&mut src, raw_entries);
        // list against any filename at the root level.
        let entries = list_tree_at_dir(&src, &root, "file00000.rs");
        assert_eq!(
            entries.len(),
            MAX_TREE_ENTRIES,
            "result must be capped at MAX_TREE_ENTRIES ({MAX_TREE_ENTRIES}), got {}",
            entries.len()
        );
    }
}

#[cfg(test)]
mod resolve_depth_tests {
    use super::*;

    /// A path deeper than MAX_PATH_DEPTH is rejected (returns Ok(None)).
    #[test]
    fn resolve_blob_over_depth_returns_none() {
        let src = CasObjectSource::new();
        let fake_root = parse_oid("0000000000000000000000000000000000000002").unwrap();
        // Build a path with MAX_PATH_DEPTH + 1 segments.
        let deep: String = (0..=MAX_PATH_DEPTH)
            .map(|i| format!("d{i}"))
            .collect::<Vec<_>>()
            .join("/");
        let result = resolve_blob_at_path(&src, &fake_root, &deep)
            .expect("must not error on over-depth path");
        assert!(result.is_none(), "over-depth path must resolve to None");
    }
}
