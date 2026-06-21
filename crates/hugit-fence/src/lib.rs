//! hugit-fence — the repo-side fence half: the secrets broker (WP-C5b) over
//! the runner wire seam.
//!
//! **Sparse materialization IS the fence** (whitepaper §6.1, §9 lock 1) — and
//! since the runner-transfer campaign (WP-R4, 2026-06-10) the enforcement
//! half that *implements* that physical fence (`materialize` — sparse hydrate
//! by path-set — and `enforce` — the ENOENT classifier/probe, WP-C5a) lives
//! with the execution core in **corelink-runners**, together
//! with the container-escape red-team harness whose load-bearing vector
//! drives the real classifier. Relocated, not weakened: every C5a proof and
//! red-team assertion runs unmodified against the same production code in its
//! new home.
//!
//! What stays here is what the FORGE owns:
//!
//! - [`broker`] — the secrets broker (WP-C5b): credentials never enter the
//!   runner; the broker alone holds secret material, performs privileged ops
//!   on a workspace's behalf, audits every call with the lease's principal
//!   chain, and **fails closed** when the store is down.
//! - [`seam`] — the minimal wire seam the broker drives instead of linking
//!   the runner crate: `BoxExec` / `CmdOutput` / `RunningContainer`,
//!   transcribed signatures whose **live implementation is the runner
//!   product across the wire** (disclosed). Equivalence with the transferred
//!   side is held by the wire contract (`conformance/` vectors,
//!   byte-identical in both repos), never by a shared crate.
//!
//! The broker keeps the fence traversal rule locally
//! (`util::normalize_path`, relocated verbatim from the transferred
//! `enforce` half) so its result-delivery guard can never diverge from the
//! fence's traversal policy.

pub mod broker;
pub mod seam;
mod util;

pub use broker::{
    AuditOutcome, AuditRecord, Broker, BrokerError, BrokerOp, BrokerRequest, BrokerResponse,
    CredentialScan, SecretRef, SecretStore, scan_credential_absent,
};
pub use seam::{BoxExec, CmdOutput, RunningContainer};
