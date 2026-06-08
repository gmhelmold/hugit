//! The bisect oracle: "is this tree green?" answered over memoized checks.
//!
//! Bisect searches a [`History`] of tree refs (oldest→tip) for the first RED
//! tree. Each probe asks a [`CheckOracle`] whether a tree's check passed. The
//! production oracle is [`MemoizedCheckOracle`], which answers each probe with a
//! single Action Cache lookup (`hugit_checks::client::ac::ActionCache`) — a HIT
//! is ≈ free (zero real check execution), which is exactly why bisect can run on
//! every red by default (whitepaper §5.1).
//!
//! The oracle counts BOTH the probes it served and the real (cache-miss)
//! executions it had to fall back to, so the suite can prove the ≤ log₂ bound
//! AND that the memoized path costs zero real runs.

use std::cell::Cell;

use hugit_checks::client::ac::ActionCache;
use hugit_refstore::compute_memo_key;

/// The ordered tree history under bisect: `trees[0]` is the oldest (expected
/// green) tree, `trees[len-1]` is the red tip. The `def_digest` and
/// `toolchain_digest` are the two non-tree memo axes shared across the history
/// (the same check, the same toolchain, varying only the tree).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct History {
    trees: Vec<String>,
    def_digest: String,
    toolchain_digest: String,
}

impl History {
    /// Build a history from an ordered list of tree refs (oldest→tip) plus the
    /// shared def/toolchain memo axes.
    pub fn new(trees: Vec<String>, def_digest: String, toolchain_digest: String) -> Self {
        Self {
            trees,
            def_digest,
            toolchain_digest,
        }
    }

    /// The ordered tree refs (oldest→tip).
    pub fn trees(&self) -> &[String] {
        &self.trees
    }

    /// Number of trees in the history.
    pub fn len(&self) -> usize {
        self.trees.len()
    }

    /// True if the history is empty (no trees to bisect).
    pub fn is_empty(&self) -> bool {
        self.trees.is_empty()
    }

    /// The shared check-definition digest (memo axis 2).
    pub fn def_digest(&self) -> &str {
        &self.def_digest
    }

    /// The shared toolchain digest (memo axis 3).
    pub fn toolchain_digest(&self) -> &str {
        &self.toolchain_digest
    }

    /// The three-axis memo key for the tree at `index`, single-sourced through
    /// [`hugit_refstore::compute_memo_key`] (never re-transcribed).
    pub(crate) fn memo_key_at(&self, index: usize) -> String {
        compute_memo_key(&self.trees[index], &self.def_digest, &self.toolchain_digest)
    }
}

/// The verdict for one probed tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeVerdict {
    /// The tree's check passed (exit 0).
    Green,
    /// The tree's check failed (non-zero exit).
    Red,
}

/// The outcome of probing one tree: its verdict plus whether the answer cost a
/// real check execution (a cache miss) or was served free from the memo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeOutcome {
    /// Green or red.
    pub verdict: ProbeVerdict,
    /// True iff this probe required a real check run (cache miss). For a fully
    /// memoized history this is always `false`.
    pub was_real_execution: bool,
}

/// The thing bisect searches over: an oracle that answers "is tree `i` green?".
///
/// The trait keeps the bisect engine independent of HOW the verdict is sourced
/// — the production [`MemoizedCheckOracle`] answers from the Action Cache, while
/// tests can supply deterministic fixtures through the same interface
/// (partial-over-fake: the in-process AC IS the reference semantics).
pub trait CheckOracle {
    /// Probe the tree at `index` within `history` and report its verdict, plus
    /// whether the probe cost a real execution.
    fn probe(&self, history: &History, index: usize) -> ProbeOutcome;

    /// Total probes served so far (bisect counts these against the log₂ bound).
    fn probe_count(&self) -> u64;

    /// Total real (cache-miss) executions incurred so far.
    fn real_execution_count(&self) -> u64;
}

/// The production oracle: answers each probe with ONE Action Cache lookup over
/// the B2 memoized-check surface. A HIT yields the memoized verdict with zero
/// real execution; a MISS would (in production) execute the check once — here
/// the seam returns red-on-miss honestly and counts the miss as a real run, so
/// a non-memoized history is never silently treated as free.
pub struct MemoizedCheckOracle<'a, A: ActionCache> {
    ac: &'a A,
    probes: Cell<u64>,
    real_execs: Cell<u64>,
}

impl<'a, A: ActionCache> MemoizedCheckOracle<'a, A> {
    /// Bind an oracle to a memoized-check Action Cache.
    pub fn new(ac: &'a A) -> Self {
        Self {
            ac,
            probes: Cell::new(0),
            real_execs: Cell::new(0),
        }
    }
}

impl<A: ActionCache> CheckOracle for MemoizedCheckOracle<'_, A> {
    fn probe(&self, history: &History, index: usize) -> ProbeOutcome {
        self.probes.set(self.probes.get() + 1);
        let key = history.memo_key_at(index);
        match self.ac.lookup(&key) {
            // HIT — verdict served free from the memo (the wedge: zero real run).
            Ok(Some(result)) => ProbeOutcome {
                verdict: if result.exit == 0 {
                    ProbeVerdict::Green
                } else {
                    ProbeVerdict::Red
                },
                was_real_execution: false,
            },
            // MISS or transport error — the memo cannot answer. In production
            // the check would execute once here (P2: live runner). We count it
            // as a real execution and treat an unknown tree as red fail-closed,
            // so a partially-memoized history never reports a fake free hit.
            Ok(None) | Err(_) => {
                self.real_execs.set(self.real_execs.get() + 1);
                ProbeOutcome {
                    verdict: ProbeVerdict::Red,
                    was_real_execution: true,
                }
            }
        }
    }

    fn probe_count(&self) -> u64 {
        self.probes.get()
    }

    fn real_execution_count(&self) -> u64 {
        self.real_execs.get()
    }
}
