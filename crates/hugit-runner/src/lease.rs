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
    /// Fails if the lease id is empty, `tmp_root` is empty or unsafe, the
    /// `net_policy` is not a C2a-supported isolated policy, or `image` is not
    /// content-(digest)-pinned (`repo@sha256:<64-hex>`). The pin requirement is
    /// the supply-chain floor (WP-X4): an unpinned image is rejected here,
    /// before the box is ever touched (fail-closed). Integrity verification of
    /// the pin against the box happens at [`Engine::spawn`](crate::isolation::Engine::spawn)
    /// time, before `docker run`.
    pub fn from_lease(lease: &RunnerLease, image: &str) -> Result<Self> {
        if lease.lease_id.trim().is_empty() {
            bail!("RunnerLease.lease_id is empty");
        }
        if lease.tmp_root.trim().is_empty() {
            bail!("RunnerLease.tmp_root is empty");
        }
        // `tmp_root` is interpolated into `--tmpfs <root>:…` and into `sh -c`
        // probe scripts on the box. An unsanitized value (e.g.
        // `"/x' ; touch /pwned ; echo '"`) is a root RCE on the runner box.
        // Restrict to an absolute path over a conservative, shell-inert charset.
        validate_tmp_root(&lease.tmp_root)?;
        if !requires_no_network(&lease.net_policy) {
            bail!(
                "net_policy {:?} is not isolated; C2a v0 supports only \
                 network-isolated leases",
                lease.net_policy
            );
        }
        // Supply-chain floor: reject any non-content-pinned image at spec-build
        // time so the unpinned/tag path can never reach `docker run`.
        crate::pin::require_pinned(image)?;
        Ok(Self {
            name: container_name(&lease.lease_id),
            image: image.to_string(),
            tmp_root: lease.tmp_root.clone(),
            no_network: true,
            path_set: lease.path_set.clone(),
        })
    }
}

/// Validate that `tmp_root` is an absolute path over a shell-inert charset.
///
/// Accepts `^/[A-Za-z0-9._/-]+$` only: a leading `/` then any of
/// alphanumeric, `.`, `_`, `/`, `-`. Every shell metacharacter (space, quote,
/// `;`, `|`, `&`, `$`, backtick, `(`, `)`, newline, …) is excluded, so the
/// value cannot break out of `--tmpfs` or a `sh -c` probe on the box.
///
/// # Errors
/// Fails if `tmp_root` is not absolute or contains a disallowed character.
fn validate_tmp_root(tmp_root: &str) -> Result<()> {
    if !tmp_root.starts_with('/') {
        bail!("RunnerLease.tmp_root {tmp_root:?} must be an absolute path (start with `/`)");
    }
    if tmp_root.len() < 2 {
        bail!("RunnerLease.tmp_root {tmp_root:?} is too short to be a real path");
    }
    if !tmp_root
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'/' | b'-'))
    {
        bail!(
            "RunnerLease.tmp_root {tmp_root:?} contains characters outside \
             ^/[A-Za-z0-9._/-]+$ — refused (shell-injection guard, fail CLOSED)"
        );
    }
    Ok(())
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

    /// Run `argv` on the box with `stdin` piped to the remote process.
    ///
    /// This is the safe channel for **untrusted bytes** (e.g. a serialized
    /// state payload): the bytes flow over stdin and are never interpolated
    /// into the command string, so no value can break out of the shell. The
    /// default implementation refuses (a transport that cannot stream stdin
    /// must not be handed untrusted payloads).
    ///
    /// # Errors
    /// Fails if the transport cannot stream stdin or the process cannot spawn.
    fn run_with_stdin(&self, _argv: &[&str], _stdin: &[u8]) -> Result<CmdOutput> {
        bail!("this BoxExec transport does not support stdin streaming");
    }
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

/// Path of the pinned `known_hosts` file for runner-box SSH.
///
/// Overridable via `HUGIT_RUNNER_KNOWN_HOSTS`; otherwise `$HOME/.hugit/known_hosts`
/// (falling back to a bare `.hugit/known_hosts` if `HOME` is unset). Paired with
/// `StrictHostKeyChecking=accept-new` this is **trust-on-first-use, pin
/// thereafter**: the first connection records the box's host key, and every
/// later connection is verified against that pin — so a MITM that swaps the host
/// key after first use is refused (unlike `StrictHostKeyChecking=no`, which
/// silently accepts ANY key on EVERY connection and thus pins nothing).
fn known_hosts_path() -> String {
    if let Ok(p) = std::env::var("HUGIT_RUNNER_KNOWN_HOSTS")
        && !p.trim().is_empty()
    {
        return p;
    }
    match std::env::var("HOME") {
        Ok(home) if !home.trim().is_empty() => format!("{home}/.hugit/known_hosts"),
        _ => ".hugit/known_hosts".to_string(),
    }
}

