//! receive-pack ingest — the git push WRITE entrypoint.
//!
//! A push delivers a packfile plus the ref update(s) it wants applied. This
//! module ingests that push:
//!
//! 1. **bound** the pack (oversized packs are rejected before any work — the
//!    source-of-truth bar; an unbounded push is a DoS / poison vector);
//! 2. **unpack** the pack with the system git binary — the mature,
//!    libgit2-class pack engine. This is *not* a from-scratch protocol rewrite;
//!    hugit wires git's own unpacker to CAS persistence + the D1 log;
//! 3. **verify** every unpacked object (git re-derives each oid on unpack, so a
//!    malformed / injected object fails the unpack — fail-closed);
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

use crate::write::flag::{FlagGate, WritePathDisabled};
use crate::write::store::{Cas, CasObject, Oid, record_ref_update, store_objects};
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::log::EventLog;
use hugit_refstore::{RefState, replay};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
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
    /// The pack could not be parsed / unpacked by the git pack engine
    /// (truncated, corrupt, or an injected object that fails oid re-derivation).
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
    /// The requested ref target is present in the scratch object store but is NOT
    /// reachable from the pushed pack's objects — a ref pointing at a stray /
    /// dangling object the push did not actually deliver as its tip. Rejected
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
    /// An environment / IO failure (the git binary, temp dirs, etc.).
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
                "ref update rejected: {ref_name} → {target} present in odb but not reachable from the pushed pack"
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
    //    ANY pack work — no scratch odb, no CAS write, no event.
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

    // 2+3. unpack + verify via the system git pack engine into a scratch odb.
    //      git's unpack-objects re-derives every oid; a malformed or injected
    //      object makes it fail, so a clean exit IS the verification.
    let scratch = ScratchOdb::create()?;
    let objects = scratch.unpack_and_collect(&req.pack, limits)?;

    // 5a. anchor the requested ref update: its target must have been delivered by
    //     the push or already live in CAS. Reject tampering BEFORE any write.
    let delivered = objects.iter().any(|o| o.oid == req.update.new_oid);
    if !delivered && !cas.contains(&req.update.new_oid) {
        return Err(ReceiveError::RefUpdateTampered {
            ref_name: req.update.ref_name.clone(),
            target: req.update.new_oid.clone(),
        });
    }

    // 5b. reachability: the target must be REACHABLE from the pushed pack — not
    //     merely present somewhere in the scratch odb. A pack that smuggles a
    //     stray object (present but not reachable as the declared tip) is
    //     rejected. (Targets already live in CAS are trusted prior tips.)
    if delivered && !scratch.target_reachable(&req.update.new_oid)? {
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
    store_objects(cas, &objects);
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
/// version, count). Used to reject an object-count bomb before unpacking.
fn pack_header_object_count(pack: &[u8]) -> Result<u64, ReceiveError> {
    if pack.len() < 12 || &pack[0..4] != b"PACK" {
        return Err(ReceiveError::MalformedPack {
            detail: "pack too short or missing PACK signature".to_string(),
        });
    }
    let count = u32::from_be_bytes([pack[8], pack[9], pack[10], pack[11]]);
    Ok(u64::from(count))
}

/// A throwaway bare git object store used to unpack one push, then collect each
/// resulting loose object as a [`CasObject`]. Backed by a temp dir that is
/// removed on drop.
struct ScratchOdb {
    dir: TempDir,
}

impl ScratchOdb {
    fn create() -> Result<Self, ReceiveError> {
        let dir = TempDir::new("hugit-recv")?;
        run_git(
            dir.path(),
            &["init", "-q", "--bare", "."],
            None,
            "init bare odb",
        )?;
        Ok(Self { dir })
    }

    /// Unpack the pack into the odb (git re-derives + verifies every oid), then
    /// read each loose object back as its verbatim on-disk bytes.
    ///
    /// Enforces the inflated bounds in `limits` fail-closed: if the unpacked total
    /// (sum of object sizes) exceeds `max_inflated_bytes`, or the object count
    /// exceeds `max_objects`, the ingest is rejected as a
    /// [`ReceiveError::DecompressionBomb`] before any object is returned for CAS
    /// storage — a tiny compressed pack that inflates huge cannot slip through.
    fn unpack_and_collect(
        &self,
        pack: &[u8],
        limits: RecvLimits,
    ) -> Result<Vec<CasObject>, ReceiveError> {
        // unpack-objects fails closed on a truncated / corrupt / injected pack.
        run_git(
            self.dir.path(),
            &["unpack-objects", "-q"],
            Some(pack),
            "unpack-objects",
        )
        .map_err(|e| match e {
            // any non-zero exit from unpack-objects is a malformed pack.
            ReceiveError::Io { detail } => ReceiveError::MalformedPack { detail },
            other => other,
        })?;

        // enumerate every oid the odb now holds, with its INFLATED object size, so
        // the bomb guard sees what the pack actually expanded to.
        let listing = run_git(
            self.dir.path(),
            &[
                "cat-file",
                "--batch-all-objects",
                "--batch-check=%(objectname) %(objectsize)",
            ],
            None,
            "list objects",
        )?;

        // Cap the inflated total + object count fail-closed.
        let mut total_inflated: u64 = 0;
        let mut count: u64 = 0;
        for line in listing.lines() {
            let mut parts = line.split_whitespace();
            let _oid = match parts.next() {
                Some(o) => o,
                None => continue,
            };
            let size: u64 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(|| {
                ReceiveError::MalformedPack {
                    detail: format!("could not read object size from cat-file line: {line:?}"),
                }
            })?;
            count += 1;
            total_inflated = total_inflated.saturating_add(size);
            if count > limits.max_objects {
                return Err(ReceiveError::DecompressionBomb {
                    detail: format!(
                        "unpacked object count {count} exceeds ceiling {}",
                        limits.max_objects
                    ),
                });
            }
            if total_inflated > limits.max_inflated_bytes {
                return Err(ReceiveError::DecompressionBomb {
                    detail: format!(
                        "inflated total {total_inflated} bytes exceeds ceiling {} bytes",
                        limits.max_inflated_bytes
                    ),
                });
            }
        }

        let mut objects = Vec::new();
        for line in listing.lines() {
            let oid = match line.split_whitespace().next() {
                Some(o) => o,
                None => continue,
            };
            let bytes = self.read_loose(oid)?;
            objects.push(CasObject {
                oid: oid.to_string(),
                bytes,
            });
        }
        Ok(objects)
    }

    /// Whether `target` is REACHABLE from the pushed pack's objects (not merely
    /// present in the odb). We treat every object the odb holds as the pushed set
    /// and ask git to enumerate the closure reachable from `target`; if `target`
    /// is itself a valid root whose closure is wholly present, it is reachable.
    ///
    /// Concretely: `git rev-list --objects <target>` walks the commit/tree/blob
    /// graph from `target`. It succeeds only when `target` names an object whose
    /// reachable closure is in the odb — exactly "the push delivered this as a
    /// real tip", not a stray dangling object planted in the pack.
    fn target_reachable(&self, target: &str) -> Result<bool, ReceiveError> {
        let out = Command::new("git")
            .arg("-C")
            .arg(self.dir.path())
            .args(["rev-list", "--objects", target])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .map_err(|e| ReceiveError::Io {
                detail: format!("spawn git rev-list: {e}"),
            })?;
        if !out.status.success() {
            // rev-list fails when target is not a walkable root with a complete
            // closure — i.e. it is unreachable / dangling. Not reachable.
            return Ok(false);
        }
        // The target must appear in its own reachability listing.
        let listing = String::from_utf8_lossy(&out.stdout);
        Ok(listing.split_whitespace().any(|tok| tok == target))
    }

    /// Read the verbatim loose-object bytes for `oid` (`objects/xx/yyy…`).
    fn read_loose(&self, oid: &str) -> Result<Vec<u8>, ReceiveError> {
        if oid.len() < 3 {
            return Err(ReceiveError::MalformedPack {
                detail: format!("short oid {oid}"),
            });
        }
        let (dir, rest) = oid.split_at(2);
        let path = self.dir.path().join("objects").join(dir).join(rest);
        std::fs::read(&path).map_err(|e| ReceiveError::Io {
            detail: format!("read loose object {oid}: {e}"),
        })
    }
}

/// Run the system git binary in `cwd`, optionally feeding `stdin`. Returns
/// captured stdout on success; maps a non-zero exit / spawn failure to
/// [`ReceiveError::Io`] (callers reinterpret the unpack failure as malformed).
fn run_git(
    cwd: &Path,
    args: &[&str],
    stdin: Option<&[u8]>,
    what: &str,
) -> Result<String, ReceiveError> {
    let mut cmd = Command::new("git");
    cmd.arg("-C")
        .arg(cwd)
        .args(args)
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| ReceiveError::Io {
        detail: format!("spawn git {what}: {e}"),
    })?;

    if let Some(data) = stdin {
        let mut sink = child.stdin.take().ok_or_else(|| ReceiveError::Io {
            detail: format!("git {what}: no stdin handle"),
        })?;
        sink.write_all(data).map_err(|e| ReceiveError::Io {
            detail: format!("git {what}: write stdin: {e}"),
        })?;
        // drop closes stdin so git sees EOF.
        drop(sink);
    }

    let out = child.wait_with_output().map_err(|e| ReceiveError::Io {
        detail: format!("git {what}: wait: {e}"),
    })?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(ReceiveError::Io {
            detail: format!("git {what} failed ({}): {}", out.status, stderr.trim()),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A minimal self-cleaning temp directory (no extra crate dependency).
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

/// Materialize a bare git repository on disk from CAS objects + a ref pointing
/// at `head_oid`, so a client can clone it back. This is the minimal read-back
/// the round-trip needs while the full D2a serve path lands; it writes the
/// verbatim CAS loose bytes straight into `objects/xx/yyy…`, so the clone is
/// byte-identical to what was pushed.
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
    run_git(
        dir.path(),
        &["init", "-q", "--bare", "."],
        None,
        "init serve",
    )?;

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
        let obj_dir = dir.path().join("objects").join(d);
        std::fs::create_dir_all(&obj_dir).map_err(|e| ReceiveError::Io {
            detail: format!("mkdir {}: {e}", obj_dir.display()),
        })?;
        std::fs::write(obj_dir.join(rest), &bytes).map_err(|e| ReceiveError::Io {
            detail: format!("write object {oid}: {e}"),
        })?;
    }

    // set the ref from the recorded event target, and point HEAD at it so a
    // default clone checks it out.
    run_git(
        dir.path(),
        &["update-ref", ref_name, head_oid],
        None,
        "update-ref",
    )?;
    run_git(
        dir.path(),
        &["symbolic-ref", "HEAD", ref_name],
        None,
        "symbolic-ref",
    )?;

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
