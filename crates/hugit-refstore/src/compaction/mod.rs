//! Compaction: bound the hot log by sealing+offloading a cold prefix.
//!
//! Durable-Object storage is capped, so the hot in-DO log must stay bounded.
//! Compaction takes the oldest records (a chain prefix), seals them into a
//! [`crate::coldtier::ColdRange`], offloads that range to a [`ColdStore`], and
//! retains only the most-recent `hot_bound` records as the **hot remainder**.
//!
//! # Replay-equivalence (acceptance item ③)
//!
//! Compaction is **replay-equivalent**: the ref state derived from
//! (cold tier + hot remainder) after compaction is byte-for-byte identical to
//! replaying the full uncompacted log. Compaction **never rewrites or drops
//! history** — it *relocates* a verified prefix; the records keep their exact
//! `seq`/`prev_hash`/`this_hash`, so the recombined chain re-verifies and
//! re-projects identically. The hot remainder is *bounded*; nothing is lost.
//!
//! Only a verified prefix is ever sealed: compaction re-runs the frozen D1a
//! [`verify_chain`] over the whole log first and refuses (fail-closed) to
//! compact a tampered log.
//!
//! Note: the D1a [`EventLog`] is a 0-based, gap-free chain and is *not* mutated
//! here (it is D1a's frozen substrate). Compaction reads it and produces a
//! [`HotRemainder`] — a `seq`-preserving suffix that recombines with the cold
//! tier into the original record sequence.

use crate::coldtier::{ColdRange, ColdStore};
use crate::log::EventLog;
use crate::tamper::{TamperError, verify_chain};
use hugit_contracts::event_record::EventRecord;

/// The records left hot after compaction — a contiguous suffix of the chain,
/// holding each record's original `seq`/`prev_hash`/`this_hash` verbatim.
///
/// This is deliberately *not* a D1a [`EventLog`] (which is 0-based and gap-free
/// by invariant): the hot remainder begins at `start_seq`, which is non-zero
/// once a prefix has been offloaded. The remainder recombines with the cold
/// tier — never alone — into the full original chain.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct HotRemainder {
    records: Vec<EventRecord>,
}

impl HotRemainder {
    /// The retained records, in chain order.
    pub fn records(&self) -> &[EventRecord] {
        &self.records
    }

    /// Number of records held hot.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether no records are held hot.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// `seq` of the first retained record, if any.
    pub fn start_seq(&self) -> Option<u64> {
        self.records.first().map(|r| r.seq)
    }
}

/// Outcome of a compaction pass.
#[derive(Debug, Clone, PartialEq)]
pub struct CompactionReport {
    /// `seq` of the first record sealed to the cold tier (inclusive).
    pub sealed_start_seq: u64,
    /// One past the last `seq` sealed (exclusive).
    pub sealed_end_seq: u64,
    /// Records moved to the cold tier in this pass.
    pub sealed_count: usize,
    /// The records remaining hot after compaction.
    pub hot: HotRemainder,
}

/// Error from a compaction pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompactionError {
    /// The log failed chain verification; compaction refuses to seal a tampered
    /// prefix (fail-closed).
    Tamper(TamperError),
}

impl std::fmt::Display for CompactionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CompactionError::Tamper(e) => write!(f, "compaction refused: {e}"),
        }
    }
}

impl std::error::Error for CompactionError {}

impl From<TamperError> for CompactionError {
    fn from(e: TamperError) -> Self {
        CompactionError::Tamper(e)
    }
}

/// Compact `log` so at most `hot_bound` records remain hot, offloading the sealed
/// prefix to `cold`.
///
/// If the log already holds `hot_bound` records or fewer, this is a no-op (no
/// range sealed; the whole log is returned as the hot remainder). Otherwise the
/// oldest `len - hot_bound` records are sealed into one [`ColdRange`], `put` to
/// the cold store, and the rest are returned as the [`HotRemainder`].
///
/// The chain is re-verified first; a tampered log is refused (fail-closed). The
/// retained hot records keep their original `seq`/`prev_hash`/`this_hash`, so
/// the cold range and the hot remainder recombine into the exact original chain
/// — the replay-equivalence guarantee (③). The D1a [`EventLog`] is read-only
/// here.
pub fn compact<S: ColdStore>(
    log: &EventLog,
    hot_bound: usize,
    cold: &mut S,
) -> Result<CompactionReport, CompactionError> {
    // Fail closed on a tampered log: never seal a corrupt prefix.
    verify_chain(log.records())?;

    let total = log.len();
    let all = log.records().to_vec();

    if total <= hot_bound {
        return Ok(CompactionReport {
            sealed_start_seq: 0,
            sealed_end_seq: 0,
            sealed_count: 0,
            hot: HotRemainder { records: all },
        });
    }

    let seal_count = total - hot_bound;
    let (prefix, remainder) = all.split_at(seal_count);

    let range = ColdRange::from_records(prefix.to_vec());
    let sealed_start_seq = range.start_seq;
    let sealed_end_seq = range.end_seq;
    cold.put(range);

    Ok(CompactionReport {
        sealed_start_seq,
        sealed_end_seq,
        sealed_count: seal_count,
        hot: HotRemainder {
            records: remainder.to_vec(),
        },
    })
}
