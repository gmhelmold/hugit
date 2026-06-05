//! The append-only, hash-chained event log core.
//!
//! There is exactly **one** mutating primitive in this WP: [`EventLog::append`].
//! Each appended [`EventRecord`] carries the hash of its predecessor; the chain
//! is the integrity spine. Nothing is ever rewritten.
//!
//! # Frozen hash-chain formula (transcribed from `hugit-contracts`, never chosen here)
//!
//! ```text
//! this_hash = H(prev_hash ‖ kind ‖ principal_chain ‖ payload ‖ seq)
//! ```
//!
//! where (per the doc comments on [`EventRecord`] and WP-00 §"Hash chain"):
//!
//! - `H` = SHA-256, emitted as a 64-char lowercase hex digest.
//! - `‖`  = concatenation of **length-prefixed** fields, "length-prefix to make
//!   `‖` unambiguous" (WP-00). Each variable-length field is preceded by a
//!   4-byte big-endian `u32` byte-length prefix.
//! - `prev_hash`, `kind`, `payload` are length-prefixed UTF-8 strings.
//! - `principal_chain` is a sequence: a 4-byte big-endian `u32` **element count**
//!   followed by each principal as a 4-byte big-endian `u32` length-prefixed
//!   UTF-8 field. The count is mandated by the "unambiguous" rule — without it
//!   `["a","b"]`, `["ab"]` and `["a"] + payload "b"` would collide on the spine.
//! - `seq` is encoded as an 8-byte big-endian `u64` (the explicit exception to
//!   length-prefixing called out in the frozen doc).
//! - genesis `prev_hash` = 64 hex zeros ([`GENESIS_PREV_HASH`]).

use hugit_contracts::event_record::EventRecord;
use sha2::{Digest, Sha256};

/// Genesis predecessor hash: 64 hex zeros (the chain's anchor).
pub const GENESIS_PREV_HASH: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

/// Append a single length-prefixed UTF-8 field to the hash pre-image.
///
/// 4-byte big-endian `u32` byte-length prefix, then the raw UTF-8 bytes.
fn push_lp_field(buf: &mut Vec<u8>, field: &str) {
    let bytes = field.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    buf.extend_from_slice(bytes);
}

/// Compute `this_hash` for an event from the four chained inputs, exactly per
/// the frozen formula. Returns a 64-char lowercase hex SHA-256 digest.
///
/// This is the *only* place the formula is realised; [`EventLog::append`] and
/// [`crate::tamper::verify_chain`] both route through it so the producer and the
/// verifier can never drift.
pub fn compute_this_hash(
    prev_hash: &str,
    kind: &str,
    principal_chain: &[String],
    payload: &str,
    seq: u64,
) -> String {
    let mut buf: Vec<u8> = Vec::new();

    // prev_hash ‖ kind   (length-prefixed UTF-8 fields)
    push_lp_field(&mut buf, prev_hash);
    push_lp_field(&mut buf, kind);

    // principal_chain    (u32 element count, then each element length-prefixed)
    buf.extend_from_slice(&(principal_chain.len() as u32).to_be_bytes());
    for principal in principal_chain {
        push_lp_field(&mut buf, principal);
    }

    // payload            (length-prefixed UTF-8 field)
    push_lp_field(&mut buf, payload);

    // seq                (8-byte big-endian u64 — the frozen exception)
    buf.extend_from_slice(&seq.to_be_bytes());

    let digest = Sha256::digest(&buf);
    hex::encode(digest)
}

/// The append-only, hash-chained event log for one repository.
///
/// In production exactly one Durable Object owns one of these and is the single
/// writer. This type holds the in-memory chain and exposes the one mutating
/// primitive ([`append`](EventLog::append)); persistence/transport is the
/// caller's concern (D1b cold-tier offload preserves the chain semantics
/// sealed here).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EventLog {
    records: Vec<EventRecord>,
}

/// Error returned by the append path when an input violates the append-only,
/// monotonic-sequence invariant. Append never rewrites history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendError {
    /// `seq` was not exactly `len()` (the next slot in a 0-based, gap-free log).
    NonMonotonicSeq { expected: u64, got: u64 },
}

impl std::fmt::Display for AppendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AppendError::NonMonotonicSeq { expected, got } => write!(
                f,
                "non-monotonic seq: expected {expected}, got {got} (log is append-only)"
            ),
        }
    }
}

impl std::error::Error for AppendError {}

impl EventLog {
    /// A fresh, empty log.
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of records currently in the log (also the `seq` of the next append).
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the log holds no records.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The records in chain order (read-only view).
    pub fn records(&self) -> &[EventRecord] {
        &self.records
    }

    /// `this_hash` of the last record, or [`GENESIS_PREV_HASH`] for an empty log.
    /// This is the `prev_hash` the next [`append`](EventLog::append) will chain to.
    pub fn head_hash(&self) -> String {
        self.records
            .last()
            .map(|r| r.this_hash.clone())
            .unwrap_or_else(|| GENESIS_PREV_HASH.to_string())
    }

    /// Append one event to the log, computing its place in the hash chain.
    ///
    /// The new record's `seq` is the current length, its `prev_hash` is the
    /// current [`head_hash`](EventLog::head_hash), and its `this_hash` is the
    /// frozen formula over `(prev_hash, kind, principal_chain, payload, seq)`.
    /// The fully-formed [`EventRecord`] is returned (cloned) for the caller to
    /// persist / broadcast.
    ///
    /// This is the only mutating primitive in the WP. It never rewrites an
    /// existing record.
    pub fn append(
        &mut self,
        kind: impl Into<String>,
        principal_chain: Vec<String>,
        payload: impl Into<String>,
        recorded_at: u64,
    ) -> EventRecord {
        let seq = self.records.len() as u64;
        let prev_hash = self.head_hash();
        let kind = kind.into();
        let payload = payload.into();

        let this_hash = compute_this_hash(&prev_hash, &kind, &principal_chain, &payload, seq);

        let record = EventRecord {
            seq,
            prev_hash,
            this_hash,
            kind,
            principal_chain,
            payload,
            recorded_at,
        };
        self.records.push(record.clone());
        record
    }

    /// Append a pre-formed [`EventRecord`] (e.g. rehydrated from storage),
    /// enforcing the monotonic, gap-free `seq` invariant.
    ///
    /// Used when loading a persisted chain. The record's hashes are taken as
    /// given here; integrity is established separately by
    /// [`crate::tamper::verify_chain`].
    pub fn push_record(&mut self, record: EventRecord) -> Result<(), AppendError> {
        let expected = self.records.len() as u64;
        if record.seq != expected {
            return Err(AppendError::NonMonotonicSeq {
                expected,
                got: record.seq,
            });
        }
        self.records.push(record);
        Ok(())
    }
}
