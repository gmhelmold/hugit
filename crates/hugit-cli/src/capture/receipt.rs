//! Immutable v1 hook receipts. Publication never exposes partial bytes.

#[cfg(unix)]
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ReceiptV1 {
    pub version: u8,
    pub repo_id: String,
    pub invocation_id: String,
    pub receipt_id: String,
    pub kind: String,
    pub principal: String,
    pub payload: serde_json::Value,
    pub recorded_at: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ReceiptFilesystemFailure {
    pub code: &'static str,
    pub message: &'static str,
}

pub fn receipt_filesystem_failure() -> Option<ReceiptFilesystemFailure> {
    #[cfg(unix)]
    {
        None
    }
    #[cfg(not(unix))]
    {
        Some(ReceiptFilesystemFailure {
            code: "receipt_filesystem_unsupported",
            message: "safe no-follow receipt filesystem is unavailable on this platform",
        })
    }
}

pub fn random_invocation_id() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| format!("generate cryptographic invocation id: {e}"))?;
    Ok(hex::encode(bytes))
}

pub fn valid_repo_id(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

pub fn receipt_id(repo_id: &str, invocation_id: &str) -> String {
    let mut hash = Sha256::new();
    hash.update(b"hugit-receipt-v1\0");
    hash.update(repo_id.as_bytes());
    hash.update(b"\0");
    hash.update(invocation_id.as_bytes());
    hex::encode(hash.finalize())
}

pub fn write_receipt(dir: &Path, receipt: &ReceiptV1) -> Result<PathBuf, String> {
    if receipt.version != 1
        || !valid_repo_id(&receipt.repo_id)
        || !valid_repo_id(&receipt.invocation_id)
        || receipt.receipt_id != receipt_id(&receipt.repo_id, &receipt.invocation_id)
    {
        return Err("invalid receipt identity".into());
    }
    ensure_private_dir(dir)?;
    let name = format!("{}.json", receipt.receipt_id);
    let path = dir.join(&name);
    let bytes = serde_json::to_vec(receipt).map_err(|e| format!("serialize receipt: {e}"))?;
    match atomic_create_immutable(dir, &name, &bytes) {
        Ok(()) => Ok(path),
        Err(error) if error == "already exists" => {
            if read_receipt_bytes(&path)? == bytes {
                Ok(path)
            } else {
                Err("receipt id collision has different immutable bytes".into())
            }
        }
        Err(error) => Err(error),
    }
}

/// Read one already-published receipt through a no-follow descriptor. This
/// binds validation to opened object, not a separately-stat'd pathname.
pub fn read_receipt_bytes(path: &Path) -> Result<Vec<u8>, String> {
    #[cfg(unix)]
    {
        let parent = path.parent().ok_or("receipt has no parent")?;
        let name = path.file_name().ok_or("receipt has no file name")?;
        let dir = open_private_dir(parent)?;
        let mut file = open_private_regular_at(&dir, name)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| format!("read receipt: {e}"))?;
        Ok(bytes)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("safe no-follow receipt reads are unavailable on this platform".into())
    }
}

pub fn read_receipt(path: &Path) -> Result<ReceiptV1, String> {
    serde_json::from_slice(&read_receipt_bytes(path)?).map_err(|e| format!("parse receipt: {e}"))
}

/// Receipt read through a no-follow descriptor. Source receipts are immutable:
/// completion is recorded separately, so drain never mutates a source pathname.
pub struct ClaimedReceipt {
    bytes: Vec<u8>,
    original_name: String,
    #[cfg(unix)]
    dir: std::fs::File,
    #[cfg(unix)]
    name: std::ffi::CString,
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
}

impl ClaimedReceipt {
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn receipt(&self) -> Result<ReceiptV1, String> {
        serde_json::from_slice(&self.bytes).map_err(|e| format!("parse receipt: {e}"))
    }

    pub fn has_original_name(&self, name: &str) -> bool {
        self.original_name == name
    }

    /// Immutable source name captured through no-follow claim. Dead letters use
    /// this frozen name rather than a content hash, so receipt identity remains
    /// visible without exposing raw receipt bytes.
    pub fn original_name(&self) -> &str {
        &self.original_name
    }

