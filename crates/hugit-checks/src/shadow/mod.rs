//! Shadow checks — snapshot-cadence, budget-capped, non-gating (WP-C8).
//!
//! # Charter
//!
//! Shadow checks run on the **workspace-snapshot cadence**, NOT per write. The
//! engineer-review objection was cost/noise: re-running checks on every write
//! is wasteful and floods signal. The answer encoded here is the
//! **snapshot-window boundary** as the single trigger — N writes inside one
//! window collapse (coalesce/dedup) to exactly ONE shadow pass at the boundary,
//! never N, never 0 (when at least one write happened and the repo is opted in).
//!
//! # Pre-decided forks (from WP-C8 contract)
//!
//! - **Cadence = snapshot boundary, not per write (①):** see [`ShadowScheduler`]
//!   — pending writes accumulate; [`ShadowScheduler::close_window`] emits at
//!   most one [`ShadowPass`].
//! - **Budget (②):** a shadow pass decrements the tenant's C7 budget
//!   ([`hugit_queue::budget::BudgetManager`]); when the cap is hit, shadows
//!   **halt** ([`ShadowDecision::CapHalted`]) while explicit jobs proceed
//!   ([`run_explicit_job`]). Shadows are the yielding workload.
//! - **Default-off + opt-in (③):** [`ShadowPolicy::optin`] is per-repo, scoped
//!   to that repo; [`is_enabled_for`] gates; zero shadow runs when off.
//! - **Non-gating (④):** a failing shadow surfaces as a signal/event ONLY
//!   ([`ShadowOutcome::Signal`]); it never gates, blocks, or fails any explicit
//!   job. A passing shadow produces an observable [`CheckResult`]. Shadow is
//!   ambient truth, never a gate.
//! - **Cap isolation (⑤):** budgets are per-tenant (rides C7's per-tenant
//!   accounting), so tenant A exhausting its budget leaves tenant B unaffected.
//!
//! Shadow execution runs on the C2 runner (container-per-job); the Firecracker
//! path is documented in the whitepaper, not built here. This module owns the
//! **scheduler** that decides *whether and when* a shadow pass fires and *how*
//! its outcome surfaces — it does not own the budget engine (C7) nor the runner
//! (C2); it consumes both through their published API.
//!
//! # Claims boundary
//!
//! This subtree (`hugit-checks/shadow/`) is disjoint from `regen` (C4),
//! `affected` (B3), and the C7 budget engine.

use hugit_contracts::{check_result::CheckResult, shadow_policy::ShadowPolicy};
use hugit_queue::budget::{BudgetManager, BudgetStatus};

/// Cost, in C7 budget units, charged per shadow pass.
///
/// Stated, not left open: each shadow pass draws exactly one budget unit from
/// the tenant. The cap is therefore "how many shadow passes a tenant may run
/// per policy period" expressed in the same currency C7 meters explicit jobs.
pub const SHADOW_PASS_COST: u64 = 1;

// ── Opt-in scope (③) ────────────────────────────────────────────────────────

/// Sentinel [`ShadowPolicy::optin`] value meaning "all repos opted in".
pub const OPTIN_ALL: &str = "*";

/// True iff shadow checks are enabled for `repo` under `policy`.
///
/// Default-off semantics (③): a policy opts a repo in only when its
/// [`ShadowPolicy::optin`] scope **exactly matches** `repo` (per-repo, scoped
/// to that repo) or is the explicit [`OPTIN_ALL`] sentinel. An empty `optin`
/// (the natural default for an unconfigured policy) opts in **nothing** — so a
/// repo with no shadow configuration runs zero shadow passes.
pub fn is_enabled_for(policy: &ShadowPolicy, repo: &str) -> bool {
    if policy.optin.is_empty() {
        return false;
    }
    policy.optin == OPTIN_ALL || policy.optin == repo
}

// ── Snapshot-window scheduler (①) ───────────────────────────────────────────

/// Identifier for a write within a snapshot window.
///
/// Writes are coalesced by this id: repeated writes to the same path within one
/// window collapse to a single pending entry, so the shadow pass evaluates the
/// window's net state once.
pub type WriteId = String;

