//! Expiry hard-kill (WP-C2b item ③).
//!
//! Expiry is **deterministic** from the frozen
//! [`RunnerLease::expiry`](hugit_contracts::RunnerLease) field (Unix epoch
//! milliseconds). When wall-clock passes that instant the container is
//! **hard-killed** (`docker kill`, SIGKILL — not a graceful stop) and the lease
//! is torn down through C2a's forensic [`teardown`](crate::teardown::teardown).
//!
//! Expiry is **distinct from a crash** (handled in [`recovery`](crate::recovery)):
//! expiry is a deterministic, expected end-of-life that the runner *initiates*;
//! a crash is an *observed* loss of liveness. The two never share a code path.

use anyhow::{Context, Result, bail};
use hugit_contracts::RunnerLease;

use crate::isolation::RunningContainer;
use crate::lease::BoxExec;
use crate::teardown::{ForensicReport, teardown};

/// Result of an expiry decision + (optional) hard-kill.
#[derive(Debug, Clone)]
pub struct ExpiryOutcome {
    /// `true` iff the lease was expired at `now_ms` and the container was
    /// hard-killed.
    pub killed: bool,
    /// Whether the container was actually present/running before the kill
    /// (`docker kill` reported success). `false` for an already-gone container.
    pub was_running: bool,
    /// Forensic re-scan after teardown — must be clean when `killed`.
    pub residue: ForensicReport,
}

/// `true` iff the lease has expired at wall-clock `now_ms`.
///
/// Deterministic from [`RunnerLease::expiry`]; `expiry == u64::MAX` is the
/// "never expires" sentinel the acceptance leases use for non-expiry items.
#[must_use]
pub fn is_expired(lease: &RunnerLease, now_ms: u64) -> bool {
    now_ms >= lease.expiry
}

/// Hard-kill a container immediately: `docker kill` (SIGKILL), then C2a
/// teardown + forensic re-scan.
///
/// Unlike a graceful `docker stop`, this sends SIGKILL so an expired job cannot
/// linger past its lease. Idempotent: an already-dead container still yields a
/// clean teardown.
///
/// # Errors
/// Fails only if the box is unreachable; a dirty box surfaces via the returned
/// [`ForensicReport`].
pub fn hard_kill<B: BoxExec>(boxx: &B, c: &RunningContainer) -> Result<ExpiryOutcome> {
    // SIGKILL the running container. Ignore "is not running" / "No such
    // container" so the call is idempotent and teardown still re-scans.
    let kill = boxx
        .run(&["docker", "kill", "--signal", "KILL", &c.name])
        .with_context(|| format!("docker kill {}", c.name))?;
    let was_running = kill.ok();
    if !was_running
        && !kill.stderr.contains("is not running")
        && !kill.stderr.contains("No such container")
        && !kill.stderr.contains("Cannot kill")
    {
        bail!("docker kill {} failed: {}", c.name, kill.stderr.trim());
    }

    let residue =
        teardown(boxx, c).with_context(|| format!("teardown after kill of {}", c.name))?;
    Ok(ExpiryOutcome {
        killed: true,
        was_running,
        residue,
    })
}

/// Enforce expiry for one lease at wall-clock `now_ms`: if expired, hard-kill +
/// teardown; otherwise a no-op outcome.
///
/// This is the single deterministic entry point the scheduler calls on its
/// expiry tick.
///
/// # Errors
/// Propagates [`hard_kill`] box errors.
pub fn enforce_expiry<B: BoxExec>(
    boxx: &B,
    lease: &RunnerLease,
    c: &RunningContainer,
    now_ms: u64,
) -> Result<ExpiryOutcome> {
    if is_expired(lease, now_ms) {
        hard_kill(boxx, c)
    } else {
        Ok(ExpiryOutcome {
            killed: false,
            was_running: true,
            residue: ForensicReport::default(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_contracts::RunnerState;

    fn lease(expiry: u64) -> RunnerLease {
        RunnerLease {
            lease_id: "exp".to_string(),
            principal_chain: vec![],
            path_set: vec![],
            expiry,
            net_policy: "none".to_string(),
            tmp_root: "/t".to_string(),
            state: RunnerState::Held,
        }
    }

    #[test]
    fn expiry_is_deterministic_from_field() {
        let l = lease(1000);
        assert!(!is_expired(&l, 999));
        assert!(is_expired(&l, 1000));
        assert!(is_expired(&l, 1001));
    }

    #[test]
    fn never_sentinel_does_not_expire() {
        assert!(!is_expired(&lease(u64::MAX), u64::MAX - 1));
    }
}
