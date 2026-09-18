//! Experimental native coordination; NEVER a replacement for the v1 lockfile.
//!
//! Explicitly create/open `coordination-v2-experimental` beneath an admitted
//! local directory. Production v1 writers do not use this namespace. Activation
//! or in-place migration remains gated by HUG-058/HUG-018, not by these locks.
//!
//! Order: installation-budget -> repo-admission -> operation, then lexical key.
//! Acquisition is nonblocking; do not hold a DB transaction across acquisition.
//! Each set holds at most 64 handles. Lockfiles contain no payload, remain on
//! disk after release, and are NEVER deleted, truncated or reclaimed by age/PID.
//! The private File is not cloned/exported. std opens handles close-on-exec /
//! non-inheritable for Command children; fork-without-exec is not supported.
//!
//! Trust: cooperative same-user local filesystem and admitted root. Native locks
//! do not contain old/incompatible writers or a hostile actor replacing ancestors.
//! Network filesystems, power-loss durability and production rollout are not
//! qualified by this primitive. No subprocess, DB, data migration or PID signaling.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

pub const EXPERIMENTAL_DIRECTORY: &str = "coordination-v2-experimental";
pub const MAX_HELD_LOCKS: usize = 64;
const MARKER: &[u8] = b"hugit.coordination/2-experimental\n";

#[derive(Debug)]
pub enum CoordinationError {
    UnsupportedPlatform,
    InvalidNamespace,
    InvalidKey,
    InvalidLockFile,
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
        let mut file = options.open(root.join("protocol"))?;
        file.write_all(MARKER)?;
        file.sync_all()?;
        // Retain a partial namespace on error; never delete potentially shared data.
        Self::open(parent)
    }

    pub fn open(parent: &Path) -> Result<Self, CoordinationError> {
        supported()?;
        let root = parent.join(EXPERIMENTAL_DIRECTORY);
        let metadata = fs::symlink_metadata(&root)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(CoordinationError::InvalidNamespace);
        }
        let path = root.join("protocol");
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.len() != MARKER.len() as u64 {
            return Err(CoordinationError::InvalidNamespace);
        }
        let mut bytes = Vec::with_capacity(MARKER.len() + 1);
        File::open(&path)?
            .take(MARKER.len() as u64 + 1)
            .read_to_end(&mut bytes)?;
        if bytes != MARKER {
            return Err(CoordinationError::InvalidNamespace);
        }
        Ok(Self {
            root: root.canonicalize()?,
        })
    }

    pub fn lock_set(&self) -> LockSet {
        LockSet {
            root: self.root.clone(),
            held: Vec::new(),
        }
    }

    /// For diagnostics. A path is not an ownership token or permission to unlink.
    pub fn lock_path(&self, request: &LockRequest) -> PathBuf {
        lock_path(&self.root, request)
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
        let path = lock_path(&self.root, &request);
        match fs::symlink_metadata(&path) {
            Ok(m) if !m.is_file() => return Err(CoordinationError::InvalidLockFile),
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Permit compatible opens, but NOT rename/delete while handles live.
            // FILE_SHARE_READ | FILE_SHARE_WRITE (no FILE_SHARE_DELETE).
            options.share_mode(0x1 | 0x2);
        }
        let file = options.open(&path)?;
        if !file.metadata()?.is_file() {
            return Err(CoordinationError::InvalidLockFile);
        }
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => return Err(CoordinationError::Busy),
            Err(TryLockError::Error(e)) => return Err(e.into()),
        }
        let on_path = fs::symlink_metadata(&path)?;
        if !on_path.is_file() {
            return Err(CoordinationError::InvalidLockFile);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            let opened = file.metadata()?;
            if (opened.dev(), opened.ino()) != (on_path.dev(), on_path.ino()) || opened.nlink() != 1
            {
                return Err(CoordinationError::InvalidLockFile);
            }
        }
        self.held.push(HeldLock { request, file });
        Ok(())
    }

    /// Close the last handle. The stable file and its bytes remain untouched.
    pub fn release_last(&mut self) -> Option<LockRequest> {
        let HeldLock { request, file } = self.held.pop()?;
        drop(file);
        Some(request)
    }
}
