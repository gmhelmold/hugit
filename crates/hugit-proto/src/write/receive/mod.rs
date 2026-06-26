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
//! # v0 scope: self-contained packs only
//!
//! A pushed pack may carry deltas (OFS_DELTA / REF_DELTA). v0 resolves a delta
//! **only if its base is itself in the pack**. A *thin pack* — a delta whose base
//! is a server-side object NOT delivered in the pack — is a documented v0
//! limitation and is rejected as [`ReceiveError::MalformedPack`]; thin-pack base
//! resolution against the CAS is a tracked follow-up, not this path.
//!
//! Likewise, **reachability requires the target's FULL closure in the pushed set**
//! ([`UnpackedPack::target_reachable`] walks the pushed objects only, never the
//! CAS): a first/orphan push (whole closure delivered) works; an *incremental*
//! push whose parent commits / unchanged trees already live server-side is
//! rejected as [`ReceiveError::UnreachableTarget`]. That is the same
//! "consult the CAS for objects the push omits" follow-up as thin-pack bases —
//! out of scope for v0, which serves complete-closure pushes.

use crate::write::flag::{FlagGate, WritePathDisabled};
use crate::write::store::{Cas, CasObject, Oid, record_ref_update, store_objects};
use gix_hash::Kind as HashKind;
use gix_object::Kind as GixKind;
use gix_pack::data::entry::Header as PackHeader;
use gix_pack::data::input::{BytesToEntriesIter, EntryDataMode, Mode};
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::log::EventLog;
use hugit_refstore::{RefState, replay};
use std::collections::HashMap;
use std::io::Read as _;
use std::path::Path;
use std::sync::Mutex;

/// Default ceiling for an accepted (compressed) packfile (16 MiB) — a v0 bound,
/// not a tuned limit. Oversized packs are rejected up-front (red-team: oversized
/// pack). This caps the bytes *on the wire*; the inflated cap below caps what the
/// pack expands to.
pub const DEFAULT_MAX_PACK_BYTES: usize = 16 * 1024 * 1024;

/// Default ceiling for the *inflated* (uncompressed) total of an accepted pack
/// (64 MiB). A small, highly-compressible pack can inflate to many gigabytes (a
/// decompression bomb); this bound fails the ingest closed once the unpacked
/// total exceeds it, independent of the compressed wire size.
pub const DEFAULT_MAX_INFLATED_BYTES: u64 = 64 * 1024 * 1024;

/// Default ceiling on the number of objects a single pack may unpack to. A pack
/// header can declare an enormous object count (an object-count bomb); this bound
/// is checked against the pack header *before* any unpack work.
pub const DEFAULT_MAX_OBJECTS: u64 = 1_000_000;

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
}

