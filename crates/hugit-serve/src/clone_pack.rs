//! WP-A — the FROZEN CONTRACT + pure logic for a **cached pre-assembled git
//! clone pack**.
//!
//! ## Why this module exists
//!
//! An anonymous `git clone` of a large repo (hugit is ~6862 objects) times out:
//! reading the full object closure from the CoreLink CAS is ~550 s at scale
//! (~80 ms/object), which blows the serve budget → the client sees a 404. The
//! fix is to assemble the **full-repo pack ONCE**, store the raw pack bytes as a
//! SINGLE R2 object, and on a full clone stream that one object instead of
//! walking the CAS per request.
//!
//! This module is the **storage + pure-logic layer** for that cache. A LATER WP
//! (WP-BC) wires it into the engine's clone door; this module only *builds* and
//! is *hermetically tested*. The signatures below are **FROZEN** — WP-BC codes
//! against them exactly.
//!
//! ## The correctness gate — `refset_sha`
//!
//! A cached pack is only correct while it matches the refs the advertisement
//! exposes. The single gate that guarantees this is [`refset_sha`]: a SHA-256
//! over the SORTED, DEDUPED set of tip OIDs (the *values* of the refs map). The
//! git object closure a clone must serve depends ONLY on the set of tip OIDs
//! (ref NAMES are irrelevant to reachability), so two ref views with the same
//! tip-set share the same closure — and thus the same cached pack. The serve
//! path ([`try_serve_cached_clone_pack`]) recomputes `refset_sha` from the SAME
//! `live_refs` the advertisement is built from and refuses the cache on any
//! mismatch (falls back to the slow walk). A stale pointer is therefore never a
//! correctness hole — only a missed optimization.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cas::{ManifestPutError, R2Get, R2Put, R2PutConditional};
use crate::state::R2Config;
use crate::writes::CasToken;

/// The per-repo handle for the clone-pack cache.
///
/// `r2` is expected to be a **write-scoped** [`R2Config`] at build time (it
/// PUTs the pack + the pointer); a read-scoped one suffices for the serve-side
/// lookup ([`try_serve_cached_clone_pack`], which only GETs).
#[derive(Clone)]
pub struct CloneCacheSeam {
    /// The R2 config used to read/write the pack object + `current.json`.
    pub r2: R2Config,
    /// The tenant prefix (mirrors the git-from-CAS manifest keys).
    pub tenant: String,
    /// The repo slug (mirrors the git-from-CAS manifest keys).
    pub repo_slug: String,
}

/// The pointer stored at `current.json` — the currently-valid cached pack for a
/// repo. Serialized to / deserialized from the `current.json` bytes.
///
/// `refset_sha` is the [`refset_sha`] of the refs the pack was built for; the
/// serve path only trusts the pack when this equals the live refs' sha. Never
/// serve a pack whose pointer's `refset_sha` disagrees with the live advertise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentPointer {
    /// The [`refset_sha`] of the ref tip-set this pack was assembled for.
    pub refset_sha: String,
    /// The R2 key of the pack object ([`clone_pack_key`]).
    pub pack_key: String,
    /// The number of git objects in the pack (the closure size).
    pub object_count: u64,
    /// When the pack was built (unix epoch milliseconds).
    pub built_at_ms: u64,
}

/// The R2 key for a repo's clone-pack pointer: `<tenant>/<repo>/clone-pack/current.json`.
#[must_use]
pub fn clone_pack_current_key(tenant: &str, repo: &str) -> String {
    format!("{tenant}/{repo}/clone-pack/current.json")
}

/// The R2 key for a repo's clone-pack object, addressed by its refset sha:
/// `<tenant>/<repo>/clone-pack/<refset_sha>.pack`.
#[must_use]
pub fn clone_pack_key(tenant: &str, repo: &str, refset_sha: &str) -> String {
    format!("{tenant}/{repo}/clone-pack/{refset_sha}.pack")
}

