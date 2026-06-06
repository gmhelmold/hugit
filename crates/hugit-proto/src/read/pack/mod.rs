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

    /// The git object id (SHA-1 over the loose-object pre-image) for this object.
    ///
    /// Delegates to the git library's canonical hash so the id matches git
    /// byte-for-byte; never reimplemented here.
    pub fn oid(&self) -> ObjectId {
        gix_object::compute_hash(HashKind::Sha1, self.kind.to_gix(), &self.data)
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
    /// The git library failed to encode an object into a pack entry.
    #[error("pack entry encoding failed: {0}")]
    Encode(String),
    /// The git library failed while writing pack bytes.
    #[error("pack write failed: {0}")]
    Write(String),
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
                // corrupt; never serve mislabeled bytes into a clone.
                let actual = obj.oid();
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
