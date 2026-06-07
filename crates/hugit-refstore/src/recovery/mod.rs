//! Recovery: rebuild ref state after a hot-DO loss (acceptance item ⑥).
//!
//! If the Durable Object that owns the hot log is lost, "nothing is lost" must
//! still hold at the CAS level. Recovery reconstructs the full record sequence
//! by reading the [`ColdStore`] (cold ranges, in `seq` order) and re-projects it
//! into a [`RefState`] — **replay-identical** to the pre-loss state.
//!
//! # Recovery-source precedence
//!
//! Cold tier **first**; the mirror is the **secondary** source (cf. E1's
//! substrate-loss DR). This WP does not build the mirror (that is E1) — it
//! consumes the mirror as a recovery source only, via the [`MirrorSource`] seam,
//! and asserts recoverability against the cold tier with the mirror path
//! stubbed/contracted ([`NoMirror`]).
//!
//! Recovery is **fail-closed**: the recombined chain is re-verified through the
//! frozen D1a [`verify_chain`] before any ref state is served; a broken chain
//! refuses recovery, it is never silently repaired.

use crate::coldtier::ColdStore;
use crate::replay::{RefState, ReplayError, replay_unchecked};
use crate::tamper::{TamperError, verify_chain};
use hugit_contracts::event_record::EventRecord;

/// A secondary recovery source (the mirror, owned by E1) — consumed here, never
/// built here.
///
/// Recovery falls back to this only when the cold tier cannot supply a complete,
/// gap-free chain. The trait is the contracted seam E1's mirror plugs into.
pub trait MirrorSource {
    /// Records the mirror can supply for the half-open `seq` interval
    /// `[start, end)`, in chain order. Empty if the mirror has nothing for the
    /// range.
    fn records_in(&self, start: u64, end: u64) -> Vec<EventRecord>;
}

/// The stubbed/contracted mirror used while E1 is not yet built: supplies
/// nothing. Recovery against [`NoMirror`] therefore proves recoverability from
/// the cold tier alone.
#[derive(Debug, Clone, Copy, Default)]
pub struct NoMirror;

impl MirrorSource for NoMirror {
    fn records_in(&self, _start: u64, _end: u64) -> Vec<EventRecord> {
        Vec::new()
    }
}

/// Which source(s) supplied the records a recovery used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoverySource {
    /// The full chain was rebuilt from the cold tier alone.
    ColdTier,
    /// The cold tier was incomplete; the mirror (secondary) filled the gap.
    ColdTierWithMirror,
}

/// Outcome of a recovery pass.
#[derive(Debug, Clone, PartialEq)]
pub struct Recovered {
    /// The rebuilt ref state, replay-identical to the pre-loss state.
    pub state: RefState,
    /// The fully reconstructed record chain (for re-seeding a fresh hot DO).
    pub records: Vec<EventRecord>,
    /// Which source supplied the chain.
    pub source: RecoverySource,
}

/// Error from a recovery pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryError {
    /// The reconstructed chain has a `seq` gap that neither the cold tier nor
    /// the mirror could fill — recovery cannot proceed.
    IncompleteChain { missing_seq: u64 },
    /// The reconstructed chain failed verification (fail-closed).
    Tamper(TamperError),
    /// Replaying the reconstructed chain failed (e.g. a malformed payload).
    Replay(ReplayError),
}

impl std::fmt::Display for RecoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecoveryError::IncompleteChain { missing_seq } => {
                write!(f, "recovery failed: chain incomplete at seq {missing_seq}")
            }
            RecoveryError::Tamper(e) => write!(f, "recovery refused: {e}"),
            RecoveryError::Replay(e) => write!(f, "recovery replay failed: {e}"),
        }
    }
}

impl std::error::Error for RecoveryError {}

impl From<TamperError> for RecoveryError {
    fn from(e: TamperError) -> Self {
        RecoveryError::Tamper(e)
    }
}

impl From<ReplayError> for RecoveryError {
    fn from(e: ReplayError) -> Self {
        RecoveryError::Replay(e)
    }
}

/// Recover ref state from the cold tier alone (mirror stubbed) after hot-DO loss.
///
/// Reads every cold range, concatenates them in `seq` order, and re-projects.
/// Equivalent to [`recover_with_mirror`] passing [`NoMirror`].
pub fn recover_from_cold<S: ColdStore>(cold: &S) -> Result<Recovered, RecoveryError> {
    recover_with_mirror(cold, &NoMirror)
}

