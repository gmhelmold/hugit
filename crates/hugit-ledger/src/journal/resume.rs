//! `ctx resume` — session reconstruction within the supported horizon.
//!
//! After a crash or agent replacement, `ctx resume` reconstructs the
//! session state from the journal within the **supported horizon**
//! (minutes-to-days, command catalog v2 §D). Beyond the horizon the
//! resume is REFUSED as documented — never a silent stale reconstruction.
//!
//! ## Honest form (whitepaper §2 Inversion 2)
//!
//! Journals are best-effort provenance — NOT bitwise context replay (cut per
//! catalog v2). The reconstruction is a summary of what the journal recorded,
//! not a deterministic re-execution. The honest contract: "here is what the
//! journal says the session did; you are not guaranteed a byte-identical
//! context window."
//!
//! ## Beyond-horizon behaviour (D11③ — documented)
//!
//! When `now_ms - last_recorded_at > DEFAULT_HORIZON_MS`, `ctx_resume`
//! returns `Err(ResumeError::BeyondHorizon { age_ms, horizon_ms })`.
//! The caller MUST surface the refusal to the agent; silent degradation is
//! not permitted.

use crate::journal::horizon::{DEFAULT_HORIZON_MS, HorizonResult, check_horizon};
use crate::journal::persist::{Journal, JournalEntry, JournalError, JournalStore};

/// A reconstructed session context — the best-effort summary of what the
/// crashed/replaced session did, derived from the journal.
///
/// This is NOT a bitwise context replay. It is provenance + notes, from
/// which a replacement agent can orient itself without starting cold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconstructedContext {
    /// Tenant that owns the session.
    pub tenant_id: String,
    /// Workspace the session was running inside.
    pub workspace_id: String,
    /// Intent the session was executing.
    pub intent_id: String,
    /// The journal entries that make up the reconstruction, in recorded order.
    pub entries: Vec<JournalEntry>,
    /// Unix epoch milliseconds of the most recent entry used in reconstruction.
    pub last_recorded_at: u64,
    /// The horizon that was active at resume time (milliseconds).
    pub horizon_ms: u64,
}

impl ReconstructedContext {
    /// Number of entries in the reconstruction.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the reconstruction is empty (no journal entries).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Errors from `ctx resume`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeError {
    /// The journal was not found for the requested binding.
    NotFound(JournalError),
    /// The journal's last recorded event is beyond the supported horizon.
    ///
    /// **Documented refusal** (D11③, catalog v2 "Journals + short-horizon
    /// resume", whitepaper §13 risk 2). The caller must surface this to the
    /// agent — silent stale reconstruction is never permitted.
    BeyondHorizon {
        /// Age of the journal at the time of the resume attempt (ms).
        age_ms: u64,
        /// The horizon that was exceeded (ms).
        horizon_ms: u64,
    },
    /// The journal is empty — nothing to reconstruct from.
    EmptyJournal,
}

impl std::fmt::Display for ResumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResumeError::NotFound(e) => write!(f, "journal not found: {e}"),
            ResumeError::BeyondHorizon { age_ms, horizon_ms } => write!(
                f,
                "ctx resume refused: journal age {age_ms}ms exceeds horizon {horizon_ms}ms \
                 (documented refusal — not a silent stale reconstruction; \
                 see command catalog v2 §D, whitepaper §13 risk 2)"
            ),
            ResumeError::EmptyJournal => {
                write!(f, "ctx resume refused: journal has no entries")
            }
        }
    }
}

impl std::error::Error for ResumeError {}

/// Attempt to reconstruct a session from its journal within the supported
/// resume horizon.
///
/// # Arguments
/// * `store` — the journal store for the requesting tenant.
/// * `tenant_id` — the tenant making the resume request.
/// * `workspace_id` — the workspace the session was running in.
/// * `intent_id` — the intent the session was executing.
/// * `now_ms` — current Unix epoch milliseconds (injected for testability).
///
/// # Returns
/// * `Ok(ReconstructedContext)` — within-horizon: reconstruction from journal.
/// * `Err(ResumeError::BeyondHorizon)` — beyond the supported horizon; the
///   resume is **refused** per the documented behaviour (D11③).
/// * `Err(ResumeError::NotFound)` — no journal exists for the binding.
/// * `Err(ResumeError::EmptyJournal)` — journal exists but has no entries.
pub fn ctx_resume(
    store: &JournalStore,
    tenant_id: &str,
    workspace_id: &str,
    intent_id: &str,
    now_ms: u64,
) -> Result<ReconstructedContext, ResumeError> {
    let journal = store
        .open(tenant_id, workspace_id, intent_id)
        .map_err(ResumeError::NotFound)?;

    ctx_resume_from_journal(journal, now_ms)
}

/// Resume from a directly-supplied `Journal` reference (for testing without
/// a store).
pub fn ctx_resume_from_journal(
    journal: &Journal,
    now_ms: u64,
) -> Result<ReconstructedContext, ResumeError> {
    let last_recorded_at = journal
        .last_recorded_at()
        .ok_or(ResumeError::EmptyJournal)?;

    // Horizon check: beyond the window → documented refusal.
    match check_horizon(last_recorded_at, now_ms, DEFAULT_HORIZON_MS) {
        HorizonResult::WithinHorizon => {}
        HorizonResult::BeyondHorizon { age_ms, horizon_ms } => {
            return Err(ResumeError::BeyondHorizon { age_ms, horizon_ms });
        }
    }

    Ok(ReconstructedContext {
        tenant_id: journal.key.tenant_id.clone(),
        workspace_id: journal.key.workspace_id.clone(),
        intent_id: journal.key.intent_id.clone(),
        entries: journal.entries.clone(),
        last_recorded_at,
        horizon_ms: DEFAULT_HORIZON_MS,
    })
}
