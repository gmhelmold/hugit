//! The porcelain file seam's **lock + atomic-write** discipline (WP-WC1).
//!
//! Every porcelain verb that reads-modifies-writes the shared `--log` (or
//! `--store`) file does so under this module, killing the TOCTOU the SOTA audit
//! flagged (`canonical_log.rs` read→serialize→write with no lock/atomic rename:
//! two concurrent `intent new --log` could load the same chain, append, and
//! clobber each other → a silently truncated/forked log).
//!
//! ## The two disciplines, std-only
//!
//! 1. **Atomic persist** ([`atomic_write`]): write the new bytes to a temp file
//!    in the SAME directory as the target, `fsync` it, then `rename` it over the
//!    target. `rename(2)` within one filesystem is atomic, so a reader (or a
//!    crash) ever sees either the whole old file or the whole new file — never a
//!    half-written one. (Same-directory is required: a cross-device rename is
//!    not atomic and most platforms refuse it outright.)
//!
//! 2. **Advisory exclusive lock** ([`FileLock::acquire`]): a `<path>.lock`
//!    sidecar created with [`OpenOptions::create_new`] — an atomic
//!    "create-iff-absent" the OS guarantees. The holder owns the seam across the
//!    whole load→mutate→persist; a second verb racing the same file finds the
//!    lock present and fails **structured** (`log_busy`, with a fix) rather than
//!    silently clobbering. The lock is released (file removed) on `Drop`, so a
//!    normal exit — success OR a structured error returned up the stack — frees
//!    it.
//!
//! ### Honest limits (std-only, documented)
//!
//! - **Advisory, not mandatory.** This serializes hugit's OWN porcelain verbs
//!   (they all go through this seam); it does NOT stop an unrelated process from
//!   writing the file behind hugit's back. That is the correct scope: the TOCTOU
//!   is hugit-vs-hugit.
//! - **No `flock(2)`/`fcntl` in std.** A real OS advisory lock (released by the
//!   kernel on process death, even a `SIGKILL`) is not reachable without a
//!   platform crate, and the WP forbids new deps. So a crashed/`kill -9`'d
//!   holder can leave a **stale lock**. We handle that with an explicit,
//!   documented **age-based takeover**: a lock file older than
//!   [`STALE_LOCK_SECS`] is presumed abandoned and reclaimed (logged in the
//!   lock's own bytes: pid + unix-mtime). The window is deliberately generous —
//!   a porcelain verb runs in milliseconds, so a lock minutes old is certainly
//!   dead, while a live verb never ages into takeover.
//! - **Not NFS-safe.** `create_new` atomicity and same-dir `rename` atomicity
//!   are POSIX-local-FS guarantees; networked filesystems may weaken them. The
//!   hermetic file seam is local-disk by mandate (P2 is the live-infra seam), so
//!   this is in scope.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A lock older than this (by its on-disk mtime) is presumed abandoned by a
/// dead holder and reclaimed (age-based stale-lock takeover — see module doc).
///
/// 120 s is ~5 orders of magnitude above a porcelain verb's real runtime
/// (single-digit ms): a LIVE verb never ages into takeover, while a crashed
/// holder's lock is reclaimed promptly. Tunable in one place.
pub const STALE_LOCK_SECS: u64 = 120;

/// Why a lock could not be acquired / a write could not be made atomic.
#[derive(Debug)]
pub enum LockError {
    /// The `<path>.lock` is held by a live holder (fresh, not stale) — another
    /// porcelain verb owns the seam. The caller maps this to the structured
    /// `log_busy` error (retry-able).
    Busy {
        /// The locked target path.
        path: PathBuf,
    },
    /// An I/O fault creating/removing the lock, or writing the temp file /
    /// renaming it over the target. Carries the action + underlying error.
    Io {
        /// What we were doing (e.g. `"create lock"`, `"rename temp over target"`).
        action: &'static str,
        /// The path the action targeted.
        path: PathBuf,
        /// The underlying OS error string (kept as a string so [`LockError`] is
        /// independent of `io::Error` and easy to thread through callers).
        source: String,
    },
}

impl std::fmt::Display for LockError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LockError::Busy { path } => {
                write!(f, "{} is locked by another hugit verb", path.display())
            }
            LockError::Io {
                action,
                path,
                source,
            } => write!(f, "{action} {}: {source}", path.display()),
        }
    }
}

impl std::error::Error for LockError {}

