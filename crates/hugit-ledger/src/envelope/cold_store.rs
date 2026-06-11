//! The cold blob store seam (WP-F2, ADR-0001 §3 storage tier).
//!
//! Trajectory blobs (transcripts, prompts, the envelope itself) are
//! write-once / read-rarely archive: **they never ride the CoreLink hot
//! AC/CAS path** (owner-directed 2026-06-10 — the hot tier exists for
//! memoized-CI latency, not archives). They go to a cheap **cold object
//! store** behind the same content-addressed ref scheme. Refs are
//! **tier-agnostic** opaque content-addressed URIs (`cas:<sha256-hex>`);
//! the resolver maps hash → tier. Dedupe-by-content applies: the same
//! bytes always yield the same ref, stored once.
//!
//! ## The trait/seam split (the AC-client pattern, PARTIAL-over-fake)
//!
//! The producer logic — redact-on-write, capture-level gating, ref wiring,
//! dedupe — is proven NOW against hermetic stores implementing the exact
//! [`ColdBlobStore`] interface a live binding would: [`InMemoryColdStore`]
//! (the reference semantics) and [`DirColdStore`] (file-per-hash in a local
//! directory). The ONLY thing deferred is the live cold-store binding:
//! [`UnwiredColdStore`] is the explicitly-marked, fail-closed disclosed
//! seam — every call surfaces [`ColdStoreError::NotWired`] so an
//! unprovisioned deployment can never silently drop a trajectory. When the
//! cold bucket exists, that seam is filled behind the same trait; the
//! producer does not change.
//!
//! Retention is **forever** by ratified design: this seam has NO delete,
//! NO TTL, NO GC surface. Content leaves storage only via the explicit
//! erasure/tombstone path ("o conteúdo é apagável; a prova, não" — ADR-0001
//! ratified) — never via a timer.
//!
//! ## Erasure is tombstoning, not deletion (the right-to-erasure path)
//!
//! [`ColdBlobStore::erase`] is the SOLE way content leaves this tier. It is
//! NOT a delete: it replaces the blob bytes with an immutable, content-
//! addressed [`Tombstone`] keyed by the SAME `cas:` ref. After an erase:
//!
//! - [`ColdBlobStore::get`] on the erased ref returns
//!   [`GetOutcome::Erased`] carrying the tombstone — never the bytes, and
//!   never [`GetOutcome::Absent`] (an erased ref is provably distinct from a
//!   never-seen one: the proof survives the content).
//! - The tombstone records WHO requested the erasure, WHEN (a caller-supplied
//!   timestamp — this module mints no clock), the policy that authorised it,
//!   and the erased ref itself. It is itself content-addressed
//!   ([`Tombstone::tombstone_ref`]) and immutable.
//!
//! **Dedup interplay (stated honestly):** erasure is BY CONTENT. The cold tier
//! dedupes — the same bytes are one blob under one `cas:` ref, regardless of
//! how many logical objects reference it. Erasing a ref therefore erases THAT
//! content for every reference that resolves to it: there is exactly one blob,
//! and the tombstone replaces it. This is correct for right-to-erasure (the
//! personal data is the content; erasing the content discharges the obligation
//! for all copies of that content) but the caller must understand a shared-
//! content blob is shared: there is no per-reference erasure of identical
//! bytes, because there is no per-reference copy of identical bytes.

use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

/// The tier-agnostic content-addressed ref prefix. A ref is
/// `cas:<64-char lowercase-hex sha256>` of the blob bytes; the resolver —
/// not the ref — decides which tier (hot CAS for CI, cold store for
/// trajectories) the hash lives in.
pub const COLD_REF_PREFIX: &str = "cas:";

/// Errors from a cold-store interaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColdStoreError {
    /// The live cold-store binding is not yet provisioned (disclosed
    /// P2-class seam). Carries the intended binding so the deferral is
    /// self-documenting at the call site. Fail-closed: never a silent drop.
    NotWired(String),
    /// A filesystem/transport failure from a backing store.
    Io(String),
    /// The supplied ref is not a canonical `cas:<64-lowercase-hex>` URI.
    /// Refusing to resolve a malformed ref is a trust-boundary guard (the
    /// hash is a path segment in the dir-backed store).
    InvalidRef(String),
    /// The bytes resolved for a ref do not hash back to that ref — the
    /// content-address guard. NEVER trust a blind read: a mismatch means the
    /// store returned (or disk now holds) different content, so it is
    /// rejected instead of served. Carries `(requested, actual)`.
    DigestMismatch {
        /// The ref that was requested.
        requested: String,
        /// The ref the returned bytes actually hash to.
        actual: String,
    },
}

