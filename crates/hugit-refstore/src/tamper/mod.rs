//! Chain re-verification + tamper detection.
//!
//! Tamper detection (acceptance item ②) is chain re-verification: any record
//! that was **altered**, **inserted**, or **dropped** breaks the predecessor-hash
//! chain (and/or the monotonic `seq`) and is detected here. Detection fails
//! **closed** — callers (e.g. [`crate::replay::replay`]) refuse to serve a
//! derived view from a broken chain; the log is never silently repaired.
//!
//! The verifier recomputes every `this_hash` through the *same*
//! [`crate::log::compute_this_hash`] the append path uses, so producer and
//! verifier can never drift.

pub mod redaction;

use crate::log::{GENESIS_PREV_HASH, compute_this_hash};
use hugit_contracts::event_record::EventRecord;

/// A detected break in the integrity chain. Every variant names the offending
/// `seq` (position in the record vector) so the failure is diagnosable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TamperError {
    /// `seq` is not the expected gap-free, monotonic value for its position —
    /// signals an inserted or dropped record.
    SeqOutOfOrder {
        position: u64,
        expected: u64,
        got: u64,
    },
    /// A record's `prev_hash` does not equal the predecessor's `this_hash`
    /// (genesis must point at [`GENESIS_PREV_HASH`]) — a snipped/spliced chain.
    PrevHashMismatch {
        seq: u64,
        expected: String,
        got: String,
    },
    /// A record's stored `this_hash` does not match the frozen formula over its
    /// fields — the record's content was altered.
    ThisHashMismatch {
        seq: u64,
        expected: String,
        got: String,
    },
}

impl std::fmt::Display for TamperError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TamperError::SeqOutOfOrder {
                position,
                expected,
                got,
            } => write!(
                f,
                "tamper: seq out of order at position {position} (expected {expected}, got {got})"
            ),
            TamperError::PrevHashMismatch { seq, expected, got } => write!(
                f,
                "tamper: prev_hash break at seq {seq} (expected {expected}, got {got})"
            ),
            TamperError::ThisHashMismatch { seq, expected, got } => write!(
                f,
                "tamper: this_hash mismatch at seq {seq} (expected {expected}, got {got})"
            ),
        }
    }
}

impl std::error::Error for TamperError {}

/// Re-verify the full hash chain of `records` (chain order).
///
/// Checks, for each record at position `i`:
/// 1. `seq == i` (gap-free, monotonic — catches insertion/drop/reorder),
/// 2. `prev_hash` equals the predecessor's `this_hash` (genesis →
///    [`GENESIS_PREV_HASH`]) — catches splicing,
/// 3. stored `this_hash` equals the frozen formula recomputed over the record's
///    fields — catches content alteration.
///
/// `Ok(())` means the chain is intact. Any `Err` is a fail-closed tamper signal.
///
/// # Honesty caveat — UNKEYED chain (tamper-EVIDENT, not tamper-PROOF)
///
/// This chain uses an **unkeyed** SHA-256 hash (a public, deterministic function).
/// It detects **partial or incomplete tampering** — a naive byte-flip, a dropped
/// record, an out-of-order insertion — and provides ordering + append-immutability
/// once a log is published to the server. What it does NOT prevent: a writer with
/// full read+write access to the log file can recompute the chain forward (using
/// the same public formula) and produce a forged log that passes this verifier,
/// e.g. changing a reject verdict to approve. This is physics for a local file;
/// no local unkeyed crypto stops the local writer.
///
/// Cryptographic authentication against a competent rewriter is the **P2
/// server-side seam (PS-8)**: the CoreLink per-repo Durable Object event-log
/// enforces
/// server-side append-only chaining, and the transparency log provides
/// an externally-verifiable inclusion proof. These are the peer of the AC HMAC
/// seam.
/// This function's role is local partial-tamper detection — it remains correct
/// and necessary for that purpose. Logic is UNCHANGED.
pub fn verify_chain(records: &[EventRecord]) -> Result<(), TamperError> {
    // First pass: collect any in-band redaction markers (ADR-0004 leg 4). For a log with
    // NO `provenance.redaction` records — the common case, and every historical log — this
    // map is EMPTY, the redacted branch below is never entered, and verify_chain behaves
    // byte-for-byte as it always has (zero regression).
    let redactions = redaction::collect_redactions(records);
    let mut prev_this = GENESIS_PREV_HASH.to_string();

    for (i, record) in records.iter().enumerate() {
        let position = i as u64;

        // 1. monotonic, gap-free seq. (ALWAYS enforced — redaction never touches this.)
        if record.seq != position {
            return Err(TamperError::SeqOutOfOrder {
                position,
                expected: position,
                got: record.seq,
            });
        }

        // 2. linkage to the predecessor. (ALWAYS enforced — a redacted record PRESERVES its
        //    original `this_hash`, so this splice/append-immutability check is intact.)
        if record.prev_hash != prev_this {
            return Err(TamperError::PrevHashMismatch {
                seq: record.seq,
                expected: prev_this,
                got: record.prev_hash.clone(),
            });
        }

        // 3. content integrity.
        if let Some(commitment) = redactions.get(&position) {
            // REDACTED record (ADR-0004): its cleartext is gone, so the original this_hash
            // is no longer re-derivable from the (now pseudonymised) fields. Instead:
            //  (a) the stored this_hash MUST still equal the preserved original hash — this
            //      binds the frozen hash the chain links through to the redaction claim; and
            //  (b) recompute over the CURRENT (redacted) fields MUST equal the committed
            //      redacted hash — so the redacted content is itself tamper-evident.
            if record.this_hash != commitment.original_this_hash {
                return Err(TamperError::ThisHashMismatch {
                    seq: record.seq,
                    expected: commitment.original_this_hash.clone(),
                    got: record.this_hash.clone(),
                });
            }
            let recomputed_redacted = compute_this_hash(
                &record.prev_hash,
                &record.kind,
                &record.principal_chain,
                &record.payload,
                record.seq,
            );
            if recomputed_redacted != commitment.redacted_this_hash {
                return Err(TamperError::ThisHashMismatch {
                    seq: record.seq,
                    expected: commitment.redacted_this_hash.clone(),
                    got: recomputed_redacted,
                });
            }
        } else {
            // Normal record: recompute this_hash via the frozen formula (unchanged).
            let recomputed = compute_this_hash(
                &record.prev_hash,
                &record.kind,
                &record.principal_chain,
                &record.payload,
                record.seq,
            );
            if recomputed != record.this_hash {
                return Err(TamperError::ThisHashMismatch {
                    seq: record.seq,
                    expected: recomputed,
                    got: record.this_hash.clone(),
                });
            }
        }

        prev_this = record.this_hash.clone();
    }

    Ok(())
}
