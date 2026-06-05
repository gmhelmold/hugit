//! Lease lifecycle: turn a frozen [`RunnerLease`] into a per-job container
//! spec, and drive commands on the runner box.
//!
//! The [`RunnerLease`] is **consumed, never modified** (it is frozen by
//! WP-00). C2a reads `lease_id`, `tmp_root`, `net_policy`, and `path_set` to
//! build an engine-agnostic [`ContainerSpec`]. Fence path enforcement (ENOENT)
//! is C5a; here the `path_set` only scopes the materialized view, and the
//! lease carries **no raw credentials** by construction (the broker is C5b).

use std::process::Command;

use anyhow::{Context, Result, bail};
use hugit_contracts::RunnerLease;

/// Network policy semantics understood by the v0 runner.
///
/// C2a's isolation contract requires an **isolated network namespace**. The
/// only v0 policy that satisfies "leaves nothing / fully isolated" is `none`
/// (no network device). Any other policy name is rejected as out of scope for
/// C2a (egress policies are a later, broker-mediated concern).
fn requires_no_network(net_policy: &str) -> bool {
    matches!(net_policy, "none" | "isolated" | "deny-all" | "")
}

/// An engine-agnostic, per-job container spec derived from a [`RunnerLease`].
///
/// Engine-agnostic on purpose: the Firecracker upgrade path (see crate docs)
/// reuses this spec unchanged. It holds no Docker-specific fields beyond the
/// image name, and no credentials (those never reach the runner — C5b).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSpec {
    /// Stable per-job container name, derived from the lease id. One lease →
    /// one job → one container.
    pub name: String,
    /// Container image to run the job in.
    pub image: String,
    /// In-container path mounted as a private tmpfs (the lease's `tmp_root`).
    pub tmp_root: String,
    /// Whether the container runs with **no** network device (`--network
    /// none`). Always `true` for a C2a-conformant lease.
    pub no_network: bool,
    /// Paths the lease scopes the materialized view to (informational in C2a;
    /// fence enforcement is C5a).
    pub path_set: Vec<String>,
}

impl ContainerSpec {
    /// Derive a per-job container spec from a frozen lease.
    ///
    /// # Errors
    /// Fails if the lease id is empty, `tmp_root` is empty, or the lease's
    /// `net_policy` is not a C2a-supported isolated policy.
    pub fn from_lease(lease: &RunnerLease, image: &str) -> Result<Self> {
        if lease.lease_id.trim().is_empty() {
            bail!("RunnerLease.lease_id is empty");
        }
        if lease.tmp_root.trim().is_empty() {
            bail!("RunnerLease.tmp_root is empty");
        }
        if !requires_no_network(&lease.net_policy) {
            bail!(
                "net_policy {:?} is not isolated; C2a v0 supports only \
                 network-isolated leases",
                lease.net_policy
            );
        }
        Ok(Self {
            name: container_name(&lease.lease_id),
            image: image.to_string(),
            tmp_root: lease.tmp_root.clone(),
            no_network: true,
            path_set: lease.path_set.clone(),
        })
    }
}

/// Sanitize a lease id into a Docker-safe container name. Docker names must
/// match `[a-zA-Z0-9][a-zA-Z0-9_.-]*`.
fn container_name(lease_id: &str) -> String {
    let mut s = String::with_capacity(lease_id.len() + 8);
    s.push_str("hugit-job-");
    for c in lease_id.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-') {
            s.push(c);
        } else {
            s.push('_');
        }
    }
    s
}

/// A seam over "run a command on the runner box and capture its output".
///
/// Abstracting the box (rather than hard-coding `ssh`) keeps the lifecycle
/// engine-agnostic and lets the Firecracker upgrade swap the transport. The
/// default implementation, [`SshBox`], drives `ssh` to the live Hetzner box.
pub trait BoxExec {
    /// Run `argv` on the box, returning `(exit_code, stdout, stderr)`.
    ///
    /// `argv` is executed as a single remote shell command (the elements are
    /// shell-quoted and joined). A `None` exit code means the command was
    /// killed by a signal.
    fn run(&self, argv: &[&str]) -> Result<CmdOutput>;
}

