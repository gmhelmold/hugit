//! Per-job isolation: spawn one container with a **private tmp** and an
//! **isolated network namespace**, run a job, and probe that the isolation
//! holds.
//!
//! v0 uses Docker on the runner box: `--tmpfs <tmp_root>` gives each job its
//! own tmpfs (invisible to and unshared with other jobs / the host), and
//! `--network none` gives each job an isolated network namespace with no
//! reachable interface. Both probes ([`IsolationProbe`]) are run inside the
//! live container against the live box.
//!
//! The Firecracker upgrade path (crate docs) implements the same [`Engine`]
//! against microVMs; the [`IsolationProbe`] contract is engine-independent.

use anyhow::{Context, Result, bail};

use crate::lease::{BoxExec, ContainerSpec};

/// Result of probing a running container's isolation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsolationProbe {
    /// `tmp_root` is mounted as a tmpfs distinct from the host filesystem.
    pub tmp_is_private: bool,
    /// The network namespace has no usable interface beyond loopback
    /// (`--network none`): no `eth*`, and outbound DNS/connect is impossible.
    pub net_is_isolated: bool,
}

impl IsolationProbe {
    /// `true` iff both tmp and network are isolated.
    #[must_use]
    pub fn fully_isolated(&self) -> bool {
        self.tmp_is_private && self.net_is_isolated
    }
}

/// A running per-job container handle.
#[derive(Debug, Clone)]
pub struct RunningContainer {
    /// Container name (== [`ContainerSpec::name`]).
    pub name: String,
}

/// The container engine seam. v0 has one implementation, [`DockerEngine`];
/// the Firecracker upgrade adds another against the same contract.
pub trait Engine {
    /// Spawn the per-job container detached, idling, per `spec`.
    fn spawn(&self, spec: &ContainerSpec) -> Result<RunningContainer>;

    /// Probe isolation of a running container against `spec`.
    fn probe(&self, c: &RunningContainer, spec: &ContainerSpec) -> Result<IsolationProbe>;

    /// Run one job command inside the container, returning its exit code.
    fn exec(&self, c: &RunningContainer, argv: &[&str]) -> Result<Option<i32>>;
}

/// Docker-backed [`Engine`], driving the box via a [`BoxExec`].
#[derive(Debug, Clone)]
pub struct DockerEngine<B: BoxExec> {
    /// The box the containers run on.
    pub boxx: B,
}

impl<B: BoxExec> DockerEngine<B> {
    /// Construct over a box transport.
    pub fn new(boxx: B) -> Self {
        Self { boxx }
    }
}

impl<B: BoxExec> Engine for DockerEngine<B> {
    fn spawn(&self, spec: &ContainerSpec) -> Result<RunningContainer> {
        if !spec.no_network {
            bail!("ContainerSpec.no_network must be true for C2a isolation");
        }
        let tmpfs = format!("{}:rw,size=64m", spec.tmp_root);
        let argv = vec![
            "docker",
            "run",
            "-d",
            "--rm",
            "--name",
            &spec.name,
            "--network",
            "none",
            "--tmpfs",
            &tmpfs,
            "--label",
            "hugit.job=1",
            &spec.image,
            "sleep",
            "3600",
        ];
        let out = self.boxx.run(&argv)?;
        if !out.ok() {
            bail!("docker run failed: {}", out.stderr.trim());
        }
        Ok(RunningContainer {
            name: spec.name.clone(),
        })
    }

    fn probe(&self, c: &RunningContainer, spec: &ContainerSpec) -> Result<IsolationProbe> {
        // tmp privacy: the mounted tmp_root must be a tmpfs, and a file written
        // there must not appear on the host filesystem.
        let marker = format!("hugit-isolation-{}", c.name);
        let write = self.boxx.run(&[
            "docker",
            "exec",
            &c.name,
            "sh",
            "-c",
            &format!(
                "mount | grep -q 'on {root} type tmpfs' && echo {m} > {root}/{m} && echo OK",
                root = spec.tmp_root,
                m = marker,
            ),
        ])?;
        let tmp_is_tmpfs = write.ok() && write.stdout.contains("OK");
        // Host must not see the in-container tmpfs marker anywhere on disk.
        let host_leak = self.boxx.run(&[
            "sh",
            "-c",
            &format!("find / -name {marker} 2>/dev/null | head -1"),
        ])?;
        let tmp_is_private = tmp_is_tmpfs && host_leak.stdout.trim().is_empty();

        // net isolation: no non-loopback interface, and an outbound connect
        // must fail (no route / no DNS) under `--network none`.
        let no_eth = self.boxx.run(&[
            "docker",
            "exec",
            &c.name,
            "sh",
            "-c",
            // Count non-loopback links. `grep -vc` exits 1 on a zero count, so
            // `printf` the result unconditionally to avoid a spurious fallback.
            "n=$(ip -o link show 2>/dev/null | grep -vc ' lo:'); printf '%s' \"$n\"",
        ])?;
        // Any non-loopback interface => not isolated. Parse defensively: a
        // non-numeric/garbled reading is treated as "interfaces present".
        let eth_count: i64 = no_eth.stdout.trim().parse().unwrap_or(i64::MAX);
        let connect = self.boxx.run(&[
            "docker",
            "exec",
            &c.name,
            "sh",
            "-c",
            // Any successful outbound connect would print REACHED; isolation
            // means this fails (timeout / unreachable).
            "timeout 4 sh -c 'echo > /dev/tcp/1.1.1.1/53 && echo REACHED' 2>/dev/null || echo BLOCKED",
        ])?;
        let net_is_isolated = eth_count == 0
            && connect.stdout.contains("BLOCKED")
            && !connect.stdout.contains("REACHED");

        Ok(IsolationProbe {
            tmp_is_private,
            net_is_isolated,
        })
    }

    fn exec(&self, c: &RunningContainer, argv: &[&str]) -> Result<Option<i32>> {
        let mut full = vec!["docker", "exec", &c.name];
        full.extend_from_slice(argv);
        let out = self
            .boxx
            .run(&full)
            .with_context(|| format!("docker exec in {}", c.name))?;
        Ok(out.code)
    }
}
