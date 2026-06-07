//! ⑥ THE MONEY GATE BINDS.
//!
//! Billing is STRUCTURALLY blocked until the exit report = PASS.
//! Rules:
//! - A FAIL report CANNOT enable billing.
//! - An InsufficientOrOutOfWindow report CANNOT enable billing.
//! - A DEGRADED gate-evaluator state = "insufficient" → fails CLOSED.
//! - The enable-billing transition is itself an audited `EventRecord`.
//!
//! This is a CONTROL, not a dashboard. Symmetric to D8⑥.

use crate::report::{ExitReport, ExitReportStatus};
use hugit_contracts::EventRecord;
use hugit_refstore::{canonical_json, compute_this_hash};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The state of the money-gate evaluator itself.
///
/// If the evaluator is `Degraded`, it cannot assess the report correctly
/// and must fail closed (treat as insufficient, block billing).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateEvaluatorState {
    /// The gate evaluator is functioning normally.
    Healthy,
    /// The gate evaluator is degraded; treat as insufficient → fail closed.
    Degraded,
}

/// The decision rendered by the money gate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MoneyGateDecision {
    /// Billing may be enabled; the exit report was PASS.
    Allow,
    /// Billing is blocked; reason provided.
    Block(String),
}

impl MoneyGateDecision {
    /// Returns `true` iff billing is allowed.
    pub fn is_allow(&self) -> bool {
        matches!(self, MoneyGateDecision::Allow)
    }
}

/// Errors that occur when attempting to enable billing.
#[derive(Debug, Error)]
pub enum EnableBillingError {
    /// The money gate blocked the enable-billing request.
    #[error("money gate blocked: {0}")]
    Blocked(String),
}

/// The money gate: structural enforcement that billing requires exit report = PASS.
pub struct MoneyGate {
    evaluator_state: GateEvaluatorState,
}

impl MoneyGate {
    /// Create a healthy money gate.
    pub fn new() -> Self {
        Self {
            evaluator_state: GateEvaluatorState::Healthy,
        }
    }

    /// Create a money gate with a specific evaluator state (for testing/production setup).
    pub fn with_state(evaluator_state: GateEvaluatorState) -> Self {
        Self { evaluator_state }
    }

    /// Evaluate the gate given an exit report.
    ///
    /// Returns `Allow` only if:
    /// 1. The evaluator is `Healthy` (not `Degraded`).
    /// 2. The report status is `Pass`.
    ///
    /// Any other combination → `Block(reason)`.
    pub fn evaluate(&self, report: &ExitReport) -> MoneyGateDecision {
        // DEGRADED evaluator → fail closed (treat as insufficient).
        if self.evaluator_state == GateEvaluatorState::Degraded {
            return MoneyGateDecision::Block(
                "gate evaluator is DEGRADED — treating as insufficient; billing blocked (fail closed)".to_string(),
            );
        }

        match &report.status {
            ExitReportStatus::Pass => MoneyGateDecision::Allow,
            ExitReportStatus::Fail(reason) => MoneyGateDecision::Block(format!(
                "exit report is FAIL ({}); billing blocked",
                reason
            )),
            ExitReportStatus::InsufficientOrOutOfWindow(reason) => {
                MoneyGateDecision::Block(format!(
                    "exit report is insufficient/out-of-window ({}); billing blocked",
                    reason
                ))
            }
        }
    }

    /// Attempt to enable billing.
    ///
    /// # Returns
    ///
    /// - `Ok(EventRecord)` — billing is allowed; returns the audited
    ///   enable-billing event that MUST be appended to the event log.
    /// - `Err(EnableBillingError::Blocked)` — gate blocked; billing cannot
    ///   be enabled.
    ///
    /// # Audit
    ///
    /// The returned `EventRecord` is the audited enable-billing event.
    /// Callers MUST append it to the append-only event log before proceeding.
    /// The event kind is `"hugit.billing.enable"`.
    pub fn try_enable_billing(
        &self,
        report: &ExitReport,
        principal: &str,
        prev_hash: &str,
        seq: u64,
    ) -> Result<EventRecord, EnableBillingError> {
        let decision = self.evaluate(report);
        match decision {
            MoneyGateDecision::Block(reason) => Err(EnableBillingError::Blocked(reason)),
            MoneyGateDecision::Allow => {
                // Construct the audited enable-billing event. The payload is
                // canonical JSON (sorted keys, no insignificant whitespace) so
                // the chained bytes match what any verifier re-canonicalises.
                let raw_payload = serde_json::json!({
                    "action": "enable_billing",
                    "exit_report_status": "PASS",
                    "retention_rate": report.retention_rate,
                    "unprompted_count": report.unprompted_count,
                    "team_count": report.team_count,
                    "gate_evaluator_state": "Healthy",
                })
                .to_string();
                let payload = canonical_json(&raw_payload).unwrap_or(raw_payload);

                // Single-source the chain hash via the canonical formula
                // (hugit_refstore::compute_this_hash) — never re-transcribe.
                let kind = "hugit.billing.enable";
                let principal_chain = vec![principal.to_string()];
                let this_hash = compute_this_hash(prev_hash, kind, &principal_chain, &payload, seq);

                Ok(EventRecord {
                    seq,
                    prev_hash: prev_hash.to_string(),
                    this_hash,
                    kind: kind.to_string(),
                    principal_chain,
                    payload,
                    recorded_at: current_epoch_ms(),
                })
            }
        }
    }
}

impl Default for MoneyGate {
    fn default() -> Self {
        Self::new()
    }
}

/// Return current Unix epoch milliseconds (stub for no-std compat).
fn current_epoch_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
