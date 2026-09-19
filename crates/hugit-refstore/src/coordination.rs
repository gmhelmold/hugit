//! Experimental native coordination; NEVER a replacement for the v1 lockfile.
//!
//! Explicitly create/open `coordination-v2-experimental` beneath an admitted
//! local directory. Production v1 writers do not use this namespace. Activation
//! or in-place migration remains gated by HUG-058/HUG-018, not by these locks.
//!
//! Order: installation-budget -> repo-admission -> operation, then lexical key.
//! Acquisition is nonblocking; do not hold a DB transaction across acquisition.
//! Each set holds at most 64 resource handles plus one shared admission handle.
//! Resource lockfiles contain no payload, remain on disk after release, and are
//! NEVER deleted, truncated or reclaimed by age/PID. The stable admission file
//! contains only its bounded Active/Disabled state.
//! Leases explicitly unlock before close, including handles copied transiently
//! by fork-before-exec. The private File is not cloned/exported. std opens handles close-on-exec /
//! non-inheritable for Command children; fork-without-exec is not supported.
//!
//! Trust: cooperative same-user local filesystem and admitted root. Native locks
//! do not contain old/incompatible writers or a hostile actor replacing ancestors.
//! The admission handle precedes resource locks and is kept until the last one
//! is released. Disabling needs an exclusive admission handle: Busy cannot be
//! overridden by age/PID. The disabled state is permanent for this namespace.
//! Lifecycle revision 1 deliberately rejects the preceding experimental marker;
//! no in-place upgrade/reactivation is provided. Old clients reject new markers.
//! Read-only inspection is observation, never a lease or a v1 cutover permit.
//!
//! Network filesystems, power-loss durability and production rollout are not
//! qualified by this primitive. No subprocess, DB, data migration or PID signaling.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, Write};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};

pub const EXPERIMENTAL_DIRECTORY: &str = "coordination-v2-experimental";
pub const MAX_HELD_LOCKS: usize = 64;
const MARKER: &[u8] = b"hugit.coordination/2-experimental-lifecycle1\n";
const WRITER_STATE_FILE: &str = "writer-state";
const ACTIVE: &[u8] = b"active\n";
const DISABLED: &[u8] = b"disabled\n";

/// A bounded read-only observation. Active is NOT admission to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriterState {
    Active,
    Disabled,
}

#[derive(Debug)]
pub enum CoordinationError {
    UnsupportedPlatform,
    InvalidNamespace,
    InvalidKey,
    InvalidLockFile,
    InvalidWriterState,
    WritersDisabled,
    OutOfOrder,
    TooManyLocks,
    Busy,
    Io(io::Error),
}
impl std::fmt::Display for CoordinationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "coordination I/O: {e}"),
            other => write!(f, "coordination refused: {other:?}"),
        }
    }
}
impl std::error::Error for CoordinationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}
impl From<io::Error> for CoordinationError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

/// The declaration order is the global acquisition order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LockClass {
    InstallationBudget,
    RepoAdmission,
    Operation,
}
impl LockClass {
    const fn prefix(self) -> &'static str {
        match self {
            Self::InstallationBudget => "0-installation-budget",
            Self::RepoAdmission => "1-repo-admission",
            Self::Operation => "2-operation",
        }
    }
}

/// A bounded path component, not a PID or a caller-selected filesystem path.
/// The adapter must derive the same canonical resource key for all writers.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct LockRequest {
    class: LockClass,
    key: String,
}
impl LockRequest {
    pub fn new(class: LockClass, key: &str) -> Result<Self, CoordinationError> {
        if key.is_empty()
            || key.len() > 64
            || !key
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-' || c == b'_')
        {
            return Err(CoordinationError::InvalidKey);
        }
        Ok(Self {
            class,
            key: key.into(),
        })
    }
}

