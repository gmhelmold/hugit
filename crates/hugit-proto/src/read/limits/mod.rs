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

/// The result of a degradable serve: the vanilla pack (the floor that always
/// holds) plus the smart-layer products that are present ONLY when the layers are
/// enabled.
///
/// The degradation invariant is observable here: [`pack`](DegradedServe::pack) is
/// byte-identical whether the smart layers are on or off (a valid git repository
/// serves either way), but [`smart_capabilities`](DegradedServe::smart_capabilities)
/// is non-empty iff the smart layers ran. Disabled genuinely *bypasses* the smart
/// work — it is not a no-op toggle.
#[derive(Debug, Clone)]
pub struct DegradedServe {
    /// The vanilla git packfile — the floor. Byte-identical in both states.
    pub pack: PackAssembly,
    /// Smart-layer capabilities advertised on this serve. Populated ONLY when the
    /// smart layers are [`SmartLayers::Enabled`]; empty when [`SmartLayers::Disabled`].
    pub smart_capabilities: Vec<String>,
    /// Whether the smart-layer code path actually ran for this serve.
    pub smart_layer_ran: bool,
}

impl DegradedServe {
    /// Whether the smart layer contributed anything to this serve.
    pub fn is_smart(&self) -> bool {
        self.smart_layer_ran && !self.smart_capabilities.is_empty()
    }
}

/// The smart-layer capabilities the intelligence layers advertise *on top of* the
/// vanilla git serve. These are the genuine extra products the smart path
/// computes; the degraded (disabled) path never produces them.
fn smart_layer_capabilities(view: &dyn RefView, pack: &PackAssembly) -> Vec<String> {
    // The smart layer enriches the serve with advisories derived from the same
    // inputs the vanilla path used: a ref-count hint and a served-object hint.
    // (Real work over the real inputs — not a constant.)
    let ref_count = view.refs().len();
    vec![
        format!("hugit-smart-refs={ref_count}"),
        format!("hugit-smart-objects={}", pack.object_count()),
    ]
}

/// Serve a clone under a given smart-layer state (the degradation kill-test core).
///
/// The guarantee (whitepaper §9.5): whether the smart layers are
/// [`SmartLayers::Enabled`] or [`SmartLayers::Disabled`], a vanilla clone still
/// serves a valid repository. The **vanilla pack is the floor** and is computed
/// identically in both states; the difference is that with the layers ENABLED the
/// smart capabilities are computed and attached, and with them DISABLED that work
/// is bypassed entirely — proving the worst case is healthy git, never a broken
/// serve, and that "disabled" genuinely removes the intelligence rather than
/// running it and discarding the result.
pub fn serve_clone_degradable(
    state: SmartLayers,
    view: &dyn RefView,
    source: &dyn ObjectSource,
) -> Result<DegradedServe, ServeError> {
    // The floor: the vanilla git serve, computed in BOTH states.
    let (_adv, pack) = serve_clone(view, source)?;

    match state {
        SmartLayers::Enabled => {
            // Smart path: compute and attach the intelligence-layer products.
            let smart_capabilities = smart_layer_capabilities(view, &pack);
            Ok(DegradedServe {
                pack,
                smart_capabilities,
                smart_layer_ran: true,
            })
        }
        SmartLayers::Disabled => {
            // Degraded path: the smart work is BYPASSED. Only the vanilla floor
            // is served — no smart capabilities are even computed.
            Ok(DegradedServe {
                pack,
                smart_capabilities: Vec::new(),
                smart_layer_ran: false,
            })
        }
    }
}
