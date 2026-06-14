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

pub mod account;
pub mod actions;
pub mod attention;
pub mod blob;
pub mod branches;
pub mod campaign;
pub mod checks;
pub mod commit_detail;
pub mod commits;
pub mod common;
pub mod compare;
pub mod dashboard;
pub mod edit;
pub mod github_app;
pub mod home;
pub mod import;
pub mod insights;
pub mod intent_detail;
pub mod issues;
pub mod knowledge;
pub mod landing;
pub mod login;
pub mod new_pr;
pub mod org;
pub mod pr_detail;
pub mod profile;
pub mod releases;
pub mod repo_chrome;
pub mod repo_settings;
pub mod review;
pub mod search;
pub mod security;
pub mod viewer_can;

pub use account::*;
pub use actions::*;
pub use attention::*;
pub use blob::*;
pub use branches::*;
pub use campaign::*;
pub use checks::*;
pub use commit_detail::*;
pub use commits::*;
pub use common::*;
pub use compare::*;
pub use dashboard::*;
pub use edit::*;
pub use github_app::*;
pub use home::*;
pub use import::*;
pub use insights::*;
pub use intent_detail::*;
pub use issues::*;
pub use knowledge::*;
pub use landing::*;
pub use login::*;
pub use new_pr::*;
pub use org::*;
pub use pr_detail::*;
pub use profile::*;
pub use releases::*;
pub use repo_chrome::*;
pub use repo_settings::*;
pub use review::*;
pub use search::*;
pub use security::*;
pub use viewer_can::*;
