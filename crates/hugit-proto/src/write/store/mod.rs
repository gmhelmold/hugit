//! Object → CAS persistence and ref-update → D1 event append (the push WRITE
//! sink).
//!
//! This module is the persistence half of the receive-pack write path. It
//! consumes two frozen surfaces and adds nothing to either:
//!
//! - the **CoreLink CAS** object surface — modelled here as the [`Cas`] trait
//!   (a content-addressed object put/get keyed by git oid). CoreLink owns the
//!   real client; hugit is a CAS *tenant* and makes zero server-side changes.
//!   v0 ships an [`InMemoryCas`] so the round-trip is provable without a live
//!   tenant.
//! - the **D1 event log** ([`hugit_refstore::log::EventLog`]) — the append-only,
//!   hash-chained source of truth. A ref update is recorded by *appending* a
//!   raw-push event; refs stay a derived view and the log is never rewritten.
//!
//! # Raw push ≠ intent (the source-of-truth bar, D3⑤ leg)
//!
//! A raw `git push` carries **no provenance**. It is recorded with one of the
//! frozen [`RAW_PUSH_KINDS`](hugit_refstore::intent::RAW_PUSH_KINDS)
//! (`ref.update` / `ref.delete`) and a payload that carries *only*
//! `{"ref", "target"}` — **never** an `intent_id`, and never the
//! [`INTENT_LANDED_KIND`](hugit_refstore::intent::INTENT_LANDED_KIND).
//! Synthesising an intent for a raw push is structurally impossible here: this
//! module emits only [`ExternalChangeKind`](hugit_refstore::intent::ExternalChangeKind)
//! events via the typed shim — it has no code path that emits `intent.landed`.

use hugit_refstore::intent::ExternalChangeKind;
use hugit_refstore::log::EventLog;
use std::collections::BTreeMap;

use crate::write::json_str;

/// A git object id (40-char lowercase hex SHA-1), the CAS key for one object.
pub type Oid = String;

/// One git object as it lives in the CAS: its oid and its on-disk loose bytes
/// (the zlib-compressed, content-addressed loose-object representation git
/// itself writes — storing this verbatim makes the clone-back byte-identical).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CasObject {
    /// The object id (CAS key).
    pub oid: Oid,
    /// The loose-object bytes exactly as git serialises them on disk.
    pub bytes: Vec<u8>,
}

/// The CoreLink CAS object surface, content-addressed by git oid.
///
/// This is the *frozen external* surface hugit consumes as a tenant; the trait
/// is the seam, not a reimplementation of CoreLink. Object storage is
/// idempotent: putting the same oid twice is a no-op (content addressing means
/// equal oid ⇒ equal bytes).
pub trait Cas {
    /// Store one object. Idempotent on oid.
    fn put(&mut self, obj: &CasObject);
    /// Fetch one object's bytes by oid, if present.
    fn get(&self, oid: &str) -> Option<Vec<u8>>;
    /// Whether the CAS already holds this oid.
    fn contains(&self, oid: &str) -> bool {
        self.get(oid).is_some()
    }
}

/// An in-memory [`Cas`] for v0 / tests. Keeps the seam honest without a live
/// CoreLink tenant.
#[derive(Debug, Clone, Default)]
pub struct InMemoryCas {
    objects: BTreeMap<Oid, Vec<u8>>,
}

impl InMemoryCas {
    /// A fresh, empty CAS.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of distinct objects held.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether the CAS holds no objects.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// All oids currently held, in sorted order (read-only view).
    pub fn oids(&self) -> impl Iterator<Item = &Oid> {
        self.objects.keys()
    }
}

impl Cas for InMemoryCas {
    fn put(&mut self, obj: &CasObject) {
        // Content-addressed ⇒ idempotent. entry().or_insert keeps it a no-op on
        // a re-put of an oid already present.
        self.objects
            .entry(obj.oid.clone())
            .or_insert_with(|| obj.bytes.clone());
    }

    fn get(&self, oid: &str) -> Option<Vec<u8>> {
        self.objects.get(oid).cloned()
    }
}

