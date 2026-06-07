//! hugit-mirror — status/badge compatibility emitter (WP-E3).

pub mod bootstrap;
pub mod dr;
pub mod status;

// WP-E1a — one-way mirror (hugit → GitHub): outbound sync, per-push hash
// verify, durable ordered/capacity-bounded outage queue.
pub mod outbound;
pub mod queue;
pub mod verify;
// WP-E1b — mirror failure modes (additive).
pub mod divergence;
pub mod outage;
pub mod poll;
pub mod refops;
// WP-E2a: git history import (byte-identity, LFS, resumable, idempotency).
// Submodules: import::history, import::lfs, import::resume, import::auth.
pub mod import;
