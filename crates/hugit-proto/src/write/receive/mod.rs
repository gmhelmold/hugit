//! receive-pack ingest — the git push WRITE entrypoint.
//!
//! A push delivers a packfile plus the ref update(s) it wants applied. This
//! module ingests that push:
//!
//! 1. **bound** the pack (oversized packs are rejected before any work — the
//!    source-of-truth bar; an unbounded push is a DoS / poison vector);
//! 2. **unpack** the pack with a **pure-Rust** pack engine (`gix-pack` /
//!    `gix-object`). No subprocess: the deployed engine is `distroless/cc` —
//!    there is NO `git` binary to spawn, so the unpack runs entirely in-process
//!    over the attacker-controlled pack bytes (fail-closed, never a panic);
//! 3. **verify** every unpacked object — each object's git oid is recomputed
//!    from its decoded `<type> <len>\0<body>` pre-image (`gix_object::compute_hash`);
//!    a malformed / injected object computes a different oid, so the recomputed
//!    id *is* the verification;
//! 4. **store** every object into the CoreLink CAS (verbatim loose bytes, so a
//!    later clone is byte-identical);
//! 5. **anchor** the requested ref update: its target oid MUST be an object the
//!    push actually delivered (or already in CAS). A ref update pointing at an
//!    oid the push never provided is *ref-update tampering* and is rejected
//!    before any event is appended;
//! 6. **record** the ref move as one append-only raw-push event on the D1 log
//!    (`ref.update`, no provenance) via [`crate::write::store::record_ref_update`].
//!
//! The result is that the pushed objects clone back byte-identical (item ①) and
//! the only thing on the intent log is an *external change*, never a fabricated
//! intent.
//!
//! # Thin packs + incremental pushes: CAS-base reachability
//!
//! A pushed pack may carry deltas (OFS_DELTA / REF_DELTA). OFS_DELTA always names
//! an in-pack base (a negative offset), so it can never be thin. A **REF_DELTA**
//! may name a base by oid that the push did NOT deliver because it already lives
//! server-side (a *thin pack*). The unpack resolves such a base against the
//! read-only [`Cas`]: when the fixpoint stalls (no in-pack base resolved a whole
//! pass) the remaining REF_DELTA bases are fetched from the CAS
//! ([`Cas::get`]) and used as the delta pre-image; the resolved object's oid is
//! still re-derived (`gix_object::compute_hash`) — that is the verification. A
//! base in NEITHER the pack NOR the CAS is a TRUE thin base nowhere and is
//! rejected as [`ReceiveError::MalformedPack`].
//!
//! Likewise **reachability is satisfied by (pushed ∪ CAS)**, not the pushed set
//! alone ([`UnpackedPack::target_reachable`]): a referenced oid that is absent
//! from the pushed objects but present in the CAS is a SATISFIED boundary leaf
//! (the walk stops there, exactly like a gitlink) — so an *incremental* push
//! whose parent commits / unchanged trees already live server-side is reachable.
//! A reference in NEITHER pushed objects NOR the CAS still fails the closure
//! ([`ReceiveError::UnreachableTarget`]) — no bare oid is trusted.
//!
//! Both new CAS-read surfaces are bounded by [`RecvLimits::max_cas_lookups`] and
//! fail closed on exceed ([`ReceiveError::CasLookupBudgetExceeded`]) so an
//! adversarial push can never trigger unbounded R2 fan-out (a DoS).

use crate::write::flag::{FlagGate, WritePathDisabled};
use crate::write::store::{Cas, CasObject, Oid, record_ref_update, store_objects};
use gix_hash::Kind as HashKind;
use gix_object::Kind as GixKind;
use gix_pack::data::entry::Header as PackHeader;
use gix_pack::data::input::{BytesToEntriesIter, EntryDataMode, Mode};
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::log::EventLog;
use hugit_refstore::{RefState, replay};
use std::collections::{BTreeMap, HashMap};
use std::io::Read as _;
use std::path::Path;
use std::sync::Mutex;

/// Default ceiling for an accepted (compressed) packfile (16 MiB) — a v0 bound,
/// not a tuned limit. Oversized packs are rejected up-front (red-team: oversized
/// pack). This caps the bytes *on the wire*; the inflated cap below caps what the
/// pack expands to.
pub const DEFAULT_MAX_PACK_BYTES: usize = 16 * 1024 * 1024;

/// Default ceiling for the *inflated* (uncompressed) total of an accepted pack
/// (32 MiB). A small, highly-compressible pack can inflate to many gigabytes (a
/// decompression bomb); this bound fails the ingest closed once the unpacked
/// total exceeds it, independent of the compressed wire size.
///
/// Sized against the engine container's memory, NOT just the logical pack size:
/// `unpack_pack` holds the inflated `raws` bodies, the `resolved` clones, the
/// `by_oid` decoded graph, and the `objects` loose bytes simultaneously, so the
/// transient aggregate peak is ~3–4× this ceiling. At 32 MiB the worst-case peak is
/// ~96–128 MiB — comfortably inside the distroless engine's footprint — whereas the
/// old 64 MiB ceiling put the peak at ~200–256 MiB, a tight-container OOM risk (the
/// write-path-hardening audit's #3 finding). Lower this further if the runtime
/// memory budget tightens; raise it only with a matching container-memory headroom.
pub const DEFAULT_MAX_INFLATED_BYTES: u64 = 32 * 1024 * 1024;

/// Default ceiling on the number of objects a single pack may unpack to. A pack
/// header can declare an enormous object count (an object-count bomb); this bound
/// is checked against the pack header *before* any unpack work.
pub const DEFAULT_MAX_OBJECTS: u64 = 1_000_000;

/// Default ceiling on the number of delta-resolution *passes* the fixpoint loop may
/// run. The loop resolves every delta whose base is now resolved each pass; a pack
/// whose REF_DELTA entries are in **reverse dependency order** resolves at most one
/// delta per pass, making the loop O(passes × remaining) = O(n²) on the object
/// count — a ~200k-delta pack would freeze the single-threaded engine (an
/// algorithmic-complexity DoS that stalls reads too). An honest pack resolves in a
/// handful of passes (git caps a delta chain at `pack.depth`, default 50; even a
/// poorly-ordered honest pack converges within a few hundred), so 1024 is generous
/// for real packs yet rejects the adversarial reverse-ordered case long before the
/// O(n²) blowup.
pub const DEFAULT_MAX_DELTA_PASSES: u32 = 1024;

/// Default ceiling on the number of read-only CAS lookups one ingest may perform
/// while resolving thin-pack REF_DELTA bases (each an R2 `get`) and walking the
/// reachability closure across the pushed/CAS boundary (each a `contains`). A push
/// that omits server-side ancestors legitimately references the CAS, but it MUST
/// NOT be able to trigger unbounded R2 fan-out: this bound fails the ingest closed
/// once the lookups exceed it ([`ReceiveError::CasLookupBudgetExceeded`]).
///
/// Sized generous for an honest incremental push (whose pushed-vs-CAS *frontier* —
/// the parent commit + the handful of unchanged top-level trees — is tiny: tens to
/// low-hundreds of objects even for a large incremental push) yet bounds the
/// adversarial fan-out. **A COUNT bound (fail-closed on exceed) is the right guard
/// HERE, not a wall-clock budget**: a push is a transaction, so truncating it
/// mid-resolution would be wrong — reject the (pathological) push outright instead.
/// **Lowered 100_000 → 4_096 (audit 2026-06-28):** on the SINGLE-THREADED engine
/// each lookup is a synchronous R2 fetch, so 100k = ~hours of blocked accept loop
/// (the count-bound-not-latency-bound systemic pattern — same class as the
/// code-search/diff DoS, here on the operator-only write path). 4_096 covers any
/// honest incremental push by a wide margin while capping a malicious thin-pack's
/// R2 fan-out to minutes, fail-closed. The two CAS-read surfaces (delta-base
/// resolution + the reachability walk) are each bounded by this independently, so
/// one ingest makes ≤ 2× this — still finite.
pub const DEFAULT_MAX_CAS_LOOKUPS: u32 = 4_096;

/// Limits applied to one receive-pack ingest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecvLimits {
    /// Maximum accepted (compressed) packfile size in bytes.
    pub max_pack_bytes: usize,
    /// Maximum accepted *inflated* (uncompressed) total across all objects.
    /// Guards against a decompression bomb whose compressed size is tiny.
    pub max_inflated_bytes: u64,
    /// Maximum number of objects the pack may carry (object-count bomb guard).
    pub max_objects: u64,
    /// Maximum delta-resolution passes the fixpoint loop may run before failing
    /// closed — the algorithmic-complexity (O(n²)) DoS guard for a pack of
    /// reverse-ordered REF_DELTA entries.
    pub max_delta_passes: u32,
    /// Maximum read-only CAS lookups one ingest may perform (thin-pack base `get`s
    /// plus reachability `contains` across the pushed/CAS boundary) before failing
    /// closed — the unbounded-R2-fan-out DoS guard for an incremental push.
    pub max_cas_lookups: u32,
}

impl Default for RecvLimits {
    fn default() -> Self {
        Self {
            max_pack_bytes: DEFAULT_MAX_PACK_BYTES,
            max_inflated_bytes: DEFAULT_MAX_INFLATED_BYTES,
            max_objects: DEFAULT_MAX_OBJECTS,
            max_delta_passes: DEFAULT_MAX_DELTA_PASSES,
            max_cas_lookups: DEFAULT_MAX_CAS_LOOKUPS,
        }
    }
}

/// One ref update a push asks to apply: move `ref_name` to `new_oid`, but only if
/// the ref currently shows `expected` (compare-and-append). `expected = None`
/// asserts the ref is currently absent (a create).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    /// The full ref name, e.g. `refs/heads/main`.
    pub ref_name: String,
    /// The tip the pusher expects the ref to hold right now. `None` asserts the
    /// ref is currently absent (a create). The update is rejected as stale unless
    /// the *current derived view* still shows exactly this tip.
    pub expected: Option<Oid>,
    /// The oid the ref should point at after the push.
    pub new_oid: Oid,
}

/// A receive-pack request: the raw pack bytes, the ref update(s) requested, and
/// the principal chain that pushed.
#[derive(Debug, Clone)]
pub struct ReceiveRequest {
    /// The packfile bytes delivered by the client.
    pub pack: Vec<u8>,
    /// The ref update the push asks to apply (v0: a single ref).
    pub update: RefUpdate,
    /// The ordered principals that produced this push (for the event record).
    pub principal_chain: Vec<String>,
    /// Unix epoch milliseconds the push was received.
    pub recorded_at: u64,
}

/// What a successful ingest produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Receipt {
    /// The oids stored to CAS by this push (sorted).
    pub stored_oids: Vec<Oid>,
    /// The append-only event recorded for the ref move (a raw-push event).
    pub event: EventRecord,
}

