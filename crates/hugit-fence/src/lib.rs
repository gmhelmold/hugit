//! hugit-fence — claim-fenced workspaces, fence half (WP-C5a).
//!
//! **Sparse materialization IS the fence** (whitepaper §6.1, §9 lock 1). A
//! claim-fenced workspace physically materializes *only* the paths listed in
//! its [`FenceManifest`](hugit_contracts::FenceManifest) `path_set`. Anything
//! outside that set is simply not there, so any access to an outside path
//! returns **ENOENT** — not a permission denial layered on top, but physical
//! absence.
//!
//! # Scope (WP-C5a)
//! - [`materialize`] — sparse hydrate by path-set: filter a set of candidate
//!   workspace entries by a frozen `FenceManifest` and write **only** the
//!   in-fence entries into the runner workspace, then record what was
//!   materialized back into the manifest.
//! - [`enforce`] — the ENOENT guarantee: classify any path as inside/outside
//!   the fence, and probe the live box to prove that an access outside the
//!   `path_set` returns ENOENT (the file isn't there).
//!
//! The secrets broker + escape red-team harness are **WP-C5b** (a `broker`
//! submodule, disjoint from this crate's claims); C5a holds no credentials.
//!
//! # Runtime: container-per-job (consumes WP-C2a)
//! The fence does **not** re-implement materialization or the box transport.
//! It consumes the C2a runner's public API — [`BoxExec`](hugit_runner::lease::BoxExec)
//! to drive the box and a [`RunningContainer`](hugit_runner::isolation::RunningContainer)
//! to scope the workspace — and filters the materialized view by the
//! `FenceManifest`. The C2a lease lifecycle / isolation / teardown are
//! consumed, never rewritten.

pub mod enforce;
pub mod materialize;

pub use enforce::{FenceVerdict, FenceViolation, classify, probe_outside_enoent};
pub use materialize::{CandidateEntry, MaterializeError, materialize_sparse};
