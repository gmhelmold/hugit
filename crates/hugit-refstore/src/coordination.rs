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
//! The private File is not cloned/exported. std opens handles close-on-exec /
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
        let mut state = open_stable_file(&path, false, true)?;
        lock_handle(&state, false)?;
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

fn lock_handle(file: &File, shared: bool) -> Result<(), CoordinationError> {
    let result = if shared {
        file.try_lock_shared()
    } else {
        file.try_lock()
    };
    match result {
        Ok(()) => Ok(()),
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
    file: File,
}

/// Owns its handles; cannot be cloned or converted into an inheritable handle.
/// A Busy result retains earlier leases. Release the set before retrying a plan.
#[derive(Debug)]
pub struct LockSet {
    root: PathBuf,
    held: Vec<HeldLock>,
    // Field drop order matters: resource handles close BEFORE admission.
    admission: Option<File>,
}
impl LockSet {
    pub fn len(&self) -> usize {
        self.held.len()
    }
    pub fn is_empty(&self) -> bool {
        self.held.is_empty()
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
            lock_handle(&file, true)?;
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
        lock_handle(&file, false)?;
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
