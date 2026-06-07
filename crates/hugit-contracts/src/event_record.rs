//! EventRecord — frozen by decomposition §1, item 7.
//!
//! One append-only hash-chained event.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One append-only hash-chained event record (decomposition §1, item 7).
///
/// # Hash-chain formula (FROZEN — BYTE-EXACT, single-sourced)
///
/// The canonical, executable realisation of this formula is
/// `hugit_refstore::compute_this_hash` — **do NOT re-transcribe it**; import and
/// call it. This doc is the spec; that function is the ground truth (if they ever
/// disagree, the function wins). Three crates previously re-transcribed and
/// diverged (brutal review 2026-06-07, R1); the only safe pattern is to call the
/// one canonical fn.
///
/// ```text
/// this_hash = lower_hex( SHA-256(
///     LP(prev_hash) ‖ LP(kind) ‖ VEC(principal_chain) ‖ LP(payload) ‖ u64_be(seq)
/// ) )
/// ```
///
/// Primitives (used identically across every hugit pre-image):
/// - `LP(s)` = `u32_be(byte_len(s)) ‖ utf8_bytes(s)` — a 4-byte big-endian `u32`
///   byte-length prefix then the raw UTF-8 bytes (makes `‖` unambiguous).
/// - `VEC(v)` = `u32_be(elem_count(v)) ‖ LP(v[0]) ‖ LP(v[1]) ‖ …` — a 4-byte
///   big-endian `u32` element count then each element `LP`-framed. The count is
///   load-bearing: without it `["a","b"]` and `["ab"]` would collide.
///
/// Details:
/// - `H` = SHA-256, emitted as a **64-char lowercase** hex digest.
/// - Field order is `prev_hash, kind, principal_chain, payload, seq`.
/// - `prev_hash`, `kind`, `payload` are `LP` UTF-8 fields; `principal_chain` is
///   `VEC(...)`; `seq` is `u64_be` (8 bytes, big-endian, NOT length-prefixed —
///   the one fixed-width exception).
/// - `payload` MUST be **canonical JSON** before chaining (sorted object keys, no
///   insignificant whitespace — see `hugit_refstore::canonical_json`). The chain
///   hashes the payload bytes verbatim, so non-canonical JSON would produce a
///   producer/verifier mismatch.
/// - `recorded_at` is **deliberately EXCLUDED** from the pre-image: it is an
///   unauthenticated observability annotation. Authenticated ordering comes from
///   `seq` plus the `prev_hash` linkage, never a wall clock.
/// - genesis `prev_hash` = **64 ASCII `'0'` characters** (the literal hex-zero
///   string, `hugit_refstore::GENESIS_PREV_HASH`).
///
/// Frozen by WP-00; byte-exact spec re-stated under owner authorisation (R0).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventRecord {
    /// Monotonically increasing sequence number within the event log.
    pub seq: u64,

    /// SHA-256 hex digest of the preceding event (64 hex zeros for genesis).
    pub prev_hash: String,

    /// SHA-256 hex digest of this event (64-char lowercase hex).
    ///
    /// FROZEN, BYTE-EXACT: see the type-level doc; computed only by
    /// `hugit_refstore::compute_this_hash`. Pre-image is
    /// `LP(prev_hash) ‖ LP(kind) ‖ VEC(principal_chain) ‖ LP(payload) ‖
    /// u64_be(seq)` — `recorded_at` is excluded; `payload` is canonical JSON.
    pub this_hash: String,

    /// Event kind / type discriminator string.
    pub kind: String,

    /// Ordered chain of principals that produced this event.
    pub principal_chain: Vec<String>,

    /// Opaque JSON payload for this event (serialised as a string to keep
    /// the envelope schema stable across payload evolution).
    ///
    /// MUST be **canonical JSON** (sorted object keys, no insignificant
    /// whitespace — `hugit_refstore::canonical_json`) before it is chained into
    /// `this_hash`, since the hash covers these bytes verbatim.
    pub payload: String,

    /// Unix epoch milliseconds when this event was recorded.
    ///
    /// UNAUTHENTICATED observability annotation: **excluded** from the
    /// `this_hash` pre-image. Authenticated ordering is `seq` + `prev_hash`.
    pub recorded_at: u64,
}