/// Store a batch of objects into the CAS. Idempotent per object.
pub fn store_objects(cas: &mut dyn Cas, objects: &[CasObject]) {
    for obj in objects {
        cas.put(obj);
    }
}

/// Inflate a [`CasObject`]'s verbatim **zlib-compressed** loose bytes to git's
/// **uncompressed loose framing** — `"<kind> <len>\0<body>"`.
///
/// A git loose object on disk is `zlib(<kind> <len>\0<body>)`; `receive_pack`
/// captures exactly those compressed bytes ([`CasObject::bytes`]). The CoreLink
/// CAS, however, stores the **uncompressed** framing (`hugit-serve`'s
/// `encode_loose`) and content-addresses it with `blake3(framing)`. So a push
/// landing into the CAS MUST inflate first: inflating the verbatim loose bytes
/// yields *exactly* that pre-image (git's loose body IS the framing), so the
/// pushed object is byte-identical to one the CAS read side would serve — the
/// load-bearing correctness invariant of CAS-mode push.
///
/// Returns the inflated framing bytes; the caller computes `blake3` over them.
/// `Err` (an `io::Error` from the zlib stream) surfaces a corrupt/non-zlib input
/// so the adapter can fail the push closed rather than store garbage.
pub fn inflate_loose_framing(obj: &CasObject) -> std::io::Result<Vec<u8>> {
    use std::io::Read as _;
    let mut decoder = flate2::read::ZlibDecoder::new(obj.bytes.as_slice());
    let mut framing = Vec::new();
    decoder.read_to_end(&mut framing)?;
    Ok(framing)
}

/// Record a raw-push ref *update* as an append-only D1 event and return the
/// freshly-appended [`EventRecord`].
///
/// The payload is canonical raw-push JSON `{"ref":<name>,"target":<oid>}` — it
/// carries **no** `intent_id` and the event kind is `ref.update`, so the
/// intent altitude ([`hugit_refstore::intent`]) reads it as an *external
/// change*, never an intent. This is the only ref-write primitive in the push
/// path; it appends, it never rewrites.
pub fn record_ref_update(
    log: &mut EventLog,
    principal_chain: Vec<String>,
    ref_name: &str,
    target_oid: &str,
    recorded_at: u64,
) -> hugit_contracts::event_record::EventRecord {
    // C4-F1: the type-level external-change shim replaces the bare `log.append`.
    // `ExternalChangeKind::RefUpdate` can only ever map to `ref.update` — the
    // raw door (`EventLog::append`) is now `pub(crate)` and unreachable here, so
    // this recorder is *structurally incapable* of emitting an intent kind (the
    // prior `debug_assert_ne!` is now a compile-time guarantee).
    let payload = raw_push_payload(ref_name, target_oid);
    log.append_external_change(
        ExternalChangeKind::RefUpdate,
        principal_chain,
        payload,
        recorded_at,
    )
}

