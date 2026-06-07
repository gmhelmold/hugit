//! THE GATE BINDS — the fail-CLOSED control surface (contract ⑥).
//!
//! This is a CONTROL, not a dashboard. Two features are structurally pinned —
//! **claims-as-oracle** stays advisory/OFF and **regen promotion** stays
//! BLOCKED — until a GENUINE, attested report shows PASS. The state machine
//! makes any other path impossible:
//!   - a FAIL or insufficient-n report CANNOT flip either feature;
//!   - a DEGRADED gate-evaluator state = "insufficient" → fails CLOSED;
//!   - a forged/swapped PASS report is rejected at this surface (⑨);
//!   - a post-hoc-mutated corpus invalidates the verdict (⑤);
//!   - the promotion event itself is audited (and every refusal too).
//!
//! Symmetric to B9⑥ (the money gate).

use hugit_contracts::{EventRecord, RegenGate};

use crate::experiment::audit::{ExperimentEvent, emit_event};
use crate::experiment::corpus::SealedCorpus;
use crate::experiment::report::{GateReport, ReportError, ReportVerdict};

/// The two gated features, and their structurally-pinned default state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeatureState {
    /// claims-as-oracle: pinned advisory/OFF until a genuine PASS.
    ClaimsOracleAdvisoryOff,
    /// claims-as-oracle: promoted to ON (only reachable via a genuine PASS).
    ClaimsOracleOn,
    /// regen promotion: structurally BLOCKED until a genuine PASS.
    RegenPromotionBlocked,
    /// regen promotion: promoted (only reachable via a genuine PASS).
    RegenPromotionPromoted,
}

/// Which feature a promotion attempt targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Promotion {
    /// Flip claims-as-oracle from advisory/OFF → ON.
    ClaimsOracle,
    /// Flip regen promotion from BLOCKED → PROMOTED.
    RegenPromotion,
}

impl Promotion {
    fn label(self) -> &'static str {
        match self {
            Promotion::ClaimsOracle => "claims-as-oracle",
            Promotion::RegenPromotion => "regen-promotion",
        }
    }
}

/// The health of the gate evaluator itself (⑥). A DEGRADED evaluator cannot be
/// trusted to have run a genuine evaluation → its verdict is treated as
/// "insufficient" and the gate fails CLOSED.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluatorHealth {
    /// Evaluator healthy — its verdict may be honored.
    Healthy,
    /// Evaluator degraded — verdict NOT trustworthy; fail-closed.
    Degraded,
}

/// Why a promotion attempt was refused (every variant fails CLOSED).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateError {
    /// The report verdict is not PASS (FAIL or insufficient-n) (⑥).
    NotPass(ReportVerdict),
    /// The gate evaluator is degraded → treated as insufficient (⑥).
    EvaluatorDegraded,
    /// The report is forged/swapped or covers a different corpus (⑨).
    ForgedReport,
    /// The sealed corpus was mutated post-hoc → verdict invalidated (⑤).
    CorpusTampered,
}

/// The outcome of a promotion attempt: the resulting (pinned-or-promoted)
/// feature state plus the audited event that recorded the decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    /// The feature state AFTER the attempt (unchanged on refusal — still pinned).
    pub state: FeatureState,
    /// The audited event emitted for this attempt (authorized OR refused).
    pub event: EventRecord,
}

/// A promotion attempt presented to the gate.
pub struct PromotionAttempt<'a> {
    /// Which feature to flip.
    pub promotion: Promotion,
    /// The generated report being presented as authorization.
    pub report: &'a GateReport,
    /// The sealed corpus the report claims to cover (re-verified here).
    pub corpus: &'a SealedCorpus,
    /// The evaluator's health at decision time.
    pub evaluator: EvaluatorHealth,
    /// Who is attempting the promotion (audit attribution).
    pub principal: &'a str,
}

/// The binding control surface.
///
/// Holds the structurally-pinned feature states and the only method that can
/// ever flip them: [`ExperimentGate::attempt_promotion`].
#[derive(Debug, Clone)]
pub struct ExperimentGate {
    claims_oracle: FeatureState,
    regen_promotion: FeatureState,
}

impl Default for ExperimentGate {
    fn default() -> Self {
        Self::new()
    }
}

impl ExperimentGate {
    /// A fresh gate with both features structurally pinned (the only valid
    /// initial state): claims-as-oracle advisory/OFF, regen promotion BLOCKED.
    pub fn new() -> Self {
        ExperimentGate {
            claims_oracle: FeatureState::ClaimsOracleAdvisoryOff,
            regen_promotion: FeatureState::RegenPromotionBlocked,
        }
    }

    /// Current claims-as-oracle state.
    pub fn claims_oracle_state(&self) -> FeatureState {
        self.claims_oracle
    }

    /// Current regen-promotion state.
    pub fn regen_promotion_state(&self) -> FeatureState {
        self.regen_promotion
    }