    /// Remove only entry still bound to opened receipt inode. A substituted
    /// pathname remains queued rather than being deleted by completed cleanup.
    pub fn remove_if_unchanged(&self) -> Result<(), String> {
        #[cfg(unix)]
        {
            use std::os::unix::io::AsRawFd;
            let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
            if unsafe {
                libc::fstatat(
                    self.dir.as_raw_fd(),
                    self.name.as_ptr(),
                    stat.as_mut_ptr(),
                    libc::AT_SYMLINK_NOFOLLOW,
                )
            } != 0
            {
                return Err(format!(
                    "revalidate claimed receipt: {}",
                    std::io::Error::last_os_error()
                ));
            }
            let stat = unsafe { stat.assume_init() };
            if stat.st_dev as u64 != self.dev || stat.st_ino != self.ino {
                return Err("claimed receipt was replaced; retained for recovery".into());
            }
            if unsafe { libc::unlinkat(self.dir.as_raw_fd(), self.name.as_ptr(), 0) } != 0 {
                return Err(format!(
                    "remove claimed receipt: {}",
                    std::io::Error::last_os_error()
                ));
            }
            self.dir
                .sync_all()
                .map_err(|e| format!("fsync receipt directory: {e}"))
        }
        #[cfg(not(unix))]
        {
            let _ = self;
            Err("safe claimed receipt removal is unavailable on this platform".into())
        }
    }
}

pub fn claim_receipt(path: &Path) -> Result<ClaimedReceipt, String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let parent = path.parent().ok_or("receipt has no parent")?;
        let entry_name = path.file_name().ok_or("receipt has no file name")?;
        let dir = open_private_dir(parent)?;
        let original_name = claimed_original_name(entry_name).unwrap_or(entry_name);
        let original_name = original_name
            .to_str()
            .ok_or("invalid receipt name")?
            .to_owned();
        let name = std::ffi::CString::new(entry_name.as_encoded_bytes())
            .map_err(|_| "invalid receipt name")?;
        let mut file = open_private_regular_at(&dir, entry_name)?;
        let meta = file
            .metadata()
            .map_err(|e| format!("stat claimed receipt: {e}"))?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|e| format!("read claimed receipt: {e}"))?;
        Ok(ClaimedReceipt {
            bytes,
            original_name,
            dir,
            name,
            dev: meta.dev(),
            ino: meta.ino(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err("safe receipt claims are unavailable on this platform".into())
    }
}

/// Includes crash-recoverable claim entries as well as public receipts.
pub fn is_receipt_entry(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let Some(name) = path.file_name() else {
            return false;
        };
        name.as_bytes().ends_with(b".json") || claimed_original_name(name).is_some()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

#[cfg(unix)]
fn claimed_original_name(name: &std::ffi::OsStr) -> Option<&std::ffi::OsStr> {
    use std::os::unix::ffi::OsStrExt;
    let bytes = name.as_bytes();
    const PREFIX: &[u8] = b".claim-";
    const TOKEN_END: usize = PREFIX.len() + 64;
    if bytes.len() <= TOKEN_END + 1
        || !bytes.starts_with(PREFIX)
        || bytes.get(TOKEN_END) != Some(&b'-')
        || !valid_repo_id(std::str::from_utf8(&bytes[PREFIX.len()..TOKEN_END]).ok()?)
    {
        return None;
    }
    Some(std::ffi::OsStr::from_bytes(&bytes[TOKEN_END + 1..]))
}

pub fn ensure_private_dir(dir: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::{ffi::OsStrExt, io::AsRawFd};
        let parent = dir.parent().ok_or("receipt directory has no parent")?;
        let name = dir.file_name().ok_or("receipt directory has no name")?;
        let parent_file = open_directory_no_follow(parent)?;
        let name = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| "invalid receipt directory name")?;
        // Create one leaf through an already-open parent. Recursive creation
        // can traverse a substituted symlink before a later check sees it.
        if unsafe { libc::mkdirat(parent_file.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(format!("create receipt directory: {error}"));
            }
        }
        let dir_file = open_owned_dir_at(&parent_file, &name)?;
        if unsafe { libc::fchmod(dir_file.as_raw_fd(), 0o700) } != 0 {
            return Err(format!(
                "restrict receipt directory: {}",
                std::io::Error::last_os_error()
            ));
        }
        check_private_dir(&dir_file)?;
    }
    #[cfg(unix)]
    return Ok(());
    #[cfg(not(unix))]
    {
        let _ = dir;
        // std exposes no directory-handle API that can create, verify ACLs,
        // and reject reparse points without a pathname re-open race.
        Err("safe private receipt directories are unavailable on this platform".into())
    }
}

