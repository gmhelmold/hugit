//! WP-B5 — auto-bisect over memoized checks + the bounded `DiagnosisObject`.
//!
//! Memoization makes bisect ≈ free (whitepaper §5.1): each probe is an Action
//! Cache lookup, not a real check run, so culprit-finding becomes a *default*
//! that fires on every red instead of a manual chore.
//!
//! ## Module layout
//!
//! - [`oracle`]    — the [`CheckOracle`] abstraction the bisect searches over,
//!   and [`MemoizedCheckOracle`], its production binding onto the B2 memoized-
//!   check Action Cache (`hugit_checks::client::ac::ActionCache`). Every probe
//!   is an AC lookup; real (cache-miss) executions are counted honestly.
//! - [`engine`]    — the [`Bisector`]: binary search that locates the culprit
//!   (first red tree) in ≤ ⌈log₂ n⌉ probes (①) and assembles the bounded
//!   [`DiagnosisObject`] — `culprit_ref` + `diff_vs_green_ref` + `suspect_targets`
//!   + `bisect_path`, carrying CAS refs to logs, NEVER inlined raw logs (②④).
//! - [`trigger`]   — the auto-trigger seam ([`RedSignal`] / [`on_red_signal`]):
//!   bisect fires automatically on ANY red signal from the landing queue, with
//!   NO manual-invocation entry point (⑤). The live production wiring onto
//!   `hugit_contracts::QueueApi` UNION-FAIL events is the documented P2 seam.
//!
//! ## Key invariants (all provable hermetically)
//!
//! - **①** [`Bisector::find_culprit`] returns the first red tree in
//!   `≤ ⌈log₂ n⌉` oracle probes, proven across a sweep of history sizes; a
//!   linear scan would break the bound.
//! - **②** [`Bisector::diagnose`] emits a `diff_vs_green_ref` spanning the
//!   last-known-green → culprit endpoints and a deduplicated, non-empty
//!   `suspect_targets` list (the targets reachable from the culprit's change).
//! - **③** End-to-end bisect+diagnose on a 1024-deep fixture completes in
//!   `<2min` (mostly free AC hits).
//! - **④** The `DiagnosisObject` is bounded structured data: its `size_bytes`
//!   is the real serialized size and is asserted `≤ DIAGNOSIS_SIZE_BOUND`; the
//!   assembler [`Bisector::try_diagnose_with_suspects`] REFUSES a payload that
//!   would exceed the bound ([`BisectError::DiagnosisTooLarge`]) — a raw-log-
//!   dump diagnosis fails closed.
//! - **⑤** The only entry point is [`on_red_signal`] over a [`RedSignal`]; a
//!   green tip (or empty history) produces NO diagnosis. There is no manual
//!   `bisect()` the caller invokes by hand outside the red-signal path.

mod engine;
mod oracle;
mod trigger;

pub use engine::{BisectError, BisectOutcome, Bisector, DIAGNOSIS_SIZE_BOUND};
pub use oracle::{CheckOracle, History, MemoizedCheckOracle, ProbeOutcome, ProbeVerdict};
pub use trigger::{RedSignal, on_red_signal};