impl std::fmt::Display for ColdStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ColdStoreError::NotWired(what) => {
                write!(f, "cold blob store not wired (disclosed seam): {what}")
            }
            ColdStoreError::Io(e) => write!(f, "cold store I/O error: {e}"),
            ColdStoreError::InvalidRef(what) => {
                write!(f, "invalid cold-store ref (refusing to resolve): {what}")
            }
            ColdStoreError::DigestMismatch { requested, actual } => write!(
                f,
                "cold store content-address violation: requested {requested} but the \
                 returned bytes hash to {actual} (refusing a blind read)"
            ),
        }
    }
}

impl std::error::Error for ColdStoreError {}

/// Compute the tier-agnostic content-addressed ref for `bytes`:
/// `cas:<sha256-hex>`. Pure — the same bytes always map to the same ref
/// (this is what makes dedupe-by-content structural).
#[must_use]
pub fn cold_ref_for(bytes: &[u8]) -> String {
    format!("{COLD_REF_PREFIX}{}", hex::encode(Sha256::digest(bytes)))
}

/// The on-disk byte sentinel that marks a file as a tombstone rather than a
/// live blob (DirColdStore). A live blob can never collide with this: a live
/// blob at `<hash>` must hash to `<hash>`, and these bytes hash to something
/// else, so the content-address guard already distinguishes them — the prefix
/// is a fast, explicit discriminator the reader checks first.
const TOMBSTONE_SENTINEL: &[u8] = b"hugit-cold-tombstone-v1\n";

/// The caller-supplied authorisation record for an erasure: the *who/when/why*
/// of a right-to-erasure request, transcribed verbatim into the immutable
/// [`Tombstone`]. This module mints no clock and trusts no ambient identity —
/// the caller (the X7 cascade owner) supplies these from the authenticated
/// request, so the tombstone is an honest record of what was authorised, not a
/// guess.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TombstoneRecord {
    /// WHO requested the erasure (an authenticated principal / request id —
    /// caller-supplied; this module does not authenticate).
    pub requested_by: String,
    /// WHEN, as a caller-supplied Unix timestamp (seconds). This module mints
    /// no clock: the time is part of the authorised request, recorded as given.
    pub requested_at_unix: u64,
    /// The policy/legal-basis ref that authorised the erasure (e.g. a DPA case
    /// id or retention-policy ref). The audit anchor for the erasure.
    pub policy_ref: String,
}

/// An immutable, content-addressed erasure marker. Minted by
/// [`ColdBlobStore::erase`], it REPLACES the blob at a `cas:` ref. It carries
/// the erased ref itself, plus the [`TombstoneRecord`] (who/when/policy), so it
/// is a self-describing record of a lawful erasure — never a void.
///
/// The tombstone is itself content-addressed: [`Tombstone::tombstone_ref`] is
/// the `cas:` ref of its own canonical bytes, and it is immutable (no setter).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Tombstone {
    /// The `cas:` ref whose content was erased. The reference that resolved to
    /// the now-erased bytes still resolves here — to this tombstone.
    pub erased_ref: String,
    /// The caller-supplied who/when/policy of the erasure request.
    pub record: TombstoneRecord,
    /// A fixed marker tag making a tombstone trivially distinguishable on the
    /// wire (`"tombstone"`).
    pub marker: String,
}

/// The fixed tombstone marker tag.
pub const TOMBSTONE_MARKER: &str = "tombstone";

impl Tombstone {
    /// Mint a tombstone for `erased_ref` from the caller-supplied request.
    #[must_use]
    pub fn new(erased_ref: impl Into<String>, record: TombstoneRecord) -> Self {
        Self {
            erased_ref: erased_ref.into(),
            record,
            marker: TOMBSTONE_MARKER.to_string(),
        }
    }

