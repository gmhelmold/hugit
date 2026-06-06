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

use crate::write::store::{Cas, CasObject, Oid, record_ref_update, store_objects};
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::log::EventLog;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// Default ceiling for an accepted pack (16 MiB) — a v0 bound, not a tuned
/// limit. Oversized packs are rejected up-front (red-team: oversized pack).
pub const DEFAULT_MAX_PACK_BYTES: usize = 16 * 1024 * 1024;

/// Limits applied to one receive-pack ingest.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecvLimits {
    /// Maximum accepted packfile size in bytes.
    pub max_pack_bytes: usize,
}

impl Default for RecvLimits {
    fn default() -> Self {
        Self {
            max_pack_bytes: DEFAULT_MAX_PACK_BYTES,
        }
    }
}

/// One ref update a push asks to apply: `ref_name` → `new_oid`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefUpdate {
    /// The full ref name, e.g. `refs/heads/main`.
    pub ref_name: String,
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
    /// The pack exceeded the configured size ceiling.
    OversizedPack { len: usize, max: usize },
    /// The pack could not be parsed / unpacked by the git pack engine
    /// (truncated, corrupt, or an injected object that fails oid re-derivation).
    MalformedPack { detail: String },
    /// The requested ref update points at an oid the push never delivered and
    /// that the CAS does not already hold — ref-update tampering.
    RefUpdateTampered { ref_name: String, target: Oid },
    /// An environment / IO failure (the git binary, temp dirs, etc.).
    Io { detail: String },
}

impl std::fmt::Display for ReceiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReceiveError::OversizedPack { len, max } => {
                write!(f, "pack rejected: {len} bytes exceeds ceiling {max}")
            }
            ReceiveError::MalformedPack { detail } => {
                write!(f, "pack rejected: malformed ({detail})")
            }
            ReceiveError::RefUpdateTampered { ref_name, target } => write!(
                f,
                "ref update rejected: {ref_name} → {target} not delivered by the push (tampered)"
            ),
            ReceiveError::Io { detail } => write!(f, "receive-pack IO error: {detail}"),
        }
    }
}

impl std::error::Error for ReceiveError {}

/// Ingest one receive-pack push: bound → unpack → verify → store → anchor →
/// record. On success the pushed objects are in CAS and one raw-push event is
/// on the log; on any error nothing is committed (fail-closed).
pub fn receive_pack(
    req: &ReceiveRequest,
    cas: &mut dyn Cas,
    log: &mut EventLog,
    limits: RecvLimits,
) -> Result<Receipt, ReceiveError> {
    // 1. bound the pack BEFORE any work.
    if req.pack.len() > limits.max_pack_bytes {
        return Err(ReceiveError::OversizedPack {
            len: req.pack.len(),
            max: limits.max_pack_bytes,
        });
    }

    // 2+3. unpack + verify via the system git pack engine into a scratch odb.
    //      git's unpack-objects re-derives every oid; a malformed or injected
    //      object makes it fail, so a clean exit IS the verification.
    let scratch = ScratchOdb::create()?;
    let objects = scratch.unpack_and_collect(&req.pack)?;

    // 5. anchor the requested ref update: its target must have been delivered by
    //    the push or already live in CAS. Reject tampering BEFORE any write.
    let delivered = objects.iter().any(|o| o.oid == req.update.new_oid);
    if !delivered && !cas.contains(&req.update.new_oid) {
        return Err(ReceiveError::RefUpdateTampered {
            ref_name: req.update.ref_name.clone(),
            target: req.update.new_oid.clone(),
        });
    }

    // 4. store every object into CAS (idempotent on oid).
    store_objects(cas, &objects);
    let mut stored_oids: Vec<Oid> = objects.iter().map(|o| o.oid.clone()).collect();
    stored_oids.sort();

    // 6. record the ref move as one append-only raw-push event (no provenance).
    let event = record_ref_update(
        log,
        req.principal_chain.clone(),
        &req.update.ref_name,
        &req.update.new_oid,
        req.recorded_at,
    );

    Ok(Receipt { stored_oids, event })
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
    fn unpack_and_collect(&self, pack: &[u8]) -> Result<Vec<CasObject>, ReceiveError> {
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

        // enumerate every oid the odb now holds.
        let listing = run_git(
            self.dir.path(),
            &[
                "cat-file",
                "--batch-all-objects",
                "--batch-check=%(objectname)",
            ],
            None,
            "list objects",
        )?;

        let mut objects = Vec::new();
        for oid in listing.split_whitespace() {
            let bytes = self.read_loose(oid)?;
            objects.push(CasObject {
                oid: oid.to_string(),
                bytes,
            });
        }
        Ok(objects)
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