pub fn sync_dir(dir: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        open_private_dir(dir)?
            .sync_all()
            .map_err(|e| format!("fsync receipt directory: {e}"))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Err("safe receipt directory synchronization is unavailable on this platform".into())
    }
}

/// Temp receipt stays unreachable. fsync bytes, atomically link into final name,
/// then fsync parent. Existing final name is never replaced.
pub(crate) fn atomic_create_immutable(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::io::FromRawFd;
        let dir_file = open_private_dir(dir)?;
        let fd = std::os::unix::io::AsRawFd::as_raw_fd(&dir_file);
        let temp = format!(".{name}.tmp-{}", random_invocation_id()?);
        let temp_c = std::ffi::CString::new(temp.as_bytes())
            .map_err(|_| "invalid temporary receipt name")?;
        let name_c = std::ffi::CString::new(name).map_err(|_| "invalid receipt name")?;
        let raw = unsafe {
            libc::openat(
                fd,
                temp_c.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if raw < 0 {
            return Err(format!(
                "create temporary receipt: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut file = unsafe { std::fs::File::from_raw_fd(raw) };
        let write = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = write {
            unsafe { libc::unlinkat(fd, temp_c.as_ptr(), 0) };
            return Err(format!("write temporary receipt: {error}"));
        }
        let linked = unsafe { libc::linkat(fd, temp_c.as_ptr(), fd, name_c.as_ptr(), 0) };
        unsafe { libc::unlinkat(fd, temp_c.as_ptr(), 0) };
        if linked != 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                return Err("already exists".into());
            }
            return Err(format!("publish receipt: {error}"));
        }
        dir_file
            .sync_all()
            .map_err(|e| format!("fsync receipt directory: {e}"))
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, name, bytes);
        Err("safe immutable receipt publication is unavailable on this platform".into())
    }
}

/// Atomically replace a private runtime diagnostic through same-directory,
/// descriptor-relative operations. Readers see old complete bytes or new bytes.
pub(crate) fn atomic_replace_private(dir: &Path, name: &str, bytes: &[u8]) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::io::{AsRawFd, FromRawFd};
        let dir_file = open_private_dir(dir)?;
        let fd = dir_file.as_raw_fd();
        let temp = format!(".{name}.tmp-{}", random_invocation_id()?);
        let temp_c = std::ffi::CString::new(temp.as_bytes())
            .map_err(|_| "invalid temporary diagnostic name")?;
        let name_c = std::ffi::CString::new(name).map_err(|_| "invalid diagnostic name")?;
        let raw = unsafe {
            libc::openat(
                fd,
                temp_c.as_ptr(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW,
                0o600,
            )
        };
        if raw < 0 {
            return Err(format!(
                "create temporary diagnostic: {}",
                std::io::Error::last_os_error()
            ));
        }
        let mut file = unsafe { std::fs::File::from_raw_fd(raw) };
        let write = file.write_all(bytes).and_then(|()| file.sync_all());
        drop(file);
        if let Err(error) = write {
            unsafe { libc::unlinkat(fd, temp_c.as_ptr(), 0) };
            return Err(format!("write temporary diagnostic: {error}"));
        }
        let renamed = unsafe { libc::renameat(fd, temp_c.as_ptr(), fd, name_c.as_ptr()) };
        if renamed != 0 {
            let error = std::io::Error::last_os_error();
            unsafe { libc::unlinkat(fd, temp_c.as_ptr(), 0) };
            return Err(format!("publish diagnostic: {error}"));
        }
        dir_file
            .sync_all()
            .map_err(|e| format!("fsync receipt directory: {e}"))
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, name, bytes);
        Err("safe private diagnostic replacement is unavailable on this platform".into())
    }
}