    /// The canonical byte image of this tombstone (the persisted form). Goes
    /// through serde_json; the field order is the struct's declared order, so
    /// the same tombstone always yields the same bytes (content-addressable).
    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        // The tombstone is a small, flat, owned struct over String/u64 — serde
        // cannot fail to serialise it. The sentinel prefix makes the persisted
        // form self-identifying on disk and in memory.
        let body = serde_json::to_vec(self).expect("tombstone serialises (flat owned struct)");
        let mut out = Vec::with_capacity(TOMBSTONE_SENTINEL.len() + body.len());
        out.extend_from_slice(TOMBSTONE_SENTINEL);
        out.extend_from_slice(&body);
        out
    }

    /// The content-addressed ref OF this tombstone (the `cas:` ref of its own
    /// [`canonical_bytes`](Tombstone::canonical_bytes)). Stable + immutable:
    /// the same tombstone is the same ref.
    #[must_use]
    pub fn tombstone_ref(&self) -> String {
        cold_ref_for(&self.canonical_bytes())
    }

    /// Whether this is a well-formed tombstone (carries the marker tag).
    #[must_use]
    pub fn is_tombstone(&self) -> bool {
        self.marker == TOMBSTONE_MARKER
    }

    /// Parse a tombstone from its persisted byte image, if `bytes` is one.
    /// Returns `None` for live blobs (anything not carrying the sentinel).
    fn from_persisted(bytes: &[u8]) -> Option<Self> {
        let body = bytes.strip_prefix(TOMBSTONE_SENTINEL)?;
        serde_json::from_slice(body).ok()
    }
}

/// The outcome of resolving a `cas:` ref against a cold tier. Erasure makes
/// this a THREE-state outcome, not an `Option`: an erased ref is provably
/// distinct from a never-present one (the proof survives the content), so it is
/// never collapsed into `Absent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GetOutcome {
    /// The live bytes (content-address verified on read).
    Present(Vec<u8>),
    /// The ref was lawfully erased; the bytes are gone and this tamper-evident
    /// [`Tombstone`] stands in their place. NEVER the bytes, never `Absent`.
    Erased(Tombstone),
    /// Nothing is stored under this ref in this tier (and it was never erased
    /// here) — a genuine miss, e.g. a ref that lives in another tier.
    Absent,
}

impl GetOutcome {
    /// The live bytes if present, else `None` (an `Option`-shaped view for the
    /// producer read path; `Erased` and `Absent` both map to `None`).
    #[must_use]
    pub fn present(self) -> Option<Vec<u8>> {
        match self {
            GetOutcome::Present(b) => Some(b),
            _ => None,
        }
    }

    /// Borrow the live bytes if present.
    #[must_use]
    pub fn as_present(&self) -> Option<&[u8]> {
        match self {
            GetOutcome::Present(b) => Some(b),
            _ => None,
        }
    }

    /// The tombstone if this ref was erased.
    #[must_use]
    pub fn erased(&self) -> Option<&Tombstone> {
        match self {
            GetOutcome::Erased(t) => Some(t),
            _ => None,
        }
    }

    /// Whether this ref resolves to a tombstone (was lawfully erased).
    #[must_use]
    pub fn is_erased(&self) -> bool {
        matches!(self, GetOutcome::Erased(_))
    }
}

/// Validate a ref and return its hex hash segment. The hash becomes a path
/// segment in [`DirColdStore`], so anything other than `cas:` + 64 lowercase
/// hex chars is refused before it can touch a path.
fn ref_hash(blob_ref: &str) -> Result<&str, ColdStoreError> {
    let Some(hash) = blob_ref.strip_prefix(COLD_REF_PREFIX) else {
        return Err(ColdStoreError::InvalidRef(format!(
            "missing '{COLD_REF_PREFIX}' prefix"
        )));
    };
    // Lowercase hex only: is_ascii_hexdigit alone would also accept A-F.
    let is_lower_hex = |b: u8| b.is_ascii_hexdigit() && !b.is_ascii_uppercase();
    if hash.len() == 64 && hash.bytes().all(is_lower_hex) {
        Ok(hash)
    } else {
        Err(ColdStoreError::InvalidRef(format!(
            "hash segment must match ^[0-9a-f]{{64}}$ (got {} chars)",
            hash.len()
        )))
    }
}

/// The cold blob store interface the envelope producer depends on. Hermetic
/// stores and the live binding implement THIS, so redact-on-write, gating,
/// and dedupe are exercised identically against all of them.
///
/// Content-addressed: `put` returns the tier-agnostic `cas:<sha256>` ref of
/// the bytes; putting the same bytes twice is a dedup no-op returning the
/// same ref. There is deliberately no DELETE (retention forever, ratified) —
/// the only way content leaves is [`erase`](ColdBlobStore::erase), which
/// tombstones rather than deletes.
pub trait ColdBlobStore {
    /// Store `bytes`, returning their content-addressed ref
    /// (`cas:<sha256-hex>`). Idempotent: same bytes → same ref, stored once.
    ///
    /// # Errors
    /// Fails if the backing tier cannot persist the bytes (or, on the
    /// disclosed live seam, because the binding is not wired).
    fn put(&self, bytes: &[u8]) -> Result<String, ColdStoreError>;

