//! The point-in-time-consistent cut reader (E5⑨).
//!
//! Export reads **one** snapshot of the D1 event log — a prefix of the
//! append-only chain up to a chosen `cut_seq`. Because the log is append-only
//! and hash-chained, any prefix is itself a valid, self-consistent chain: a cut
//! can never contain an event that references an object that doesn't exist yet
//! in the same cut, and concurrent appends *after* the cut are simply not in it.
//!
//! Concretely the cut is taken by snapshotting the log's records once
//! ([`Cut::take`]); concurrent mutation (landings + mirror sync + event append)
//! that lands after the snapshot is invisible to this export, yielding ONE
//! point-in-time-consistent cut.

use hugit_contracts::EventRecord;
use hugit_refstore::{EventLog, RefState, replay, verify_chain};

use crate::export::schema::ProvenanceLink;

/// A self-consistent prefix snapshot of the D1 event log.
#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    /// The records in the cut, chain order, `seq` 0..cut_len.
    records: Vec<EventRecord>,
}

/// Why taking or validating a cut failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CutError {
    /// The snapshotted chain failed re-verification — never serve a cut off a
    /// tampered log (fail-closed).
    Tamper(String),
    /// A provenance link references an event past the cut — would be a dangling
    /// link in the export (E5⑨). Caught here, before the envelope is built.
    DanglingLink {
        /// The object whose link dangles.
        object_id: String,
        /// The event sequence that is absent from the cut.
        event_seq: u64,
    },
    /// Replaying the cut's refs failed — a malformed ref payload in the chain.
    /// Propagated as a HARD export failure, never swallowed into an empty
    /// ref-set (which would silently export zero refs with exit 0).
    RefReplay(String),
}

impl std::fmt::Display for CutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CutError::Tamper(e) => write!(f, "cut refused (tamper): {e}"),
            CutError::DanglingLink {
                object_id,
                event_seq,
            } => write!(
                f,
                "cut: provenance link for '{object_id}' references absent event seq {event_seq}"
            ),
            CutError::RefReplay(e) => write!(f, "cut: ref replay failed (malformed payload): {e}"),
        }
    }
}

impl std::error::Error for CutError {}

impl Cut {
    /// Take a point-in-time cut over the FULL current log: snapshot every record
    /// once. Any append that lands after this call is not in the cut.
    pub fn take(log: &EventLog) -> Result<Self, CutError> {
        Self::take_to(log, log.len() as u64)
    }

    /// Take a cut over the prefix `seq < cut_seq`. Used to model "the export
    /// began before later events were appended": records appended concurrently
    /// (seq >= cut_seq) are excluded, and the prefix is still a valid chain.
    pub fn take_to(log: &EventLog, cut_seq: u64) -> Result<Self, CutError> {
        let records: Vec<EventRecord> = log
            .records()
            .iter()
            .filter(|r| r.seq < cut_seq)
            .cloned()
            .collect();
        // A prefix of a valid hash chain is itself a valid chain; re-verify
        // fail-closed so we never export off a tampered snapshot.
        //
        // PS-13 chokepoint exemption (`readpath-verify-exempt`): this is NOT a
        // disk read — it re-verifies a PREFIX of an `EventLog` that was already
        // loaded through the single chokepoint (`run_export` → `load_event_log`
        // → `rehydrate_and_verify`). It is defence-in-depth over an in-memory
        // snapshot, not a verb's read-path load, so it does not route through
        // `checks::rehydrate_and_verify` (which rebuilds a whole log from
        // records — a cut is a sub-range, not a fresh disk load).
        verify_chain(&records).map_err(|e| CutError::Tamper(e.to_string()))?; // readpath-verify-exempt
        Ok(Self { records })
    }

    /// The records in the cut (read-only).
    pub fn records(&self) -> &[EventRecord] {
        &self.records
    }

    /// Number of events in the cut.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Whether the cut is empty.
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// The highest event `seq` present in the cut + 1 (the exclusive bound).
    pub fn seq_bound(&self) -> u64 {
        self.records.len() as u64
    }

    /// Project the cut's refs (re-uses D1's deterministic [`replay`]).
    ///
    /// Returns [`CutError::RefReplay`] on a malformed ref payload — a hard
    /// export failure. NEVER swallows the error into an empty ref-set: a
    /// silent-empty export with exit 0 would ship a corpus that drops every
    /// branch/tag without warning.
    pub fn ref_state(&self) -> Result<RefState, CutError> {
        // Build a fresh log over the cut's records, then replay. The cut is
        // already chain-verified, so replay cannot fail on tamper here; an
        // internal malformed payload still fails closed (propagated, not eaten).
        let mut log = EventLog::new();
        for r in &self.records {
            // push_record only fails on non-monotonic seq; a verified prefix is
            // gap-free by construction.
            log.push_record(r.clone())
                .map_err(|e| CutError::RefReplay(e.to_string()))?;
        }
        replay(&log).map_err(|e| CutError::RefReplay(e.to_string()))
    }

    /// Assert that every provided provenance link references an event inside the
    /// cut (E5⑨ self-consistency). Called before the envelope is built so a
    /// dangling link is refused at the source, not discovered downstream.
    pub fn assert_links_resolved(&self, links: &[ProvenanceLink]) -> Result<(), CutError> {
        let bound = self.seq_bound();
        for link in links {
            if link.event_seq >= bound {
                return Err(CutError::DanglingLink {
                    object_id: link.object_id.clone(),
                    event_seq: link.event_seq,
                });
            }
        }
        Ok(())
    }
}