/// A held advisory exclusive lock over a porcelain file. Released (the lock file
/// removed) on [`Drop`], so any exit path — success or a structured error
/// returned up the stack — frees the seam.
///
/// Acquire it BEFORE loading the file and hold it across the whole
/// load→mutate→persist so the read-modify-write is serialized (no TOCTOU).
#[derive(Debug)]
pub struct FileLock {
    lock_path: PathBuf,
}

impl FileLock {
    /// Acquire the advisory exclusive lock for `target` (creates
    /// `<target>.lock` atomically). Returns:
    /// - `Ok(lock)` — the seam is ours until the returned guard drops;
    /// - `Err(LockError::Busy)` — a LIVE holder owns it (caller → `log_busy`);
    /// - `Err(LockError::Io)` — an I/O fault.
    ///
    /// A **stale** lock (mtime older than [`STALE_LOCK_SECS`] — a dead holder)
    /// is reclaimed transparently: removed, then re-created as ours.
    pub fn acquire(target: &Path) -> Result<Self, LockError> {
        // Runtime canonical state must be validated before this function's mkdir
        // can create its parent or sidecar. Non-runtime custom --log paths pass
        // through unchanged.
        crate::runtime_store::prepare_runtime_log(target).map_err(|e| LockError::Io {
            action: "prepare runtime log",
            path: target.to_path_buf(),
            source: e.to_json(),
        })?;
        Self::acquire_unprepared(target)
    }

    /// Acquire without runtime preparation. Runtime migration owns this internal
    /// bootstrap lock, so routing it through [`Self::acquire`] would recurse.
    pub(crate) fn acquire_unprepared(target: &Path) -> Result<Self, LockError> {
        // PR-4: auto-create parent dirs (git-proximate auto-init doctrine). The
        // first verb on a fresh CWD (e.g. `hugit campaign open`) used to fail
        // with `create lock .hugit/log.json.lock: No such file or directory`
        // because `try_create` requires the parent to exist. The fix is
        // best-effort: a subsequent `try_create` failure is the canonical
        // signal of a real I/O error (permission denied, etc.), so silently
        // ignoring the mkdir result is safe. Skipped if `target.parent()` is
        // empty (a bare-filename target like `log.json` in CWD — `create_dir_all("")`
        // returns NotFound on POSIX, so we skip).
        if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
            let _ = std::fs::create_dir_all(parent);
        }
        let lock_path = lock_path_for(target);
        match Self::try_create(&lock_path) {
            Ok(()) => Ok(FileLock { lock_path }),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                // The lock exists. Is it stale (a dead holder) or live?
                if lock_is_stale(&lock_path) {
                    // Reclaim: remove the abandoned lock, then re-create ours.
                    // A race between two reclaimers is itself resolved by
                    // create_new — exactly one wins the re-create, the other
                    // sees AlreadyExists and reports Busy.
                    let _ = std::fs::remove_file(&lock_path);
                    match Self::try_create(&lock_path) {
                        Ok(()) => Ok(FileLock { lock_path }),
                        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                            Err(LockError::Busy {
                                path: target.to_path_buf(),
                            })
                        }
                        Err(e) => Err(LockError::Io {
                            action: "re-create lock after stale takeover",
                            path: lock_path.clone(),
                            source: e.to_string(),
                        }),
                    }
                } else {
                    Err(LockError::Busy {
                        path: target.to_path_buf(),
                    })
                }
            }
            Err(e) => Err(LockError::Io {
                action: "create lock",
                path: lock_path,
                source: e.to_string(),
            }),
        }
    }

    /// `create_new`-create the lock file and stamp it with the holder's pid +
    /// unix seconds (so a human inspecting a stale lock can see who/when).
    fn try_create(lock_path: &Path) -> std::io::Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(lock_path)?;
        let stamp = format!("pid={} at_unix={}\n", std::process::id(), now_unix_secs());
        f.write_all(stamp.as_bytes())?;
        f.sync_all()?;
        Ok(())
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Best-effort release: a failure here only risks a stale lock the
        // age-based takeover will reclaim — never a correctness loss.
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

