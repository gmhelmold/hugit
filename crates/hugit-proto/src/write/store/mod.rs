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

    #[test]
    fn raw_push_payload_has_no_provenance_field() {
        let p = raw_push_payload("refs/heads/main", "deadbeef");
        assert_eq!(p, r#"{"ref":"refs/heads/main","target":"deadbeef"}"#);
        assert!(!p.contains("intent"));
    }
}
