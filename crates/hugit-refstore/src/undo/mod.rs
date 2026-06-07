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

/// Undo the operation at `target` by appending its compensating event to `log`.
///
/// Computes the compensator with [`compute_compensation`], then appends it via
/// the frozen D1a [`EventLog::append`] — a normal append that extends the hash
/// chain. The original event is untouched; both it and the compensator remain
/// addressable. Returns the compensation that was applied.
///
/// `principal_chain` and `recorded_at` describe who issued the undo and when,
/// exactly as for any other appended event.
pub fn undo(
    log: &mut EventLog,
    target: u64,
    principal_chain: Vec<String>,
    recorded_at: u64,
) -> Result<Compensation, UndoError> {
    let comp = compute_compensation(log, target)?;
    log.append(
        comp.kind.clone(),
        principal_chain,
        comp.payload.clone(),
        recorded_at,
    );
    Ok(comp)
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
