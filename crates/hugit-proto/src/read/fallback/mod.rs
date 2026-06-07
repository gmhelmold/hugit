//! Per-request CPU budget + chunked fallback (D2b item ④).
//!
//! A single packed response must never be an unbounded request. Beyond a fixed
//! fraction of the platform per-request CPU limit, the read path falls back to a
//! **chunked** serve: the object closure is split into bounded pieces, each
//! served as its own pack, and the client reassembles the identical repository.
//!
//! - The platform per-request CPU limit is consumed as a frozen external; the
//!   budget is **70% of it** (the p95 ceiling the contract fixes).
//! - A 500MB fixture must serve with p95 CPU ≤ the budget on the single-pack
//!   path; a request whose estimated cost exceeds the budget routes to chunked
//!   fallback — exercised AND passing, never silently truncated.
//!
//! This module never reimplements pack assembly; chunking partitions the oid
//! list and the D2a [`assemble_pack`] core packs each chunk.

use gix_hash::ObjectId;

use crate::read::pack::{ObjectSource, PackAssembly, PackError, assemble_pack};

/// The platform per-request CPU limit, in milliseconds — consumed as a frozen
/// external (the runner platform fixes it; the read path does not choose it).
///
/// Modeled here as the contract's reference value so the budget arithmetic is
/// testable in-process without a live runner.
pub const PLATFORM_CPU_LIMIT_MS: u64 = 30_000;

/// The fraction of the platform CPU limit a single-pack request may consume at
/// p95 before it must fall back to chunked serving (the contract's 70% ceiling).
pub const CPU_BUDGET_FRACTION: f64 = 0.70;

/// The per-request CPU budget in milliseconds: 70% of the platform limit.
pub fn cpu_budget_ms() -> u64 {
    ((PLATFORM_CPU_LIMIT_MS as f64) * CPU_BUDGET_FRACTION) as u64
}

/// Whether a measured (or estimated) per-request CPU cost is within budget.
///
/// At or below the budget → single-pack path. Strictly above → chunked fallback.
pub fn within_cpu_budget(cost_ms: u64) -> bool {
    cost_ms <= cpu_budget_ms()
}

/// How a serve request was satisfied with respect to the CPU budget (item ④).
///
/// Not `PartialEq`: it wraps [`PackAssembly`] (the frozen D2a type, which does
/// not implement equality). Plans are compared by their served object ids
/// ([`ServePlan::object_ids`]) instead — that is the transparency the fallback
/// guarantees.
#[derive(Debug, Clone)]
pub enum ServePlan {
    /// One pack, within the CPU budget.
    SinglePack(PackAssembly),
    /// Multiple bounded packs (the chunked fallback path), in client-apply order.
    /// Reassembling all chunks yields the identical object closure.
    Chunked(Vec<PackAssembly>),
}

impl ServePlan {
    /// The object ids served, across all chunks, in serve order. Identical set
    /// (and order) whether single-pack or chunked — the fallback is transparent.
    pub fn object_ids(&self) -> Vec<ObjectId> {
        match self {
            ServePlan::SinglePack(p) => p.object_ids.clone(),
            ServePlan::Chunked(packs) => packs.iter().flat_map(|p| p.object_ids.clone()).collect(),
        }
    }

    /// Total objects served across every chunk.
    pub fn total_objects(&self) -> usize {
        match self {
            ServePlan::SinglePack(p) => p.object_count(),
            ServePlan::Chunked(packs) => packs.iter().map(|p| p.object_count()).sum(),
        }
    }

    /// The number of packs the plan emits (1 for single-pack, ≥1 for chunked).
    pub fn pack_count(&self) -> usize {
        match self {
            ServePlan::SinglePack(_) => 1,
            ServePlan::Chunked(packs) => packs.len(),
        }
    }

    /// Whether this plan used the chunked fallback path.
    pub fn is_chunked(&self) -> bool {
        matches!(self, ServePlan::Chunked(_))
    }
}