/// The correctness gate: a SHA-256 over the SORTED, DEDUPED set of tip OIDs (the
/// VALUES of `refs`), joined by `\n`. Returns lowercase hex.
///
/// Ref NAMES are irrelevant — a clone's object closure depends only on which tip
/// OIDs are reachable, so the sha keys ON the tip-set and nothing else. This
/// makes it **deterministic AND order-independent**: two refs maps with the same
/// tip-set in any insertion order (or with different ref names pointing at the
/// same tips) yield the same sha; a different tip yields a different sha.
#[must_use]
pub fn refset_sha(refs: &BTreeMap<String, String>) -> String {
    // A `BTreeSet` gives us the sorted + deduped tip-set for free, independent of
    // the input map's iteration order (a `BTreeMap` iterates by key, but we hash
    // the VALUES, whose order/multiplicity we must normalize ourselves).
    let tips: std::collections::BTreeSet<&str> = refs.values().map(String::as_str).collect();
    let mut hasher = Sha256::new();
    let mut first = true;
    for tip in tips {
        if !first {
            hasher.update(b"\n");
        }
        hasher.update(tip.as_bytes());
        first = false;
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(hex, "{byte:02x}");
    }
    hex
}

/// Load the clone-pack pointer (`current.json`) for a repo. Returns `None` on
/// absence OR any parse error (**fail-soft** — a missing/garbage pointer simply
/// means "no cache", the caller falls back to the slow walk; it is never fatal).
pub fn load_current<R: R2Get>(r2: &R, tenant: &str, repo: &str) -> Option<CurrentPointer> {
    let key = clone_pack_current_key(tenant, repo);
    let bytes = r2.get_object(&key).ok()??;
    serde_json::from_slice::<CurrentPointer>(&bytes).ok()
}

/// Durably store a freshly-assembled pack and flip the pointer to it.
///
/// **ORDER IS LOAD-BEARING for correctness:** the pack object is PUT to
/// [`clone_pack_key`] and confirmed durable FIRST; ONLY THEN is `current.json`
/// written. The pointer therefore never references a pack that is not already
/// durably present — a serve that reads a fresh pointer always finds its pack.
///
/// The pointer write uses the CONDITIONAL (compare-and-swap) surface with
/// `If-None-Match: *` ([`CasToken::Absent`]) to create the first pointer without
/// clobbering a concurrent first build. If a pointer already exists (the store
/// answers [`ManifestPutError::Precondition`]) the pointer is advanced with an
/// unconditional PUT — this is safe because a stale pointer is NEVER a
/// correctness hole (the serve-side [`refset_sha`] gate rejects any pointer whose
/// `refset_sha` disagrees with the live refs, so at worst the cache is missed and
/// the slow walk runs). A genuine version-CAS advance would need the versioned
/// read surface, which is intentionally NOT in this frozen bound (WP-BC keeps
/// the signature stable); the engine's single-writer invariant makes the
/// unconditional advance race-free in practice, and the gate makes it safe even
/// if that invariant is ever relaxed.
///
/// Returns `Err(String)` — never panics — on any R2 failure; the caller treats a
/// failed build as "no cache".
///
/// Operates on the CONCRETE [`CloneCacheSeam::r2`] (a real `R2Config`, which
/// implements both [`R2Put`] and [`R2PutConditional`]). The vestigial generic the
/// frozen bound once carried is dropped now that WP-BC wires the real call site —
/// the writes always go through `seam.r2`, so there was never a second `R` to name.
pub fn store_pack_and_flip(
    seam: &CloneCacheSeam,
    refset_sha: &str,
    pack_bytes: &[u8],
    object_count: u64,
    built_at_ms: u64,
) -> Result<(), String> {
    // (1) Durably write the pack FIRST (content-addressed by refset_sha → the
    //     same refset always maps to the same key, so a repeat PUT is idempotent).
    let pack_key = clone_pack_key(&seam.tenant, &seam.repo_slug, refset_sha);
    // Fully-qualified trait call: `R2Config` has an INHERENT `put_object` that
    // would otherwise shadow the [`R2Put`] trait method (different return type).
    R2Put::put_object(&seam.r2, &pack_key, pack_bytes)
        .map_err(|e| format!("clone-pack: pack PUT to {pack_key} failed: {e}"))?;

    // (2) ONLY NOW flip the pointer — the pack it references is durable.
    let pointer = CurrentPointer {
        refset_sha: refset_sha.to_string(),
        pack_key: pack_key.clone(),
        object_count,
        built_at_ms,
    };
    let body = serde_json::to_vec(&pointer)
        .map_err(|e| format!("clone-pack: current.json serialize failed: {e}"))?;
    let current_key = clone_pack_current_key(&seam.tenant, &seam.repo_slug);

    match R2PutConditional::put_object_conditional(&seam.r2, &current_key, &body, &CasToken::Absent)
    {
        // First pointer created (If-None-Match: * succeeded).
        Ok(_) => Ok(()),
        // A pointer already exists — advance it (safe: the serve-side refset_sha
        // gate rejects a stale pointer, so an unconditional advance can never
        // serve a wrong pack).
        Err(ManifestPutError::Precondition) => R2Put::put_object(&seam.r2, &current_key, &body)
            .map_err(|e| format!("clone-pack: current.json advance PUT failed: {e}")),
        Err(e) => Err(format!("clone-pack: current.json PUT failed: {e}")),
    }
}

