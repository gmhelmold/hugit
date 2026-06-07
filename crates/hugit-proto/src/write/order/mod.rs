//! Concurrent-push **total order + stale rejection** (WP-D3b item ②).
//!
//! Concurrent pushes against one repository are serialized through the D1
//! single-writer append point — in production exactly one Durable Object owns the
//! [`EventLog`] and is the single writer. That serialization point **is** the
//! total order: every accepted push lands at a distinct, monotonically increasing
//! log `seq`, and the order is a strict total order with no two pushes sharing a
//! slot and no lost update.
//!
//! Each push is **compare-and-append**: the pusher declares the ref tip it
//! expects ([`expected`](RefUpdate::expected)); the writer accepts only if the
//! *current derived view* still shows that tip, then appends. A push against a
//! stale tip — one another push already advanced — is correctly **rejected**
//! ([`StaleRef`]), never silently overwriting the winner. No false accept, no
//! lost update.
//!
//! The single-writer point here is a [`Mutex`]-guarded [`EventLog`]
//! ([`SerializedWriter`]); it is the in-process stand-in for the per-repo DO. The
//! acceptance suite drives it from real overlapping threads (held under a barrier,
//! not instant) to prove the total order holds under genuine contention.

use std::sync::Mutex;

use hugit_contracts::event_record::EventRecord;
use hugit_refstore::{EventLog, RefState, replay};

use crate::write::external::{Attribution, RawPush, record_external_change};

/// A compare-and-append ref update: move `ref_name` to `target`, but only if the
/// ref currently shows `expected` (or is absent, when `expected` is `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    /// The ref to move.
    pub ref_name: String,
    /// The tip the pusher expects the ref to hold right now. `None` asserts the
    /// ref is currently absent (a create).
    pub expected: Option<String>,
    /// The object id to move the ref to.
    pub target: String,
    /// *Who* is pushing — the ordered principal chain (attribution).
    pub principal_chain: Vec<String>,
    /// *When* — unix epoch milliseconds.
    pub recorded_at: u64,
}

/// A push was rejected because the ref had already moved off the tip the pusher
/// expected — a stale compare-and-append. The winner is preserved; nothing lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaleRef {
    /// The ref the rejected push targeted.
    pub ref_name: String,
    /// The tip the pusher expected.
    pub expected: Option<String>,
    /// The tip the ref actually held (the value that won the race).
    pub actual: Option<String>,
}

impl std::fmt::Display for StaleRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "stale ref {}: expected {:?}, but it is {:?} (push rejected, no lost update)",
            self.ref_name, self.expected, self.actual
        )
    }
}

impl std::error::Error for StaleRef {}

/// The outcome of a serialized push: either it landed (with its log record +
/// attribution + assigned total-order seq), or it was rejected as stale.
///
/// (`EventRecord` is `PartialEq` but not `Eq`, so this enum is `PartialEq` only.)
#[derive(Debug, Clone, PartialEq)]
pub enum PushOutcome {
    /// The push landed. Carries the appended external-change record, its
    /// attribution, and the total-order `seq` it was assigned.
    Landed {
        /// The appended external-change event.
        record: EventRecord,
        /// Read-back attribution (who/when/which ref).
        attribution: Attribution,
        /// The total-order position assigned to this push (the log seq).
        seq: u64,
    },
    /// The push was rejected: the ref had moved off the expected tip.
    Stale(StaleRef),
}

impl PushOutcome {
    /// The assigned total-order seq, if the push landed.
    pub fn landed_seq(&self) -> Option<u64> {
        match self {
            PushOutcome::Landed { seq, .. } => Some(*seq),
            PushOutcome::Stale(_) => None,
        }
    }
}

/// The **single-writer serialization point** for one repository's pushes.
///
/// Wraps the per-repo [`EventLog`] behind a [`Mutex`] — the in-process stand-in
/// for the per-repo Durable Object. Every push is serialized through
/// [`push`](SerializedWriter::push); the lock provides the total order and the
/// compare-and-append atomicity (read derived view → check tip → append) in one
/// critical section, so no two concurrent pushes can both win a stale check.
#[derive(Debug, Default)]
pub struct SerializedWriter {
    log: Mutex<EventLog>,
}

impl SerializedWriter {
    /// A fresh writer over an empty log.
    pub fn new() -> Self {
        Self {
            log: Mutex::new(EventLog::new()),
        }
    }

    /// A writer seeded with an existing log (e.g. rehydrated state).
    pub fn from_log(log: EventLog) -> Self {
        Self {
            log: Mutex::new(log),
        }
    }

    /// Serialize one compare-and-append push through the single-writer point.
    ///
    /// In one critical section: derive the current ref view, check the ref still
    /// shows the expected tip, and — only on a match — append the external-change
    /// event. A mismatch is a [`StaleRef`] rejection (no append, no lost update).
    /// The returned [`PushOutcome::Landed`] carries the total-order `seq`.
    ///
    /// If the mutex was poisoned by a prior panic, the inner [`EventLog`] is
    /// RECOVERED (`into_inner`) rather than re-panicking: the append-only log is
    /// internally consistent (the append primitive never leaves it half-written),
    /// so the single-writer point keeps serving instead of becoming a permanent
    /// DoS for every later push.
    pub fn push(&self, update: RefUpdate) -> PushOutcome {
        let mut log = self.log.lock().unwrap_or_else(|p| p.into_inner());

        // Derive the current ref view from the log (D1 derived view, never owned).
        let state: RefState = replay(&log).expect("own appended log replays cleanly");
        let actual = state.get(&update.ref_name).map(str::to_string);

        // Compare-and-append: accept only if the tip still matches the expectation.
        if actual != update.expected {
            return PushOutcome::Stale(StaleRef {
                ref_name: update.ref_name,
                expected: update.expected,
                actual,
            });
        }

        let push = RawPush::Update {
            ref_name: update.ref_name,
            target: update.target,
        };
        // Attribution is enforced by the external-change recorder; a push with an
        // empty principal chain is refused there. We surface that as a panic-free
        // contract: callers in this WP always pass a non-empty chain.
        let (record, attribution) =
            record_external_change(&mut log, &push, update.principal_chain, update.recorded_at)
                .expect("serialized push carries attribution");
        let seq = record.seq;

        PushOutcome::Landed {
            record,
            attribution,
            seq,
        }
    }

    /// The current number of records (also the next total-order seq).
    pub fn len(&self) -> usize {
        self.log.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    /// Whether the log is empty.
    pub fn is_empty(&self) -> bool {
        self.log
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .is_empty()
    }

    /// Snapshot the underlying log (clone) for assertions / replay.
    pub fn snapshot(&self) -> EventLog {
        self.log.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    /// The current derived ref view.
    pub fn ref_view(&self) -> RefState {
        let log = self.log.lock().unwrap_or_else(|p| p.into_inner());
        replay(&log).expect("own log replays cleanly")
    }
}
