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
//! erasure/tombstone path (X7 cascade owner) — never via a timer.

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
/// same ref. There is deliberately no delete (retention forever, ratified).
pub trait ColdBlobStore {
    /// Store `bytes`, returning their content-addressed ref
    /// (`cas:<sha256-hex>`). Idempotent: same bytes → same ref, stored once.
    ///
    /// # Errors
    /// Fails if the backing tier cannot persist the bytes (or, on the
    /// disclosed live seam, because the binding is not wired).
    fn put(&self, bytes: &[u8]) -> Result<String, ColdStoreError>;

    /// Resolve a ref to its bytes. `Ok(None)` = not present in this tier.
    ///
    /// # Errors
    /// Fails on a malformed ref, a backing-tier failure, or a
    /// content-address violation (bytes that don't hash back to the ref).
    fn get(&self, blob_ref: &str) -> Result<Option<Vec<u8>>, ColdStoreError>;
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
        self.blobs
            .lock()
            .expect("blobs lock poisoned")
            .entry(blob_ref.clone())
            .or_insert_with(|| bytes.to_vec());
        Ok(blob_ref)
    }

    fn get(&self, blob_ref: &str) -> Result<Option<Vec<u8>>, ColdStoreError> {
        ref_hash(blob_ref)?;
        Ok(self
            .blobs
            .lock()
            .expect("blobs lock poisoned")
            .get(blob_ref)
            .cloned())
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
            // a half-written blob; an existing file is the dedup hit.
            let tmp = self.root.join(format!("{hash}.tmp-{}", std::process::id()));
            fs::write(&tmp, bytes)
                .map_err(|e| ColdStoreError::Io(format!("writing {}: {e}", tmp.display())))?;
            fs::rename(&tmp, &path)
                .map_err(|e| ColdStoreError::Io(format!("publishing {}: {e}", path.display())))?;
        }
        Ok(blob_ref)
    }

    fn get(&self, blob_ref: &str) -> Result<Option<Vec<u8>>, ColdStoreError> {
        let hash = ref_hash(blob_ref)?;
        let path = self.root.join(hash);
        let bytes = match fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(ColdStoreError::Io(format!(
                    "reading {}: {e}",
                    path.display()
                )));
            }
        };
        // Content-address guard: never serve bytes that don't hash back.
        let actual = cold_ref_for(&bytes);
        if actual != blob_ref {
            return Err(ColdStoreError::DigestMismatch {
                requested: blob_ref.to_string(),
                actual,
            });
        }
        Ok(Some(bytes))
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

    fn get(&self, _blob_ref: &str) -> Result<Option<Vec<u8>>, ColdStoreError> {
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
        assert_eq!(store.get(&r1).expect("get"), Some(b"same bytes".to_vec()));
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
    }
}