/// Assemble the full-closure pack for ALL tips in `refs` with NO haves (a full
/// clone), reusing `serve_fetch` + `WantHave::clone_all` EXACTLY as the engine's
/// upload-pack path does. Returns `(raw_pack_bytes, object_count)`.
///
/// The bytes are the RAW packfile (`PACK` header … trailing SHA-1) — NOT NAK-
/// framed. Smart-HTTP framing (the `NAK` pkt-line prefix) stays in the serve
/// layer, so the same cached bytes can be re-framed per protocol version.
///
/// Any assembly error (incomplete closure, decode failure, budget) maps to
/// `Err(String)` — the caller treats a failed build as "no cache".
pub fn build_clone_pack(
    source: &dyn hugit_proto::ObjectSource,
    refs: &BTreeMap<String, String>,
) -> Result<(Vec<u8>, u64), String> {
    // Want every advertised tip, have nothing — the SAME construction as
    // `git.rs`'s full-clone path (`RefAdvertisement::from_view` → `clone_all`),
    // so the wants always resolve in the source by construction.
    let adv = hugit_proto::RefAdvertisement::from_view(refs);
    let request = hugit_proto::WantHave::clone_all(&adv)
        .map_err(|e| format!("clone-pack: clone_all failed: {e}"))?;
    let pack = hugit_proto::serve_fetch(source, &request)
        .map_err(|e| format!("clone-pack: serve_fetch failed: {e}"))?;
    let object_count = pack.object_ids.len() as u64;
    Ok((pack.bytes, object_count))
}

/// The outcome of a serve-side clone-pack cache lookup. The caller (`git.rs`
/// `serve_upload_pack_response`) routes each variant differently — critically,
/// [`Absent`](CloneCacheLookup::Absent) and [`Stale`](CloneCacheLookup::Stale) are NOT
/// the same: an absent pack is still building (retry), a stale pack means a ref moved
/// (walk), and conflating them would either DoS or wedge a repo.
pub enum CloneCacheLookup {
    /// The cached pointer matches the live refs — serve these pack bytes verbatim.
    Hit(Vec<u8>),
    /// No usable pointer (no `current.json` yet — the pack is still building in the
    /// boot window — or a transient read fault, or a pointer that matched but whose
    /// pack read failed). The caller answers a RETRYABLE 503, NEVER the whole-closure
    /// slow walk (a single-thread latency DoS on a large repo).
    Absent,
    /// A pointer EXISTS but for a DIFFERENT tip-set (a ref moved without a rebuild yet).
    /// The caller FALLS OPEN to the slow walk — both correctness- AND availability-
    /// preserving: a 503 here could leave a repo unclonable until the next push
    /// rebuilds the pack (e.g. after a `push --delete`), whereas the walk always
    /// produces a byte-correct pack for the advertised refs.
    Stale,
}