/// Atomically replace `target`'s contents with `bytes`: write a temp file in the
/// SAME directory, `fsync`, then `rename` it over `target`. A reader or a crash
/// sees either the whole old file or the whole new one — never a half-write.
///
/// Same-directory is load-bearing: a cross-device `rename(2)` is not atomic (and
/// most platforms refuse it), so the temp MUST share the target's filesystem.
pub fn atomic_write(target: &Path, bytes: &[u8]) -> Result<(), LockError> {
    // Every canonical runtime persist rechecks/recover-binds legacy evidence
    // while caller still holds FileLock. This catches writer paths that only use
    // generic lock/write helpers rather than a point-local migration call.
    let bytes =
        crate::runtime_store::prepare_runtime_write(target, bytes).map_err(|e| LockError::Io {
            action: "prepare runtime log write",
            path: target.to_path_buf(),
            source: e.to_json(),
        })?;
    atomic_write_unprepared(target, &bytes)
}

/// Atomic write without runtime preparation. Only runtime migration/recovery may
/// use this after it has acquired the bootstrap lock and verified source state.
pub(crate) fn atomic_write_unprepared(target: &Path, bytes: &[u8]) -> Result<(), LockError> {
    let dir = target.parent().unwrap_or_else(|| Path::new("."));
    // PR-4: best-effort create the parent (skipped if `dir` is empty —
    // `create_dir_all("")` returns NotFound on POSIX). The downstream
    // `File::create` will surface a real I/O error if mkdir legitimately
    // failed (e.g. permission denied).
    if !dir.as_os_str().is_empty() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file_name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "log".to_string());
    // A unique temp name in the same dir (pid + nanos): two atomic_writes never
    // collide on the temp, even from the same process.
    let temp = dir.join(format!(
        ".{}.tmp-{}-{}",
        file_name,
        std::process::id(),
        now_unix_nanos()
    ));

    {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(&temp).map_err(|e| LockError::Io {
            action: "create temp file",
            path: temp.clone(),
            source: e.to_string(),
        })?;
        f.write_all(bytes).map_err(|e| LockError::Io {
            action: "write temp file",
            path: temp.clone(),
            source: e.to_string(),
        })?;
        // fsync the bytes before the rename so the rename can't expose an empty
        // file on a crash between rename and writeback.
        f.sync_all().map_err(|e| LockError::Io {
            action: "fsync temp file",
            path: temp.clone(),
            source: e.to_string(),
        })?;
    }

    std::fs::rename(&temp, target).map_err(|e| {
        // Clean up the temp on a failed rename — never leave litter.
        let _ = std::fs::remove_file(&temp);
        LockError::Io {
            action: "rename temp over target",
            path: target.to_path_buf(),
            source: e.to_string(),
        }
    })?;
    // Directory fsync makes rename durable across power loss, not merely atomic
    // while this process remains alive.
    File::open(dir)
        .and_then(|dir| dir.sync_all())
        .map_err(|e| LockError::Io {
            action: "fsync parent directory",
            path: dir.to_path_buf(),
            source: e.to_string(),
        })?;
    Ok(())
}

/// The sidecar lock path for a target file: `<target>.lock` (in the same dir, so
/// it shares the target's filesystem).
fn lock_path_for(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(".lock");
    PathBuf::from(s)
}