    /// Resolve a ref to its outcome: [`GetOutcome::Present`] (live bytes,
    /// content-address verified), [`GetOutcome::Erased`] (a tombstone — the ref
    /// was lawfully erased), or [`GetOutcome::Absent`] (nothing here). An erased
    /// ref is NEVER `Absent` and NEVER returns the bytes.
    ///
    /// # Errors
    /// Fails on a malformed ref, a backing-tier failure, or a
    /// content-address violation (live bytes that don't hash back to the ref).
    fn get(&self, blob_ref: &str) -> Result<GetOutcome, ColdStoreError>;

    /// Erase the content at `blob_ref`, replacing it with an immutable,
    /// content-addressed [`Tombstone`] minted from the caller-supplied
    /// [`TombstoneRecord`]. This is the SOLE right-to-erasure path — NOT a
    /// delete: after it, [`get`](ColdBlobStore::get) on `blob_ref` returns
    /// [`GetOutcome::Erased`], never the bytes and never `Absent`.
    ///
    /// Erasure is BY CONTENT (the cold tier dedupes): there is exactly one blob
    /// per `cas:` ref, so erasing the ref erases that content for every logical
    /// reference that resolves to it. Erasing an absent-or-already-erased ref is
    /// idempotent: it installs / keeps a tombstone (an erased ref never decays
    /// to `Absent`).
    ///
    /// # Errors
    /// Fails on a malformed ref, a backing-tier failure, or — on the disclosed
    /// live seam — because the binding is not wired (fail-closed).
    fn erase(&self, blob_ref: &str, record: TombstoneRecord) -> Result<Tombstone, ColdStoreError>;
}

// HERMETIC FIXTURE — the reference semantics of the cold-store contract
// (content-keyed, write-once, no delete); the live binding must match its
// behavior. `#[doc(hidden)]` keeps it out of rendered docs so it is never
// mistaken for a production archive backend.
/// A deterministic, in-process cold blob store.
#[doc(hidden)]
#[derive(Debug, Default)]
pub struct InMemoryColdStore {
    /// ref → bytes. `Mutex` keeps the impl `Sync` without leaking the lock
    /// into the trait surface.
    blobs: Mutex<HashMap<String, Vec<u8>>>,
    /// ref → tombstone for erased refs. An erased ref is removed from `blobs`
    /// and recorded here, so `get` distinguishes erased from absent.
    tombstones: Mutex<HashMap<String, Tombstone>>,
}

impl InMemoryColdStore {
    /// A fresh, empty store.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// How many distinct blobs are stored (dedupe evidence: N identical puts
    /// leave this at 1).
    #[must_use]
    pub fn len(&self) -> usize {
        self.blobs.lock().expect("blobs lock poisoned").len()
    }

    /// True iff no blobs are stored.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every stored `(ref, bytes)` pair, sorted by ref — the byte image a
    /// grep-class absence scan reads (X3: verify ABSENCE in the stored
    /// bytes, not flag state).
    #[must_use]
    pub fn blobs(&self) -> Vec<(String, Vec<u8>)> {
        let map = self.blobs.lock().expect("blobs lock poisoned");
        let mut out: Vec<(String, Vec<u8>)> =
            map.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }
}

impl ColdBlobStore for InMemoryColdStore {
    fn put(&self, bytes: &[u8]) -> Result<String, ColdStoreError> {
        let blob_ref = cold_ref_for(bytes);
        // Erasure is permanent: a re-put of erased content must NOT resurrect
        // it. If the ref is tombstoned, the put is a no-op that returns the ref
        // (the content stays erased; `get` keeps returning the tombstone).
        if self
            .tombstones
            .lock()
            .expect("tombstones lock poisoned")
            .contains_key(&blob_ref)
        {
            return Ok(blob_ref);
        }
        self.blobs
            .lock()
            .expect("blobs lock poisoned")
            .entry(blob_ref.clone())
            .or_insert_with(|| bytes.to_vec());
        Ok(blob_ref)
    }

