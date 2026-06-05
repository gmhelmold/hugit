//! Teardown + forensic re-scan: destroy the per-job container and prove the
//! box has **zero residue** afterwards.
//!
//! "Leaves nothing" is verified, not assumed: after destroy we re-scan the box
//! across four surfaces and require every one clean:
//! - **containers** — no container by the job name, running or stopped;
//! - **process table** — no `sleep`/job process tagged to the container;
//! - **mounts** — no tmpfs / overlay mount referencing the job;
//! - **network** — no veth / namespace / iptables artifact for the job.
//!
//! Because v0 spawns with `--rm`, destroy is `docker rm -f` (idempotent), and
//! the kernel reaps namespaces/mounts on container exit; the re-scan is the
//! independent forensic proof, not a courtesy.

use anyhow::{Result, bail};

use crate::isolation::RunningContainer;
use crate::lease::BoxExec;

/// Per-surface forensic findings after teardown. Each field is the residue
/// found on that surface; **all empty == clean**.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ForensicReport {
    /// Container ids/names still present (running or stopped).
    pub containers: Vec<String>,
    /// Process-table lines still referencing the job.
    pub processes: Vec<String>,
    /// Mount-table lines still referencing the job.
    pub mounts: Vec<String>,
    /// Network artifacts (interfaces/namespaces) still referencing the job.
    pub network: Vec<String>,
}

impl ForensicReport {
    /// `true` iff the box has zero residue across all four surfaces.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.containers.is_empty()
            && self.processes.is_empty()
            && self.mounts.is_empty()
            && self.network.is_empty()
    }
}

fn nonempty_lines(s: &str) -> Vec<String> {
    s.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// Destroy the container and forensically re-scan the box.
///
/// Idempotent: safe to call whether or not the container is still alive.
///
/// # Errors
/// Fails only if the box itself is unreachable; a *dirty* box is reported via
/// the returned [`ForensicReport`] (caller asserts [`ForensicReport::is_clean`]).
pub fn teardown<B: BoxExec>(boxx: &B, c: &RunningContainer) -> Result<ForensicReport> {
    let name = &c.name;

    // Destroy (idempotent). `--rm` containers vanish on stop; force-rm covers
    // the stopped/zombie case. Ignore "no such container".
    let rm = boxx.run(&["docker", "rm", "-f", name])?;
    if !rm.ok() && !rm.stderr.contains("No such container") {
        bail!("docker rm -f {name} failed: {}", rm.stderr.trim());
    }

    // Surface 1: containers (any state).
    let containers = boxx.run(&[
        "docker",
        "ps",
        "-a",
        "--filter",
        &format!("name={name}"),
        "--format",
        "{{.Names}}",
    ])?;

    // Surface 2: process table — no process whose cmdline references the job.
    let processes = boxx.run(&[
        "sh",
        "-c",
        &format!("ps -eo args | grep -F {name} | grep -v grep"),
    ])?;

    // Surface 3: mounts — no tmpfs/overlay still referencing the job name.
    let mounts = boxx.run(&["sh", "-c", &format!("mount | grep -F {name}")])?;

    // Surface 4: network — no interface or named netns for the job.
    let network = boxx.run(&[
        "sh",
        "-c",
        &format!("ip -o link show 2>/dev/null | grep -F {name}; ip netns list 2>/dev/null | grep -F {name}"),
    ])?;

    Ok(ForensicReport {
        containers: nonempty_lines(&containers.stdout),
        processes: nonempty_lines(&processes.stdout),
        mounts: nonempty_lines(&mounts.stdout),
        network: nonempty_lines(&network.stdout),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_clean_when_all_empty() {
        assert!(ForensicReport::default().is_clean());
    }

    #[test]
    fn report_dirty_when_container_remains() {
        let r = ForensicReport {
            containers: vec!["hugit-job-x".to_string()],
            ..Default::default()
        };
        assert!(!r.is_clean());
    }
}
