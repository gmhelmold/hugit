//! hugit-runner — ephemeral runner v0 (WP-C2a).
//!
//! The lifecycle + isolation half of the ephemeral runner: acquire a
//! [`RunnerLease`](hugit_contracts::RunnerLease), run **one** job in an
//! isolated container on a Hetzner-class box, and tear it down so that a
//! forensic re-scan of the box (disk + mounts + process table + network)
//! finds **zero** residue.
//!
//! # Scope (WP-C2a)
//! - [`lease`] — derive the per-job container spec from a frozen `RunnerLease`
//!   and drive commands on the runner box.
//! - [`isolation`] — spawn the per-job container with a **private tmp** and an
//!   **isolated network namespace**, and probe that isolation holds.
//! - [`teardown`] — destroy the container and forensically re-scan the box to
//!   prove nothing was left behind.
//!
//! Concurrency/throughput, expiry hard-kill, and crash recovery are **WP-C2b**;
//! cache-warm boot is **C3**; the Actions-YAML shim is **E4**; fence path
//! enforcement (ENOENT) is **C5a** and the secrets broker is **C5b**. None of
//! those are implemented here.
//!
//! # Runtime: container-per-job (Firecracker upgrade path)
//! v0 runs each job as a single Docker container on one Hetzner-class box.
//! Isolation is provided by Docker's default mount namespace plus an explicit
//! `--tmpfs` for the private tmp and `--network none` for the isolated network
//! namespace.
//!
//! **Firecracker upgrade path (documented, NOT built):** the same lease →
//! spec → spawn → teardown lifecycle is intended to retarget from a Docker
//! container to a Firecracker microVM. The lease carries no Docker-specific
//! fields, [`ContainerSpec`](lease::ContainerSpec) is engine-agnostic, and the
//! [`BoxExec`](lease::BoxExec) seam abstracts the box. To upgrade, implement a
//! Firecracker [`isolation::Engine`] (microVM per job, jailer for the mount
//! namespace, a tap-less / no-network device for net isolation) and the same
//! [`teardown`] forensic re-scan. The acceptance contract (destroy leaves
//! nothing; tmp/net isolated) is unchanged across engines.

pub mod concurrency;
pub mod expiry;
pub mod isolation;
pub mod lease;
pub mod pin;
pub mod recovery;
pub mod shim;
pub mod teardown;
pub mod ws;

pub use isolation::{Engine, IsolationProbe, RunningContainer};
pub use lease::{BoxExec, ContainerSpec, SshBox};
pub use pin::{PinnedImageRef, require_pinned};
pub use teardown::{ForensicReport, teardown};
