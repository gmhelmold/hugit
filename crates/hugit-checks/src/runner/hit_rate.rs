//! Honest partial hit-rate measurement (item ⑤).
//!
//! The wedge promise — "your green checks never re-run" — is FULL only for
//! hermetic checks. Non-hermetic ecosystems (npm, pip) cannot be fully
//! memoized: an `npm install` reaches the network, a build can pick up ambient
//! state, so only PART of the work is content-addressable. The honest behavior
//! is to MEASURE the actual Action-Cache hit-rate and DISPLAY IT AS-IS — a
//! partial figure — and to NEVER claim full memoization for such an ecosystem
//! (whitepaper §5.2/§11; command-catalog: "hermetic full, npm/pip partial —
//! measured, never promised").
//!
//! This meter is ecosystem-agnostic: it counts real `lookup` hits vs misses
//! over a run sequence and computes the rate from the counts. The npm fixture in
//! the acceptance suite drives it with a partially-cacheable workload so the
//! measured rate is genuinely partial (`0 < rate < 1`) — never a hardcoded or
//! gamed full-memo claim.

/// A measured hit-rate over a sequence of Action-Cache lookups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HitRateReport {
    /// Total checks looked up.
    pub total: u64,
    /// Of those, how many were AC HITs (memoized, zero re-execution).
    pub hits: u64,
    /// Of those, how many were MISSes (had to execute).
    pub misses: u64,
}

impl HitRateReport {
    /// The measured hit-rate in `[0.0, 1.0]`. Defined as `0.0` for an empty
    /// sequence (no claim can be made with no data — honest by default).
    pub fn rate(&self) -> f64 {
        if self.total == 0 {
            0.0
        } else {
            self.hits as f64 / self.total as f64
        }
    }

    /// The measured rate as a whole-number percentage, AS-IS (never rounded up
    /// to a full-memo claim). Display surface for item ⑤.
    pub fn rate_pct(&self) -> f64 {
        self.rate() * 100.0
    }

    /// True iff EVERY lookup was a hit — i.e. the workload was fully memoized.
    /// For a non-hermetic ecosystem this MUST be false; the meter never asserts
    /// it, it only reports what was measured.
    pub fn is_full_memo(&self) -> bool {
        self.total > 0 && self.misses == 0
    }

    /// True iff the measured rate is strictly partial (`0 < rate < 1`): some work
    /// memoized, some not. The honest shape for npm/pip.
    pub fn is_partial(&self) -> bool {
        self.total > 0 && self.hits > 0 && self.misses > 0
    }

    /// A human-honest one-line display, e.g. `"hit-rate: 60.0% (3/5 memoized,
    /// 2 re-executed) — PARTIAL"`. The label reflects the MEASURED shape; it is
    /// never a promise.
    pub fn display_line(&self) -> String {
        let label = if self.total == 0 {
            "NO DATA"
        } else if self.is_full_memo() {
            "FULL"
        } else if self.hits == 0 {
            "NONE"
        } else {
            "PARTIAL"
        };
        format!(
            "hit-rate: {:.1}% ({}/{} memoized, {} re-executed) — {}",
            self.rate_pct(),
            self.hits,
            self.total,
            self.misses,
            label
        )
    }
}

/// Measures the Action-Cache hit-rate over a sequence of lookups.
///
/// The caller records the outcome of each AC lookup (`hit` / `miss`) as the run
/// sequence proceeds; [`HitRateMeter::report`] returns the measured figures. The
/// meter makes NO assumption about the workload — the rate is purely whatever
/// was observed, so the displayed figure is honest by construction.
#[derive(Debug, Default, Clone)]
pub struct HitRateMeter {
    hits: u64,
    misses: u64,
}

impl HitRateMeter {
    /// A fresh meter.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record one lookup outcome (`true` = HIT, `false` = MISS).
    pub fn observe(&mut self, hit: bool) {
        if hit {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
    }

    /// Record an AC `lookup` result directly: `Some(_)` is a HIT, `None` a MISS.
    pub fn observe_lookup<T>(&mut self, lookup: &Option<T>) {
        self.observe(lookup.is_some());
    }

    /// The measured report.
    pub fn report(&self) -> HitRateReport {
        HitRateReport {
            total: self.hits + self.misses,
            hits: self.hits,
            misses: self.misses,
        }
    }
}
