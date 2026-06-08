//! The auto-trigger: bisect fires on ANY red, with no manual invocation (⑤).
//!
//! Memoization makes bisect ≈ free, so culprit-finding is not a button a human
//! presses — it is a DEFAULT that fires automatically the instant the landing
//! queue reports red (whitepaper §6.4: `red → bisect batch over memoized checks
//! → minimal failing pair → structured UNION-FAIL`).
//!
//! ## The single entry point
//!
//! [`on_red_signal`] is the ONLY way to obtain a diagnosis from this crate. It
//! consumes a [`RedSignal`] — which can itself only be constructed from a queue
//! signal ([`RedSignal::from_red_tip`] / [`RedSignal::from_queue`]). There is no
//! public `bisect()` a caller invokes by hand outside this path: the auto-
//! trigger IS the API. A green tip (or empty history) produces NO diagnosis.
//!
//! ## P2 SEAM — live QueueApi wiring (DEFERRED, documented)
//!
//! In production the red signal arrives as a `hugit_contracts::QueueApi`
//! UNION-FAIL event from the landing queue / DO. [`RedSignal::from_queue`] is
//! the seam that maps that live event onto the in-process [`History`] the bisect
//! searches; the bytes that resolve a `tree_hash` into a memoized check live in
//! CoreLink's prod AC tenant (the same P2 deferral as B2's `HttpAcClient`).
//! Until that tenant + the live queue subscription are wired, the live mapping
//! is gated behind the `HUGIT_QUEUE_AUTOTRIGGER` env flag and documented here.
//! The auto-INVOCATION path — "a red signal yields a diagnosis with no manual
//! call" — is proven hermetically NOW via [`RedSignal::from_red_tip`]; only the
//! live event subscription is deferred, never the auto-trigger semantics.

use hugit_contracts::DiagnosisObject;

use super::engine::{BisectError, Bisector};
use super::oracle::{CheckOracle, History};

/// A red signal from the landing queue that auto-triggers a bisect.
///
/// This is the bisect's ONLY input. It carries the [`History`] (oldest→tip)
/// whose tip the queue reported red. It is deliberately the sole carrier: there
/// is no constructor that hands the engine a culprit directly, so a diagnosis
/// can only ever be produced in response to a real red signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedSignal {
    history: History,
}

impl RedSignal {
    /// Construct a red signal from a history whose tip the queue reported red.
    ///
    /// This is the hermetic constructor used to prove the auto-invocation path:
    /// given a red-tipped history, [`on_red_signal`] produces a diagnosis with
    /// NO manual bisect call. (The live `QueueApi`-event constructor is
    /// [`Self::from_queue`], the documented P2 seam.)
    pub fn from_red_tip(history: History) -> Self {
        Self { history }
    }

    /// P2 SEAM: construct a red signal from a live `hugit_contracts::QueueApi`
    /// UNION-FAIL event by resolving its batch's tree history.
    ///
    /// The mapping from queue `tree_hash`es to the memoized-check history lives
    /// behind CoreLink's prod AC tenant + the live queue subscription (deferred,
    /// gated behind `HUGIT_QUEUE_AUTOTRIGGER`). The caller supplies the resolved
    /// [`History`]; this constructor is the named seam where that live wiring
    /// lands without changing the auto-trigger semantics proven by
    /// [`Self::from_red_tip`].
    pub fn from_queue(resolved_history: History) -> Self {
        Self {
            history: resolved_history,
        }
    }

    /// The history this signal carries.
    pub fn history(&self) -> &History {
        &self.history
    }
}

/// THE auto-trigger (⑤). Given a red signal, bisect over the memoized-check
/// oracle and return the bounded diagnosis — automatically, with no manual
/// invocation.
///
/// - `Ok(Some(diag))` — the signal carried a real red tip; the culprit was
///   located and a bounded [`DiagnosisObject`] assembled.
/// - `Ok(None)` — the signal's tip is actually green (or the history is empty):
///   nothing to bisect, so NO diagnosis is fabricated.
/// - `Err(_)` — a genuine bisect/diagnosis failure (e.g. an over-bound payload).
///
/// This is the single entry point: there is no manual `bisect()` reachable
/// outside this red-signal path.
pub fn on_red_signal<O: CheckOracle>(
    oracle: &O,
    signal: &RedSignal,
) -> Result<Option<DiagnosisObject>, BisectError> {
    let bisect = Bisector::new(oracle);
    match bisect.find_culprit(signal.history()) {
        Ok(outcome) => Ok(Some(bisect.diagnose(signal.history(), &outcome))),
        // A green/empty tip is not an error — it simply means no bisect fires.
        Err(BisectError::NoRedTip) => Ok(None),
        Err(e) => Err(e),
    }
}