/// Why a receive-pack ingest was refused. Fail-closed: on any of these no
/// object is committed to CAS and no event is appended to the log.
#[derive(Debug)]
pub enum ReceiveError {
    /// The write path is gated off (the `self-hosted-alpha` flag is not set).
    /// This is checked as step 0, before any pack work — a push to a deployment
    /// without the flag never even touches CAS.
    WritePathDisabled,
    /// The pack exceeded the configured *compressed* size ceiling.
    OversizedPack {
        /// The packfile's compressed length in bytes.
        len: usize,
        /// The configured compressed ceiling.
        max: usize,
    },
    /// The pack inflated past the configured uncompressed ceiling, or declared
    /// more objects than allowed — a decompression / object-count bomb. Rejected
    /// fail-closed: no object is committed, no event is appended.
    DecompressionBomb {
        /// A human-readable description of which inflated bound was exceeded.
        detail: String,
    },
    /// The pack could not be parsed / unpacked by the pure-Rust pack engine
    /// (truncated, corrupt, an injected object that fails oid re-derivation, or a
    /// thin-pack delta whose base is not in the pack — a v0 limitation).
    MalformedPack {
        /// The underlying parse / unpack failure detail.
        detail: String,
    },
    /// The requested ref update points at an oid the push never delivered and
    /// that the CAS does not already hold — ref-update tampering.
    RefUpdateTampered {
        /// The ref the rejected update targeted.
        ref_name: String,
        /// The target oid that was not delivered.
        target: Oid,
    },
    /// The requested ref target is present in the pushed object set but is NOT
    /// reachable from itself as a complete closure — a ref pointing at a stray /
    /// dangling object whose closure the push did not actually deliver. Rejected
    /// before any write.
    UnreachableTarget {
        /// The ref whose target is unreachable.
        ref_name: String,
        /// The target oid that is not reachable from the pushed objects.
        target: Oid,
    },
    /// The ingest exceeded its read-only CAS lookup budget while resolving
    /// thin-pack bases or walking the reachability closure across the pushed/CAS
    /// boundary — a guard against an incremental push triggering unbounded R2
    /// fan-out (DoS). Rejected fail-closed: no object committed, no event appended.
    CasLookupBudgetExceeded {
        /// The configured ceiling that was exceeded.
        max: u32,
    },
    /// The compare-and-append check failed: the ref had already moved off the tip
    /// the pusher expected (a concurrent push won). The winner is preserved;
    /// nothing is overwritten and nothing is appended (no lost update).
    StaleRef {
        /// The ref the rejected push targeted.
        ref_name: String,
        /// The tip the pusher expected.
        expected: Option<Oid>,
        /// The tip the ref actually held (the value that won the race).
        actual: Option<Oid>,
    },
    /// The push carried no principal — an external change must be attributable.
    /// Fail-closed: an unattributed push is refused BEFORE any event is recorded,
    /// never silently recorded blind (symmetry with the order path /
    /// [`crate::write::external::ExternalChangeError::MissingAttribution`]).
    MissingAttribution,
    /// An environment / IO failure (temp dirs, the read-back materializer, etc.).
    Io {
        /// The underlying IO failure detail.
        detail: String,
    },
}

impl std::fmt::Display for ReceiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReceiveError::WritePathDisabled => {
                write!(
                    f,
                    "receive-pack refused: write path disabled (self-hosted-alpha flag off)"
                )
            }
            ReceiveError::OversizedPack { len, max } => {
                write!(f, "pack rejected: {len} bytes exceeds ceiling {max}")
            }
            ReceiveError::DecompressionBomb { detail } => {
                write!(
                    f,
                    "pack rejected: decompression/object-count bomb ({detail})"
                )
            }
            ReceiveError::MalformedPack { detail } => {
                write!(f, "pack rejected: malformed ({detail})")
            }
            ReceiveError::RefUpdateTampered { ref_name, target } => write!(
                f,
                "ref update rejected: {ref_name} → {target} not delivered by the push (tampered)"
            ),
            ReceiveError::UnreachableTarget { ref_name, target } => write!(
                f,
                "ref update rejected: {ref_name} → {target} not reachable as a complete closure over (pushed ∪ CAS)"
            ),
            ReceiveError::CasLookupBudgetExceeded { max } => write!(
                f,
                "pack rejected: exceeded the CAS lookup budget ({max}) resolving thin-pack bases / reachability — refusing unbounded R2 fan-out (fail-closed)"
            ),
            ReceiveError::StaleRef {
                ref_name,
                expected,
                actual,
            } => write!(
                f,
                "ref update rejected: stale {ref_name}: expected {expected:?}, but it is {actual:?} (no lost update)"
            ),
            ReceiveError::MissingAttribution => write!(
                f,
                "receive-pack refused: external change must carry attribution"
            ),
            ReceiveError::Io { detail } => write!(f, "receive-pack IO error: {detail}"),
        }
    }
}

impl From<WritePathDisabled> for ReceiveError {
    fn from(_: WritePathDisabled) -> Self {
        ReceiveError::WritePathDisabled
    }
}

impl std::error::Error for ReceiveError {}

/// Ingest one receive-pack push: gate → bound → unpack → verify → cap-inflated →
/// reachable → compare-and-append → store → record.
///
/// Step 0 is the **flag gate**: with the `self-hosted-alpha` flag off the push is
/// refused before any pack work ([`ReceiveError::WritePathDisabled`]) — no CAS
/// write, no event. The ref move is **compare-and-append**: it is applied only if
/// the *current derived view* still shows `req.update.expected`, otherwise it is
/// rejected as [`ReceiveError::StaleRef`] (no lost update). On success the pushed
/// objects are in CAS and one raw-push event is on the log; on any error nothing
/// is committed (fail-closed).
///
/// This free function applies the compare-and-append atomically because it holds
/// `&mut` exclusive access to `cas` and `log`. For genuine concurrent pushes use
/// [`SerializedReceiver`], which serializes the same core through a single-writer
/// lock so two overlapping pushes on one ref cannot both win the stale check.
///
/// `current_refs` is the **authoritative** `ref name → tip oid` view the stale
/// check compares against — the SAME projection the advertise is built from (the
/// serve path passes its live `git_refs` snapshot; the in-memory
/// [`SerializedReceiver`] passes `replay(log)`). It is NOT derived from `replay(log)`
/// here: a CAS-ingested repo has NO `ref.update` events for its branches (those come
/// only from application verbs, never from git ingest), so a log-derived view would
/// show every ingested branch as absent and false-reject the normal "update an
/// existing branch" push as stale. The log stays the APPEND target only (step 8).
pub fn receive_pack(
    gate: &FlagGate,
    req: &ReceiveRequest,
    cas: &mut dyn Cas,
    log: &mut EventLog,
    current_refs: &BTreeMap<String, String>,
    limits: RecvLimits,
) -> Result<Receipt, ReceiveError> {
    // 0. flag gate: the write path is OFF unless self-hosted-alpha. Refuse before
    //    ANY pack work — no unpack, no CAS write, no event.
    gate.admit_write()?;

    // 0b. attribution gate: a raw push MUST be attributable. An empty principal
    //     chain is refused with a typed error BEFORE any event is recorded —
    //     symmetry with the order path and `record_external_change`'s fail-closed
    //     `MissingAttribution`. (Was: silently recorded an unattributed event.)
    if req.principal_chain.is_empty() {
        return Err(ReceiveError::MissingAttribution);
    }

    // 1. bound the COMPRESSED pack BEFORE any work.
    if req.pack.len() > limits.max_pack_bytes {
        return Err(ReceiveError::OversizedPack {
            len: req.pack.len(),
            max: limits.max_pack_bytes,
        });
    }

    // 1b. object-count bomb guard: reject a pack whose header declares more
    //     objects than allowed, before unpacking a single one.
    let declared = pack_header_object_count(&req.pack)?;
    if declared > limits.max_objects {
        return Err(ReceiveError::DecompressionBomb {
            detail: format!(
                "pack header declares {declared} objects, exceeds ceiling {}",
                limits.max_objects
            ),
        });
    }

    // 2+3. unpack + verify with the pure-Rust pack engine. Every object's oid is
    //      recomputed from its decoded pre-image; a malformed / injected object
    //      computes a different id, so the recomputed oid IS the verification.
    //      A REF_DELTA base or an unchanged ancestor that already lives server-side
    //      (a thin pack / incremental push) is resolved against the read-only CAS,
    //      bounded by `limits.max_cas_lookups` (fail-closed on unbounded fan-out).
    let unpacked = unpack_pack(&req.pack, &*cas, limits)?;
    let objects = &unpacked.objects;

    // 5a. anchor the requested ref update: its target must have been delivered by
    //     the push or already live in CAS. Reject tampering BEFORE any write.
    let delivered = unpacked.by_oid.contains_key(&req.update.new_oid);
    if !delivered && !cas.contains(&req.update.new_oid) {
        return Err(ReceiveError::RefUpdateTampered {
            ref_name: req.update.ref_name.clone(),
            target: req.update.new_oid.clone(),
        });
    }

    // 5b. reachability: the target must be REACHABLE — its full commit→tree→
    //     (subtree|blob) closure present in (the delivered set ∪ the CAS), not
    //     merely present somewhere. A pack that smuggles a stray object (present but
    //     with an incomplete closure that is satisfied by NEITHER the push NOR the
    //     CAS) is rejected. An unchanged ancestor already in CAS is a satisfied
    //     boundary leaf — so an incremental push reaches. (Targets already live in
    //     CAS are trusted prior tips, handled by the `delivered` guard above.)
    if delivered
        && !unpacked.target_reachable(&req.update.new_oid, &*cas, limits.max_cas_lookups)?
    {
        return Err(ReceiveError::UnreachableTarget {
            ref_name: req.update.ref_name.clone(),
            target: req.update.new_oid.clone(),
        });
    }

    // 6. compare-and-append: apply only if the AUTHORITATIVE current ref view
    //    (`current_refs` — the advertise's projection, NOT `replay(log)`) still
    //    shows the tip the pusher expected. A stale expectation is rejected with NO
    //    write and NO append (the concurrent winner is preserved). Using the
    //    advertise projection is load-bearing: a CAS-ingested branch has no
    //    `ref.update` event on the log, so a log-derived view would wrongly show it
    //    absent and false-reject every update of an existing ingested branch.
    let actual = current_refs.get(&req.update.ref_name).cloned();
    if actual != req.update.expected {
        return Err(ReceiveError::StaleRef {
            ref_name: req.update.ref_name.clone(),
            expected: req.update.expected.clone(),
            actual,
        });
    }

    // 7. store every object into CAS (idempotent on oid).
    store_objects(cas, objects);
    let mut stored_oids: Vec<Oid> = objects.iter().map(|o| o.oid.clone()).collect();
    stored_oids.sort();

    // 8. record the ref move as one append-only raw-push event (no provenance).
    let event = record_ref_update(
        log,
        req.principal_chain.clone(),
        &req.update.ref_name,
        &req.update.new_oid,
        req.recorded_at,
    );

    Ok(Receipt { stored_oids, event })
}