/// Captured result of a command run on the box.
#[derive(Debug, Clone)]
pub struct CmdOutput {
    /// Exit status; `None` if the process was killed by a signal.
    pub code: Option<i32>,
    /// Captured standard output.
    pub stdout: String,
    /// Captured standard error.
    pub stderr: String,
}

impl CmdOutput {
    /// `true` iff the command exited 0.
    #[must_use]
    pub fn ok(&self) -> bool {
        self.code == Some(0)
    }
}

/// `BoxExec` backed by `ssh` to the live runner box.
///
/// Host is taken from `HUGIT_RUNNER_HOST` (the suite pins
/// `91.99.11.196`). Reads the identity file from `~/.ssh/hugit-runner-01` if
/// present; otherwise relies on the agent / default key. This driver **never**
/// touches the box's ssh/firewall/fail2ban config.
#[derive(Debug, Clone)]
pub struct SshBox {
    /// `user@host` target for ssh.
    pub target: String,
    /// Optional identity file path.
    pub identity: Option<String>,
}

impl SshBox {
    /// Construct from `HUGIT_RUNNER_HOST` (the env the acceptance suite pins),
    /// defaulting the user to `root` and the identity to
    /// `~/.ssh/hugit-runner-01` when that file exists.
    ///
    /// # Errors
    /// Fails if `HUGIT_RUNNER_HOST` is unset/empty.
    pub fn from_env() -> Result<Self> {
        let host = std::env::var("HUGIT_RUNNER_HOST")
            .ok()
            .filter(|h| !h.trim().is_empty())
            .context("HUGIT_RUNNER_HOST is unset; the runner box is required")?;
        let identity = std::env::var("HOME").ok().and_then(|home| {
            let p = format!("{home}/.ssh/hugit-runner-01");
            std::path::Path::new(&p).exists().then_some(p)
        });
        Ok(Self {
            target: format!("root@{host}"),
            identity,
        })
    }
}

impl BoxExec for SshBox {
    fn run(&self, argv: &[&str]) -> Result<CmdOutput> {
        let remote = shell_join(argv);
        let mut cmd = Command::new("ssh");
        if let Some(id) = &self.identity {
            cmd.arg("-i").arg(id);
        }
        cmd.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=no")
            .arg("-o")
            .arg("ConnectTimeout=15")
            .arg(&self.target)
            .arg(&remote);
        let out = cmd
            .output()
            .with_context(|| format!("failed to spawn ssh to {}", self.target))?;
        Ok(CmdOutput {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// POSIX single-quote a command vector into one remote shell string.
fn shell_join(argv: &[&str]) -> String {
    argv.iter()
        .map(|a| {
            if a.is_empty() {
                "''".to_string()
            } else if a
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-' | b'/' | b':'))
            {
                (*a).to_string()
            } else {
                format!("'{}'", a.replace('\'', r"'\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_contracts::RunnerState;

    fn lease() -> RunnerLease {
        RunnerLease {
            lease_id: "lease/abc 123".to_string(),
            principal_chain: vec!["agent:1".to_string()],
            path_set: vec!["src/".to_string()],
            expiry: 0,
            net_policy: "none".to_string(),
            tmp_root: "/work/tmp".to_string(),
            state: RunnerState::Held,
        }
    }

    #[test]
    fn spec_sanitizes_name_and_forces_no_network() {
        let spec = ContainerSpec::from_lease(&lease(), "alpine:3.20").unwrap();
        assert_eq!(spec.name, "hugit-job-lease_abc_123");
        assert!(spec.no_network);
        assert_eq!(spec.tmp_root, "/work/tmp");
        assert_eq!(spec.image, "alpine:3.20");
    }

    #[test]
    fn spec_rejects_non_isolated_policy() {
        let mut l = lease();
        l.net_policy = "egress-allow".to_string();
        assert!(ContainerSpec::from_lease(&l, "alpine:3.20").is_err());
    }

    #[test]
    fn spec_rejects_empty_ids() {
        let mut l = lease();
        l.lease_id = "  ".to_string();
        assert!(ContainerSpec::from_lease(&l, "alpine:3.20").is_err());
    }

    #[test]
    fn shell_join_quotes_spaces() {
        assert_eq!(shell_join(&["echo", "a b"]), "echo 'a b'");
        assert_eq!(shell_join(&["ls", "/tmp"]), "ls /tmp");
    }
}