#[cfg(unix)]
pub(crate) fn open_private_dir(path: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::{ffi::OsStrExt, io::FromRawFd};
    let name = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "invalid receipt directory path")?;
    let raw = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if raw < 0 {
        return Err(format!(
            "open receipt directory safely: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { std::fs::File::from_raw_fd(raw) };
    check_private_dir(&file)?;
    Ok(file)
}

#[cfg(unix)]
fn open_directory_no_follow(path: &Path) -> Result<std::fs::File, String> {
    use std::os::unix::{ffi::OsStrExt, io::FromRawFd};
    let name = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| "invalid receipt directory path")?;
    let raw = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if raw < 0 {
        return Err(format!(
            "open receipt directory safely: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { std::fs::File::from_raw_fd(raw) };
    let meta = file
        .metadata()
        .map_err(|e| format!("stat receipt directory: {e}"))?;
    if !meta.is_dir() {
        return Err("receipt directory type is unsafe".into());
    }
    Ok(file)
}

/// Open a final directory without following it, then establish restrictive
/// permissions on its descriptor. Existing owner-controlled directories may
/// legitimately be broader before this first receipt operation.
#[cfg(unix)]
fn open_owned_dir_at(
    parent: &std::fs::File,
    name: &std::ffi::CString,
) -> Result<std::fs::File, String> {
    use std::os::unix::{
        fs::MetadataExt,
        io::{AsRawFd, FromRawFd},
    };
    let raw = unsafe {
        libc::openat(
            parent.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW,
        )
    };
    if raw < 0 {
        return Err(format!(
            "open receipt directory safely: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { std::fs::File::from_raw_fd(raw) };
    let meta = file
        .metadata()
        .map_err(|e| format!("stat receipt directory: {e}"))?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } as u32 {
        return Err("receipt directory ownership or type is unsafe".into());
    }
    Ok(file)
}

#[cfg(unix)]
pub(crate) fn open_private_regular_at(
    dir: &std::fs::File,
    name: &std::ffi::OsStr,
) -> Result<std::fs::File, String> {
    use std::os::unix::{
        ffi::OsStrExt,
        io::{AsRawFd, FromRawFd},
    };
    let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| "invalid receipt name")?;
    let raw = unsafe {
        libc::openat(
            dir.as_raw_fd(),
            name.as_ptr(),
            libc::O_RDONLY | libc::O_NOFOLLOW,
        )
    };
    if raw < 0 {
        return Err(format!(
            "open receipt safely: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { std::fs::File::from_raw_fd(raw) };
    check_private_regular(&file)?;
    Ok(file)
}

#[cfg(unix)]
fn check_private_dir(file: &std::fs::File) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let meta = file
        .metadata()
        .map_err(|e| format!("stat receipt directory: {e}"))?;
    if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } as u32 || meta.mode() & 0o077 != 0
    {
        return Err("receipt directory ownership or mode is unsafe".into());
    }
    Ok(())
}

#[cfg(unix)]
fn check_private_regular(file: &std::fs::File) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let meta = file.metadata().map_err(|e| format!("stat receipt: {e}"))?;
    if !meta.is_file()
        || meta.uid() != unsafe { libc::geteuid() } as u32
        || meta.mode() & 0o077 != 0
    {
        return Err("receipt ownership, type, or mode is unsafe".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_identity_format_is_exact_lowercase_hex() {
        assert!(valid_repo_id(&"a".repeat(64)));
        assert!(!valid_repo_id(&"A".repeat(64)));
        assert!(!valid_repo_id(&"g".repeat(64)));
        assert!(!valid_repo_id(&"a".repeat(63)));
    }

    #[test]
    fn invalid_identity_rejects_before_platform_storage() {
        let receipt = ReceiptV1 {
            version: 1,
            repo_id: "not-a-repository-id".into(),
            invocation_id: "also-invalid".into(),
            receipt_id: "invalid".into(),
            kind: "ref.update".into(),
            principal: "orchestrator:hugit-hook".into(),
            payload: serde_json::Value::Null,
            recorded_at: 0,
        };
        assert_eq!(
            write_receipt(Path::new("not-created"), &receipt).unwrap_err(),
            "invalid receipt identity"
        );
    }

    #[cfg(unix)]
    #[test]
    fn immutable_publish_never_overwrites_first_complete_bytes() {
        let dir = std::env::temp_dir().join(format!(
            "hugit-receipt-publish-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        ensure_private_dir(&dir).unwrap();
        let name = "receipt.json";
        atomic_create_immutable(&dir, name, b"first").unwrap();
        assert_eq!(
            atomic_create_immutable(&dir, name, b"second").unwrap_err(),
            "already exists"
        );
        assert_eq!(read_receipt_bytes(&dir.join(name)).unwrap(), b"first");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