    fn get(&self, blob_ref: &str) -> Result<GetOutcome, ColdStoreError> {
        ref_hash(blob_ref)?;
        // Live bytes take precedence; an erased ref is never live.
        let live = self
            .blobs
            .lock()
            .expect("blobs lock poisoned")
            .get(blob_ref)
            .cloned();
        if let Some(bytes) = live {
            // Read-guard parity with DirColdStore: re-hash on read and refuse
            // bytes that no longer hash back to the ref (defends against an
            // in-memory map mutated out of band, e.g. via a poisoned-recovered
            // lock or unsafe aliasing — never serve a blind read).
            let actual = cold_ref_for(&bytes);
            if actual != blob_ref {
                return Err(ColdStoreError::DigestMismatch {
                    requested: blob_ref.to_string(),
                    actual,
                });
            }
            return Ok(GetOutcome::Present(bytes));
        }
        if let Some(ts) = self
            .tombstones
            .lock()
            .expect("tombstones lock poisoned")
            .get(blob_ref)
            .cloned()
        {
            return Ok(GetOutcome::Erased(ts));
        }
        Ok(GetOutcome::Absent)
    }

    fn erase(&self, blob_ref: &str, record: TombstoneRecord) -> Result<Tombstone, ColdStoreError> {
        ref_hash(blob_ref)?;
        let tombstone = Tombstone::new(blob_ref, record);
        // Drop the bytes (erasure is by content: this is the one shared blob),
        // then install the immutable tombstone keyed by the SAME ref. Order:
        // tombstone-then-drop would briefly show both; drop-then-tombstone
        // briefly shows Absent — under the shared lock-pair below neither is
        // observable, but we hold blobs first so a concurrent get never sees
        // the bytes after the tombstone is published.
        let mut blobs = self.blobs.lock().expect("blobs lock poisoned");
        let mut tombstones = self.tombstones.lock().expect("tombstones lock poisoned");
        blobs.remove(blob_ref);
        tombstones.insert(blob_ref.to_string(), tombstone.clone());
        Ok(tombstone)
    }
}

/// A local directory-backed cold store: one file per content hash under a
/// root directory ("até Google Drive resolve" — the bar is cost, not
/// latency; a directory of hash-named files is the minimal honest model of
/// that tier). Hermetic: no network, suitable for temp-dir-rooted proofs.
///
/// Reads are guarded by the content-address check: bytes that no longer
/// hash to the requested ref (disk tamper/corruption) are REJECTED with
/// [`ColdStoreError::DigestMismatch`], never served.
#[derive(Debug)]
pub struct DirColdStore {
    /// Root directory; blobs live at `<root>/<sha256-hex>`.
    root: PathBuf,
}

impl DirColdStore {
    /// Open (creating if needed) a dir-backed store rooted at `root`.
    ///
    /// # Errors
    /// Fails if the root directory cannot be created.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, ColdStoreError> {
        let root = root.into();
        fs::create_dir_all(&root).map_err(|e| {
            ColdStoreError::Io(format!("creating cold-store root {}: {e}", root.display()))
        })?;
        Ok(Self { root })
    }
}

impl ColdBlobStore for DirColdStore {
    fn put(&self, bytes: &[u8]) -> Result<String, ColdStoreError> {
        let blob_ref = cold_ref_for(bytes);
        // `ref_hash` cannot fail on a ref we just derived, but route through
        // the same guard so the path segment is provably canonical.
        let hash = ref_hash(&blob_ref)?;
        let path = self.root.join(hash);
        if !path.exists() {
            // Write-once via temp + rename so a concurrent reader never sees
            // a half-written blob; an existing file is the dedup hit. An
            // existing file may also be a TOMBSTONE (erasure is permanent): the
            // `exists` guard makes a re-put of erased content a no-op, so it is
            // never resurrected — `get` keeps returning the tombstone.
            let tmp = self.root.join(format!("{hash}.tmp-{}", std::process::id()));
            fs::write(&tmp, bytes)
                .map_err(|e| ColdStoreError::Io(format!("writing {}: {e}", tmp.display())))?;
            fs::rename(&tmp, &path)
                .map_err(|e| ColdStoreError::Io(format!("publishing {}: {e}", path.display())))?;
        }
        Ok(blob_ref)
    }