#[derive(Debug, Clone)]
pub struct ExperimentalCoordinator {
    root: PathBuf,
}
impl ExperimentalCoordinator {
    /// Create a NEW isolated namespace. Existing or partially initialized
    /// directories are not adopted/reset. No parent directories are created.
    pub fn create(parent: &Path) -> Result<Self, CoordinationError> {
        supported()?;
        let root = parent.join(EXPERIMENTAL_DIRECTORY);
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&root)?;
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        // State is initialized before the protocol marker advertises readiness.
        // A failed initialization stays visibly incomplete; it is never reset.
        let mut state = options.open(root.join(WRITER_STATE_FILE))?;
        state.write_all(ACTIVE)?;
        state.sync_all()?;
        drop(state);
        let mut file = options.open(root.join("protocol"))?;
        file.write_all(MARKER)?;
        file.sync_all()?;
        // Retain a partial namespace on error; never delete potentially shared data.
        Self::open(parent)
    }

    pub fn open(parent: &Path) -> Result<Self, CoordinationError> {
        supported()?;
        let root = parent.join(EXPERIMENTAL_DIRECTORY);
        validate_namespace(&root)?;
        read_writer_state(&open_stable_file(
            &root.join(WRITER_STATE_FILE),
            false,
            false,
        )?)?;
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    /// Inspect even on targets without qualified locking. Never creates files,
    /// takes a writer lease, migrates data, or treats an unknown state as Active.
    pub fn inspect(parent: &Path) -> Result<WriterState, CoordinationError> {
        let root = parent.join(EXPERIMENTAL_DIRECTORY);
        validate_namespace(&root)?;
        read_writer_state(&open_stable_file(
            &root.join(WRITER_STATE_FILE),
            false,
            false,
        )?)
    }

    /// Permanently disable compatible writers only after proving quiescence.
    /// This is NOT permission to enable a v1 writer or migrate a store.
    /// No wait, owner signaling, file replacement, or reactivation is performed.
    pub fn try_disable(&self) -> Result<(), CoordinationError> {
        validate_namespace(&self.root)?;
        let path = self.root.join(WRITER_STATE_FILE);
        let state = open_stable_file(&path, false, true)?;
        let mut state = lock_handle(state, false)?;
        validate_opened_file(&path, &state)?;
        match read_writer_state(&state)? {
            WriterState::Active => {
                // A process crash during this longer in-place value leaves either
                // Active (not yet changed), Disabled, or an invalid state. Invalid
                // blocks writers. Never recover a torn value by assuming Active.
                state.rewind()?;
                state.write_all(DISABLED)?;
            }
            WriterState::Disabled => {}
        }
        // Retry also syncs: a prior sync failure must not become unsynced success.
        state.sync_all()?;
        Ok(())
    }

    pub fn lock_set(&self) -> LockSet {
        LockSet {
            root: self.root.clone(),
            held: Vec::new(),
            admission: None,
        }
    }

    /// For diagnostics. A path is not an ownership token or permission to unlink.
    pub fn lock_path(&self, request: &LockRequest) -> PathBuf {
        lock_path(&self.root, request)
    }
}

fn validate_namespace(root: &Path) -> Result<(), CoordinationError> {
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(CoordinationError::InvalidNamespace);
    }
    let path = root.join("protocol");
    let file = open_stable_file(&path, false, false)?;
    if file.metadata()?.len() != MARKER.len() as u64 {
        return Err(CoordinationError::InvalidNamespace);
    }
    let mut bytes = Vec::with_capacity(MARKER.len() + 1);
    file.take(MARKER.len() as u64 + 1).read_to_end(&mut bytes)?;
    if bytes != MARKER {
        return Err(CoordinationError::InvalidNamespace);
    }
    Ok(())
}

fn read_writer_state(file: &File) -> Result<WriterState, CoordinationError> {
    let mut reader = file;
    reader.rewind()?;
    let mut bytes = Vec::with_capacity(DISABLED.len() + 1);
    reader
        .take(DISABLED.len() as u64 + 1)
        .read_to_end(&mut bytes)?;
    match bytes.as_slice() {
        ACTIVE => Ok(WriterState::Active),
        DISABLED => Ok(WriterState::Disabled),
        _ => Err(CoordinationError::InvalidWriterState),
    }
}

