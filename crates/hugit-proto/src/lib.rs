//! hugit-proto — the git wire protocol over the CoreLink CAS.
//!
//! This crate is the **projection layer**: it serves a hugit repository to any
//! git client over smart-HTTP **protocol v2**, assembling packfiles from objects
//! held in the content-addressed store. Git is never broken — a clone produces a
//! valid git repository whose object bytes are byte-identical to what the GitHub
//! mirror holds for the same refs (whitepaper §2, §4 projection rule).
//!
//! Responsibilities, all READ-only (WP-D2a — D2 items ①②):
//!
//! - **negotiate** ([`read::negotiate`]) — protocol v2 capability advertisement,
//!   ref advertisement, and `want`/`have` negotiation. Refs are the D1 event-log
//!   *derived view* ([`hugit_refstore::RefState`]); this layer never owns ref
//!   state, it reads it.
//! - **pack** ([`read::pack`]) — pack assembly from CAS objects. Objects are
//!   fetched from the content-addressed store and encoded into a real git pack
//!   via a libgit2-class library (`gix-pack`); the pack format is NOT
//!   reimplemented here.
//! - **serve** ([`read::serve`]) — the `clone` and delta-only `fetch`
//!   entrypoints that tie negotiation to pack assembly.
//!
//! The client matrix / jj stacks / CPU-chunked fallback / scale ceilings +
//! degradation invariant (WP-D2b — D2 items ③④⑤⑥⑦) load and prove this read
//! path at its edges ([`read::clients`], [`read::fallback`], [`read::limits`]).
//! The write path (D3) is out of scope and lives elsewhere.

pub mod read;

pub use read::negotiate::{Capabilities, NegotiationError, RefAdvertisement, RefView, WantHave};
pub use read::pack::{
    CasObjectSource, GitObject, ObjectKind, ObjectSource, PackAssembly, PackError,
};
pub use read::serve::{ServeError, serve_clone, serve_fetch};
// WP-D2b — read-path edges: client matrix + jj stacks, CPU/chunked fallback,
// scale ceilings + degradation invariant.
pub use read::clients::{
    ChangeId, ClientKind, Stack, StackEntry, git_version_meets_floor, served_object_ids,
};
pub use read::fallback::{ServePlan, cpu_budget_ms, plan_serve, within_cpu_budget};
pub use read::limits::{
    Admission, CeilingRow, Dimension, SmartLayers, admit, ceiling_table, serve_clone_degradable,
};