/// Record a raw-push ref *delete* as an append-only D1 event.
pub fn record_ref_delete(
    log: &mut EventLog,
    principal_chain: Vec<String>,
    ref_name: &str,
    recorded_at: u64,
) -> hugit_contracts::event_record::EventRecord {
    let payload = format!(r#"{{"ref":{}}}"#, json_str(ref_name));
    log.append_external_change(
        ExternalChangeKind::RefDelete,
        principal_chain,
        payload,
        recorded_at,
    )
}

/// Canonical raw-push update payload: `{"ref":<name>,"target":<oid>}`.
///
/// Deliberately carries no provenance field. Exposed so tests can assert the
/// exact bytes that land on the log.
pub fn raw_push_payload(ref_name: &str, target_oid: &str) -> String {
    format!(
        r#"{{"ref":{},"target":{}}}"#,
        json_str(ref_name),
        json_str(target_oid)
    )
}

/// A [`Cas`] backed by a real git object directory (`<git-dir>/objects/<xx>/<rest>`).
///
/// `CasObject.bytes` are git's verbatim on-disk loose-object bytes — the SAME
/// representation `ScratchOdb` reads back after `git unpack-objects` — so a `put`
/// just writes them to `objects/<first-2>/<rest>` and a `get` reads that file.
/// This is the write-side analogue of the read path's git-dir object source: a
/// push lands loose objects exactly where a git-dir read serves them. Used by the
/// receive-pack serve wiring in `HUGIT_SERVE_GIT_DIR` mode and the hermetic push
/// test; the live CoreLink-CAS (blake3 + `oid-index`) adapter is a later piece.
pub struct GitDirCas {
    objects_dir: std::path::PathBuf,
}

impl GitDirCas {
    /// Wrap `<git_dir>/objects`. Errors if that directory does not exist — a push
    /// into a non-git dir must fail closed, not silently create a broken store.
    pub fn new(git_dir: impl AsRef<std::path::Path>) -> Result<Self, String> {
        let objects_dir = git_dir.as_ref().join("objects");
        if !objects_dir.is_dir() {
            return Err(format!("not a git object dir: {}", objects_dir.display()));
        }
        Ok(Self { objects_dir })
    }

    /// The `objects/<xx>/<rest>` path for an oid, or `None` for a too-short oid.
    fn object_path(&self, oid: &str) -> Option<std::path::PathBuf> {
        if oid.len() < 3 {
            return None;
        }
        let (dir, rest) = oid.split_at(2);
        Some(self.objects_dir.join(dir).join(rest))
    }
}

impl Cas for GitDirCas {
    fn put(&mut self, obj: &CasObject) {
        // Content-addressed ⇒ idempotent: an already-present loose object is
        // byte-identical, so a re-put is skipped. A short/garbage oid is dropped
        // (the unpack step only ever yields full oids). The `Cas` trait's `put`
        // is infallible by contract; the live W3 adapter will need fallible
        // storage (a transport can fail) — tracked in the receive-pack design.
        let Some(path) = self.object_path(&obj.oid) else {
            return;
        };
        if path.exists() {
            return;
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, &obj.bytes);
    }

    fn get(&self, oid: &str) -> Option<Vec<u8>> {
        std::fs::read(self.object_path(oid)?).ok()
    }
}

#[cfg(test)]
mod tests {
    use hugit_refstore::intent::{INTENT_LANDED_KIND, RAW_PUSH_KINDS};

    use super::*;
    use crate::write::external::REF_UPDATE_KIND;

    #[test]
    fn cas_put_is_idempotent_on_oid() {
        let mut cas = InMemoryCas::new();
        let o = CasObject {
            oid: "abc".into(),
            bytes: vec![1, 2, 3],
        };
        cas.put(&o);
        cas.put(&o);
        assert_eq!(cas.len(), 1);
        assert_eq!(cas.get("abc"), Some(vec![1, 2, 3]));
        assert!(cas.contains("abc"));
    }

    #[test]
    fn ref_update_is_a_raw_push_kind_never_an_intent() {
        assert!(RAW_PUSH_KINDS.contains(&REF_UPDATE_KIND));
        assert_ne!(REF_UPDATE_KIND, INTENT_LANDED_KIND);
        let mut log = EventLog::new();
        let rec = record_ref_update(
            &mut log,
            vec!["user:alice".into()],
            "refs/heads/main",
            "fe8f5f1e013d57d0629ff3999a71986ffc2b05fb",
            1_717_000_000_000,
        );
        assert_eq!(rec.kind, REF_UPDATE_KIND);
        assert!(!rec.payload.contains("intent_id"));
        assert!(!rec.payload.contains(INTENT_LANDED_KIND));
    }

    /// Build a verbatim git loose object: `zlib(<kind> <len>\0<body>)` — the EXACT
    /// on-disk bytes `receive_pack` reads back via `read_loose`.
    fn zlib_loose(kind: &str, body: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let mut framing = format!("{kind} {}\0", body.len()).into_bytes();
        framing.extend_from_slice(body);
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&framing).expect("deflate");
        enc.finish().expect("finish")
    }

    #[test]
    fn inflate_loose_framing_recovers_the_uncompressed_pre_image() {
        let body = b"the file contents that git stored loose";
        let compressed = zlib_loose("blob", body);
        // The verbatim loose bytes are genuinely compressed (not the framing itself).
        let want_framing = {
            let mut f = format!("blob {}\0", body.len()).into_bytes();
            f.extend_from_slice(body);
            f
        };
        assert_ne!(
            compressed, want_framing,
            "input is zlib-compressed, not framing"
        );

        let obj = CasObject {
            oid: "fe8f5f1e013d57d0629ff3999a71986ffc2b05fb".into(),
            bytes: compressed,
        };
        let framing = inflate_loose_framing(&obj).expect("inflates");
        // Inflating the verbatim loose bytes yields git's loose pre-image exactly —
        // the byte string the CoreLink CAS stores + blake3-keys (the CAS-push invariant).
        assert_eq!(framing, want_framing);
        assert_eq!(&framing[..7], b"blob 39");
    }

    #[test]
    fn inflate_loose_framing_rejects_non_zlib_bytes() {
        let obj = CasObject {
            oid: "1111111111111111111111111111111111111111".into(),
            bytes: vec![0xde, 0xad, 0xbe, 0xef], // not a zlib stream
        };
        assert!(
            inflate_loose_framing(&obj).is_err(),
            "corrupt input fails closed"
        );
    }

    #[test]
    fn raw_push_payload_has_no_provenance_field() {
        let p = raw_push_payload("refs/heads/main", "deadbeef");
        assert_eq!(p, r#"{"ref":"refs/heads/main","target":"deadbeef"}"#);
        assert!(!p.contains("intent"));
    }

    /// A unique scratch git-object dir (`<tmp>/<unique>/objects`) for the
    /// `GitDirCas` tests. Returns the git-dir root (parent of `objects`).
    fn scratch_git_dir(tag: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let root = std::env::temp_dir().join(format!("gitdircas-{tag}-{nanos}"));
        std::fs::create_dir_all(root.join("objects")).expect("mk objects dir");
        root
    }

    #[test]
    fn git_dir_cas_round_trips_a_loose_object() {
        let root = scratch_git_dir("rt");
        let mut cas = GitDirCas::new(&root).expect("git dir exists");
        // A full 40-hex oid; bytes stand in for the verbatim loose-object content.
        let oid = "fe8f5f1e013d57d0629ff3999a71986ffc2b05fb";
        let obj = CasObject {
            oid: oid.into(),
            bytes: vec![0x78, 0x01, 9, 9, 9], // looks like a zlib loose blob
        };
        assert!(!cas.contains(oid));
        cas.put(&obj);
        assert!(cas.contains(oid), "present after put");
        assert_eq!(
            cas.get(oid),
            Some(obj.bytes.clone()),
            "byte-identical read-back"
        );
        // It landed at objects/<xx>/<rest> — the exact path a git-dir read serves.
        assert!(
            root.join("objects/fe/8f5f1e013d57d0629ff3999a71986ffc2b05fb")
                .exists()
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_dir_cas_put_is_idempotent() {
        let root = scratch_git_dir("idem");
        let mut cas = GitDirCas::new(&root).expect("git dir");
        let oid = "1111111111111111111111111111111111111111";
        let obj = CasObject {
            oid: oid.into(),
            bytes: vec![1, 2, 3],
        };
        cas.put(&obj);
        cas.put(&obj); // a re-put is a no-op (content-addressed) — must not error/change
        assert_eq!(cas.get(oid), Some(vec![1, 2, 3]));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_dir_cas_missing_object_is_none() {
        let root = scratch_git_dir("miss");
        let cas = GitDirCas::new(&root).expect("git dir");
        assert_eq!(cas.get("2222222222222222222222222222222222222222"), None);
        assert!(!cas.contains("2222222222222222222222222222222222222222"));
        assert_eq!(cas.get("x"), None, "too-short oid is None, not a panic");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_dir_cas_refuses_a_non_git_dir() {
        let bogus = std::env::temp_dir().join("gitdircas-nonexistent-xyzzy-dir");
        let _ = std::fs::remove_dir_all(&bogus);
        assert!(
            GitDirCas::new(&bogus).is_err(),
            "no objects/ dir → fail closed"
        );
    }
}