fn open_stable_file(path: &Path, create: bool, writable: bool) -> Result<File, CoordinationError> {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.is_file() => return Err(CoordinationError::InvalidLockFile),
        Ok(_) => {}
        Err(e) if create && e.kind() == io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(writable)
        .create(create)
        .truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        // FILE_SHARE_READ | FILE_SHARE_WRITE, never FILE_SHARE_DELETE.
        options.share_mode(0x1 | 0x2);
    }
    let file = options.open(path)?;
    validate_opened_file(path, &file)?;
    Ok(file)
}

fn validate_opened_file(path: &Path, file: &File) -> Result<(), CoordinationError> {
    let opened = file.metadata()?;
    let on_path = fs::symlink_metadata(path)?;
    if !opened.is_file() || !on_path.is_file() {
        return Err(CoordinationError::InvalidLockFile);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if (opened.dev(), opened.ino()) != (on_path.dev(), on_path.ino()) || opened.nlink() != 1 {
            return Err(CoordinationError::InvalidLockFile);
        }
    }
    Ok(())
}

/// Private ownership guard. Closing alone can leave the same open-file lock in
/// a descriptor copied by a concurrently spawning thread before exec. Release
/// our lease explicitly first; no handle is exported by the public API.
#[derive(Debug)]
struct NativeLease(File);
impl Deref for NativeLease {
    type Target = File;
    fn deref(&self) -> &File {
        &self.0
    }
}
impl DerefMut for NativeLease {
    fn deref_mut(&mut self) -> &mut File {
        &mut self.0
    }
}
impl Drop for NativeLease {
    fn drop(&mut self) {
        // No destructive recovery on error. Closing the owned File remains the
        // fallback; a concurrent contender can conservatively report Busy.
        let _ = self.0.unlock();
    }
}

fn lock_handle(file: File, shared: bool) -> Result<NativeLease, CoordinationError> {
    let result = if shared {
        file.try_lock_shared()
    } else {
        file.try_lock()
    };
    match result {
        Ok(()) => Ok(NativeLease(file)),
        Err(TryLockError::WouldBlock) => Err(CoordinationError::Busy),
        Err(TryLockError::Error(e)) => Err(e.into()),
    }
}

fn supported() -> Result<(), CoordinationError> {
    if cfg!(any(target_os = "linux", target_os = "macos", windows)) {
        Ok(())
    } else {
        Err(CoordinationError::UnsupportedPlatform)
    }
}
fn lock_path(root: &Path, request: &LockRequest) -> PathBuf {
    root.join(format!("{}-{}.lock", request.class.prefix(), request.key))
}

#[derive(Debug)]
struct HeldLock {
    request: LockRequest,
    file: NativeLease,
}

/// Owns its handles; cannot be cloned or converted into an inheritable handle.
/// Single acquisition retains earlier leases on failure. Plan acquisition rolls
/// back only the leases added by that call; pre-existing ownership is preserved.
#[derive(Debug)]
pub struct LockSet {
    root: PathBuf,
    held: Vec<HeldLock>,
    // Field drop order matters: resource handles close BEFORE admission.
    admission: Option<NativeLease>,
}
impl LockSet {
    pub fn len(&self) -> usize {
        self.held.len()
    }
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// Acquire an ordered extension, or release only this call's new leases.
    /// The whole slice is checked for order and the 64-handle budget before any
    /// filesystem access. Requests are never sorted or deduplicated implicitly.
    ///
    /// This is NOT an atomic multi-file lock: contenders may observe a temporary
    /// prefix. Begin operation effects only after the entire call succeeds. On
    /// error, rollback releases the new prefix in reverse acquisition order,
    /// retaining earlier leases and their admission; stable files stay on disk.
    /// An empty plan is a no-op, not writer admission, even after disable.
    /// No wait, automatic retry, owner signaling, or v1 protocol change.
    pub fn try_acquire_plan(&mut self, requests: &[LockRequest]) -> Result<(), CoordinationError> {
        if requests.len() > MAX_HELD_LOCKS - self.held.len() {
            return Err(CoordinationError::TooManyLocks);
        }
        let mut previous = self.held.last().map(|held| &held.request);
        for request in requests {
            if previous.is_some_and(|last| request <= last) {
                return Err(CoordinationError::OutOfOrder);
            }
            previous = Some(request);
        }
        let original_len = self.held.len();
        for request in requests {
            if let Err(error) = self.try_acquire(request.clone()) {
                while self.held.len() > original_len {
                    self.release_last();
                }
                return Err(error);
            }
        }
        Ok(())
    }