/// The **single-writer serialization point** for one repository's receive-pack
/// ingests — the in-process stand-in for the per-repo Durable Object.
///
/// Wraps the per-repo [`EventLog`] *and* the CAS behind one [`Mutex`], so every
/// push is serialized: the compare-and-append (read derived view → check tip →
/// store objects → append) runs in one critical section. Two concurrent pushes on
/// the same ref therefore cannot both win the stale check — exactly one lands at a
/// distinct, monotonically increasing total-order seq; the other is rejected as
/// [`ReceiveError::StaleRef`] with no lost update.
pub struct SerializedReceiver<C: Cas> {
    inner: Mutex<(C, EventLog)>,
    gate: FlagGate,
    limits: RecvLimits,
}

impl<C: Cas> SerializedReceiver<C> {
    /// A fresh receiver over the given CAS + an empty log, under `gate`/`limits`.
    pub fn new(cas: C, gate: FlagGate, limits: RecvLimits) -> Self {
        Self {
            inner: Mutex::new((cas, EventLog::new())),
            gate,
            limits,
        }
    }

    /// Serialize one receive-pack ingest through the single-writer point. The
    /// whole gate→…→compare-and-append→append sequence runs under the lock.
    pub fn receive(&self, req: &ReceiveRequest) -> Result<Receipt, ReceiveError> {
        let mut guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let (cas, log) = &mut *guard;
        // The in-memory serialization point has no external advertise projection:
        // its log IS the authoritative ref source, so a second push that updates a
        // ref must observe the first push's `ref.update`. Derive the current view
        // from `replay(log)` and pass it as the authoritative `current_refs` —
        // preserving the original (log-driven) compare-and-append semantics.
        let state: RefState = replay(log).map_err(|e| ReceiveError::Io {
            detail: format!("replay derived view: {e:?}"),
        })?;
        let current_refs: BTreeMap<String, String> = state
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        receive_pack(&self.gate, req, cas, log, &current_refs, self.limits)
    }

    /// The current number of records on the log (also the next total-order seq).
    pub fn log_len(&self) -> usize {
        self.inner.lock().unwrap_or_else(|p| p.into_inner()).1.len()
    }

    /// The current derived ref view.
    pub fn ref_view(&self) -> RefState {
        let guard = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        replay(&guard.1).expect("own log replays cleanly")
    }

    /// Snapshot the underlying log (clone) for assertions.
    pub fn snapshot_log(&self) -> EventLog {
        self.inner
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .1
            .clone()
    }
}

/// Parse the big-endian 32-bit object count from a git pack header (`PACK`,
/// version, count). Used to reject an object-count bomb before unpacking, and to
/// fail closed on a non-V2 / unsigned pack before the pack engine sees it.
fn pack_header_object_count(pack: &[u8]) -> Result<u64, ReceiveError> {
    if pack.len() < 12 || &pack[0..4] != b"PACK" {
        return Err(ReceiveError::MalformedPack {
            detail: "pack too short or missing PACK signature".to_string(),
        });
    }
    // Reject any non-V2 pack up-front. The pack engine `assert!`s on an
    // undocumented version; the pack bytes are attacker-controlled, so we never
    // hand it a version it would panic on — fail closed with a typed error.
    let version = u32::from_be_bytes([pack[4], pack[5], pack[6], pack[7]]);
    if version != 2 {
        return Err(ReceiveError::MalformedPack {
            detail: format!("unsupported pack version {version} (only V2 is accepted)"),
        });
    }
    let count = u32::from_be_bytes([pack[8], pack[9], pack[10], pack[11]]);
    Ok(u64::from(count))
}

/// The decoded git type of one unpacked object.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Commit,
    Tree,
    Blob,
    Tag,
}

impl Kind {
    fn from_gix(k: GixKind) -> Self {
        match k {
            GixKind::Commit => Kind::Commit,
            GixKind::Tree => Kind::Tree,
            GixKind::Blob => Kind::Blob,
            GixKind::Tag => Kind::Tag,
        }
    }

    fn to_gix(self) -> GixKind {
        match self {
            Kind::Commit => GixKind::Commit,
            Kind::Tree => GixKind::Tree,
            Kind::Blob => GixKind::Blob,
            Kind::Tag => GixKind::Tag,
        }
    }
}

/// The result of a pure-Rust unpack: the CAS-ready loose objects plus the decoded
/// `(kind, data)` of every object keyed by oid, for the reachability walk.
#[derive(Debug)]
struct UnpackedPack {
    /// Every unpacked object as a [`CasObject`] (verbatim zlib loose bytes).
    objects: Vec<CasObject>,
    /// oid → decoded `(kind, body)` for the reachability closure walk.
    by_oid: HashMap<Oid, (Kind, Vec<u8>)>,
}

impl UnpackedPack {
    /// Whether `target` is REACHABLE over (the pushed set ∪ the CAS): it is present
    /// AND its full closure (commit → tree → subtree/blob, tag → target, …) is
    /// wholly satisfiable. A referenced oid absent from the pushed set but present
    /// in `cas` is a SATISFIED boundary leaf — the walk stops there (an incremental
    /// push's unchanged server-side ancestors), exactly like the gitlink skip. A
    /// reference in NEITHER the pushed set NOR the CAS → NOT reachable. Fail-closed
    /// and panic-free on malformed objects: an object that fails to parse makes the
    /// closure incomplete (unreachable), never a panic.
    ///
    /// Each pushed/CAS-boundary `cas.contains` is one CAS lookup; the walk fails
    /// closed with [`ReceiveError::CasLookupBudgetExceeded`] once it exceeds
    /// `max_cas_lookups` (the unbounded-R2-fan-out guard).
    fn target_reachable(
        &self,
        target: &str,
        cas: &dyn Cas,
        max_cas_lookups: u32,
    ) -> Result<bool, ReceiveError> {
        // BFS over the object graph. Every referenced oid must be present in the
        // pushed set OR resolvable in the CAS (a satisfied boundary); a reference in
        // neither fails the closure (not reachable).
        let mut stack: Vec<String> = vec![target.to_string()];
        let mut seen: HashMap<String, ()> = HashMap::new();
        let mut cas_lookups: u32 = 0;
        while let Some(oid) = stack.pop() {
            if seen.insert(oid.clone(), ()).is_some() {
                continue;
            }
            let Some((kind, data)) = self.by_oid.get(&oid) else {
                // Absent from the push → consult the CAS. Present → a satisfied
                // boundary leaf (stop the walk here). Absent in both → unreachable.
                cas_lookups += 1;
                if cas_lookups > max_cas_lookups {
                    return Err(ReceiveError::CasLookupBudgetExceeded {
                        max: max_cas_lookups,
                    });
                }
                if cas.contains(&oid) {
                    continue;
                }
                return Ok(false);
            };
            match kind {
                Kind::Blob => {}
                Kind::Commit => {
                    // tree + parents; a malformed commit fails closed.
                    let mut iter = gix_object::CommitRefIter::from_bytes(data);
                    match iter.tree_id() {
                        Ok(tree) => stack.push(tree.to_hex().to_string()),
                        Err(_) => return Ok(false),
                    }
                    for parent in gix_object::CommitRefIter::from_bytes(data).parent_ids() {
                        stack.push(parent.to_hex().to_string());
                    }
                }
                Kind::Tree => {
                    let entries = match gix_object::TreeRefIter::from_bytes(data).entries() {
                        Ok(e) => e,
                        Err(_) => return Ok(false),
                    };
                    for entry in entries {
                        // A gitlink (submodule commit) is a boundary object git
                        // does not require to be present — skip it, like
                        // `git rev-list --objects`. Trees and blobs must be in set.
                        if entry.mode.is_commit() {
                            continue;
                        }
                        stack.push(entry.oid.to_hex().to_string());
                    }
                }
                Kind::Tag => match gix_object::TagRefIter::from_bytes(data).target_id() {
                    Ok(t) => stack.push(t.to_hex().to_string()),
                    Err(_) => return Ok(false),
                },
            }
        }
        // The target itself must have been present (the very first pop checks it).
        Ok(self.by_oid.contains_key(target))
    }
}

/// One raw pack entry, post header-parse but pre delta-resolution.
struct RawEntry {
    header: PackHeader,
    pack_offset: u64,
    /// The decompressed entry payload — object body (base) or delta instructions.
    data: Vec<u8>,
}