/// Recover ref state after hot-DO loss, cold tier first, mirror as the secondary
/// source for any `seq` gap the cold tier leaves.
///
/// Reconstructs the chain from `[0, max_seq)`, re-verifies it (fail-closed), and
/// replays it into a [`RefState`] that is replay-identical to the pre-loss state.
///
/// Equivalent to [`recover_with_sources`] with an empty hot tail.
pub fn recover_with_mirror<S: ColdStore, M: MirrorSource>(
    cold: &S,
    mirror: &M,
) -> Result<Recovered, RecoveryError> {
    recover_with_sources(cold, mirror, &[])
}

/// Recover ref state after hot-DO loss from **cold tier + mirror + a surviving
/// hot tail**.
///
/// Compaction only ever seals a *prefix* to the cold tier; the most-recent
/// records live hot. A partial hot-DO loss can lose the Durable Object while its
/// unsealed tail is still recoverable (re-read from the storage edge, replayed
/// by the client, or held by a sibling). Those `hot_tail` records — a contiguous
/// suffix beyond the sealed prefix — are spliced in **after** the cold/mirror
/// span and before chain verification, so recovery includes them instead of
/// rebuilding a silently-stale state that omits the tail.
///
/// Precedence is unchanged: cold tier first, mirror second for any prefix gap,
/// then the hot tail for the suffix. `max_seq` is one past the highest seq seen
/// across **all** sources, so a hot tail extends the recovered span. The
/// recombined chain is re-verified (fail-closed) before any ref state is served.
pub fn recover_with_sources<S: ColdStore, M: MirrorSource>(
    cold: &S,
    mirror: &M,
    hot_tail: &[EventRecord],
) -> Result<Recovered, RecoveryError> {
    // 1. Gather every cold-tier record, keyed by seq (cold tier FIRST).
    let mut by_seq: std::collections::BTreeMap<u64, EventRecord> =
        std::collections::BTreeMap::new();
    for key in cold.list() {
        if let Some(range) = cold.get(&key) {
            for r in range.records {
                by_seq.insert(r.seq, r);
            }
        }
    }

    // 1b. Splice the surviving hot tail in. It is authoritative for its own
    // seqs (the unsealed records the DO held); it never overwrites a sealed cold
    // record at the same seq (cold is the canonical seal), so insert only where
    // vacant. Crucially this extends `max_seq` below so the suffix is recovered
    // instead of silently dropped (a stale rebuild).
    for r in hot_tail {
        by_seq.entry(r.seq).or_insert_with(|| r.clone());
    }

    let mut used_mirror = false;
    // The chain runs [0, max_seq); max_seq is one past the highest seq seen
    // across cold + hot tail (the hot tail can extend the span past the sealed
    // prefix — without this, a partial-hot-loss recovery would be stale).
    let max_seq = by_seq.keys().next_back().map(|s| s + 1).unwrap_or(0);

    // 2. Fill any gap from the mirror (secondary source).
    if max_seq > 0 {
        // Find the contiguous reach from 0 over the cold tier; if it stops short
        // of max_seq, ask the mirror for the remainder of the full span.
        let mut reach = 0u64;
        while by_seq.contains_key(&reach) {
            reach += 1;
        }
        if reach < max_seq {
            for r in mirror.records_in(reach, max_seq) {
                if let std::collections::btree_map::Entry::Vacant(e) = by_seq.entry(r.seq) {
                    e.insert(r);
                    used_mirror = true;
                }
            }
        }
    }

    // 3. Assemble the chain in seq order, refusing on the first gap.
    let mut records: Vec<EventRecord> = Vec::with_capacity(by_seq.len());
    for expected in 0..max_seq {
        match by_seq.remove(&expected) {
            Some(r) => records.push(r),
            None => {
                return Err(RecoveryError::IncompleteChain {
                    missing_seq: expected,
                });
            }
        }
    }

    // 4. Fail-closed: re-verify before serving any derived view.
    verify_chain(&records)?;
    let state = replay_unchecked(&records)?;

    let source = if used_mirror {
        RecoverySource::ColdTierWithMirror
    } else {
        RecoverySource::ColdTier
    };

    Ok(Recovered {
        state,
        records,
        source,
    })
}
