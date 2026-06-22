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
//!
//! The **write path** ([`write`]) carries the push concurrency / total-order +
//! external-change + flag/negative halves (WP-D3b — D3 items ②③④⑤): concurrent
//! pushes get a strict total order with stale rejection, a raw push is an opaque
//! external-change event with attribution (never a fabricated intent), and the
//! whole path is off unless the self-hosted-alpha flag is set.

pub mod read;
pub mod write;

pub use read::negotiate::{Capabilities, NegotiationError, RefAdvertisement, RefView, WantHave};
pub use read::pack::{
    CasObjectSource, FileChange, FileDiff, GitObject, ObjectKind, ObjectSource, PackAssembly,
    PackError, TreeEntry as ProtoTreeEntry, commit_root_tree, list_tree_at_dir,
    resolve_blob_at_path, tree_diff,
};
pub use read::serve::{ServeError, serve_clone, serve_fetch};
// WP-D2b — read-path edges: client matrix + jj stacks, CPU/chunked fallback,
// scale ceilings + degradation invariant.
pub use read::clients::{
    ChangeId, ClientKind, Stack, StackEntry, git_version_meets_floor, served_object_ids,
};
pub use read::fallback::{
    PACK_BYTES_PER_MS, ServePlan, cpu_budget_ms, measure_serve_cost_ms, plan_serve,
    plan_serve_measured, within_cpu_budget,
};
pub use read::limits::{
    Admission, CeilingRow, DegradedServe, Dimension, SmartLayers, admit, ceiling_table,
    serve_clone_degradable,
};

// WP-D3b — write path: push concurrency/total-order, external-change, flag-gate.
pub use write::external::{
    Attribution, ExternalChangeError, REF_DELETE_KIND, REF_UPDATE_KIND, RawPush,
    is_external_change_kind, record_external_change,
};
pub use write::flag::{FlagGate, WritePathDisabled};
pub use write::order::{PushOutcome, PushReject, RefUpdate, SerializedWriter, StaleRef};
// The real receive-pack ingest single-writer point (compare-and-append + total
// order across concurrent pushes). Its `RefUpdate`/`ReceiveError`/… are reached
// via `write::receive::` to avoid colliding with the order module's `RefUpdate`.
pub use write::receive::SerializedReceiver;