/// Unpack `pack` with the pure-Rust pack engine, enforcing the inflated / object
/// bombs fail-closed. Returns the CAS-ready loose objects + the decoded graph.
///
/// Resolves SELF-CONTAINED deltas (base in the pack) AND thin-pack REF_DELTA bases
/// that already live server-side: when the in-pack fixpoint stalls, a remaining
/// REF_DELTA's base is fetched from the read-only `cas` and used as the delta
/// pre-image (the resolved object's oid is still re-derived — the verify). A base
/// in NEITHER the pack NOR the CAS is rejected as [`ReceiveError::MalformedPack`].
/// Each CAS base fetch is bounded by `limits.max_cas_lookups`
/// ([`ReceiveError::CasLookupBudgetExceeded`] on exceed). The pack bytes are
/// attacker-controlled: every parse / decompress / delta step is fallible and
/// bounds-checked — a malformed pack is a typed error, never a panic.
fn unpack_pack(
    pack: &[u8],
    cas: &dyn Cas,
    limits: RecvLimits,
) -> Result<UnpackedPack, ReceiveError> {
    // Defensive re-validation (the caller already checked, but unpack is reachable
    // in isolation): signature + V2 before the engine, which `assert!`s on non-V2.
    pack_header_object_count(pack)?;

    let iter = BytesToEntriesIter::new_from_header(
        std::io::Cursor::new(pack),
        // Verify: the engine hashes every byte and verifies the pack trailer
        // checksum on the final entry — a tampered/truncated pack errors here.
        Mode::Verify,
        // Keep the compressed bytes so we can decompress each payload ourselves.
        EntryDataMode::Keep,
        HashKind::Sha1,
    )
    .map_err(|e| ReceiveError::MalformedPack {
        detail: format!("pack header: {e}"),
    })?;

    // Phase 1: stream the entries. The engine decompresses each payload to a sink
    // to learn its size WITHOUT materializing it, so we can enforce the inflated
    // and object-count bombs here, before allocating anything large.
    let mut raws: Vec<RawEntry> = Vec::new();
    let mut total_inflated: u64 = 0;
    let mut count: u64 = 0;
    for item in iter {
        let entry = item.map_err(|e| ReceiveError::MalformedPack {
            detail: format!("pack entry: {e}"),
        })?;
        count += 1;
        if count > limits.max_objects {
            return Err(ReceiveError::DecompressionBomb {
                detail: format!(
                    "unpacked object count {count} exceeds ceiling {}",
                    limits.max_objects
                ),
            });
        }
        total_inflated = total_inflated.saturating_add(entry.decompressed_size);
        if total_inflated > limits.max_inflated_bytes {
            return Err(ReceiveError::DecompressionBomb {
                detail: format!(
                    "inflated total {total_inflated} bytes exceeds ceiling {} bytes",
                    limits.max_inflated_bytes
                ),
            });
        }
        let compressed = entry
            .compressed
            .ok_or_else(|| ReceiveError::MalformedPack {
                detail: "pack entry carried no compressed payload".to_string(),
            })?;
        // Decompress this entry's payload. The cap above bounds the total, so this
        // can never materialize a bomb.
        let data = inflate_entry(&compressed, entry.decompressed_size)?;
        raws.push(RawEntry {
            header: entry.header,
            pack_offset: entry.pack_offset,
            data,
        });
    }

    // Phase 2: resolve. Bases decode immediately; deltas resolve against an
    // in-pack base (OFS by pack offset, REF by oid). A fixpoint loop drains the
    // deferred deltas; if a pass makes no progress, a base is missing → thin pack.
    let offset_to_index: HashMap<u64, usize> = raws
        .iter()
        .enumerate()
        .map(|(i, r)| (r.pack_offset, i))
        .collect();

    // index → resolved (kind, body); None until resolved.
    let mut resolved: Vec<Option<(Kind, Vec<u8>)>> = vec![None; raws.len()];
    // oid → resolved index, for REF_DELTA base lookup.
    let mut oid_to_index: HashMap<Oid, usize> = HashMap::new();
    // The full closure-walk map + the CAS objects, built as we resolve.
    let mut by_oid: HashMap<Oid, (Kind, Vec<u8>)> = HashMap::new();
    let mut objects: Vec<CasObject> = Vec::new();

    // Helper: finalize a resolved object — compute its oid, frame + zlib it.
    // Returns the oid.
    fn finalize(
        kind: Kind,
        data: &[u8],
        by_oid: &mut HashMap<Oid, (Kind, Vec<u8>)>,
        objects: &mut Vec<CasObject>,
        total_resolved: &mut u64,
        max_inflated: u64,
    ) -> Result<Oid, ReceiveError> {
        // VERIFY: the oid IS the sha1 over `<type> <len>\0<body>`. A tampered
        // object computes a different oid here (that is the verification).
        let oid = gix_object::compute_hash(HashKind::Sha1, kind.to_gix(), data)
            .map_err(|e| ReceiveError::MalformedPack {
                detail: format!("oid hashing failed: {e}"),
            })?
            .to_hex()
            .to_string();
        if by_oid.contains_key(&oid) {
            // Idempotent on oid: a duplicate object in the pack is harmless
            // (already counted toward the aggregate; do not double-count).
            return Ok(oid);
        }
        // AGGREGATE inflated-size cap — the load-bearing bomb guard. Sum the
        // RESOLVED body size of EVERY object (bases AND delta-expanded) and fail
        // closed. Phase-1's per-entry cap sees only a delta's INSTRUCTION-stream
        // size, not its expansion, so a tiny `1 base + N deltas` pack could resolve
        // to many GB while the raw total stays under the ceiling. (The replaced
        // system-git path summed `cat-file %(objectsize)` = resolved sizes; this
        // restores that parity.) NOTE on PEAK memory: a single `apply_delta` is
        // capped at the REMAINING budget, so no ONE allocation exceeds `max_inflated`
        // — but the AGGREGATE live footprint of `unpack_pack` is NOT bounded to
        // `max_inflated`. The inflated `raws`, the `resolved` clones, this `by_oid`
        // copy, and the `objects` loose bytes coexist, so the transient peak is
        // ~3–4× `max_inflated`. `DEFAULT_MAX_INFLATED_BYTES` is sized for THAT
        // aggregate against the container memory (see its doc).
        *total_resolved = total_resolved.saturating_add(data.len() as u64);
        if *total_resolved > max_inflated {
            return Err(ReceiveError::DecompressionBomb {
                detail: format!(
                    "resolved object total {total_resolved} bytes exceeds ceiling {max_inflated} bytes"
                ),
            });
        }
        // CasObject.bytes = zlib(`<type> <len>\0<body>`) — git's verbatim loose
        // on-disk form, so a later clone is byte-identical (the CAS contract).
        let framing = encode_loose(kind, data);
        let bytes = zlib_compress(&framing)?;
        by_oid.insert(oid.clone(), (kind, data.to_vec()));
        objects.push(CasObject {
            oid: oid.clone(),
            bytes,
        });
        Ok(oid)
    }

    // Running total of RESOLVED object bytes (bases + delta-expanded) for the
    // aggregate bomb cap in `finalize` — the guard that survives delta expansion.
    let mut total_resolved: u64 = 0;

    // First pass: resolve all base objects.
    for (i, raw) in raws.iter().enumerate() {
        if let Some(gk) = raw.header.as_kind() {
            let kind = Kind::from_gix(gk);
            let oid = finalize(
                kind,
                &raw.data,
                &mut by_oid,
                &mut objects,
                &mut total_resolved,
                limits.max_inflated_bytes,
            )?;
            resolved[i] = Some((kind, raw.data.clone()));
            oid_to_index.insert(oid, i);
        }
    }

    // Fixpoint: resolve deltas whose base is now resolved, until none remain.
    let mut remaining: Vec<usize> = (0..raws.len()).filter(|&i| resolved[i].is_none()).collect();
    // Bound the pass count — the algorithmic-complexity DoS guard. A pack of
    // reverse-ordered REF_DELTAs resolves ≤1 delta/pass (O(n²)); cap the passes so a
    // pathological ordering fails closed instead of freezing the single-threaded
    // engine. An honest pack converges in a few passes (chain depth ≪ the cap).
    let mut passes: u32 = 0;
    // Read-only CAS base fetches performed (thin-pack resolution) — bounded by
    // `max_cas_lookups` so an incremental push can never trigger unbounded R2 fan-out.
    let mut cas_lookups: u32 = 0;
    while !remaining.is_empty() {
        passes += 1;
        if passes > limits.max_delta_passes {
            return Err(ReceiveError::DecompressionBomb {
                detail: format!(
                    "delta resolution exceeded {} passes with {} entries still \
                     unresolved — pathological (reverse-ordered) delta dependency \
                     graph, rejected fail-closed (algorithmic-complexity DoS guard)",
                    limits.max_delta_passes,
                    remaining.len()
                ),
            });
        }
        let mut progressed = false;
        let mut still_pending: Vec<usize> = Vec::new();
        for &i in &remaining {
            let raw = &raws[i];
            // Find the resolved base for this delta, if present.
            let base_index = match raw.header {
                PackHeader::OfsDelta { base_distance } => {
                    let base_offset = match raw.pack_offset.checked_sub(base_distance) {
                        Some(o) if base_distance != 0 => o,
                        _ => {
                            return Err(ReceiveError::MalformedPack {
                                detail: "OFS_DELTA base offset out of range".to_string(),
                            });
                        }
                    };
                    match offset_to_index.get(&base_offset) {
                        Some(&bi) => bi,
                        None => {
                            return Err(ReceiveError::MalformedPack {
                                detail: "OFS_DELTA base offset not an entry boundary".to_string(),
                            });
                        }
                    }
                }
                PackHeader::RefDelta { base_id } => {
                    let base_oid = base_id.to_hex().to_string();
                    match oid_to_index.get(&base_oid) {
                        Some(&bi) => bi,
                        None => {
                            // Base not yet resolved this pass — defer; if it is
                            // never resolved the no-progress guard rejects it.
                            still_pending.push(i);
                            continue;
                        }
                    }
                }
                // Unreachable: base objects were resolved in the first pass.
                _ => {
                    return Err(ReceiveError::MalformedPack {
                        detail: "non-delta entry left unresolved".to_string(),
                    });
                }
            };

            let Some((base_kind, base_data)) = resolved[base_index].clone() else {
                // OFS base entry not resolved yet — defer.
                still_pending.push(i);
                continue;
            };

            // Cap this delta's target at the REMAINING aggregate budget (not the
            // full ceiling) so a single delta can't expand past what's left — no ONE
            // `apply_delta` allocation exceeds `max_inflated` (the aggregate live
            // footprint is still ~3–4× it; see `finalize` + the const doc).
            let remaining = limits.max_inflated_bytes.saturating_sub(total_resolved);
            let resolved_data = apply_delta(&base_data, &raw.data, remaining)?;
            let oid = finalize(
                base_kind,
                &resolved_data,
                &mut by_oid,
                &mut objects,
                &mut total_resolved,
                limits.max_inflated_bytes,
            )?;
            resolved[i] = Some((base_kind, resolved_data));
            oid_to_index.insert(oid, i);
            progressed = true;
        }
        if !progressed {
            // No IN-PACK base resolved this pass: every remaining REF_DELTA's base is
            // absent from the pack. Before failing, consult the CAS — a thin pack /
            // incremental push whose base already lives server-side. OFS_DELTAs only
            // ever name an in-pack base, so they are deferred (their chain root is a
            // REF_DELTA resolved here, which unblocks them on the next normal pass).
            let mut cas_progress = false;
            let mut still_thin: Vec<usize> = Vec::new();
            for &i in &remaining {
                let PackHeader::RefDelta { base_id } = raws[i].header else {
                    // An OFS_DELTA (or any non-REF) — its base is in-pack by
                    // construction; defer to the next normal pass.
                    still_thin.push(i);
                    continue;
                };
                let base_oid = base_id.to_hex().to_string();
                cas_lookups += 1;
                if cas_lookups > limits.max_cas_lookups {
                    return Err(ReceiveError::CasLookupBudgetExceeded {
                        max: limits.max_cas_lookups,
                    });
                }
                let Some(raw_base) = cas.get(&base_oid) else {
                    // Base in NEITHER the pack NOR the CAS — a TRUE thin base nowhere.
                    still_thin.push(i);
                    continue;
                };
                // The CAS base was verified on its own ingest; decode it to its
                // (kind, body) pre-image (the loose framing — zlib OR uncompressed).
                let (base_kind, base_data) = decode_cas_object(&raw_base)?;
                let remaining_budget = limits.max_inflated_bytes.saturating_sub(total_resolved);
                let resolved_data = apply_delta(&base_data, &raws[i].data, remaining_budget)?;
                // The RESOLVED object's oid is re-derived in `finalize` (the verify).
                let oid = finalize(
                    base_kind,
                    &resolved_data,
                    &mut by_oid,
                    &mut objects,
                    &mut total_resolved,
                    limits.max_inflated_bytes,
                )?;
                resolved[i] = Some((base_kind, resolved_data));
                oid_to_index.insert(oid, i);
                cas_progress = true;
            }
            if cas_progress {
                // CAS-resolved a base → re-run the fixpoint; the now-resolved object
                // may itself be the base for a deferred in-pack (OFS/REF) delta.
                remaining = still_thin;
                continue;
            }
            // No base found in the pack OR the CAS for ANY remaining delta → a true
            // thin base nowhere, rejected fail-closed.
            return Err(ReceiveError::MalformedPack {
                detail: "thin pack: a delta's base object is in neither the pack \
                         nor the CAS"
                    .to_string(),
            });
        }
        remaining = still_pending;
    }

    Ok(UnpackedPack { objects, by_oid })
}