    fn get(&self, blob_ref: &str) -> Result<GetOutcome, ColdStoreError> {
        let hash = ref_hash(blob_ref)?;
        let path = self.root.join(hash);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(GetOutcome::Absent);
            }
            Err(e) => {
                return Err(ColdStoreError::Io(format!(
                    "reading {}: {e}",
                    path.display()
                )));
            }
        };
        // A tombstone occupies the live-blob slot at <hash>; recognise it first
        // (it carries the sentinel and self-describes the erased ref).
        if let Some(ts) = Tombstone::from_persisted(&bytes) {
            if ts.erased_ref != blob_ref {
                // The persisted tombstone names a different ref than the path it
                // sits under — a corrupted/forged marker. Fail closed.
                return Err(ColdStoreError::DigestMismatch {
                    requested: blob_ref.to_string(),
                    actual: ts.erased_ref,
                });
            }
            return Ok(GetOutcome::Erased(ts));
        }
        // Content-address guard: never serve bytes that don't hash back.
        let actual = cold_ref_for(&bytes);
        if actual != blob_ref {
            return Err(ColdStoreError::DigestMismatch {
                requested: blob_ref.to_string(),
                actual,
            });
        }
        Ok(GetOutcome::Present(bytes))
    }

    fn erase(&self, blob_ref: &str, record: TombstoneRecord) -> Result<Tombstone, ColdStoreError> {
        let hash = ref_hash(blob_ref)?;
        let path = self.root.join(hash);
        let tombstone = Tombstone::new(blob_ref, record);
        // Atomic replace: write the tombstone to a temp file then rename OVER
        // the live blob (rename is atomic on POSIX) so a concurrent reader sees
        // EITHER the live blob OR the tombstone, never a half-written file and
        // never a NotFound gap. This is erasure-by-content: the single shared
        // blob at <hash> becomes the tombstone for every ref that resolves here.
        let tmp = self
            .root
            .join(format!("{hash}.tombstone.tmp-{}", std::process::id()));
        fs::write(&tmp, tombstone.canonical_bytes())
            .map_err(|e| ColdStoreError::Io(format!("writing {}: {e}", tmp.display())))?;
        fs::rename(&tmp, &path).map_err(|e| {
            ColdStoreError::Io(format!("publishing tombstone {}: {e}", path.display()))
        })?;
        Ok(tombstone)
    }
}

/// The DISCLOSED live cold-store seam — NOT built (same class as the P2
/// live-infra seams). Binding the real cold object store (a cheap archive
/// bucket behind the same `cas:` refs) is owner-gated provisioning, not
/// code; until then every call fails CLOSED with
/// [`ColdStoreError::NotWired`] so an unprovisioned deployment surfaces a
/// clear error instead of silently dropping trajectory blobs.
#[derive(Debug, Default)]
pub struct UnwiredColdStore;

impl UnwiredColdStore {
    /// The disclosed seam, unprovisioned.
    #[must_use]
    pub fn new() -> Self {
        Self
    }

    /// The self-documenting deferral message every call carries.
    fn not_wired() -> ColdStoreError {
        ColdStoreError::NotWired(
            "live cold object store binding (archive tier behind tier-agnostic cas: refs) \
             is owner-gated provisioning; hermetic proofs use InMemoryColdStore/DirColdStore"
                .to_string(),
        )
    }
}

impl ColdBlobStore for UnwiredColdStore {
    fn put(&self, _bytes: &[u8]) -> Result<String, ColdStoreError> {
        Err(Self::not_wired())
    }

    fn get(&self, _blob_ref: &str) -> Result<GetOutcome, ColdStoreError> {
        Err(Self::not_wired())
    }

