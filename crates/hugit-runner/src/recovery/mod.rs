//! Crash recovery (WP-C2b item ⑤ / R2).
//!
//! A **box crash mid-job is distinct from expiry** (see [`expiry`](crate::expiry)).
//! Expiry is a deterministic end-of-life the runner *initiates*; a crash is an
//! *observed* loss of liveness — the container vanishes (box reboot, OOM-kill,
//! Docker daemon restart) while the job is still in flight.
//!
//! The degradation invariant (whitepaper §9 lock 5): a lost job must **fail
//! toward surfacing**. It is detected lost, then **requeued or surfaced** as a
//! defined [`LostJob`] status — **never silently dropped, never falsely green**.
//! Lease/fence cleanup reuses C2a's forensic [`teardown`](crate::teardown::teardown);
//! recovery does not re-implement teardown.
//!
//! # Liveness signal
//! Liveness is "the per-job container is still present/running on the box".
//! C2a spawns with `--rm`, so a crashed/killed container *disappears* from
//! `docker ps`. Absence of the container while the job has not reported a
//! terminal result is the lost-detection signal — the same signal a
//! heartbeat/lease-liveness probe would carry.

use anyhow::{Context, Result};
use hugit_contracts::{RunnerLease, RunnerState};

use crate::isolation::RunningContainer;
use crate::lease::BoxExec;
use crate::teardown::{ForensicReport, teardown};

/// Liveness of a per-job container as observed on the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Liveness {
    /// Container is present and running — job is alive.
    Alive,
    /// Container is gone (crash / OOM / daemon restart) — job lost.
    Lost,
}

/// What the runner decided to do with a job whose liveness was lost.
///
/// Both variants **surface** the loss; neither silently drops it and neither
/// reports success. This is the degradation invariant made type-level: a lost
/// job can only become `Requeued` or `Surfaced`, never green.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LostDisposition {
    /// Re-enqueued for another attempt (idempotent landing makes this safe).
    Requeued,
    /// Surfaced to the operator as a terminal lost-job status (no retry).
    Surfaced,
}

/// A detected lost job: the surfaced record that proves the loss was not
/// dropped and not faked green.
#[derive(Debug, Clone)]
pub struct LostJob {
    /// The C2b-namespaced container that went missing.
    pub container: String,
    /// The lost lease id.
    pub lease_id: String,
    /// Disposition: requeued or surfaced (never green).
    pub disposition: LostDisposition,
    /// The lease state after recovery — must be [`RunnerState::Crashed`].
    pub lease_state: RunnerState,
    /// Forensic re-scan after lease/fence cleanup — must be clean.
    pub residue: ForensicReport,
}

impl LostJob {
    /// `true` iff this lost job was handled per the degradation invariant:
    /// surfaced (requeued or surfaced), marked `Crashed`, and cleaned up — the
    /// negation of "silent drop or false green".
    #[must_use]
    pub fn is_surfaced_not_green(&self) -> bool {
        matches!(
            self.disposition,
            LostDisposition::Requeued | LostDisposition::Surfaced
        ) && self.lease_state == RunnerState::Crashed
            && self.residue.is_clean()
    }
}

/// Probe whether a per-job container is still alive on the box.
///
/// `Alive` iff `docker ps` (running only) reports the exact container name.
/// A `--rm` container that crashed has already been reaped → `Lost`.
///
/// # Errors
/// Fails only if the box is unreachable.
pub fn probe_liveness<B: BoxExec>(boxx: &B, c: &RunningContainer) -> Result<Liveness> {
    let out = boxx
        .run(&[
            "docker",
            "ps",
            "--filter",
            &format!("name={}", c.name),
            "--format",
            "{{.Names}}",
        ])
        .with_context(|| format!("liveness probe for {}", c.name))?;
    let alive = out.stdout.lines().map(str::trim).any(|l| l == c.name);
    Ok(if alive {
        Liveness::Alive
    } else {
        Liveness::Lost
    })
}

/// Recover a job whose container is **observed lost** mid-flight.
///
/// Steps, in order, all surfacing — never dropping:
/// 1. mark the lease [`RunnerState::Crashed`] (distinct from `Expired`);
/// 2. clean lease/fence by reusing C2a's forensic [`teardown`];
/// 3. record a [`LostJob`] with the requested `disposition` (requeue/surface).
///
/// The lease is **consumed, not mutated** (frozen): the new `Crashed` state is
/// reported in the returned [`LostJob`], not written back into the caller's
/// lease.
///
/// # Errors
/// Propagates box errors from teardown.
pub fn recover_lost<B: BoxExec>(
    boxx: &B,
    lease: &RunnerLease,
    c: &RunningContainer,
    disposition: LostDisposition,
) -> Result<LostJob> {
    // Lease/fence cleanup via C2a teardown (idempotent; the container may
    // already be gone after the crash).
    let residue =
        teardown(boxx, c).with_context(|| format!("lease/fence cleanup for lost {}", c.name))?;

    Ok(LostJob {
        container: c.name.clone(),
        lease_id: lease.lease_id.clone(),
        disposition,
        // Crash → Crashed, never Released/Expired: a lost job is never green.
        lease_state: RunnerState::Crashed,
        residue,
    })
}

/// Detect-and-recover in one call: probe liveness, and if `Lost`, recover.
///
/// Returns `Some(LostJob)` only when the job was detected lost; `None` when the
/// job is still alive (the happy path — no false-positive recovery).
///
/// # Errors
/// Propagates box errors.
pub fn detect_and_recover<B: BoxExec>(
    boxx: &B,
    lease: &RunnerLease,
    c: &RunningContainer,
    disposition: LostDisposition,
) -> Result<Option<LostJob>> {
    match probe_liveness(boxx, c)? {
        Liveness::Alive => Ok(None),
        Liveness::Lost => Ok(Some(recover_lost(boxx, lease, c, disposition)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lease() -> RunnerLease {
        RunnerLease {
            lease_id: "crash-1".to_string(),
            principal_chain: vec![],
            path_set: vec![],
            expiry: u64::MAX,
            net_policy: "none".to_string(),
            tmp_root: "/t".to_string(),
            state: RunnerState::Held,
        }
    }

    #[test]
    fn surfaced_lost_is_not_green() {
        let lj = LostJob {
            container: "hugit-c2b-x".to_string(),
            lease_id: lease().lease_id,
            disposition: LostDisposition::Requeued,
            lease_state: RunnerState::Crashed,
            residue: ForensicReport::default(),
        };
        assert!(lj.is_surfaced_not_green());
    }

    #[test]
    fn green_state_would_violate_invariant() {
        let lj = LostJob {
            container: "hugit-c2b-x".to_string(),
            lease_id: lease().lease_id,
            disposition: LostDisposition::Surfaced,
            lease_state: RunnerState::Released, // a false-green would set this
            residue: ForensicReport::default(),
        };
        assert!(!lj.is_surfaced_not_green());
    }

    #[test]
    fn dirty_residue_fails_invariant() {
        let lj = LostJob {
            container: "hugit-c2b-x".to_string(),
            lease_id: lease().lease_id,
            disposition: LostDisposition::Surfaced,
            lease_state: RunnerState::Crashed,
            residue: ForensicReport {
                containers: vec!["hugit-c2b-x".to_string()],
                ..Default::default()
            },
        };
        assert!(!lj.is_surfaced_not_green());
    }
}