/// Parse a git loose-object pre-image `"<type> <len>\0<body>"` into `(Kind, body)`,
/// validating the declared length against the actual body. `None` for any
/// malformation (no NUL, no space, an unknown kind token, a non-numeric or
/// mismatched length) so the caller can try the other framing form.
fn parse_loose_framing(framing: &[u8]) -> Option<(Kind, Vec<u8>)> {
    let nul = framing.iter().position(|&b| b == 0)?;
    let header = &framing[..nul];
    let body = &framing[nul + 1..];
    let sp = header.iter().position(|&b| b == b' ')?;
    let kind = match &header[..sp] {
        b"blob" => Kind::Blob,
        b"tree" => Kind::Tree,
        b"commit" => Kind::Commit,
        b"tag" => Kind::Tag,
        _ => return None,
    };
    let len: usize = std::str::from_utf8(&header[sp + 1..]).ok()?.parse().ok()?;
    if len != body.len() {
        return None;
    }
    Some((kind, body.to_vec()))
}

/// Decode a CAS-returned loose object (a thin-pack REF_DELTA base) into its
/// `(Kind, body)` pre-image.
///
/// The [`Cas`] read surface is NOT byte-uniform across implementations: the
/// trait-documented form (and `InMemoryCas` / `GitDirCas`) returns git's verbatim
/// **zlib-compressed** loose bytes, while the live serve adapter
/// (`the live CAS adapter`, backed by the oid-index→R2 read source) returns the
/// **uncompressed** `"<type> <len>\0<body>"` framing it serves on the read path. So
/// this accepts BOTH: it first tries to parse the bytes directly as the loose
/// framing (the uncompressed form — a zlib stream never parses as a valid framing
/// because its leading CMF byte is not a kind token), and otherwise zlib-inflates
/// and parses the result. A base that is neither → [`ReceiveError::MalformedPack`].
fn decode_cas_object(bytes: &[u8]) -> Result<(Kind, Vec<u8>), ReceiveError> {
    if let Some(decoded) = parse_loose_framing(bytes) {
        return Ok(decoded);
    }
    let mut inflated = Vec::new();
    flate2::read::ZlibDecoder::new(bytes)
        .read_to_end(&mut inflated)
        .map_err(|e| ReceiveError::MalformedPack {
            detail: format!("CAS thin-pack base inflate failed: {e}"),
        })?;
    parse_loose_framing(&inflated).ok_or_else(|| ReceiveError::MalformedPack {
        detail: "CAS thin-pack base is not a valid git loose object".to_string(),
    })
}

/// Inflate one pack-entry payload (a raw zlib stream) to exactly
/// `expected_size` bytes. A size mismatch or non-zlib bytes → [`ReceiveError::MalformedPack`].
fn inflate_entry(compressed: &[u8], expected_size: u64) -> Result<Vec<u8>, ReceiveError> {
    let mut decoder = flate2::read::ZlibDecoder::new(compressed);
    let mut out = Vec::with_capacity(expected_size.min(1 << 20) as usize);
    decoder
        .read_to_end(&mut out)
        .map_err(|e| ReceiveError::MalformedPack {
            detail: format!("entry inflate failed: {e}"),
        })?;
    if out.len() as u64 != expected_size {
        return Err(ReceiveError::MalformedPack {
            detail: format!(
                "entry inflated to {} bytes, header declared {expected_size}",
                out.len()
            ),
        });
    }
    Ok(out)
}

/// Build git's loose-object pre-image for `(kind, body)`: `<type> <len>\0<body>`.
/// Uses the canonical `gix_object` header so the framing is byte-identical to the
/// one git itself hashes + stores (the CAS byte-identity contract).
fn encode_loose(kind: Kind, body: &[u8]) -> Vec<u8> {
    let header = gix_object::encode::loose_header(kind.to_gix(), body.len() as u64);
    let mut framing = Vec::with_capacity(header.len() + body.len());
    framing.extend_from_slice(&header);
    framing.extend_from_slice(body);
    framing
}

/// zlib-compress `framing` into the verbatim loose bytes git stores on disk. The
/// compression LEVEL is immaterial to correctness — the oid + every read-back path
/// is computed from the INFLATED framing — but it must be a valid zlib stream.
fn zlib_compress(framing: &[u8]) -> Result<Vec<u8>, ReceiveError> {
    use std::io::Write as _;
    let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(framing).map_err(|e| ReceiveError::Io {
        detail: format!("zlib compress loose object: {e}"),
    })?;
    enc.finish().map_err(|e| ReceiveError::Io {
        detail: format!("zlib finish loose object: {e}"),
    })
}

/// Apply a git binary delta (`base` + `delta` instructions → resolved object).
///
/// The `delta` bytes are attacker-controlled: every read is bounds-checked, no
/// index can panic, and the declared target size is capped against `max_inflated`
/// (a delta-expansion bomb) BEFORE allocating. A copy that runs off the base, a
/// reserved `0x00` command, or a size mismatch → [`ReceiveError::MalformedPack`].
fn apply_delta(base: &[u8], delta: &[u8], max_inflated: u64) -> Result<Vec<u8>, ReceiveError> {
    let malformed = |detail: &str| ReceiveError::MalformedPack {
        detail: detail.to_string(),
    };
    let mut idx = 0usize;

    // Header: source size, then target size (git LEB128, little-endian 7-bit).
    let (declared_base, n) = decode_delta_size(delta, idx)?;
    idx += n;
    if declared_base != base.len() as u64 {
        return Err(malformed("delta base size does not match the base object"));
    }
    let (target_size, n) = decode_delta_size(delta, idx)?;
    idx += n;
    if target_size > max_inflated {
        return Err(ReceiveError::DecompressionBomb {
            detail: format!(
                "delta resolves to {target_size} bytes, exceeds ceiling {max_inflated} bytes"
            ),
        });
    }

    let mut out: Vec<u8> = Vec::with_capacity(target_size.min(1 << 20) as usize);
    while let Some(&cmd) = delta.get(idx) {
        idx += 1;
        if cmd & 0x80 != 0 {
            // COPY from base: assemble a little-endian offset (4 bytes) + size (3).
            let mut copy_offset: u64 = 0;
            for i in 0..4 {
                if cmd & (1 << i) != 0 {
                    let b = *delta
                        .get(idx)
                        .ok_or_else(|| malformed("truncated copy offset"))?;
                    idx += 1;
                    copy_offset |= (b as u64) << (8 * i);
                }
            }
            let mut copy_size: u64 = 0;
            for i in 0..3 {
                if cmd & (1 << (4 + i)) != 0 {
                    let b = *delta
                        .get(idx)
                        .ok_or_else(|| malformed("truncated copy size"))?;
                    idx += 1;
                    copy_size |= (b as u64) << (8 * i);
                }
            }
            if copy_size == 0 {
                copy_size = 0x10000;
            }
            let start =
                usize::try_from(copy_offset).map_err(|_| malformed("copy offset overflow"))?;
            let len = usize::try_from(copy_size).map_err(|_| malformed("copy size overflow"))?;
            let end = start
                .checked_add(len)
                .ok_or_else(|| malformed("copy range overflow"))?;
            let slice = base
                .get(start..end)
                .ok_or_else(|| malformed("copy runs off the base object"))?;
            out.extend_from_slice(slice);
        } else if cmd != 0 {
            // INSERT: the next `cmd` literal bytes.
            let len = cmd as usize;
            let end = idx
                .checked_add(len)
                .ok_or_else(|| malformed("insert length overflow"))?;
            let slice = delta
                .get(idx..end)
                .ok_or_else(|| malformed("truncated insert literal"))?;
            idx = end;
            out.extend_from_slice(slice);
        } else {
            // 0x00 is a reserved command code.
            return Err(malformed("reserved delta command code 0x00"));
        }
        if out.len() as u64 > target_size {
            return Err(malformed("delta produced more bytes than declared"));
        }
    }
    if out.len() as u64 != target_size {
        return Err(malformed("delta produced fewer bytes than declared"));
    }
    Ok(out)
}

/// Decode a git delta-header size (LEB128, little-endian 7-bit groups) at `start`.
/// Returns `(size, bytes_consumed)`. Bounds- and overflow-checked; never panics.
fn decode_delta_size(d: &[u8], start: usize) -> Result<(u64, usize), ReceiveError> {
    let mut size: u64 = 0;
    let mut shift: u32 = 0;
    let mut idx = start;
    loop {
        let b = *d.get(idx).ok_or_else(|| ReceiveError::MalformedPack {
            detail: "truncated delta size varint".to_string(),
        })?;
        idx += 1;
        size |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return Err(ReceiveError::MalformedPack {
                detail: "delta size varint overflows u64".to_string(),
            });
        }
    }
    Ok((size, idx - start))
}

/// Materialize a bare git repository on disk from CAS objects + a ref pointing
/// at `head_oid`, so a client can clone it back. This is the minimal read-back
/// the round-trip needs while the full D2a serve path lands; it writes the
/// verbatim CAS loose bytes straight into `objects/xx/yyy…` and lays down the ref
/// and HEAD by hand, so the clone is byte-identical to what was pushed AND no
/// `git` binary is spawned (the deployed engine is distroless — none to invoke).
///
/// Returns the path of the bare repo (a self-cleaning [`TempDir`] kept alive by
/// the returned handle).
pub fn materialize_bare_repo(
    cas: &dyn Cas,
    oids: &[Oid],
    ref_name: &str,
    head_oid: &str,
) -> Result<MaterializedRepo, ReceiveError> {
    let dir = TempDir::new("hugit-serve")?;
    let root = dir.path().to_path_buf();

    // Lay down the minimal bare-repo skeleton by hand (no `git init`).
    for sub in ["objects", "refs/heads", "refs/tags"] {
        std::fs::create_dir_all(root.join(sub)).map_err(|e| ReceiveError::Io {
            detail: format!("mkdir {sub}: {e}"),
        })?;
    }

    for oid in oids {
        if oid.len() < 3 {
            return Err(ReceiveError::MalformedPack {
                detail: format!("short oid {oid}"),
            });
        }
        let bytes = cas.get(oid).ok_or_else(|| ReceiveError::Io {
            detail: format!("CAS missing object {oid}"),
        })?;
        let (d, rest) = oid.split_at(2);
        let obj_dir = root.join("objects").join(d);
        std::fs::create_dir_all(&obj_dir).map_err(|e| ReceiveError::Io {
            detail: format!("mkdir {}: {e}", obj_dir.display()),
        })?;
        std::fs::write(obj_dir.join(rest), &bytes).map_err(|e| ReceiveError::Io {
            detail: format!("write object {oid}: {e}"),
        })?;
    }

    // Write the requested ref and point HEAD at it (a default clone checks it out).
    // A bare repo with `HEAD`, `objects/`, `refs/heads/<name>` is a valid clone
    // source for both system git and the pure-Rust serve path.
    if !ref_name.starts_with("refs/") || ref_name.contains("..") {
        return Err(ReceiveError::MalformedPack {
            detail: format!("refusing to write a non-ref name {ref_name}"),
        });
    }
    let ref_path = root.join(ref_name);
    if let Some(parent) = ref_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| ReceiveError::Io {
            detail: format!("mkdir ref dir {}: {e}", parent.display()),
        })?;
    }
    std::fs::write(&ref_path, format!("{head_oid}\n")).map_err(|e| ReceiveError::Io {
        detail: format!("write ref {ref_name}: {e}"),
    })?;
    std::fs::write(root.join("HEAD"), format!("ref: {ref_name}\n")).map_err(|e| {
        ReceiveError::Io {
            detail: format!("write HEAD: {e}"),
        }
    })?;

    Ok(MaterializedRepo { dir })
}