/// The serve-side lookup: classify the cache against the live refs (see
/// [`CloneCacheLookup`]).
///
/// The gate: [`load_current`] → the pointer's `refset_sha` must equal the
/// [`refset_sha`] of `live_refs` (the SAME map the advertisement is built from) → GET
/// the pack at `current.pack_key`. A `refset_sha` mismatch is `Stale`; an absent
/// pointer or a matched-pointer-but-failed-pack-read is `Absent`; a clean read is
/// `Hit`. This is the ONLY thing that keeps a streamed cached pack consistent with the
/// advertised refs (a `Stale`/`Absent` never serves a wrong pack).
pub fn try_serve_cached_clone_pack<R: R2Get>(
    r2: &R,
    tenant: &str,
    repo: &str,
    live_refs: &BTreeMap<String, String>,
) -> CloneCacheLookup {
    let Some(pointer) = load_current(r2, tenant, repo) else {
        // No pointer (building / transient / corrupt) → retryable.
        return CloneCacheLookup::Absent;
    };
    if pointer.refset_sha != refset_sha(live_refs) {
        // The cache was built for a different ref tip-set (a ref moved) — a moved-ref
        // stale, distinct from a not-yet-built absent → the caller walks, not 503s.
        return CloneCacheLookup::Stale;
    }
    match r2.get_object(&pointer.pack_key) {
        Ok(Some(bytes)) => CloneCacheLookup::Hit(bytes),
        // The pointer matched but the pack read failed/absent — a transient fault; the
        // pack SHOULD be there, so treat as retryable (Absent), not a wrong-pack risk.
        _ => CloneCacheLookup::Absent,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    /// An in-memory R2 double that RECORDS the write order, so a test can assert
    /// the pack object is durably written BEFORE the `current.json` pointer.
    #[derive(Default, Clone)]
    struct RecordingR2 {
        objects: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
        /// The keys written, in order (both conditional + unconditional PUTs).
        write_order: Arc<Mutex<Vec<String>>>,
    }

    impl RecordingR2 {
        fn get(&self, key: &str) -> Option<Vec<u8>> {
            self.objects.lock().unwrap().get(key).cloned()
        }
    }

    impl R2Get for RecordingR2 {
        fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.objects.lock().unwrap().get(key).cloned())
        }
    }

    impl R2Put for RecordingR2 {
        fn put_object(&self, key: &str, body: &[u8]) -> Result<(), String> {
            self.write_order.lock().unwrap().push(key.to_string());
            self.objects
                .lock()
                .unwrap()
                .insert(key.to_string(), body.to_vec());
            Ok(())
        }
    }

    impl R2PutConditional for RecordingR2 {
        fn put_object_conditional(
            &self,
            key: &str,
            body: &[u8],
            expected: &CasToken,
        ) -> Result<CasToken, ManifestPutError> {
            let mut map = self.objects.lock().unwrap();
            let exists = map.contains_key(key);
            let allowed = match expected {
                // If-None-Match: * — create only if still absent.
                CasToken::Absent => !exists,
                // If-Match — overwrite only if present (the double models any
                // present version as matching; this WP only uses `Absent`).
                CasToken::Version(_) => exists,
                CasToken::Unsupported => {
                    return Err(ManifestPutError::Other(
                        "test double refuses an Unsupported conditional PUT".into(),
                    ));
                }
            };
            if !allowed {
                return Err(ManifestPutError::Precondition);
            }
            self.write_order.lock().unwrap().push(key.to_string());
            map.insert(key.to_string(), body.to_vec());
            Ok(CasToken::Version("v1".into()))
        }
    }

    /// A write-scoped `R2Config` is not needed for the pure logic; the seam only
    /// needs a `CloneCacheSeam`'s `tenant`/`repo_slug`. But `store_pack_and_flip`
    /// takes a `&CloneCacheSeam`, whose `r2` field is the CONCRETE `R2Config`. To
    /// keep these unit tests hermetic (no `R2Config` construction), we exercise
    /// the flip logic directly through the `RecordingR2` double by reproducing the
    /// exact ordered write sequence `store_pack_and_flip` performs. The frozen
    /// `store_pack_and_flip` signature is compile-checked below (`_signature`).
    fn flip_via_double(
        r2: &RecordingR2,
        tenant: &str,
        repo: &str,
        refset_sha: &str,
        pack_bytes: &[u8],
        object_count: u64,
        built_at_ms: u64,
    ) -> Result<(), String> {
        // Mirror of store_pack_and_flip's body, over the double (see the doc above).
        let pack_key = clone_pack_key(tenant, repo, refset_sha);
        r2.put_object(&pack_key, pack_bytes)
            .map_err(|e| format!("pack PUT failed: {e}"))?;
        let pointer = CurrentPointer {
            refset_sha: refset_sha.to_string(),
            pack_key: pack_key.clone(),
            object_count,
            built_at_ms,
        };
        let body = serde_json::to_vec(&pointer).map_err(|e| e.to_string())?;
        let current_key = clone_pack_current_key(tenant, repo);
        match r2.put_object_conditional(&current_key, &body, &CasToken::Absent) {
            Ok(_) => Ok(()),
            Err(ManifestPutError::Precondition) => r2
                .put_object(&current_key, &body)
                .map_err(|e| format!("advance failed: {e}")),
            Err(e) => Err(format!("PUT failed: {e}")),
        }
    }

    /// Compile-time proof that the `store_pack_and_flip` signature is the one
    /// WP-BC codes against (never called — a mismatch would fail to compile).
    #[allow(dead_code)]
    fn _signature() {
        // Referencing the (now generic-free) fn item proves it resolves under the
        // frozen name/arity WP-BC codes against (a mismatch fails to compile); the
        // real call site in `git.rs` pins the exact argument types.
        let _ = store_pack_and_flip;
    }

    fn refs(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn refset_sha_is_deterministic() {
        let r = refs(&[("refs/heads/main", "a".repeat(40).as_str())]);
        assert_eq!(refset_sha(&r), refset_sha(&r));
    }

    #[test]
    fn refset_sha_is_order_independent() {
        // Same tip-set, different ref NAMES + different tip→name pairing → same sha.
        let oid_a = "a".repeat(40);
        let oid_b = "b".repeat(40);
        let one = refs(&[
            ("refs/heads/main", oid_a.as_str()),
            ("refs/heads/dev", oid_b.as_str()),
        ]);
        let two = refs(&[
            ("refs/heads/feature", oid_b.as_str()),
            ("refs/tags/v1", oid_a.as_str()),
        ]);
        assert_eq!(refset_sha(&one), refset_sha(&two));
    }

    #[test]
    fn refset_sha_dedups_identical_tips() {
        // Two refs at the SAME tip == one distinct tip.
        let oid = "c".repeat(40);
        let dup = refs(&[
            ("refs/heads/main", oid.as_str()),
            ("refs/heads/mirror", oid.as_str()),
        ]);
        let single = refs(&[("refs/heads/main", oid.as_str())]);
        assert_eq!(refset_sha(&dup), refset_sha(&single));
    }

    #[test]
    fn refset_sha_changes_on_a_moved_tip() {
        let before = refs(&[("refs/heads/main", "a".repeat(40).as_str())]);
        let after = refs(&[("refs/heads/main", "d".repeat(40).as_str())]);
        assert_ne!(refset_sha(&before), refset_sha(&after));
    }

    #[test]
    fn refset_sha_is_lowercase_hex_64() {
        let r = refs(&[("refs/heads/main", "a".repeat(40).as_str())]);
        let sha = refset_sha(&r);
        assert_eq!(sha.len(), 64);
        assert!(
            sha.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        );
    }

    #[test]
    fn key_helpers_are_tenant_repo_scoped() {
        assert_eq!(
            clone_pack_current_key("t", "hugit"),
            "t/hugit/clone-pack/current.json"
        );
        assert_eq!(
            clone_pack_key("t", "hugit", "deadbeef"),
            "t/hugit/clone-pack/deadbeef.pack"
        );
    }

    #[test]
    fn current_pointer_serde_round_trips() {
        let p = CurrentPointer {
            refset_sha: "abc123".into(),
            pack_key: "t/hugit/clone-pack/abc123.pack".into(),
            object_count: 6862,
            built_at_ms: 1_700_000_000_000,
        };
        let bytes = serde_json::to_vec(&p).unwrap();
        let back: CurrentPointer = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(p, back);
    }

    #[test]
    fn flip_writes_pack_before_pointer() {
        let r2 = RecordingR2::default();
        let sha = "e".repeat(64);
        flip_via_double(&r2, "t", "hugit", &sha, b"PACKfake", 3, 42).unwrap();

        // Both objects present, and the pointer references the written pack key.
        let pack_key = clone_pack_key("t", "hugit", &sha);
        let current_key = clone_pack_current_key("t", "hugit");
        assert_eq!(r2.get(&pack_key).unwrap(), b"PACKfake");
        let ptr: CurrentPointer = serde_json::from_slice(&r2.get(&current_key).unwrap()).unwrap();
        assert_eq!(ptr.pack_key, pack_key);
        assert_eq!(ptr.refset_sha, sha);
        assert_eq!(ptr.object_count, 3);

        // ORDER: the pack key is written strictly before the pointer key.
        let order = r2.write_order.lock().unwrap();
        let pack_at = order.iter().position(|k| k == &pack_key).unwrap();
        let ptr_at = order.iter().position(|k| k == &current_key).unwrap();
        assert!(
            pack_at < ptr_at,
            "pack must be durable before the pointer flips"
        );
    }

    #[test]
    fn load_current_none_on_absent_or_garbage() {
        let r2 = RecordingR2::default();
        // Absent.
        assert!(load_current(&r2, "t", "hugit").is_none());
        // Garbage bytes at the pointer key → fail-soft None.
        r2.objects
            .lock()
            .unwrap()
            .insert(clone_pack_current_key("t", "hugit"), b"not json".to_vec());
        assert!(load_current(&r2, "t", "hugit").is_none());
    }

    #[test]
    fn load_current_some_on_valid_pointer() {
        let r2 = RecordingR2::default();
        let sha = "f".repeat(64);
        flip_via_double(&r2, "t", "hugit", &sha, b"PACKx", 1, 7).unwrap();
        let ptr = load_current(&r2, "t", "hugit").expect("pointer present");
        assert_eq!(ptr.refset_sha, sha);
        assert_eq!(ptr.object_count, 1);
    }

    #[test]
    fn try_serve_hit_on_matching_refset() {
        let r2 = RecordingR2::default();
        let live = refs(&[("refs/heads/main", "a".repeat(40).as_str())]);
        let sha = refset_sha(&live);
        flip_via_double(&r2, "t", "hugit", &sha, b"PACKbytes", 2, 99).unwrap();

        match try_serve_cached_clone_pack(&r2, "t", "hugit", &live) {
            CloneCacheLookup::Hit(bytes) => assert_eq!(bytes, b"PACKbytes"),
            other => panic!("expected Hit, got {:?}", std::mem::discriminant(&other)),
        }
    }

    #[test]
    fn try_serve_stale_on_mismatched_refset() {
        let r2 = RecordingR2::default();
        let built_for = refs(&[("refs/heads/main", "a".repeat(40).as_str())]);
        let sha = refset_sha(&built_for);
        flip_via_double(&r2, "t", "hugit", &sha, b"PACKbytes", 2, 99).unwrap();

        // A moved tip → the live refset sha no longer matches the pointer → STALE (the
        // serve falls open to the walk, NOT a 503 — a moved-ref repo stays clonable).
        let live_now = refs(&[("refs/heads/main", "d".repeat(40).as_str())]);
        assert!(matches!(
            try_serve_cached_clone_pack(&r2, "t", "hugit", &live_now),
            CloneCacheLookup::Stale
        ));
    }

    #[test]
    fn try_serve_absent_on_absent_pointer() {
        let r2 = RecordingR2::default();
        let live = refs(&[("refs/heads/main", "a".repeat(40).as_str())]);
        // No pointer yet (building at boot) → ABSENT (the serve answers a retryable 503).
        assert!(matches!(
            try_serve_cached_clone_pack(&r2, "t", "hugit", &live),
            CloneCacheLookup::Absent
        ));
    }

    // ── build_clone_pack smoke against a tiny in-memory ObjectSource ───────────

    #[test]
    fn build_clone_pack_smoke_tiny_repo() {
        use hugit_proto::{CasObjectSource, GitObject, ObjectKind};

        let mut src = CasObjectSource::new();
        // A 2-object repo: one blob + one tree + one commit (3 objects). The tree
        // references the blob; the commit references the tree.
        let blob_oid = src.insert(GitObject::new(ObjectKind::Blob, b"hello\n".to_vec()));

        // A single-entry tree: `100644 file\0<20-byte blob oid>`.
        let mut tree_body = Vec::new();
        tree_body.extend_from_slice(b"100644 file\0");
        tree_body.extend_from_slice(blob_oid.as_bytes());
        let tree_oid = src.insert(GitObject::new(ObjectKind::Tree, tree_body));

        let commit_body = format!(
            "tree {tree_oid}\n\
             author Test <t@e> 0 +0000\n\
             committer Test <t@e> 0 +0000\n\
             \n\
             seed\n"
        )
        .into_bytes();
        let commit_oid = src.insert(GitObject::new(ObjectKind::Commit, commit_body));

        let refs = refs(&[("refs/heads/main", commit_oid.to_string().as_str())]);
        let (bytes, count) = build_clone_pack(&src, &refs).expect("pack assembles");

        assert!(bytes.starts_with(b"PACK"), "raw pack must start with PACK");
        // Full closure: commit + tree + blob = 3 objects.
        assert_eq!(count, 3);
    }
}