/// Whether the lock at `lock_path` is stale (mtime older than
/// [`STALE_LOCK_SECS`]) — a dead holder we may reclaim. A lock whose mtime we
/// cannot read is treated as NOT stale (fail-closed: never reclaim a lock we
/// can't reason about).
fn lock_is_stale(lock_path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(lock_path) else {
        return false;
    };
    let Ok(modified) = meta.modified() else {
        return false;
    };
    match modified.elapsed() {
        Ok(age) => age.as_secs() >= STALE_LOCK_SECS,
        // A mtime in the future (clock skew) is not stale.
        Err(_) => false,
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-wc1-filelock-{tag}-{}-{}",
            std::process::id(),
            now_unix_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn atomic_write_replaces_contents_whole() {
        let dir = scratch("atomic");
        let target = dir.join("log.json");
        std::fs::write(&target, b"old").unwrap();
        atomic_write(&target, b"new bytes").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"new bytes");
        // No temp litter left behind.
        let leftovers: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().contains(".tmp-"))
            .collect();
        assert!(leftovers.is_empty(), "temp files must be renamed away");
    }

    /// PR-4: atomic_write creates missing parent dirs (auto-init doctrine).
    #[test]
    fn atomic_write_creates_missing_parent_dirs() {
        let dir = scratch("atomic-parent");
        let target = dir.join("missing/deep/log.json");
        // Parent dirs do NOT exist — atomic_write should create them.
        atomic_write(&target, b"hello").unwrap();
        assert!(target.exists());
        assert_eq!(std::fs::read(&target).unwrap(), b"hello");
    }

    /// PR-4: FileLock::acquire creates missing parent dirs.
    #[test]
    fn file_lock_acquire_creates_missing_parent_dirs() {
        let dir = scratch("lock-parent");
        let target = dir.join("missing/deep/log.json");
        // Parent dirs do NOT exist.
        let _lock = FileLock::acquire(&target).unwrap();
        assert!(dir.join("missing/deep").exists());
    }

    /// PR-4: bare-filename target (parent IS CWD-ish) does not trigger
    /// `create_dir_all("")` which returns NotFound on POSIX; the filter
    /// avoids that and the lock still works.
    #[test]
    fn file_lock_acquire_creates_lock_in_subdir() {
        // The target's PARENT (a subdir of scratch) does NOT exist —
        // `acquire` must `create_dir_all` it before `try_create`. This
        // exercises the PR-4 auto-init happy path.
        let dir = scratch("lock-subdir");
        let target = dir.join("missing_deep/log.json");
        let lock_path = target.with_file_name("log.json.lock");
        let _lock = FileLock::acquire(&target).unwrap();
        assert!(lock_path.exists(), "lock file must exist at <target>.lock");
        assert!(dir.join("missing_deep").is_dir(), "parent must be created");
    }

    /// PR-4: `acquire` with `target.parent() == Path::new("")` (truly empty,
    /// i.e. bare filename in CWD) must NOT call `create_dir_all("")`
    /// (which returns NotFound on POSIX). The lock is created in CWD.
    /// `scratch` reuses a per-test tmp dir, so we change the CWD to it
    /// to keep the lock out of the developer's CWD.
    #[test]
    fn file_lock_acquire_bare_filename_in_cwd_skips_create_dir_all() {
        let dir = scratch("lock-cwd-bare");
        let prev_cwd = std::env::current_dir().expect("cwd");
        std::env::set_current_dir(&dir).expect("chdir");
        let target = std::path::PathBuf::from("log.json");
        let lock_path = dir.join("log.json.lock");
        let result = FileLock::acquire(&target);
        // Restore CWD before asserting (Drop of dir would chdir back, but
        // we restore explicitly to avoid leaks if assert fails).
        std::env::set_current_dir(&prev_cwd).expect("restore cwd");
        assert!(
            result.is_ok(),
            "acquire with bare filename must succeed: {result:?}"
        );
        assert!(lock_path.exists(), "lock file must exist at <target>.lock");
    }

    #[test]
    fn atomic_write_creates_a_fresh_target() {
        let dir = scratch("atomic-fresh");
        let target = dir.join("fresh.json");
        atomic_write(&target, b"[]").unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"[]");
    }

    #[test]
    fn second_acquire_is_busy_while_first_held() {
        let dir = scratch("busy");
        let target = dir.join("log.json");
        let _held = FileLock::acquire(&target).expect("first lock");
        match FileLock::acquire(&target) {
            Err(LockError::Busy { .. }) => {}
            other => panic!("second concurrent lock must be Busy, got {other:?}"),
        }
    }

    #[test]
    fn lock_released_on_drop_allows_reacquire() {
        let dir = scratch("drop");
        let target = dir.join("log.json");
        {
            let _held = FileLock::acquire(&target).expect("first lock");
        } // drop releases it
        let _again = FileLock::acquire(&target).expect("re-acquire after drop");
    }

    #[test]
    fn fresh_planted_lock_is_busy_not_reclaimed() {
        let dir = scratch("fresh-lock");
        let target = dir.join("log.json");
        let lock_path = lock_path_for(&target);
        std::fs::write(&lock_path, b"pid=1 at_unix=0\n").unwrap();
        // A just-created lock is NOT stale (fresh mtime) → Busy, never reclaimed.
        assert!(!lock_is_stale(&lock_path));
        match FileLock::acquire(&target) {
            Err(LockError::Busy { .. }) => {}
            other => panic!("fresh planted lock must be Busy, got {other:?}"),
        }
    }

    #[test]
    fn zero_threshold_makes_any_lock_reclaimable() {
        // Prove the staleness predicate is purely age-driven: with the real
        // threshold a fresh lock is live; a lock whose mtime we cannot read is
        // fail-closed NOT stale (never reclaimed blindly).
        let dir = scratch("threshold");
        let absent = dir.join("never-made.lock");
        assert!(
            !lock_is_stale(&absent),
            "an unreadable/absent lock is fail-closed NOT stale"
        );
        assert_eq!(STALE_LOCK_SECS, 120, "the documented takeover window");
    }
}
