//! Frozen wire view-models for the `/v1` HTTP surface (githugr window ⇄ hugit
//! engine), backend-API-v1 §1 reads.
//!
//! Every type here is transcribed **byte-for-field** from the canonical source
//! `../githugr/crates/githugr-vm/src/provider.rs` — field names, types, and serde
//! attributes (`#[serde(default)]`, `#[serde(rename)]`) and even the exact derives
//! (`Eq` only where the source has it — f64-bearing types are `PartialEq`-only)
//! match EXACTLY, so `hugit-serve` serializes precisely what the frozen
//! `githugr-live` client deserializes. A field/type/attr change here is a WIRE
//! BREAK; make it only in lock-step with the window (same discipline as the
//! byte-identical `conformance/` vectors). Each screen module carries a round-trip
//! test against the contract's Appendix-A canonical JSON — a single wrong field
//! turns that test red.
//!
//! ## Layout
//! - [`common`] — atoms reused across ≥2 screens (diff/verdict/union/cost/…),
//!   frozen ONCE so per-screen modules never redefine them (no drift).
//! - [`home`] · [`commits`] · [`landing`] · [`checks`] · [`pr_detail`] — one module
//!   per Wave-1 read; each owns only its screen-specific types and imports the
//!   shared atoms from [`common`].
//!
//! All types are re-exported flat (`hugit_http_contracts::RepoHomeVm`), mirroring
//! the single-namespace shape of the canonical `githugr-vm` source.
//!
//! Server-impl note: the `/v1` server (`hugit-serve`) is a minimal SYNCHRONOUS
//! HTTP server (not axum/tokio) — consistent with this sync, supply-chain-strict
//! workspace; the wire contract is server-impl-agnostic. Pure serde — NO heavy deps.

pub mod checks;
pub mod commits;
pub mod common;
pub mod home;
pub mod landing;
pub mod pr_detail;

pub use checks::*;
pub use commits::*;
pub use common::*;
pub use home::*;
pub use landing::*;
pub use pr_detail::*;