    /// Checks ordering and resource budget BEFORE opening/creating another file.
    /// No blocking acquisition and no lease takeover/recovery heuristic.
    pub fn try_acquire(&mut self, request: LockRequest) -> Result<(), CoordinationError> {
        if self.held.len() >= MAX_HELD_LOCKS {
            return Err(CoordinationError::TooManyLocks);
        }
        if self.held.last().is_some_and(|last| request <= last.request) {
            return Err(CoordinationError::OutOfOrder);
        }
        validate_namespace(&self.root)?;
        // A first acquisition holds a temporary admission until its resource lock
        // succeeds. Any failure drops it, so an empty set cannot prevent disable.
        let pending_admission = if self.admission.is_none() {
            let path = self.root.join(WRITER_STATE_FILE);
            let file = open_stable_file(&path, false, false)?;
            let file = lock_handle(file, true)?;
            validate_opened_file(&path, &file)?;
            Some(file)
        } else {
            None
        };
        let admission = self
            .admission
            .as_ref()
            .or(pending_admission.as_ref())
            .ok_or(CoordinationError::InvalidWriterState)?;
        if read_writer_state(admission)? != WriterState::Active {
            return Err(CoordinationError::WritersDisabled);
        }
        let path = lock_path(&self.root, &request);
        let file = open_stable_file(&path, true, true)?;
        let file = lock_handle(file, false)?;
        validate_opened_file(&path, &file)?;
        if pending_admission.is_some() {
            self.admission = pending_admission;
        }
        self.held.push(HeldLock { request, file });
        Ok(())
    }

    /// Close the last handle. The stable file and its bytes remain untouched.
    pub fn release_last(&mut self) -> Option<LockRequest> {
        let HeldLock { request, file } = self.held.pop()?;
        drop(file);
        if self.held.is_empty() {
            drop(self.admission.take());
        }
        Some(request)
    }
}

#[cfg(test)]
mod release_regressions {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "hugit-lock-release-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }
    #[test]
    fn releasing_resource_unlocks_even_while_a_pre_exec_duplicate_exists() {
        let f = Fixture::new();
        let c = ExperimentalCoordinator::create(&f.0).unwrap();
        let request = LockRequest::new(LockClass::Operation, "release").unwrap();
        let mut held = c.lock_set();
        held.try_acquire(request.clone()).unwrap();
        // A private test-only duplicate models the descriptor copy during fork-before-exec.
        // No API gives callers this handle or grants the duplicate writer authority.
        let duplicate = held.held[0].file.try_clone().unwrap();
        held.release_last();
        let observed = c.lock_set().try_acquire(request);
        drop(duplicate);
        assert!(
            observed.is_ok(),
            "released resource remains held: {observed:?}"
        );
    }
    #[test]
    fn releasing_admission_unlocks_even_while_a_pre_exec_duplicate_exists() {
        let f = Fixture::new();
        let c = ExperimentalCoordinator::create(&f.0).unwrap();
        let mut held = c.lock_set();
        held.try_acquire(LockRequest::new(LockClass::Operation, "release").unwrap())
            .unwrap();
        let duplicate = held.admission.as_ref().unwrap().try_clone().unwrap();
        drop(held);
        let observed = c.try_disable();
        drop(duplicate);
        assert!(
            observed.is_ok(),
            "released admission remains held: {observed:?}"
        );
        assert_eq!(
            ExperimentalCoordinator::inspect(&f.0).unwrap(),
            WriterState::Disabled
        );
    }
}
