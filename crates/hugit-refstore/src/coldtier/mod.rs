//! Cold-tier storage of sealed log ranges (designed in from day 1).
//!
//! Durable-Object storage is capped, so the hot log cannot grow without bound.
//! [`crate::compaction`] seals a prefix of the log into a **cold range** and
//! offloads it here; [`crate::recovery`] reads cold ranges back to rebuild ref
//! state after a hot-DO loss. This module is the *storage abstraction* for those
//! ranges.
//!
//! # Storage is content-addressed, R2 is a binding
//!
//! A cold range is serialized to a deterministic byte blob and stored under a
//! key derived from the range it covers. In production the [`ColdStore`] impl is
//! backed by CoreLink's R2 cold tier (object `put`/`get`/`list`) — a *frozen
//! external API*, consumed as a tenant, zero server-side change. R2 is therefore
//! a **configuration/binding concern**, not a requirement for this WP to be
//! correct: the semantics are proven against [`InMemoryColdStore`], the local
//! storage abstraction, and any `put`/`get`/`list` backend (R2 included)
//! satisfies the same [`ColdStore`] trait.
//!
//! Nothing here ever rewrites or drops history — a cold range is an exact,
//! verifiable copy of the records it sealed (relocated, never mutated).

use hugit_contracts::event_record::EventRecord;
use std::collections::BTreeMap;

/// A contiguous, sealed run of records offloaded out of the hot log.
///
/// A cold range covers the half-open `seq` interval `[start_seq, end_seq)` and
/// carries the records verbatim, in chain order. It is the unit of cold-tier
/// storage and of recovery replay.
#[derive(Debug, Clone, PartialEq)]
pub struct ColdRange {
    /// First `seq` covered by this range (inclusive).
    pub start_seq: u64,
    /// One past the last `seq` covered (exclusive); `end_seq - start_seq` records.
    pub end_seq: u64,
    /// The sealed records, in chain order, exactly as they were in the hot log.
    pub records: Vec<EventRecord>,
}

impl ColdRange {
    /// Build a cold range from a contiguous record slice.
    ///
    /// `records` must be non-empty and gap-free in `seq` (the caller —
    /// [`crate::compaction`] — only ever seals a verified hot-log prefix). The
    /// `[start_seq, end_seq)` bounds are derived from the records themselves.
    pub fn from_records(records: Vec<EventRecord>) -> Self {
        let start_seq = records.first().map(|r| r.seq).unwrap_or(0);
        let end_seq = records.last().map(|r| r.seq + 1).unwrap_or(start_seq);
        Self {
            start_seq,
            end_seq,
            records,
        }
    }

    /// Number of records sealed in this range.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether this range carries no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The content-addressed cold-tier key for this range.
    ///
    /// Derived deterministically from the covered interval and the chain head it
    /// ends on, so the same sealed range always maps to the same key (idempotent
    /// `put`) and recovery can `list` ranges in `seq` order.
    pub fn key(&self) -> String {
        let head = self
            .records
            .last()
            .map(|r| r.this_hash.as_str())
            .unwrap_or("genesis");
        format!(
            "coldrange/{:020}-{:020}/{head}",
            self.start_seq, self.end_seq
        )
    }
}

/// A cold-tier object store: `put` / `get` / `list` over cold ranges.
///
/// This is the seam CoreLink's R2 cold tier plugs into. The trait deliberately
/// mirrors R2's object-store surface (put by key, get by key, list keys) so the
/// production binding is a thin adapter and the semantics proven here carry over
/// unchanged.
pub trait ColdStore {
    /// Store a cold range under its content-addressed [`ColdRange::key`].
    /// Idempotent: storing an identical range under the same key is a no-op.
    fn put(&mut self, range: ColdRange);

    /// Fetch a cold range by key, if present.
    fn get(&self, key: &str) -> Option<ColdRange>;

    /// All stored keys, sorted (so recovery can read ranges in `seq` order).
    fn list(&self) -> Vec<String>;
}

/// The local storage abstraction used to prove cold-tier semantics without a
/// live R2 binding. An ordinary in-process key→range map.
///
/// Per the WP's PARTIAL-over-fake rule, the cold tier is a *local storage
/// abstraction*; the R2 binding is config, not required for green. Any backend
/// satisfying [`ColdStore`] (R2 included) behaves identically here.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct InMemoryColdStore {
    objects: BTreeMap<String, ColdRange>,
}

impl InMemoryColdStore {
    /// A fresh, empty cold store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of cold ranges currently stored.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    /// Whether the store holds no cold ranges.
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }
}

impl ColdStore for InMemoryColdStore {
    fn put(&mut self, range: ColdRange) {
        self.objects.insert(range.key(), range);
    }

    fn get(&self, key: &str) -> Option<ColdRange> {
        self.objects.get(key).cloned()
    }

    fn list(&self) -> Vec<String> {
        self.objects.keys().cloned().collect()
    }
}
