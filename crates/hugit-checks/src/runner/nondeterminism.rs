//! Non-determinism detection (item ④).
//!
//! A memoized check is only honest if running it again for the SAME `memo_key`
//! reproduces the SAME outputs. A check that produces different artifacts
//! run-to-run for the same key is **non-deterministic** — and a non-deterministic
//! check must NEVER be silently re-memoized or claimed as a clean hit, because
//! the "your green checks never re-run" wedge would then be a lie.
//!
//! This tracker records, per `memo_key`, the fingerprint of each run's produced
//! artifacts. After **3 divergent runs** for the same key (i.e. the key has been
//! observed producing 3 distinct artifact fingerprints) the check is flagged
//! [`DeterminismState::NonDeterministic`] and surfaced HONESTLY. The threshold
//! is exactly the contract's "flag after 3 divergent runs".
//!
//! A run's fingerprint is the canonical digest of its `(path, content-digest)`
//! artifact set — so two runs that produced byte-identical outputs share a
//! fingerprint (no false divergence), and any content change yields a new one.

use std::collections::BTreeMap;
use std::collections::HashMap;

use hugit_contracts::CheckResult;
use sha2::{Digest, Sha256};

/// How many DISTINCT artifact fingerprints for one `memo_key` constitute
/// non-determinism. The contract: "flagged after 3 divergent runs".
pub const NON_DETERMINISM_THRESHOLD: usize = 3;

/// The honest determinism verdict for a `memo_key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeterminismState {
    /// Fewer than two distinct fingerprints observed — every run so far agreed.
    /// Carries the number of runs observed.
    Deterministic {
        /// Total runs observed for this key.
        runs: usize,
    },
    /// At least two but fewer than [`NON_DETERMINISM_THRESHOLD`] distinct
    /// fingerprints — divergence seen, but not yet enough to flag. Honest
    /// interim state: NOT yet claimed non-deterministic, NOT a clean hit.
    Diverging {
        /// Total runs observed for this key.
        runs: usize,
        /// Distinct artifact fingerprints observed so far.
        distinct_fingerprints: usize,
    },
    /// [`NON_DETERMINISM_THRESHOLD`] or more distinct fingerprints — the check is
    /// FLAGGED non-deterministic. It must not be memoized as a clean hit.
    NonDeterministic {
        /// Total runs observed for this key.
        runs: usize,
        /// Distinct artifact fingerprints observed.
        distinct_fingerprints: usize,
    },
}

impl DeterminismState {
    /// True iff the check is flagged non-deterministic — the load-bearing
    /// honesty predicate. A flagged check is never a clean memoized hit.
    pub fn is_non_deterministic(&self) -> bool {
        matches!(self, DeterminismState::NonDeterministic { .. })
    }
}

/// A single recorded run observation for a `memo_key`.
#[derive(Debug, Clone)]
struct KeyHistory {
    /// Total runs recorded for this key.
    runs: usize,
    /// Distinct artifact fingerprints observed (insertion-ordered count via the
    /// set length).
    fingerprints: BTreeMap<String, usize>,
}

/// One observation handed to the tracker: a `memo_key` and the [`CheckResult`]
/// a run produced. Convenience over passing the two separately at the call site.
#[derive(Debug, Clone)]
pub struct RunObservation<'a> {
    /// The memo key the run was executed under.
    pub memo_key: &'a str,
    /// The result the run produced (its artifacts form the fingerprint).
    pub result: &'a CheckResult,
}

/// Tracks produced-artifact fingerprints per `memo_key` across runs and flags
/// non-determinism after [`NON_DETERMINISM_THRESHOLD`] divergent runs.
#[derive(Debug, Default)]
pub struct NonDeterminismTracker {
    history: HashMap<String, KeyHistory>,
}

impl NonDeterminismTracker {
    /// A fresh tracker.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one run's result and return the current [`DeterminismState`] for
    /// its key. Idempotent on the *content*: re-recording a byte-identical run
    /// does not invent divergence.
    pub fn record(&mut self, obs: RunObservation<'_>) -> DeterminismState {
        let fp = fingerprint(obs.result);
        let entry = self
            .history
            .entry(obs.memo_key.to_string())
            .or_insert_with(|| KeyHistory {
                runs: 0,
                fingerprints: BTreeMap::new(),
            });
        entry.runs += 1;
        *entry.fingerprints.entry(fp).or_insert(0) += 1;
        Self::classify(entry)
    }

    /// The current [`DeterminismState`] for a key without recording a new run.
    /// Returns `None` if the key has never been observed.
    pub fn state(&self, memo_key: &str) -> Option<DeterminismState> {
        self.history.get(memo_key).map(Self::classify)
    }

    fn classify(entry: &KeyHistory) -> DeterminismState {
        let distinct = entry.fingerprints.len();
        let runs = entry.runs;
        if distinct >= NON_DETERMINISM_THRESHOLD {
            DeterminismState::NonDeterministic {
                runs,
                distinct_fingerprints: distinct,
            }
        } else if distinct >= 2 {
            DeterminismState::Diverging {
                runs,
                distinct_fingerprints: distinct,
            }
        } else {
            DeterminismState::Deterministic { runs }
        }
    }
}

/// Canonical fingerprint of a run's produced artifacts: a SHA-256 over the
/// sorted `(path, content-digest)` set. Two runs share a fingerprint iff they
/// produced byte-identical outputs.
fn fingerprint(result: &CheckResult) -> String {
    let sorted: BTreeMap<&str, &str> = result
        .artifacts
        .iter()
        .map(|a| (a.path.as_str(), a.digest.as_str()))
        .collect();
    let mut hasher = Sha256::new();
    hasher.update((sorted.len() as u32).to_be_bytes());
    for (path, digest) in sorted {
        hasher.update((path.len() as u32).to_be_bytes());
        hasher.update(path.as_bytes());
        hasher.update((digest.len() as u32).to_be_bytes());
        hasher.update(digest.as_bytes());
    }
    hex::encode(hasher.finalize())
}