impl BoxExec for SshBox {
    fn run(&self, argv: &[&str]) -> Result<CmdOutput> {
        let remote = shell_join(argv);
        let known_hosts = known_hosts_path();
        let mut cmd = Command::new("ssh");
        if let Some(id) = &self.identity {
            cmd.arg("-i").arg(id);
        }
        // pin-on-first-use: accept-new records the host key on first contact and
        // verifies against the pinned UserKnownHostsFile on every connection
        // thereafter (not the blind-accept of StrictHostKeyChecking=no).
        cmd.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg(format!("UserKnownHostsFile={known_hosts}"))
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

    fn run_with_stdin(&self, argv: &[&str], stdin: &[u8]) -> Result<CmdOutput> {
        use std::io::Write;
        use std::process::Stdio;

        let remote = shell_join(argv);
        let known_hosts = known_hosts_path();
        let mut cmd = Command::new("ssh");
        if let Some(id) = &self.identity {
            cmd.arg("-i").arg(id);
        }
        // pin-on-first-use: accept-new records the host key on first contact and
        // verifies against the pinned UserKnownHostsFile on every connection
        // thereafter (not the blind-accept of StrictHostKeyChecking=no).
        cmd.arg("-o")
            .arg("BatchMode=yes")
            .arg("-o")
            .arg("StrictHostKeyChecking=accept-new")
            .arg("-o")
            .arg(format!("UserKnownHostsFile={known_hosts}"))
            .arg("-o")
            .arg("ConnectTimeout=15")
            .arg(&self.target)
            .arg(&remote)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn ssh to {}", self.target))?;
        // Write the untrusted payload over stdin (never via the command string).
        child
            .stdin
            .take()
            .context("ssh child has no stdin pipe")?
            .write_all(stdin)
            .context("writing payload to ssh stdin")?;
        let out = child
            .wait_with_output()
            .with_context(|| format!("waiting on ssh to {}", self.target))?;
        Ok(CmdOutput {
            code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

/// POSIX single-quote a command vector into one remote shell string.
///
/// Every argument is **always** single-quoted (no "looks-safe" allowlist
/// passthrough): an allowlist is one missed character away from an injection,
/// so the only safe rule is to quote unconditionally. Embedded single quotes
/// are escaped via the standard `'\''` idiom.
fn shell_join(argv: &[&str]) -> String {
    argv.iter()
        .map(|a| {
            if a.is_empty() {
                "''".to_string()
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

    /// A content-pinned image reference (the only kind `from_lease` accepts).
    const PIN: &str =
        "alpine@sha256:d9e853e87e55526f6b2917df91a2115c36dd7c696a35be12163d44e6e2a4b6bc";

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
        let spec = ContainerSpec::from_lease(&lease(), PIN).unwrap();
        assert_eq!(spec.name, "hugit-job-lease_abc_123");
        assert!(spec.no_network);
        assert_eq!(spec.tmp_root, "/work/tmp");
        assert_eq!(spec.image, PIN);
    }

    #[test]
    fn spec_rejects_non_isolated_policy() {
        let mut l = lease();
        l.net_policy = "egress-allow".to_string();
        assert!(ContainerSpec::from_lease(&l, PIN).is_err());
    }

    #[test]
    fn spec_rejects_empty_ids() {
        let mut l = lease();
        l.lease_id = "  ".to_string();
        assert!(ContainerSpec::from_lease(&l, PIN).is_err());
    }

    #[test]
    fn spec_rejects_unpinned_image() {
        // The supply-chain floor: a floating tag is refused at spec-build time,
        // before any box contact (WP-X4 on the real spawn surface).
        assert!(ContainerSpec::from_lease(&lease(), "alpine:3.20").is_err());
        assert!(ContainerSpec::from_lease(&lease(), "alpine").is_err());
        let tampered = "alpine@sha256:\
                        0000000000000000000000000000000000000000000000000000000000000000";
        // (tampered digest is *syntactically* pinned; integrity is caught at
        // spawn-time verify, not here — see the spawn-path tests.)
        assert!(ContainerSpec::from_lease(&lease(), tampered).is_ok());
    }

    #[test]
    fn spec_rejects_tmp_root_injection() {
        // tmp_root RCE guard (brutal review R4): a value that escapes `sh -c`
        // must be refused at from_lease before it can reach the box.
        let mut l = lease();
        l.tmp_root = "/x' ; touch /pwned ; echo '".to_string();
        let err = ContainerSpec::from_lease(&l, PIN).unwrap_err().to_string();
        assert!(
            err.contains("tmp_root"),
            "tmp_root injection must be rejected at from_lease; got: {err}"
        );
        // relative path also refused
        l.tmp_root = "relative/tmp".to_string();
        assert!(ContainerSpec::from_lease(&l, PIN).is_err());
        // command substitution refused
        l.tmp_root = "/$(reboot)".to_string();
        assert!(ContainerSpec::from_lease(&l, PIN).is_err());
        // a clean absolute path is accepted
        l.tmp_root = "/hugit/tmp".to_string();
        assert!(ContainerSpec::from_lease(&l, PIN).is_ok());
    }

    #[test]
    fn shell_join_always_quotes() {
        assert_eq!(shell_join(&["echo", "a b"]), "'echo' 'a b'");
        // Even "looks-safe" tokens are quoted — no allowlist passthrough.
        assert_eq!(shell_join(&["ls", "/tmp"]), "'ls' '/tmp'");
        // An injection attempt is fully neutralized by quoting.
        assert_eq!(shell_join(&["echo", "; rm -rf /"]), "'echo' '; rm -rf /'");
    }
}