/// Handle to a materialized bare repo; the on-disk dir lives until this drops.
pub struct MaterializedRepo {
    dir: TempDir,
}

impl MaterializedRepo {
    /// Path to the bare repository (a valid `git clone` source).
    pub fn path(&self) -> &Path {
        self.dir.path()
    }
}

/// A minimal self-cleaning temp directory (no extra crate dependency). Used only
/// by [`materialize_bare_repo`]'s read-back; the receive INGEST path touches no
/// filesystem.
struct TempDir {
    path: std::path::PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Result<Self, ReceiveError> {
        let base = std::env::temp_dir();
        // unique-enough name: pid + a monotonic counter + nanos.
        use std::sync::atomic::{AtomicU64, Ordering};
        static CTR: AtomicU64 = AtomicU64::new(0);
        let n = CTR.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = base.join(format!("{prefix}-{}-{n}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&path).map_err(|e| ReceiveError::Io {
            detail: format!("create temp dir {}: {e}", path.display()),
        })?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    //! GIT-FREE acceptance of the pure-Rust unpacker. The whole point of the
    //! rewrite is that NO test (and no runtime path) spawns `git` — the deployed
    //! engine is distroless and has no `git` binary. These tests assert the
    //! unpack/verify/bomb/reachability logic *by construction* over a committed,
    //! real, self-contained pack fixture. Grep this file: zero `Command::new`.

    use super::*;
    use crate::write::store::InMemoryCas;

    /// An empty read-only CAS — the v0 "pushed-set-only" oracle: with no objects in
    /// the CAS, the new thin-base / reachability-boundary fallbacks resolve nothing,
    /// so these tests pin exactly the pre-thin-pack behaviour.
    fn no_cas() -> InMemoryCas {
        InMemoryCas::new()
    }

    /// A REAL, self-contained git pack (committed as raw bytes): one commit, its
    /// tree, and one blob — no deltas, no external bases. The acceptance harness
    /// for the pure-Rust unpack; never re-generated at runtime.
    const FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/selfcontained.pack");

    const COMMIT_OID: &str = "75b57154f97eb6744462b9859dc1f19f52f3733d";
    const TREE_OID: &str = "19039e578c9c6f5a251a6b28ac43618979e3c2b5";
    const BLOB_OID: &str = "0f5cd78782fbbfc23ea3d3076043e9c2d636b87c";

    /// (1) The fixture unpacks to EXACTLY the three expected objects, and each
    /// CasObject's bytes zlib-inflate to its `<type> <len>\0<body>` framing that
    /// re-derives its oid (the verify).
    #[test]
    fn unpacks_fixture_to_three_verified_objects() {
        let up = unpack_pack(FIXTURE, &no_cas(), RecvLimits::default())
            .expect("self-contained pack unpacks");

        assert_eq!(up.objects.len(), 3, "exactly three objects");
        let oids: std::collections::BTreeSet<&str> =
            up.objects.iter().map(|o| o.oid.as_str()).collect();
        assert!(oids.contains(COMMIT_OID), "commit present");
        assert!(oids.contains(TREE_OID), "tree present");
        assert!(oids.contains(BLOB_OID), "blob present");

        // kinds map correctly.
        assert_eq!(
            up.by_oid.get(COMMIT_OID).map(|(k, _)| *k),
            Some(Kind::Commit)
        );
        assert_eq!(up.by_oid.get(TREE_OID).map(|(k, _)| *k), Some(Kind::Tree));
        assert_eq!(up.by_oid.get(BLOB_OID).map(|(k, _)| *k), Some(Kind::Blob));

        // Each CasObject.bytes = zlib(encode_loose(kind, body)); inflating yields
        // exactly the framing, and re-hashing the (kind, body) re-derives the oid.
        for obj in &up.objects {
            let (kind, body) = up.by_oid.get(&obj.oid).expect("decoded body present");
            let mut decoder = flate2::read::ZlibDecoder::new(obj.bytes.as_slice());
            let mut framing = Vec::new();
            decoder
                .read_to_end(&mut framing)
                .expect("CasObject.bytes is a valid zlib stream");
            assert_eq!(
                framing,
                encode_loose(*kind, body),
                "inflated bytes == <type> <len>\\0<body> framing"
            );
            let rederived = gix_object::compute_hash(HashKind::Sha1, kind.to_gix(), body)
                .expect("hash")
                .to_hex()
                .to_string();
            assert_eq!(rederived, obj.oid, "framing re-derives the stored oid");
        }
    }

    /// (2) Reachability: the commit is reachable (its closure — tree + blob — is
    /// wholly present); a random non-present oid is NOT reachable.
    #[test]
    fn reachability_holds_for_complete_closure_only() {
        let up = unpack_pack(FIXTURE, &no_cas(), RecvLimits::default()).expect("unpacks");
        assert!(
            up.target_reachable(COMMIT_OID, &no_cas(), 100_000).unwrap(),
            "commit reachable: tree + blob closure present"
        );
        assert!(
            up.target_reachable(TREE_OID, &no_cas(), 100_000).unwrap(),
            "tree reachable: its blob is present"
        );
        let absent = "0000000000000000000000000000000000000000";
        assert!(
            !up.target_reachable(absent, &no_cas(), 100_000).unwrap(),
            "a random non-present oid is not reachable"
        );
    }

    /// (3) Bomb caps fail closed: a too-low object OR inflated-byte ceiling rejects
    /// the unpack as a DecompressionBomb before returning any object.
    #[test]
    fn bomb_caps_reject_fail_closed() {
        let tiny_objects = RecvLimits {
            max_objects: 1,
            ..RecvLimits::default()
        };
        let err =
            unpack_pack(FIXTURE, &no_cas(), tiny_objects).expect_err("object-count cap trips");
        assert!(
            matches!(err, ReceiveError::DecompressionBomb { .. }),
            "expected DecompressionBomb, got {err:?}"
        );

        let tiny_inflated = RecvLimits {
            max_inflated_bytes: 1,
            ..RecvLimits::default()
        };
        let err = unpack_pack(FIXTURE, &no_cas(), tiny_inflated).expect_err("inflated cap trips");
        assert!(
            matches!(err, ReceiveError::DecompressionBomb { .. }),
            "expected DecompressionBomb, got {err:?}"
        );
    }

    /// (4) A truncated pack (trailing bytes dropped) is a MalformedPack, NOT a
    /// panic — the pack bytes are attacker-controlled.
    #[test]
    fn truncated_pack_is_malformed_not_a_panic() {
        // Drop the trailing checksum + part of the last entry.
        let truncated = &FIXTURE[..FIXTURE.len() - 24];
        let err = unpack_pack(truncated, &no_cas(), RecvLimits::default())
            .expect_err("a truncated pack must be rejected");
        assert!(
            matches!(err, ReceiveError::MalformedPack { .. }),
            "expected MalformedPack, got {err:?}"
        );
    }

    /// The header guard rejects a non-V2 pack with a typed error instead of the
    /// pack engine's `assert!` panic (attacker-controlled version byte).
    #[test]
    fn non_v2_pack_is_rejected_without_panic() {
        let mut bad = FIXTURE.to_vec();
        bad[7] = 3; // version 3
        let err = unpack_pack(&bad, &no_cas(), RecvLimits::default()).expect_err("non-V2 rejected");
        assert!(
            matches!(err, ReceiveError::MalformedPack { .. }),
            "expected MalformedPack, got {err:?}"
        );
    }

    // delta.pack: a REAL self-contained pack (6 objects: 2 commits, 2 trees, 2 blobs)
    // carrying ONE OFS_DELTA — blob a73bf03… is delta-encoded against in-pack base
    // 13fad81…. Exercises the highest-risk hand-written delta resolver, git-free.
    const DELTA_FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/delta.pack");
    const DELTA_HEAD: &str = "7570fef006adb473de5205123de88be8efe99554";
    const DELTA_BASE_BLOB: &str = "13fad811388600a77e3139c4c67cb9d02b1c8107";
    const DELTA_OBJ_BLOB: &str = "a73bf03948e93062fcbb79a67fe11351d2cc4a60";

    /// (6) OFS_DELTA resolution: the delta object resolves against its in-pack base
    /// and its RESOLVED body re-derives the declared oid (the verify holds THROUGH
    /// delta application — a tampered delta would compute a different oid). git-free.
    #[test]
    fn delta_pack_resolves_ofs_delta_and_verifies() {
        let up = unpack_pack(DELTA_FIXTURE, &no_cas(), RecvLimits::default())
            .expect("delta pack unpacks");
        assert_eq!(up.objects.len(), 6, "two commits + two trees + two blobs");
        assert_eq!(
            up.by_oid.get(DELTA_BASE_BLOB).map(|(k, _)| *k),
            Some(Kind::Blob),
            "the delta base blob decoded"
        );
        let (k, body) = up
            .by_oid
            .get(DELTA_OBJ_BLOB)
            .expect("the OFS_DELTA object resolved against its in-pack base");
        assert_eq!(*k, Kind::Blob);
        let rederived = gix_object::compute_hash(HashKind::Sha1, k.to_gix(), body)
            .expect("hash")
            .to_hex()
            .to_string();
        assert_eq!(
            rederived, DELTA_OBJ_BLOB,
            "the delta-RESOLVED body re-derives its oid (apply_delta is correct)"
        );
        assert!(
            up.target_reachable(DELTA_HEAD, &no_cas(), 100_000).unwrap(),
            "HEAD's full multi-commit closure (parent + trees + blobs) is present"
        );
    }

    /// (7) AGGREGATE bomb guard on the DELTA path (the BLOCKER fix): a delta pack
    /// under a tight inflated ceiling is rejected FAIL-CLOSED — the delta's RESOLVED
    /// size counts toward the budget (`apply_delta` caps at the REMAINING budget,
    /// `finalize` sums resolved sizes), so a `1 base + N deltas` COPY-amplification
    /// cannot resolve to GB while the raw entry total stays tiny. A typed rejection,
    /// never an unbounded allocation or panic. (cap chosen below the
    /// base+delta-resolved total but above the bases alone.)
    #[test]
    fn delta_pack_inflated_cap_rejects_fail_closed() {
        let tight = RecvLimits {
            max_inflated_bytes: 5000,
            ..RecvLimits::default()
        };
        let err = unpack_pack(DELTA_FIXTURE, &no_cas(), tight)
            .expect_err("tight inflated cap rejects the delta pack");
        assert!(
            matches!(
                err,
                ReceiveError::DecompressionBomb { .. } | ReceiveError::MalformedPack { .. }
            ),
            "delta expansion must be bounded fail-closed, got {err:?}"
        );
    }

    // incomplete.pack: a pack delivering ONLY the commit object 7570fef… — its tree
    // and blobs are absent (an incomplete closure / stray tip).
    const INCOMPLETE_FIXTURE: &[u8] = include_bytes!("../../../tests/fixtures/incomplete.pack");

    // ── FIX #2(a): ref-projection reconciliation (the compare-and-append now
    //    compares against the AUTHORITATIVE advertise projection, not `replay(log)`).
    //    A CAS-ingested branch has NO `ref.update` event on the log, so a log-derived
    //    view would false-reject every UPDATE of an existing ingested branch. These
    //    git-free oracles drive `receive_pack` over the committed self-contained pack
    //    fixture (COMMIT_OID) against a crafted authoritative `current_refs`. ──

    use crate::write::flag::FlagGate;

    /// Build a CREATE/UPDATE request that pushes the fixture's commit to
    /// `refs/heads/main` with the given `expected` old tip.
    fn fixture_req(expected: Option<&str>) -> ReceiveRequest {
        ReceiveRequest {
            pack: FIXTURE.to_vec(),
            update: RefUpdate {
                ref_name: "refs/heads/main".to_string(),
                expected: expected.map(str::to_string),
                new_oid: COMMIT_OID.to_string(),
            },
            principal_chain: vec!["user:gustavo".to_string()],
            recorded_at: 1_717_000_000_000,
        }
    }

    /// (FIX1-a) A CREATE (`expected = None`) against an EMPTY authoritative view
    /// succeeds — nothing on the log, nothing advertised, the branch is born.
    #[test]
    fn create_against_absent_ref_succeeds() {
        let gate = FlagGate::self_hosted_alpha();
        let mut cas = InMemoryCas::new();
        let mut log = EventLog::new();
        let current_refs = BTreeMap::new();
        let req = fixture_req(None);
        let receipt = receive_pack(
            &gate,
            &req,
            &mut cas,
            &mut log,
            &current_refs,
            RecvLimits::default(),
        )
        .expect("a create against an absent ref succeeds");
        assert!(receipt.stored_oids.contains(&COMMIT_OID.to_string()));
        assert_eq!(log.len(), 1, "one ref.update event appended");
    }

    /// (FIX1-b) THE REGRESSION: UPDATE an EXISTING (CAS-ingested) branch with the
    /// CORRECT current old tip succeeds — even though the log carries NO `ref.update`
    /// for that branch (the bug false-rejected this as stale). The authoritative
    /// `current_refs` (the advertise projection) shows the prior tip, matching the
    /// pusher's `expected`.
    #[test]
    fn update_existing_branch_with_correct_old_oid_succeeds() {
        let gate = FlagGate::self_hosted_alpha();
        let mut cas = InMemoryCas::new();
        let mut log = EventLog::new(); // EMPTY log: an ingested branch has no ref.update.
        let prior = "1111111111111111111111111111111111111111";
        let mut current_refs = BTreeMap::new();
        current_refs.insert("refs/heads/main".to_string(), prior.to_string());
        let req = fixture_req(Some(prior));
        let receipt = receive_pack(
            &gate,
            &req,
            &mut cas,
            &mut log,
            &current_refs,
            RecvLimits::default(),
        )
        .expect("updating an existing ingested branch with the correct old tip succeeds");
        assert!(receipt.stored_oids.contains(&COMMIT_OID.to_string()));
        assert_eq!(log.len(), 1, "the ref move is appended");
    }

    /// (FIX1-c) UPDATE with a GENUINELY STALE old tip still fails `StaleRef` — the
    /// authoritative view shows a different tip than the pusher expected (no lost
    /// update). Compare-and-append still protects the concurrent winner.
    #[test]
    fn update_with_stale_old_oid_fails_staleref() {
        let gate = FlagGate::self_hosted_alpha();
        let mut cas = InMemoryCas::new();
        let mut log = EventLog::new();
        let actual_tip = "2222222222222222222222222222222222222222";
        let mut current_refs = BTreeMap::new();
        current_refs.insert("refs/heads/main".to_string(), actual_tip.to_string());
        // The pusher expects a DIFFERENT (stale) old tip.
        let stale = "3333333333333333333333333333333333333333";
        let req = fixture_req(Some(stale));
        let err = receive_pack(
            &gate,
            &req,
            &mut cas,
            &mut log,
            &current_refs,
            RecvLimits::default(),
        )
        .expect_err("a stale old tip must be rejected");
        match err {
            ReceiveError::StaleRef {
                expected, actual, ..
            } => {
                assert_eq!(expected.as_deref(), Some(stale));
                assert_eq!(actual.as_deref(), Some(actual_tip));
            }
            other => panic!("expected StaleRef, got {other:?}"),
        }
        assert!(
            cas.is_empty(),
            "fail-closed: no object committed on a stale push"
        );
        assert_eq!(
            log.len(),
            0,
            "fail-closed: no event appended on a stale push"
        );
    }

    /// (8) The HEADLINE reachability security property: a tip whose closure is NOT
    /// wholly in the pushed set is UNREACHABLE — it can never be advertised (which
    /// would 404 mid-clone). Only an ABSENT oid was tested before; this proves a
    /// PRESENT-but-incomplete-closure target is rejected.
    #[test]
    fn incomplete_closure_target_is_unreachable() {
        let up = unpack_pack(INCOMPLETE_FIXTURE, &no_cas(), RecvLimits::default())
            .expect("commit-only pack unpacks");
        assert_eq!(up.objects.len(), 1, "only the commit object is delivered");
        assert!(
            !up.target_reachable(DELTA_HEAD, &no_cas(), 100_000).unwrap(),
            "a commit whose tree/blob closure is absent must be UNREACHABLE (fail-closed)"
        );
    }

    // ── FIX #3: the inflated ceiling was LOWERED to bound the ~3–4× aggregate
    //    unpack peak within the engine container. ──

    /// The inflated ceiling is the documented container-safe value, and an
    /// over-budget pack fails closed. Proven CHEAPLY with a tiny explicit ceiling —
    /// no need to allocate a 256 MiB pack to demonstrate the rejection.
    #[test]
    fn inflated_ceiling_is_container_safe_and_rejects_over_budget() {
        assert_eq!(
            DEFAULT_MAX_INFLATED_BYTES,
            32 * 1024 * 1024,
            "the inflated ceiling was lowered to bound the aggregate unpack peak"
        );
        let tight = RecvLimits {
            max_inflated_bytes: 1,
            ..RecvLimits::default()
        };
        let err = unpack_pack(FIXTURE, &no_cas(), tight).expect_err("over-budget pack rejected");
        assert!(
            matches!(err, ReceiveError::DecompressionBomb { .. }),
            "expected DecompressionBomb, got {err:?}"
        );
    }

    // ── FIX #2: resolver algorithmic-complexity (O(n²)) DoS cap. A pack of
    //    reverse-ordered REF_DELTAs resolves ≤1 delta/pass; the pass cap rejects it
    //    fail-closed instead of freezing the single-threaded engine. The fixture is
    //    built GIT-FREE from raw bytes — a base blob + a REF_DELTA chain in reverse
    //    dependency order. ──

    /// Encode a git pack entry type+size header (variable length, 7-bit groups):
    /// first byte = `more<<7 | type<<4 | (size & 0xf)`, continuations carry 7 bits.
    fn pack_entry_header(type_id: u8, size: u64) -> Vec<u8> {
        let mut out = Vec::new();
        let mut byte = (type_id << 4) | (size & 0x0f) as u8;
        let mut sz = size >> 4;
        while sz != 0 {
            out.push(byte | 0x80);
            byte = (sz & 0x7f) as u8;
            sz >>= 7;
        }
        out.push(byte);
        out
    }

    /// Encode a git delta-header size as a LEB128 little-endian 7-bit varint.
    fn encode_size_varint(out: &mut Vec<u8>, mut v: u64) {
        loop {
            let mut b = (v & 0x7f) as u8;
            v >>= 7;
            if v != 0 {
                b |= 0x80;
            }
            out.push(b);
            if v == 0 {
                break;
            }
        }
    }

    /// A pure-INSERT git delta producing `target` from a base of `base_len` bytes.
    fn delta_insert(base_len: u64, target: &[u8]) -> Vec<u8> {
        assert!(
            target.len() <= 127,
            "single INSERT command fits the literal"
        );
        let mut d = Vec::new();
        encode_size_varint(&mut d, base_len);
        encode_size_varint(&mut d, target.len() as u64);
        d.push(target.len() as u8); // INSERT command: literal length 1..=127
        d.extend_from_slice(target);
        d
    }

    fn zlib_raw(data: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    fn blob_oid_bytes(body: &[u8]) -> [u8; 20] {
        let id = gix_object::compute_hash(HashKind::Sha1, GixKind::Blob, body).unwrap();
        let mut a = [0u8; 20];
        a.copy_from_slice(id.as_bytes());
        a
    }

    /// Build a valid V2 pack: one base blob + `n` REF_DELTA blobs forming a chain
    /// (delta-k deltas against link-(k-1)'s RESOLVED oid), laid out in REVERSE
    /// dependency order so the resolver resolves exactly one delta per pass → `n`
    /// passes (the O(n²) pathology, in miniature).
    fn reverse_ordered_ref_delta_pack(n: usize) -> Vec<u8> {
        const BL: usize = 16;
        // Resolved bodies: link 0 = base blob, link k = delta-k's resolved target.
        // Fixed length so every delta's declared base_size == BL — and the link
        // number sits in the FIRST bytes (within BL) so every body is DISTINCT (a
        // suffix index would be truncated away by the pad, collapsing the oids).
        let bodies: Vec<Vec<u8>> = (0..=n)
            .map(|k| {
                let mut b = format!("lnk{k:03}-").into_bytes();
                b.resize(BL, b'.');
                b
            })
            .collect();
        let oids: Vec<[u8; 20]> = bodies.iter().map(|b| blob_oid_bytes(b)).collect();

        let base_entry = {
            let mut e = pack_entry_header(3 /* blob */, BL as u64);
            e.extend_from_slice(&zlib_raw(&bodies[0]));
            e
        };
        let mut delta_entries: Vec<Vec<u8>> = Vec::new();
        for k in 1..=n {
            let delta = delta_insert(BL as u64, &bodies[k]);
            let mut e = pack_entry_header(7 /* REF_DELTA */, delta.len() as u64);
            e.extend_from_slice(&oids[k - 1]); // 20-byte base oid
            e.extend_from_slice(&zlib_raw(&delta));
            delta_entries.push(e);
        }

        let mut body = Vec::new();
        body.extend_from_slice(b"PACK");
        body.extend_from_slice(&2u32.to_be_bytes());
        body.extend_from_slice(&(n as u32 + 1).to_be_bytes());
        // Deltas in REVERSE (D_n … D_1), base LAST: each pass resolves one delta.
        for e in delta_entries.iter().rev() {
            body.extend_from_slice(e);
        }
        body.extend_from_slice(&base_entry);

        // Trailer: raw SHA-1 over the pack body (Mode::Verify checks it).
        let mut hasher = gix_hash::hasher(HashKind::Sha1);
        hasher.update(&body);
        let digest = hasher.try_finalize().unwrap();
        body.extend_from_slice(digest.as_bytes());
        body
    }

    /// (FIX2) A reverse-ordered REF_DELTA chain resolves fully under a generous pass
    /// budget (the pack is valid) but is rejected FAIL-CLOSED under a tight pass cap
    /// — the O(n²) resolver can never freeze the single-threaded engine. No hang.
    #[test]
    fn reverse_ordered_ref_delta_pack_hits_pass_cap_fail_closed() {
        let pack = reverse_ordered_ref_delta_pack(6);
        // Valid: with the default (generous) pass budget it resolves to base + 6.
        let ok = unpack_pack(&pack, &no_cas(), RecvLimits::default())
            .expect("a valid reverse-ordered chain resolves under the default pass budget");
        assert_eq!(ok.objects.len(), 7, "base blob + 6 delta-resolved targets");

        // A 6-link chain needs 6 passes; cap at 3 → the pass guard trips fail-closed.
        let capped = RecvLimits {
            max_delta_passes: 3,
            ..RecvLimits::default()
        };
        let err = unpack_pack(&pack, &no_cas(), capped)
            .expect_err("the pass cap must reject the reverse-ordered chain");
        assert!(
            matches!(err, ReceiveError::DecompressionBomb { .. }),
            "expected DecompressionBomb (pass cap), got {err:?}"
        );
    }

    // ── Thin-pack / CAS-base reachability (the incremental-push capability) ──────
    //    The same git-free builders drive REF_DELTA packs whose base lives ONLY in
    //    a mock CAS (server-side history), proving an incremental push lands.

    /// Seed `cas` with a blob in the **zlib-compressed** loose form (the
    /// trait-documented `Cas::get` shape — `InMemoryCas` / `GitDirCas`). Returns the
    /// blob's 20-byte git oid (for the REF_DELTA base field).
    fn seed_cas_blob(cas: &mut InMemoryCas, body: &[u8]) -> [u8; 20] {
        let framing = encode_loose(Kind::Blob, body);
        let bytes = zlib_compress(&framing).expect("zlib loose");
        let oid_hex = gix_object::compute_hash(HashKind::Sha1, GixKind::Blob, body)
            .expect("hash")
            .to_hex()
            .to_string();
        cas.put(&CasObject {
            oid: oid_hex,
            bytes,
        });
        blob_oid_bytes(body)
    }

    /// Build a valid V2 pack of `n` REF_DELTA blob entries, each a pure-INSERT delta
    /// against an EXTERNAL base oid (NOT delivered in the pack — a thin pack). The
    /// bases must be supplied to the CAS for resolution to succeed.
    fn thin_ref_delta_pack(entries: &[([u8; 20], u64, Vec<u8>)]) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(b"PACK");
        body.extend_from_slice(&2u32.to_be_bytes());
        body.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for (base_oid, base_len, target) in entries {
            let delta = delta_insert(*base_len, target);
            let mut e = pack_entry_header(7 /* REF_DELTA */, delta.len() as u64);
            e.extend_from_slice(base_oid);
            e.extend_from_slice(&zlib_raw(&delta));
            body.extend_from_slice(&e);
        }
        let mut hasher = gix_hash::hasher(HashKind::Sha1);
        hasher.update(&body);
        body.extend_from_slice(hasher.try_finalize().unwrap().as_bytes());
        body
    }

    /// The tolerant CAS-base decoder accepts BOTH the trait-documented zlib loose
    /// form AND the uncompressed `<type> <len>\0<body>` framing the live `CasRw`
    /// serve adapter returns — and rejects garbage without a panic.
    #[test]
    fn decode_cas_object_handles_both_zlib_and_raw_framing() {
        let body = b"a base object body";
        let framing = encode_loose(Kind::Blob, body);
        // Uncompressed framing — the CasRw serve form.
        let (k1, b1) = decode_cas_object(&framing).expect("raw framing decodes");
        assert_eq!(k1, Kind::Blob);
        assert_eq!(b1.as_slice(), body);
        // zlib-compressed loose — the InMemoryCas / GitDirCas / trait-documented form.
        let zlib = zlib_compress(&framing).unwrap();
        let (k2, b2) = decode_cas_object(&zlib).expect("zlib loose decodes");
        assert_eq!(k2, Kind::Blob);
        assert_eq!(b2.as_slice(), body);
        // Garbage → MalformedPack, never a panic.
        assert!(matches!(
            decode_cas_object(&[0xde, 0xad, 0xbe, 0xef]),
            Err(ReceiveError::MalformedPack { .. })
        ));
    }

    /// An INCREMENTAL push: a child commit (delivered) whose parent + unchanged
    /// trees/blobs are pre-seeded in the CAS (server-side history) is REACHABLE over
    /// (pushed ∪ CAS) — with an empty CAS the same target is unreachable (the v0
    /// limit), proving the CAS boundary is what unblocks it.
    #[test]
    fn incremental_push_on_cas_ancestor_reachable() {
        // Pre-seed the CAS with the full closure (parent commit + all trees/blobs).
        let full = unpack_pack(DELTA_FIXTURE, &no_cas(), RecvLimits::default())
            .expect("full closure unpacks");
        let mut cas = InMemoryCas::new();
        for o in &full.objects {
            cas.put(o);
        }
        // The push delivers ONLY the head commit (its tree + parent are server-side).
        let up = unpack_pack(INCOMPLETE_FIXTURE, &no_cas(), RecvLimits::default())
            .expect("commit-only pack unpacks");
        assert_eq!(up.objects.len(), 1, "only the commit is delivered");
        // Empty CAS → unreachable (the pre-thin-pack v0 behaviour).
        assert!(
            !up.target_reachable(DELTA_HEAD, &no_cas(), 100_000).unwrap(),
            "with no CAS ancestors the incomplete closure is unreachable"
        );
        // Ancestors in CAS → the closure is satisfied → reachable, lands.
        assert!(
            up.target_reachable(DELTA_HEAD, &cas, 100_000).unwrap(),
            "the commit's tree + parent are satisfied CAS boundary leaves → reachable"
        );
    }

    /// A REF_DELTA whose base is ONLY in the CAS (a thin pack) resolves against the
    /// CAS base and the RESOLVED object's oid is re-derived (the verify holds).
    #[test]
    fn ref_delta_base_from_cas_resolves() {
        let base_body = b"the server-side base blob the delta builds on";
        let mut cas = InMemoryCas::new();
        let base_oid = seed_cas_blob(&mut cas, base_body);

        let target = b"the resolved object, produced by delta from the CAS base";
        let pack = thin_ref_delta_pack(&[(base_oid, base_body.len() as u64, target.to_vec())]);

        let up = unpack_pack(&pack, &cas, RecvLimits::default())
            .expect("thin REF_DELTA resolves vs CAS");
        assert_eq!(up.objects.len(), 1, "the single delta-resolved object");
        let target_oid = gix_object::compute_hash(HashKind::Sha1, GixKind::Blob, target)
            .unwrap()
            .to_hex()
            .to_string();
        let (k, body) = up
            .by_oid
            .get(&target_oid)
            .expect("resolved + oid-verified (compute_hash re-derived this oid)");
        assert_eq!(*k, Kind::Blob);
        assert_eq!(
            body.as_slice(),
            target,
            "delta applied against the CAS base"
        );
    }

    /// A REF_DELTA whose base is in NEITHER the pack NOR the CAS → MalformedPack (a
    /// true thin base nowhere; no bare oid is trusted).
    #[test]
    fn true_thin_base_nowhere_rejected() {
        let base_body = b"a base object that exists nowhere";
        let base_oid = blob_oid_bytes(base_body);
        let pack = thin_ref_delta_pack(&[(base_oid, base_body.len() as u64, b"target".to_vec())]);
        // Empty CAS: the base is in neither the pack nor the CAS.
        let err = unpack_pack(&pack, &no_cas(), RecvLimits::default())
            .expect_err("a base nowhere must be rejected");
        assert!(
            matches!(err, ReceiveError::MalformedPack { .. }),
            "expected MalformedPack (thin base nowhere), got {err:?}"
        );
    }

    /// A push that needs MORE CAS base lookups than `max_cas_lookups` fails closed
    /// with [`ReceiveError::CasLookupBudgetExceeded`] — the unbounded-R2-fan-out DoS
    /// guard. Under a generous budget the same pack resolves fully.
    #[test]
    fn cas_lookup_cap_rejects_fail_closed() {
        let b1 = b"server base number one (distinct)";
        let b2 = b"server base number two (distinct)";
        let mut cas = InMemoryCas::new();
        let oid1 = seed_cas_blob(&mut cas, b1);
        let oid2 = seed_cas_blob(&mut cas, b2);
        let pack = thin_ref_delta_pack(&[
            (oid1, b1.len() as u64, b"target one".to_vec()),
            (oid2, b2.len() as u64, b"target two".to_vec()),
        ]);

        // Cap at 1 CAS lookup: the second base fetch trips the budget, fail-closed.
        let capped = RecvLimits {
            max_cas_lookups: 1,
            ..RecvLimits::default()
        };
        let err = unpack_pack(&pack, &cas, capped).expect_err("the CAS lookup cap must trip");
        assert!(
            matches!(err, ReceiveError::CasLookupBudgetExceeded { max: 1 }),
            "expected CasLookupBudgetExceeded {{ max: 1 }}, got {err:?}"
        );

        // A generous budget resolves both thin bases (the pack is otherwise valid).
        let ok = unpack_pack(&pack, &cas, RecvLimits::default())
            .expect("both thin bases resolve under a generous budget");
        assert_eq!(ok.objects.len(), 2, "both delta-resolved objects");
    }
}