/// A single shadow pass produced at a snapshot-window boundary (①).
///
/// Exactly one of these is produced per non-empty window for an opted-in repo;
/// it carries the coalesced set of writes the pass must evaluate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShadowPass {
    /// The repo this pass belongs to.
    pub repo: String,
    /// The tenant whose budget this pass draws on.
    pub tenant_id: String,
    /// Monotonic window sequence number this pass closes.
    pub window_seq: u64,
    /// Coalesced (deduplicated) set of write ids observed in the window,
    /// in first-seen order.
    pub coalesced_writes: Vec<WriteId>,
}

/// Accumulates writes within one snapshot window and collapses them to at most
/// one [`ShadowPass`] at the boundary (①).
///
/// The scheduler is the mechanism that makes cadence = the `snapshot_window`
/// boundary rather than per write: any number of [`ShadowScheduler::record_write`]
/// calls within a window produce a single pass on
/// [`ShadowScheduler::close_window`]. The `snapshot_boundary` is the single
/// trigger; `record_write` never fires a pass on its own.
#[derive(Debug, Clone)]
pub struct ShadowScheduler {
    repo: String,
    tenant_id: String,
    window_seq: u64,
    pending: Vec<WriteId>,
}

impl ShadowScheduler {
    /// Open a scheduler for `repo`/`tenant_id`, starting at window 0.
    pub fn new(repo: impl Into<String>, tenant_id: impl Into<String>) -> Self {
        Self {
            repo: repo.into(),
            tenant_id: tenant_id.into(),
            window_seq: 0,
            pending: Vec::new(),
        }
    }

    /// The repo this scheduler serves.
    pub fn repo(&self) -> &str {
        &self.repo
    }

    /// The tenant this scheduler bills.
    pub fn tenant_id(&self) -> &str {
        &self.tenant_id
    }

    /// The current (open) window's sequence number.
    pub fn current_window(&self) -> u64 {
        self.window_seq
    }

    /// Number of distinct (coalesced) writes pending in the open window.
    pub fn pending_writes(&self) -> usize {
        self.pending.len()
    }

    /// Record a write in the current window.
    ///
    /// Coalescing (dedup): a `write_id` already present in the open window is a
    /// no-op — repeated writes to the same target collapse, so N raw writes
    /// (with K distinct targets) leave K pending entries, and the boundary still
    /// fires exactly one pass.
    pub fn record_write(&mut self, write_id: impl Into<WriteId>) {
        let id = write_id.into();
        if !self.pending.contains(&id) {
            self.pending.push(id);
        }
    }

    /// Close the current snapshot window, producing at most one [`ShadowPass`].
    ///
    /// - With ≥1 pending write → exactly **one** pass over the coalesced set
    ///   (① "not N").
    /// - With **zero** pending writes → **no** pass (① "not 0 spurious"): an
    ///   empty window does not trigger a shadow run.
    ///
    /// Either way the window counter advances and the pending set is cleared, so
    /// the next window starts fresh.
    pub fn close_window(&mut self) -> Option<ShadowPass> {
        let seq = self.window_seq;
        self.window_seq += 1;

        if self.pending.is_empty() {
            return None;
        }
        let coalesced = std::mem::take(&mut self.pending);
        Some(ShadowPass {
            repo: self.repo.clone(),
            tenant_id: self.tenant_id.clone(),
            window_seq: seq,
            coalesced_writes: coalesced,
        })
    }
}

// ── Budget decrement + cap halt (②) ─────────────────────────────────────────

/// Outcome of asking the budget engine to admit a shadow pass (②).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowDecision {
    /// Budget had capacity; one unit was drawn (decremented) and the shadow may
    /// run.
    Admitted,
    /// The per-tenant cap is reached: shadows **halt** for this tenant. The
    /// budget is left untouched. Explicit jobs are unaffected — they do not
    /// route through this gate.
    CapHalted,
}

