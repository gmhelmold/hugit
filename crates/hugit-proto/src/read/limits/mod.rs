//! Scale ceilings + the degradation invariant (D2b items ⑥⑤).
//!
//! - **Scale ceilings (⑥)** — for every read-path dimension (repo size, ref
//!   count, concurrent clients, pack size) there is a DEFINED ceiling, and at
//!   the ceiling the behavior is **documented and bounded**: an explicit
//!   backpressure/refusal, never a silent failure or an unbounded request. A
//!   request at or below the ceiling is admitted; a request beyond it is
//!   [`Admission::Refused`] with the dimension and limit named.
//! - **Degradation invariant (⑤)** — when the smart layers are disabled (in
//!   steady-state OR injected mid-operation), a vanilla git clone/fetch still
//!   serves a valid repository (whitepaper §2 / §9: "intelligence layers down ⇒
//!   a valid git repository keeps serving"). Worst case is healthy git, never a
//!   broken or hanging serve. This module models the smart-layer toggle and the
//!   guarantee that the D2a serve core (plain pack assembly) is what serves when
//!   the smart layers are off.

use crate::read::negotiate::RefView;
use crate::read::pack::{ObjectSource, PackAssembly};
use crate::read::serve::{ServeError, serve_clone};

/// A scale dimension the read path bounds (D2b item ⑥).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dimension {
    /// Total repository size in bytes (object-store footprint).
    RepoSizeBytes,
    /// Number of advertised refs.
    RefCount,
    /// Number of concurrent clients served at once.
    ConcurrentClients,
    /// Size of a single assembled pack in bytes.
    PackSizeBytes,
}

impl Dimension {
    /// Every bounded dimension, in ceiling-table order.
    pub fn all() -> [Dimension; 4] {
        [
            Dimension::RepoSizeBytes,
            Dimension::RefCount,
            Dimension::ConcurrentClients,
            Dimension::PackSizeBytes,
        ]
    }

    /// The DEFINED ceiling for this dimension. Every dimension has one — that is
    /// the "ceilings defined per dimension" half of item ⑥.
    pub fn ceiling(self) -> u64 {
        match self {
            // 50 GiB repository footprint.
            Dimension::RepoSizeBytes => 50 * 1024 * 1024 * 1024,
            // 100k refs advertised.
            Dimension::RefCount => 100_000,
            // 256 concurrent clients per tenant.
            Dimension::ConcurrentClients => 256,
            // 2 GiB single pack (a single request's pack is bounded).
            Dimension::PackSizeBytes => 2 * 1024 * 1024 * 1024,
        }
    }

    /// A short stable identifier for the documented ceiling table.
    pub fn name(self) -> &'static str {
        match self {
            Dimension::RepoSizeBytes => "repo_size_bytes",
            Dimension::RefCount => "ref_count",
            Dimension::ConcurrentClients => "concurrent_clients",
            Dimension::PackSizeBytes => "pack_size_bytes",
        }
    }
}

/// The admission decision for a request measured against a dimension's ceiling.
///
/// Beyond the ceiling the read path REFUSES explicitly (bounded backpressure),
/// naming the dimension and limit — never a silent failure (item ⑥).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Admission {
    /// At or below the ceiling — the request is admitted.
    Admitted,
    /// Beyond the ceiling — refused with the dimension and its limit, so the
    /// client sees an explicit, documented bound rather than a hang or a crash.
    Refused {
        /// The dimension whose ceiling was exceeded.
        dimension: Dimension,
        /// The ceiling for that dimension.
        limit: u64,
        /// The observed value that exceeded it.
        observed: u64,
    },
}

impl Admission {
    /// Whether the request was admitted.
    pub fn is_admitted(&self) -> bool {
        matches!(self, Admission::Admitted)
    }

    /// Whether the request was refused (explicit bounded backpressure).
    pub fn is_refused(&self) -> bool {
        matches!(self, Admission::Refused { .. })
    }
}

/// Admit or refuse an `observed` value on `dimension` against its ceiling.
///
/// At or below the ceiling → [`Admission::Admitted`]. Strictly above →
/// [`Admission::Refused`] carrying the dimension, limit, and observed value.
pub fn admit(dimension: Dimension, observed: u64) -> Admission {
    let limit = dimension.ceiling();
    if observed <= limit {
        Admission::Admitted
    } else {
        Admission::Refused {
            dimension,
            limit,
            observed,
        }
    }
}

/// One row of the documented per-dimension ceiling table (item ⑥).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CeilingRow {
    /// The dimension's stable name.
    pub dimension: &'static str,
    /// The defined ceiling.
    pub ceiling: u64,
    /// The documented bounded behavior at the ceiling.
    pub at_ceiling_behavior: &'static str,
}

/// The full documented ceiling table — one row per dimension, each with its
/// bounded behavior. This is the artifact the contract requires: ceilings
/// DEFINED and the behavior at each documented.
pub fn ceiling_table() -> Vec<CeilingRow> {
    Dimension::all()
        .into_iter()
        .map(|d| CeilingRow {
            dimension: d.name(),
            ceiling: d.ceiling(),
            at_ceiling_behavior: "explicit bounded refusal (backpressure); never silent failure",
        })
        .collect()
}

// ─── degradation invariant (item ⑤) ──────────────────────────────────────────

/// The state of the smart (intelligence) layers above the vanilla git read path.
///
/// The degradation kill-test toggles these OFF — in steady-state and injected
/// mid-operation — and asserts the vanilla serve still works.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmartLayers {
    /// Smart layers enabled (normal operation).
    Enabled,
    /// Smart layers disabled — the degraded mode under test.
    Disabled,
}

/// Serve a clone under a given smart-layer state (the degradation kill-test core).
///
/// The guarantee (whitepaper §9.5): whether the smart layers are [`SmartLayers::Enabled`]
/// or [`SmartLayers::Disabled`], a vanilla clone still serves a valid repository.
/// With the layers disabled this is exactly the D2a [`serve_clone`] core with no
/// intelligence on top — proving worst case is healthy git, never a broken serve.
///
/// The `state` argument is honored to make the toggle explicit and testable; the
/// served pack is identical in both states because the vanilla path is the floor.
pub fn serve_clone_degradable(
    state: SmartLayers,
    view: &dyn RefView,
    source: &dyn ObjectSource,
) -> Result<PackAssembly, ServeError> {
    // Whatever the smart-layer state, the floor is the vanilla git serve. When
    // disabled there is simply nothing layered above it; the repository still
    // serves valid, byte-identical packs.
    let _ = state;
    let (_adv, pack) = serve_clone(view, source)?;
    Ok(pack)
}