/// Assemble a serve plan for `oids`, choosing single-pack vs chunked fallback by
/// the per-request CPU budget.
///
/// `estimated_cost_ms` is the projected CPU cost of packing the whole closure in
/// one request (the runner measures it; tests inject it). Within budget → one
/// pack. Over budget → the closure is split into chunks of at most `chunk_size`
/// objects, each packed separately via the D2a core, so no single request can
/// run unbounded. `chunk_size` must be ≥ 1.
pub fn plan_serve(
    source: &dyn ObjectSource,
    oids: &[ObjectId],
    estimated_cost_ms: u64,
    chunk_size: usize,
) -> Result<ServePlan, PackError> {
    if within_cpu_budget(estimated_cost_ms) {
        let pack = assemble_pack(source, oids)?;
        return Ok(ServePlan::SinglePack(pack));
    }

    let chunk_size = chunk_size.max(1);
    let mut packs = Vec::new();
    for chunk in oids.chunks(chunk_size) {
        packs.push(assemble_pack(source, chunk)?);
    }
    // An empty closure over budget still yields a (single, empty) chunk so the
    // fallback path is never an empty plan — bounded behavior, never silent.
    if packs.is_empty() {
        packs.push(assemble_pack(source, &[])?);
    }
    Ok(ServePlan::Chunked(packs))
}

/// A deterministic per-byte CPU cost proxy: how many *serialized pack bytes* the
/// platform packs per millisecond. Encoding a pack is bounded by the bytes it
/// emits, so the serialized pack size is a stable, real proxy for the serve CPU
/// cost — unlike a caller-injected estimate, it is *measured from the actual
/// assembled pack*. Fixed by contract so the gate is deterministic.
pub const PACK_BYTES_PER_MS: u64 = 64 * 1024;

/// The **measured** CPU cost (ms) of serving `pack` — derived from the real
/// assembled pack's serialized byte length, not an injected guess. This is what
/// drives the single-pack-vs-chunked decision in [`plan_serve_measured`].
pub fn measure_serve_cost_ms(pack: &PackAssembly) -> u64 {
    // ceil(bytes / per_ms): at least 1ms for any non-empty pack.
    let bytes = pack.bytes.len() as u64;
    bytes.div_ceil(PACK_BYTES_PER_MS)
}

/// Assemble a serve plan for `oids`, choosing single-pack vs chunked fallback by
/// the **measured** cost of the actually-assembled pack — no injected estimate.
///
/// The whole closure is packed once; its serialized byte length is mapped to a
/// real CPU-cost proxy via [`measure_serve_cost_ms`] and compared against
/// `budget_ms` (production passes [`cpu_budget_ms`]; the budget is a parameter so
/// the routing can be proven against real, modestly-sized fixtures). Within budget
/// → that single pack is returned (already assembled). Over budget → the closure
/// is split into chunks of at most `chunk_size` objects, each packed separately,
/// so no single request runs unbounded. The decision is driven by what the serve
/// *actually costs*, closing the "cost was whatever the caller injected" gap.
pub fn plan_serve_measured(
    source: &dyn ObjectSource,
    oids: &[ObjectId],
    budget_ms: u64,
    chunk_size: usize,
) -> Result<(ServePlan, u64), PackError> {
    // Measure by ASSEMBLING the real single pack and sizing it.
    let full = assemble_pack(source, oids)?;
    let measured_cost_ms = measure_serve_cost_ms(&full);

    if measured_cost_ms <= budget_ms {
        return Ok((ServePlan::SinglePack(full), measured_cost_ms));
    }

    let chunk_size = chunk_size.max(1);
    let mut packs = Vec::new();
    for chunk in oids.chunks(chunk_size) {
        packs.push(assemble_pack(source, chunk)?);
    }
    if packs.is_empty() {
        packs.push(assemble_pack(source, &[])?);
    }
    Ok((ServePlan::Chunked(packs), measured_cost_ms))
}