/// Attempt to admit a shadow pass against the tenant's C7 budget (②).
///
/// On success, [`SHADOW_PASS_COST`] units are **decremented** from the tenant's
/// budget via the C7 engine and [`ShadowDecision::Admitted`] is returned. When
/// the cap is hit (insufficient budget), no units are drawn and
/// [`ShadowDecision::CapHalted`] is returned — the shadow is the **yielding**
/// workload, so it simply does not run; nothing is queued.
///
/// This deliberately differs from how C7 treats explicit jobs (which are queued,
/// never dropped, on exhaustion): a halted shadow is *skipped*, because a missed
/// shadow is merely deferred ambient truth, never lost work.
pub fn admit_shadow(mgr: &mut BudgetManager, tenant_id: &str) -> ShadowDecision {
    // C7's `try_dispatch` decrements on success and enqueues on exhaustion.
    // Shadows must NOT enqueue (they yield), so we inspect the tenant budget
    // directly and only draw when there is capacity.
    let has_capacity = mgr
        .budget(tenant_id)
        .map(|b| b.remaining >= SHADOW_PASS_COST)
        .unwrap_or(false);

    if !has_capacity {
        return ShadowDecision::CapHalted;
    }

    // Draw the unit through the published budget engine. Because we checked
    // capacity above, this returns `Available` and performs the decrement; we
    // discard any (empty) event stream since an admitted shadow emits no budget
    // event.
    let mut sink = Vec::new();
    let status = mgr.try_dispatch(
        tenant_id,
        format!("shadow:{tenant_id}"),
        SHADOW_PASS_COST,
        0,
        &mut sink,
    );
    debug_assert_eq!(status, BudgetStatus::Available);
    ShadowDecision::Admitted
}

// ── Non-gating surface (④) ──────────────────────────────────────────────────

/// The observable surface of a completed shadow pass (④).
///
/// A shadow NEVER gates an explicit job. Its outcome is *ambient truth*:
/// - a **passing** shadow produces an observable [`CheckResult`]
///   ([`ShadowOutcome::Result`]);
/// - a **failing** shadow surfaces as a signal/event ONLY
///   ([`ShadowOutcome::Signal`]) — it is information, never a verdict that can
///   block, fail, or gate any explicit job.
#[derive(Debug, Clone, PartialEq)]
pub enum ShadowOutcome {
    /// The shadow check passed; here is the observable, content-addressed
    /// result.
    Result(CheckResult),
    /// The shadow check failed. This is a **signal/event only** — it carries a
    /// human/agent-readable reason and the failing exit code, and is fed to
    /// observability sinks. It is structurally incapable of gating: there is no
    /// API on this type that an explicit-job path could consult to block.
    Signal {
        /// The window/pass that produced this signal.
        window_seq: u64,
        /// Non-zero exit code of the failing shadow check.
        exit: i32,
        /// Human/agent-readable summary for the event stream.
        reason: String,
    },
}

impl ShadowOutcome {
    /// True iff this outcome represents a passing shadow (an observable result).
    pub fn is_pass(&self) -> bool {
        matches!(self, ShadowOutcome::Result(_))
    }

    /// True iff this outcome is a non-gating failure signal.
    pub fn is_signal(&self) -> bool {
        matches!(self, ShadowOutcome::Signal { .. })
    }

    /// Interpret a [`CheckResult`] produced by a shadow runner as a non-gating
    /// outcome (④).
    ///
    /// `exit == 0` → an observable passing [`ShadowOutcome::Result`].
    /// `exit != 0` → a [`ShadowOutcome::Signal`] (event only). Note that EITHER
    /// way the explicit-job path is untouched: this function returns ambient
    /// truth and has no side effect on any job.
    pub fn from_check_result(window_seq: u64, result: CheckResult) -> Self {
        if result.exit == 0 {
            ShadowOutcome::Result(result)
        } else {
            ShadowOutcome::Signal {
                window_seq,
                exit: result.exit,
                reason: format!(
                    "shadow check failed (exit {}) on tree {}",
                    result.exit, result.tree_hash
                ),
            }
        }
    }
}

/// Terminal status of an explicit (non-shadow) job (④).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplicitJobStatus {
    /// The explicit job ran to completion and passed.
    Passed,
    /// The explicit job ran to completion and failed **on its own merits** —
    /// never because of any shadow outcome.
    Failed,
}

/// Run an explicit job to its verdict (④).
///
/// CRITICAL non-gating invariant: this function takes **no shadow input at all**
/// — there is deliberately no `ShadowOutcome` parameter. An explicit job's
/// verdict is a pure function of its own success, so it is *structurally*
/// impossible for a failing shadow to gate, block, or fail it. The shadow
/// surface ([`ShadowOutcome`]) and the explicit-job path are decoupled by
/// construction.
pub fn run_explicit_job(job_passes: bool) -> ExplicitJobStatus {
    if job_passes {
        ExplicitJobStatus::Passed
    } else {
        ExplicitJobStatus::Failed
    }
}

#[cfg(test)]
mod tests;