    fn erase(
        &self,
        _blob_ref: &str,
        _record: TombstoneRecord,
    ) -> Result<Tombstone, ColdStoreError> {
        // Fail-closed: an unprovisioned deployment can never claim to have
        // erased anything (a false discharge of a right-to-erasure obligation
        // would be worse than an honest "not wired").
        Err(Self::not_wired())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_is_cas_prefixed_sha256_hex() {
        let r = cold_ref_for(b"hello");
        assert!(r.starts_with(COLD_REF_PREFIX));
        assert_eq!(r.len(), COLD_REF_PREFIX.len() + 64);
        assert!(ref_hash(&r).is_ok());
    }

    #[test]
    fn ref_validation_refuses_malformed() {
        assert!(matches!(
            ref_hash("sha256:abcd"),
            Err(ColdStoreError::InvalidRef(_))
        ));
        assert!(matches!(
            ref_hash("cas:../escape"),
            Err(ColdStoreError::InvalidRef(_))
        ));
        // Uppercase hex is non-canonical.
        let upper = format!("cas:{}", "A".repeat(64));
        assert!(matches!(
            ref_hash(&upper),
            Err(ColdStoreError::InvalidRef(_))
        ));
    }

    #[test]
    fn in_memory_put_get_round_trip_and_dedupe() {
        let store = InMemoryColdStore::new();
        let r1 = store.put(b"same bytes").expect("put");
        let r2 = store.put(b"same bytes").expect("put again");
        assert_eq!(r1, r2, "same content must yield the same ref");
        assert_eq!(store.len(), 1, "dedupe: stored once");
        assert_eq!(
            store.get(&r1).expect("get"),
            GetOutcome::Present(b"same bytes".to_vec())
        );
        let r3 = store.put(b"other bytes").expect("put other");
        assert_ne!(r1, r3);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn unwired_seam_fails_closed() {
        let live = UnwiredColdStore::new();
        assert!(matches!(live.put(b"x"), Err(ColdStoreError::NotWired(_))));
        assert!(matches!(
            live.get(&cold_ref_for(b"x")),
            Err(ColdStoreError::NotWired(_))
        ));
        // Erasure on the unwired seam must ALSO fail closed: never a false
        // discharge of a right-to-erasure obligation.
        assert!(matches!(
            live.erase(&cold_ref_for(b"x"), rec()),
            Err(ColdStoreError::NotWired(_))
        ));
    }

    /// A caller-supplied erasure record (the X7 cascade owner supplies these).
    fn rec() -> TombstoneRecord {
        TombstoneRecord {
            requested_by: "rtbf-request:case-42".to_string(),
            requested_at_unix: 1_717_900_000,
            policy_ref: "policy:gdpr-art17-v1".to_string(),
        }
    }

    #[test]
    fn tombstone_is_content_addressed_and_immutable() {
        let r = cold_ref_for(b"subject data");
        let t1 = Tombstone::new(&r, rec());
        let t2 = Tombstone::new(&r, rec());
        assert!(t1.is_tombstone());
        assert_eq!(t1.marker, TOMBSTONE_MARKER);
        // Same erased-ref + same record → same content-addressed tombstone ref.
        assert_eq!(t1.tombstone_ref(), t2.tombstone_ref());
        assert!(t1.tombstone_ref().starts_with(COLD_REF_PREFIX));
        // A different policy ref yields a different tombstone ref (the record is
        // part of the addressed content).
        let mut other_rec = rec();
        other_rec.policy_ref = "policy:other".to_string();
        let t3 = Tombstone::new(&r, other_rec);
        assert_ne!(t1.tombstone_ref(), t3.tombstone_ref());
    }

    #[test]
    fn in_memory_erase_returns_erased_never_bytes_never_absent() {
        let store = InMemoryColdStore::new();
        let r = store.put(b"subject personal data").expect("put");
        assert!(matches!(
            store.get(&r).expect("get"),
            GetOutcome::Present(_)
        ));

        let ts = store.erase(&r, rec()).expect("erase");
        assert_eq!(ts.erased_ref, r);
        assert_eq!(ts.record, rec());

        // get on the erased ref is Erased(tombstone) — never the bytes, never
        // Absent (distinguishable from a never-present ref).
        match store.get(&r).expect("get erased") {
            GetOutcome::Erased(got) => assert_eq!(got, ts),
            other => panic!("erased ref must resolve to a tombstone, got {other:?}"),
        }
        // A genuinely-absent ref is Absent, NOT Erased — the two are distinct.
        let absent = cold_ref_for(b"never stored anywhere");
        assert_eq!(store.get(&absent).expect("get absent"), GetOutcome::Absent);
    }

    #[test]
    fn in_memory_erase_is_idempotent_and_blocks_resurrection() {
        let store = InMemoryColdStore::new();
        let bytes = b"resurrect me?";
        let r = store.put(bytes).expect("put");
        store.erase(&r, rec()).expect("erase");
        // Erasing again is fine (idempotent), stays erased.
        store.erase(&r, rec()).expect("erase again");
        assert!(store.get(&r).expect("get").is_erased());
        // A re-put of the SAME content must NOT resurrect it (erasure is by
        // content and permanent).
        let r2 = store.put(bytes).expect("re-put erased content");
        assert_eq!(r2, r);
        assert!(
            store.get(&r).expect("get").is_erased(),
            "re-put must not resurrect erased content",
        );
    }

    #[test]
    fn in_memory_get_re_hashes_on_read() {
        // The audit's read-guard gap: InMemoryColdStore::get must re-hash on
        // read like DirColdStore. Inject a blob under a ref it does NOT hash to
        // (simulating an out-of-band map mutation) and prove get fails CLOSED
        // with DigestMismatch — never serving the mismatched bytes.
        let store = InMemoryColdStore::new();
        let real_ref = cold_ref_for(b"the real content");
        let wrong_ref = cold_ref_for(b"a different ref");
        store
            .blobs
            .lock()
            .expect("lock")
            .insert(wrong_ref.clone(), b"the real content".to_vec());
        assert_ne!(real_ref, wrong_ref);
        match store.get(&wrong_ref) {
            Err(ColdStoreError::DigestMismatch { requested, actual }) => {
                assert_eq!(requested, wrong_ref);
                assert_eq!(actual, real_ref, "re-hash exposes the true content ref");
            }
            other => panic!("re-hash guard must fail closed, got {other:?}"),
        }
        // A clean put round-trips fine (the guard only rejects mismatches).
        let r = store.put(b"clean").expect("put");
        assert!(matches!(
            store.get(&r).expect("get"),
            GetOutcome::Present(_)
        ));
    }

    #[test]
    fn dir_erase_round_trip_and_get_re_hash_guard() {
        let root = std::env::temp_dir().join(format!(
            "hugit-coldstore-erase-{}-{}",
            std::process::id(),
            line!()
        ));
        let store = DirColdStore::open(&root).expect("open");
        let r = store.put(b"subject data on disk").expect("put");
        assert!(matches!(
            store.get(&r).expect("get"),
            GetOutcome::Present(_)
        ));

        // Read-guard: tamper the live blob → DigestMismatch (never served).
        let hash = r.strip_prefix(COLD_REF_PREFIX).expect("cas");
        fs::write(root.join(hash), b"tampered live bytes").expect("tamper");
        assert!(matches!(
            store.get(&r),
            Err(ColdStoreError::DigestMismatch { .. })
        ));

        // Erase atomically replaces the (tampered) blob with a tombstone.
        let ts = store.erase(&r, rec()).expect("erase");
        match store.get(&r).expect("get erased") {
            GetOutcome::Erased(got) => assert_eq!(got, ts),
            other => panic!("expected tombstone, got {other:?}"),
        }
        // Re-put of the same content must not resurrect (file exists = no-op).
        let _ = store.put(b"subject data on disk").expect("re-put");
        assert!(store.get(&r).expect("get").is_erased());

        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn dir_absent_is_absent_not_erased() {
        let root = std::env::temp_dir().join(format!(
            "hugit-coldstore-absent-{}-{}",
            std::process::id(),
            line!()
        ));
        let store = DirColdStore::open(&root).expect("open");
        let absent = cold_ref_for(b"never stored");
        assert_eq!(store.get(&absent).expect("get"), GetOutcome::Absent);
        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn dir_forged_tombstone_for_wrong_ref_fails_closed() {
        // A tombstone whose self-described erased_ref does not match the path it
        // sits under is corrupt/forged → fail closed (not served as Erased).
        let root = std::env::temp_dir().join(format!(
            "hugit-coldstore-forge-{}-{}",
            std::process::id(),
            line!()
        ));
        let store = DirColdStore::open(&root).expect("open");
        let r = store.put(b"victim").expect("put");
        let hash = r.strip_prefix(COLD_REF_PREFIX).expect("cas");
        // Write a tombstone that claims a DIFFERENT erased_ref at this path.
        let forged = Tombstone::new(cold_ref_for(b"some other ref"), rec());
        fs::write(root.join(hash), forged.canonical_bytes()).expect("forge");
        assert!(matches!(
            store.get(&r),
            Err(ColdStoreError::DigestMismatch { .. })
        ));
        fs::remove_dir_all(&root).expect("cleanup");
    }

    #[test]
    fn dedup_erasure_is_by_content_shared_blob() {
        // Honest dedup disclosure: two logical references to the SAME content
        // are the SAME blob under the SAME ref. Erasing it erases the content
        // for both — there is one blob, by design.
        let store = InMemoryColdStore::new();
        let ra = store.put(b"shared personal data").expect("put a");
        let rb = store.put(b"shared personal data").expect("put b");
        assert_eq!(ra, rb, "same content → same ref (dedup)");
        assert_eq!(store.len(), 1, "one blob");
        store.erase(&ra, rec()).expect("erase");
        // Both references now resolve to the tombstone — by content.
        assert!(store.get(&ra).expect("get a").is_erased());
        assert!(store.get(&rb).expect("get b").is_erased());
    }
}