    /// Whether claims-as-oracle is still pinned advisory/OFF.
    pub fn is_claims_oracle_pinned(&self) -> bool {
        self.claims_oracle == FeatureState::ClaimsOracleAdvisoryOff
    }

    /// Whether regen promotion is still structurally blocked.
    pub fn is_regen_blocked(&self) -> bool {
        self.regen_promotion == FeatureState::RegenPromotionBlocked
    }

    /// Attempt to promote a feature — the ONLY mutation path (⑥).
    ///
    /// The gate flips a feature iff, in order, ALL hold:
    ///   1. the evaluator is healthy (degraded ⇒ insufficient ⇒ CLOSED),
    ///   2. the report verifies — not forged/swapped, covers this corpus (⑨),
    ///      and the bound corpus is untampered (⑤),
    ///   3. the report verdict is exactly PASS (FAIL/insufficient ⇒ CLOSED).
    ///
    /// On success the targeted feature flips and a `PromotionAuthorized` event
    /// is emitted. On ANY failure the feature stays pinned and a
    /// `PromotionRefused` event is emitted. Either way the decision is audited
    /// and returned in the [`Verdict`].
    pub fn attempt_promotion(
        &mut self,
        log: &mut Vec<EventRecord>,
        attempt: PromotionAttempt<'_>,
    ) -> Result<Verdict, GateError> {
        // 1 — degraded evaluator = insufficient = fail-closed.
        if attempt.evaluator == EvaluatorHealth::Degraded {
            let event = self.refuse(
                log,
                attempt.promotion,
                attempt.principal,
                "evaluator-degraded",
            );
            return Err(refusal_error(GateError::EvaluatorDegraded, event));
        }

        // 2 — authenticity + corpus integrity (⑨ + ⑤).
        match attempt.report.verify(attempt.corpus) {
            Ok(()) => {}
            Err(ReportError::Forged) => {
                let event = self.refuse(log, attempt.promotion, attempt.principal, "forged-report");
                return Err(refusal_error(GateError::ForgedReport, event));
            }
            Err(ReportError::CorpusTampered) => {
                let event =
                    self.refuse(log, attempt.promotion, attempt.principal, "corpus-tampered");
                return Err(refusal_error(GateError::CorpusTampered, event));
            }
        }

        // 3 — only a genuine PASS authorizes.
        let verdict = attempt.report.verdict();
        if verdict != ReportVerdict::Pass {
            let event = self.refuse(log, attempt.promotion, attempt.principal, "not-pass");
            return Err(refusal_error(GateError::NotPass(verdict), event));
        }

        // Authorized — flip the targeted feature and audit it.
        let new_state = match attempt.promotion {
            Promotion::ClaimsOracle => {
                self.claims_oracle = FeatureState::ClaimsOracleOn;
                self.claims_oracle
            }
            Promotion::RegenPromotion => {
                self.regen_promotion = FeatureState::RegenPromotionPromoted;
                self.regen_promotion
            }
        };

        let payload = format!(
            r#"{{"feature":"{}","verdict":"PASS","corpus_seal":"{}","n":{}}}"#,
            attempt.promotion.label(),
            attempt.report.corpus_seal(),
            attempt.report.n()
        );
        let event = emit_event(
            log,
            ExperimentEvent::PromotionAuthorized,
            attempt.principal,
            &payload,
            0,
        );

        Ok(Verdict {
            state: new_state,
            event,
        })
    }

    /// Apply a genuine, gate-authorized regen promotion to a [`RegenGate`].
    ///
    /// This is the ONLY way the consumed `RegenGate.repass` flips to `true` via
    /// this control: the targeted feature must already be PROMOTED (i.e. a prior
    /// [`attempt_promotion`](ExperimentGate::attempt_promotion) succeeded).
    /// While still blocked, the gate is returned unchanged — fail-closed.
    pub fn apply_regen_promotion(&self, gate: RegenGate) -> RegenGate {
        if self.regen_promotion == FeatureState::RegenPromotionPromoted {
            RegenGate {
                repass: true,
                ..gate
            }
        } else {
            RegenGate {
                repass: false,
                ..gate
            }
        }
    }

    /// Emit a `PromotionRefused` audit event and leave all features pinned.
    fn refuse(
        &self,
        log: &mut Vec<EventRecord>,
        promotion: Promotion,
        principal: &str,
        reason: &str,
    ) -> EventRecord {
        let payload = format!(
            r#"{{"feature":"{}","reason":"{}","action":"refused"}}"#,
            promotion.label(),
            reason
        );
        emit_event(
            log,
            ExperimentEvent::PromotionRefused,
            principal,
            &payload,
            0,
        )
    }
}

/// Pack a refusal error with its audited event. The error variant carries the
/// reason; the event is the tamper-evident record. (We return the error to the
/// caller and the event is already in the log.)
fn refusal_error(err: GateError, _event: EventRecord) -> GateError {
    err
}
