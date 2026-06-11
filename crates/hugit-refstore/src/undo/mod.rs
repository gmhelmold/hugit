//! Undo as a **compensating event** (acceptance item ④).
//!
//! `undo` is never a rewrite and never a deletion. Undoing an operation appends
//! a *compensating* event to the log that restores the ref state to what it was
//! **before** the target operation. Both the original event and its compensator
//! remain in the chain, fully addressable — history is preserved, force-push
//! data loss is unexpressible by construction.
//!
//! # How a compensator is computed
//!
//! To undo the operation at `seq = target`, we replay the log up to (but not
//! including) `target` to recover the *prior* ref state, look at what ref the
//! target operation touched, and emit the event that restores that ref's prior
//! value:
//!
//! - target was `ref.update`/`ref.delete` on ref `R`, and `R` had value `v`
//!   before → compensator is `ref.update {R -> v}`.
//! - target touched ref `R`, and `R` did not exist before → compensator is
//!   `ref.delete {R}`.
//! - target was an inert event (touched no ref) → no compensator is needed; the
//!   ref state is already unchanged (a [`UndoError::NothingToCompensate`]).
//!
//! The compensator is a normal event: appending it via the frozen D1a
//! [`EventLog::append`] extends the same hash chain, so the result re-verifies
//! and the undo is itself part of history (and is itself undoable).

use crate::authz::{AuditedGuard, Decision, DenyReason, Endpoint};
use crate::intent::model::INTENT_LANDED_KIND;
use crate::log::EventLog;
use crate::replay::{ReplayError, replay_unchecked};
use crate::tamper::{TamperError, verify_chain};

/// The compensating action computed for an undo, before it is appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compensation {
    /// The event kind to append (`ref.update` or `ref.delete`).
    pub kind: String,
    /// The canonical-JSON payload for the compensating event.
    pub payload: String,
    /// The ref name being restored (for diagnostics / audit).
    pub ref_name: String,
}

/// Error from computing an undo compensator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UndoError {
    /// The target `seq` is not present in the log.
    OutOfRange { target: u64, len: u64 },
    /// The target operation touched no ref (inert event) — nothing to undo.
    NothingToCompensate { target: u64, kind: String },
    /// The target event's payload is malformed and cannot be interpreted.
    BadTargetPayload { target: u64, kind: String },
    /// The log failed chain verification (fail-closed).
    Tamper(TamperError),
    /// Replaying the pre-target prefix failed.
    Replay(ReplayError),
    /// The D14 guard denied the undo: `undo` is a Human-only stakeholder verb
    /// (the matrix in [`crate::authz`]), and the issuing principal was not a
    /// human (or was unrecognized). The compensating event was **not** appended;
    /// an `authz.denied` audit record (③) was written instead, so the denial is
    /// attributable. Fail-closed. Carries the [`DenyReason`] for the caller to
    /// map to its own structured error.
    Denied(DenyReason),
}

impl std::fmt::Display for UndoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UndoError::OutOfRange { target, len } => {
                write!(f, "undo target seq {target} out of range (log len {len})")
            }
            UndoError::NothingToCompensate { target, kind } => write!(
                f,
                "nothing to compensate at seq {target}: kind {kind} touches no ref"
            ),
            UndoError::BadTargetPayload { target, kind } => {
                write!(f, "undo target seq {target} has malformed {kind} payload")
            }
            UndoError::Tamper(e) => write!(f, "undo refused: {e}"),
            UndoError::Replay(e) => write!(f, "undo prefix replay failed: {e}"),
            UndoError::Denied(reason) => {
                write!(f, "undo denied: {} (undo is Human-only)", reason.code())
            }
        }
    }
}

impl std::error::Error for UndoError {}

impl From<TamperError> for UndoError {
    fn from(e: TamperError) -> Self {
        UndoError::Tamper(e)
    }
}

impl From<ReplayError> for UndoError {
    fn from(e: ReplayError) -> Self {
        UndoError::Replay(e)
    }
}