impl Default for RecvLimits {
    fn default() -> Self {
        Self {
            max_pack_bytes: DEFAULT_MAX_PACK_BYTES,
            max_inflated_bytes: DEFAULT_MAX_INFLATED_BYTES,
            max_objects: DEFAULT_MAX_OBJECTS,
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
                "ref update rejected: {ref_name} → {target} present in the pushed set but not reachable as a complete closure"
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
pub fn receive_pack(
    gate: &FlagGate,
    req: &ReceiveRequest,
    cas: &mut dyn Cas,
    log: &mut EventLog,
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
    let unpacked = unpack_pack(&req.pack, limits)?;
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

    // 5b. reachability: the target must be REACHABLE from the pushed pack — its
    //     full commit→tree→(subtree|blob) closure present in the delivered set —
    //     not merely present somewhere. A pack that smuggles a stray object
    //     (present but with an incomplete closure) is rejected. (Targets already
    //     live in CAS are trusted prior tips.)
    if delivered && !unpacked.target_reachable(&req.update.new_oid) {
        return Err(ReceiveError::UnreachableTarget {
            ref_name: req.update.ref_name.clone(),
            target: req.update.new_oid.clone(),
        });
    }

    // 6. compare-and-append: apply only if the current derived view still shows
    //    the tip the pusher expected. A stale expectation is rejected with NO
    //    write and NO append (the concurrent winner is preserved).
    let state: RefState = replay(log).map_err(|e| ReceiveError::Io {
        detail: format!("replay derived view: {e:?}"),
    })?;
    let actual = state.get(&req.update.ref_name).map(str::to_string);
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
        receive_pack(&self.gate, req, cas, log, self.limits)
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
    /// Whether `target` is REACHABLE from the pushed set: it is present AND its
    /// full closure (commit → tree → subtree/blob, tag → target, …) is wholly
    /// present in the unpacked set. A stray/dangling target or an incomplete
    /// closure → NOT reachable. Fail-closed and panic-free on malformed objects:
    /// an object that fails to parse makes the closure incomplete (unreachable),
    /// never a panic.
    fn target_reachable(&self, target: &str) -> bool {
        // BFS over the object graph. Every referenced oid must be present in the
        // unpacked set; a missing reference fails the closure (not reachable).
        let mut stack: Vec<String> = vec![target.to_string()];
        let mut seen: HashMap<String, ()> = HashMap::new();
        while let Some(oid) = stack.pop() {
            if seen.insert(oid.clone(), ()).is_some() {
                continue;
            }
            let Some((kind, data)) = self.by_oid.get(&oid) else {
                // A referenced object is absent → the closure is incomplete.
                return false;
            };
            match kind {
                Kind::Blob => {}
                Kind::Commit => {
                    // tree + parents; a malformed commit fails closed.
                    let mut iter = gix_object::CommitRefIter::from_bytes(data);
                    match iter.tree_id() {
                        Ok(tree) => stack.push(tree.to_hex().to_string()),
                        Err(_) => return false,
                    }
                    for parent in gix_object::CommitRefIter::from_bytes(data).parent_ids() {
                        stack.push(parent.to_hex().to_string());
                    }
                }
                Kind::Tree => {
                    let entries = match gix_object::TreeRefIter::from_bytes(data).entries() {
                        Ok(e) => e,
                        Err(_) => return false,
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
                    Err(_) => return false,
                },
            }
        }
        // The target itself must have been present (the very first pop checks it).
        self.by_oid.contains_key(target)
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
/// v0 resolves only SELF-CONTAINED deltas (base in the pack). A thin-pack delta
/// (base absent) is rejected as [`ReceiveError::MalformedPack`]. The pack bytes
/// are attacker-controlled: every parse / decompress / delta step is fallible and
/// bounds-checked — a malformed pack is a typed error, never a panic.
fn unpack_pack(pack: &[u8], limits: RecvLimits) -> Result<UnpackedPack, ReceiveError> {
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
        // restores that parity. Per-object `apply_delta` capping at the REMAINING
        // budget bounds peak allocation to `max_inflated`, never the sum.)
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
    while !remaining.is_empty() {
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
            // full ceiling) so a single delta can't expand past what's left — peak
            // allocation is bounded to `max_inflated`, not `max_inflated × deltas`.
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
            // No delta resolved this pass: every remaining base is missing from the
            // pack → a thin pack (v0 limitation), rejected fail-closed.
            return Err(ReceiveError::MalformedPack {
                detail: "thin pack: a delta's base object is not in the pack (v0 \
                         resolves self-contained packs only)"
                    .to_string(),
            });
        }
        remaining = still_pending;
    }

    Ok(UnpackedPack { objects, by_oid })
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
        let up = unpack_pack(FIXTURE, RecvLimits::default()).expect("self-contained pack unpacks");

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
        let up = unpack_pack(FIXTURE, RecvLimits::default()).expect("unpacks");
        assert!(
            up.target_reachable(COMMIT_OID),
            "commit reachable: tree + blob closure present"
        );
        assert!(
            up.target_reachable(TREE_OID),
            "tree reachable: its blob is present"
        );
        let absent = "0000000000000000000000000000000000000000";
        assert!(
            !up.target_reachable(absent),
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
        let err = unpack_pack(FIXTURE, tiny_objects).expect_err("object-count cap trips");
        assert!(
            matches!(err, ReceiveError::DecompressionBomb { .. }),
            "expected DecompressionBomb, got {err:?}"
        );

        let tiny_inflated = RecvLimits {
            max_inflated_bytes: 1,
            ..RecvLimits::default()
        };
        let err = unpack_pack(FIXTURE, tiny_inflated).expect_err("inflated cap trips");
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
        let err = unpack_pack(truncated, RecvLimits::default())
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
        let err = unpack_pack(&bad, RecvLimits::default()).expect_err("non-V2 rejected");
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
        let up = unpack_pack(DELTA_FIXTURE, RecvLimits::default()).expect("delta pack unpacks");
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
            up.target_reachable(DELTA_HEAD),
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
        let err = unpack_pack(DELTA_FIXTURE, tight)
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

    /// (8) The HEADLINE reachability security property: a tip whose closure is NOT
    /// wholly in the pushed set is UNREACHABLE — it can never be advertised (which
    /// would 404 mid-clone). Only an ABSENT oid was tested before; this proves a
    /// PRESENT-but-incomplete-closure target is rejected.
    #[test]
    fn incomplete_closure_target_is_unreachable() {
        let up = unpack_pack(INCOMPLETE_FIXTURE, RecvLimits::default())
            .expect("commit-only pack unpacks");
        assert_eq!(up.objects.len(), 1, "only the commit object is delivered");
        assert!(
            !up.target_reachable(DELTA_HEAD),
            "a commit whose tree/blob closure is absent must be UNREACHABLE (fail-closed)"
        );
    }
}