/// Compute the compensating action that would undo the operation at `target`,
/// without mutating the log.
///
/// Re-verifies the chain first (fail-closed), then replays the prefix `[0,
/// target)` to recover the prior value of the ref the target touched.
pub fn compute_compensation(log: &EventLog, target: u64) -> Result<Compensation, UndoError> {
    verify_chain(log.records())?;

    let records = log.records();
    let len = records.len() as u64;
    if target >= len {
        return Err(UndoError::OutOfRange { target, len });
    }

    let target_rec = &records[target as usize];
    let ref_name = match target_rec.kind.as_str() {
        // All ref-mutating kinds carry a `"ref"` field. `intent.landed` is a ref
        // mutation too (it advances `ref -> target`), so it is undoable exactly
        // like a raw push: the compensator is a raw ref event restoring the prior
        // value — never a fabricated intent.
        "ref.update" | "ref.delete" | INTENT_LANDED_KIND => {
            let v: serde_json::Value = serde_json::from_str(&target_rec.payload).map_err(|_| {
                UndoError::BadTargetPayload {
                    target,
                    kind: target_rec.kind.clone(),
                }
            })?;
            v.get("ref")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| UndoError::BadTargetPayload {
                    target,
                    kind: target_rec.kind.clone(),
                })?
                .to_string()
        }
        other => {
            return Err(UndoError::NothingToCompensate {
                target,
                kind: other.to_string(),
            });
        }
    };

    // Prior ref state = replay of everything strictly before the target.
    let prior = replay_unchecked(&records[..target as usize])?;

    let compensation = match prior.get(&ref_name) {
        // The ref existed before → restore it to its prior target.
        Some(prior_target) => Compensation {
            kind: "ref.update".to_string(),
            payload: canonical_update(&ref_name, prior_target),
            ref_name,
        },
        // The ref did not exist before → delete it to restore "absent".
        None => Compensation {
            kind: "ref.delete".to_string(),
            payload: canonical_delete(&ref_name),
            ref_name,
        },
    };

    Ok(compensation)
}

/// Undo the operation at `target` by appending its compensating event to `log`
/// — **through the D14 authorization guard** (`undo` is a Human-only verb).
///
/// `undo` is one of the four mutating forge verbs the D14 matrix gates
/// ([`crate::authz`]); the matrix declares it **Human-only** (a human stakeholder
/// control — `authz/mod.rs` row `undo`). This production path therefore routes
/// the authorization through the D14 [`AuditedGuard`] under [`Endpoint::Undo`],
/// classifying the issuing `principal_chain` (the [`authorize`](crate::authz::authorize)
/// chain route), rather than appending the compensator with the trusted raw
/// [`EventLog::append`] — closing the bypass the adversarial round caught
/// (P-GUARD-PATHS). Only a `human:`/`user:` actor is authorized; an
/// orchestrator/worker/model — or an unrecognized/empty principal — is **denied**
/// fail-closed: the compensating event is **not** appended, an `authz.denied`
/// audit record is written by the guard (③), and [`UndoError::Denied`] carries
/// the [`DenyReason`] (correctly distinguishing `not_permitted` from
/// `unrecognized_principal`).
///
/// On allow: computes the compensator with [`compute_compensation`], appends it
/// via the raw [`EventLog::append`] (a normal hash-chain extension), and returns
/// the compensation. The original event is untouched; both it and the
/// compensator remain addressable. (The guard authorizes and audits *denials*;
/// the authorized compensating mutation is recorded by this verb, exactly as
/// [`AuditedGuard`] is contracted — it audits only the denial.)
///
/// `principal_chain` and `recorded_at` describe who issued the undo and when,
/// exactly as for any other appended event. An empty chain has no actor and is
/// denied [`UnrecognizedPrincipal`](crate::authz::DenyReason::UnrecognizedPrincipal).
pub fn undo(
    log: &mut EventLog,
    target: u64,
    principal_chain: Vec<String>,
    recorded_at: u64,
) -> Result<Compensation, UndoError> {
    // Compute the compensator first (re-verifies the chain, fail-closed) so a
    // denial does not even reach the append path on a malformed target.
    let comp = compute_compensation(log, target)?;
    // D14 guard on the chain-classified principal. The guard audits denials (③)
    // into the same log; only an authorized (Human) actor proceeds to append the
    // compensating event.
    let decision = {
        let mut guard = AuditedGuard::new(log);
        let (decision, _audit) = guard.authorize(&principal_chain, Endpoint::Undo, recorded_at);
        decision
    };
    match decision {
        Decision::Allow => {
            log.append(
                comp.kind.clone(),
                principal_chain,
                comp.payload.clone(),
                recorded_at,
            );
            Ok(comp)
        }
        Decision::Deny(reason) => Err(UndoError::Denied(reason)),
    }
}

/// Canonical `ref.update` payload, byte-identical to the grammar D1a's replay
/// reads (`{"ref":<name>,"target":<oid>}`).
fn canonical_update(name: &str, target: &str) -> String {
    serde_json::json!({ "ref": name, "target": target }).to_string()
}

/// Canonical `ref.delete` payload (`{"ref":<name>}`).
fn canonical_delete(name: &str) -> String {
    serde_json::json!({ "ref": name }).to_string()
}

#[cfg(test)]
mod guard_tests {
    //! D14 guard on the `undo` verb (P-GUARD-PATHS). `undo` is Human-only; a
    //! non-human (or unrecognized) principal is denied + audited, and the
    //! compensating event is never appended. A human undo still succeeds.
    use super::*;
    use crate::log::EventLog;

    /// A log with two `ref.update`s on the same ref, so undoing seq 1 has a real
    /// compensator (restore the ref to the seq-0 value). Built with the trusted
    /// raw append (these are not the verb under guard).
    fn two_update_log() -> EventLog {
        let mut log = EventLog::new();
        let r = "refs/heads/main";
        log.append(
            "ref.update",
            vec!["user:gustavo".into()],
            canonical_update(r, &"a".repeat(40)),
            1,
        );
        log.append(
            "ref.update",
            vec!["user:gustavo".into()],
            canonical_update(r, &"b".repeat(40)),
            2,
        );
        log
    }

    #[test]
    fn human_undo_succeeds() {
        let mut log = two_update_log();
        let before = log.len();
        let comp = undo(&mut log, 1, vec!["user:gustavo".into()], 3).expect("a human may undo");
        assert_eq!(comp.kind, "ref.update");
        // exactly one event appended (the compensator), no audit record.
        assert_eq!(log.len(), before + 1);
        let last = log.records().last().unwrap();
        assert_eq!(last.kind, "ref.update");
        assert!(!log.records().iter().any(|r| r.kind == "authz.denied"));
    }

    #[test]
    fn non_human_undo_denied_and_audited() {
        // orchestrator, worker (subagent), and model are all denied for undo.
        for actor in ["orchestrator:lead", "agent:runner-03", "model:claude"] {
            let mut log = two_update_log();
            let before = log.len();
            let err =
                undo(&mut log, 1, vec![actor.into()], 3).expect_err("non-human undo is denied");
            match err {
                UndoError::Denied(DenyReason::NotPermitted { .. }) => {}
                other => panic!("expected NotPermitted denial for {actor}, got {other:?}"),
            }
            // The compensating event was NOT appended; only the audit record was.
            assert_eq!(
                log.len(),
                before + 1,
                "only the authz.denied audit was appended"
            );
            let last = log.records().last().unwrap();
            assert_eq!(last.kind, "authz.denied");
            assert!(last.payload.contains("\"endpoint\":\"undo\""));
            // No new ref.update/ref.delete compensator slipped onto the log.
            assert_eq!(
                log.records()
                    .iter()
                    .filter(|r| r.kind == "ref.update")
                    .count(),
                2,
                "no compensating ref.update appended on a denied undo"
            );
        }
    }

    #[test]
    fn unrecognized_principal_undo_denied() {
        let mut log = two_update_log();
        let err =
            undo(&mut log, 1, vec!["queue".into()], 3).expect_err("unrecognized principal denied");
        assert!(matches!(
            err,
            UndoError::Denied(DenyReason::UnrecognizedPrincipal)
        ));
        // empty chain → also unrecognized, denied.
        let mut log2 = two_update_log();
        let err2 = undo(&mut log2, 1, vec![], 3).expect_err("empty chain denied");
        assert!(matches!(
            err2,
            UndoError::Denied(DenyReason::UnrecognizedPrincipal)
        ));
    }
}
