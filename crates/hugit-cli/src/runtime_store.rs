//! Shared, untracked runtime state for one Git repository.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::porcelain::PorcelainError;
use crate::pr::filelock::{FileLock, atomic_write_unprepared};
use hugit_refstore::authz::{Endpoint, PrincipalClass};

pub const RUNTIME_DIR: &str = "hugit";
pub const CANONICAL_LOG: &str = "event-log.json";
pub const LEGACY_LOG: &str = "legacy-log.json";
pub const RUNTIME_METADATA: &str = "runtime.json";
pub const RECEIPTS_DIR: &str = "receipts";
pub const COMPLETED_RECEIPTS_DIR: &str = "completed-receipts";
pub const DEAD_LETTER_DIR: &str = "dead-letter";
pub const STATUS: &str = "status.json";
/// Versioned ownership evidence for explicitly adopted hook dispatchers.
pub const HOOK_MANIFEST: &str = "hook-manifest.json";
/// Immutable copies of foreign hooks retained only after explicit adoption.
pub const HOOK_BACKUPS_DIR: &str = "hook-backups";
pub const LEGACY_PREFIX_KIND: &str = "runtime.legacy_prefix";

#[derive(Clone, Debug)]
pub struct RuntimeStore {
    pub root: PathBuf,
}

impl RuntimeStore {
    pub fn canonical_log(&self) -> PathBuf {
        self.root.join(CANONICAL_LOG)
    }

    pub fn legacy_log(&self) -> PathBuf {
        self.root.join(LEGACY_LOG)
    }

    pub fn metadata(&self) -> PathBuf {
        self.root.join(RUNTIME_METADATA)
    }

    pub fn receipts(&self) -> PathBuf {
        self.root.join(RECEIPTS_DIR)
    }
    pub fn dead_letters(&self) -> PathBuf {
        self.root.join(DEAD_LETTER_DIR)
    }
    pub fn status(&self) -> PathBuf {
        self.root.join(STATUS)
    }
    pub fn hook_manifest(&self) -> PathBuf {
        self.root.join(HOOK_MANIFEST)
    }
    pub fn hook_backups(&self) -> PathBuf {
        self.root.join(HOOK_BACKUPS_DIR)
    }
}

/// Stable local repository identity for receipt IDs. This is runtime metadata,
/// never working-tree state, and is created once under an exclusive lock.
pub fn repository_id(log_path: &Path) -> Result<String, String> {
    prepare_runtime_log(log_path).map_err(|e| e.to_json())?;
    let root = log_path.parent().ok_or("runtime log has no parent")?;
    let metadata = root.join(RUNTIME_METADATA);
    // Concurrent detached hooks may initialize/read receipt identity together.
    // Wait through bounded metadata handoff so neither drops a durable receipt.
    let mut last = String::new();
    let mut lock = None;
    for _ in 0..500 {
        match FileLock::acquire_unprepared(&metadata) {
            Ok(acquired) => {
                lock = Some(acquired);
                break;
            }
            Err(crate::pr::filelock::LockError::Busy { .. }) => {
                last = "runtime metadata is locked by another hugit verb".into();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Err(error) => {
                last = error.to_string();
                break;
            }
        }
    }
    let _lock = lock.ok_or_else(|| format!("runtime metadata busy: {last}"))?;
    let mut value: Value = if metadata.exists() {
        serde_json::from_slice(
            &std::fs::read(&metadata).map_err(|e| format!("read runtime metadata: {e}"))?,
        )
        .map_err(|e| format!("runtime metadata is invalid: {e}"))?
    } else {
        json!({"version": 1, "migration": {"legacy_source": Value::Null, "legacy_prefix_sha256": Value::Null}})
    };
    if let Some(id) = value.get("repository_id").and_then(Value::as_str) {
        if crate::capture::receipt::valid_repo_id(id) {
            return Ok(id.to_string());
        }
        return Err("runtime repository id has invalid format".into());
    }
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| format!("generate cryptographic repository id: {e}"))?;
    let id = hex::encode(bytes);
    value["repository_id"] = Value::String(id.clone());
    let encoded = serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?;
    atomic_write_unprepared(&metadata, &encoded).map_err(|e| e.to_string())?;
    Ok(id)
}

/// Runtime state belongs to Git's common directory, shared by all worktrees and
/// outside every tracked worktree.
pub fn for_repo(repo: &Path) -> Result<RuntimeStore, PorcelainError> {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .map_err(|e| PorcelainError::io("resolve Git common directory", repo, &e))?;
    if !output.status.success() {
        return Err(PorcelainError::new(
            "not_a_git_repo",
            format!("{} is not a Git repository", repo.display()),
            "run this command inside a Git repository",
        ));
    }
    let common = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if common.is_empty() {
        return Err(PorcelainError::new(
            "git_common_dir_invalid",
            "Git returned no common directory",
            "use a non-bare Git working tree",
        ));
    }
    Ok(RuntimeStore {
        root: PathBuf::from(common).join(RUNTIME_DIR),
    })
}

pub fn legacy_path(repo: &Path) -> PathBuf {
    repo.join(".hugit/log.json")
}

fn absolute_normalized(path: &Path) -> PathBuf {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(path)
    };
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other.as_os_str()),
        }
    }
    normalized
}

fn git_dir_arg(path: &Path) -> std::ffi::OsString {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;

        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        let without_verbatim =
            wide.strip_prefix(&['\\' as u16, '\\' as u16, '?' as u16, '\\' as u16]);
        return without_verbatim
            .map(std::ffi::OsString::from_wide)
            .unwrap_or_else(|| path.as_os_str().to_os_string());
    }
    #[cfg(not(windows))]
    path.as_os_str().to_os_string()
}

/// Resolve a canonical runtime log from its owning Git common directory, never
/// from the caller's CWD. A path shaped like runtime state but not owned by its
/// claimed common directory is rejected rather than treated as an ordinary log.
fn store_for_runtime_log(log_path: &Path) -> Result<Option<RuntimeStore>, PorcelainError> {
    let log = absolute_normalized(log_path);
    if log.file_name().and_then(|name| name.to_str()) != Some(CANONICAL_LOG)
        || log
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str())
            != Some(RUNTIME_DIR)
    {
        return Ok(None);
    }
    let Some(common_candidate) = log.parent().and_then(Path::parent) else {
        return Ok(None);
    };
    let common = std::fs::canonicalize(common_candidate).map_err(|e| {
        PorcelainError::io("resolve runtime log common directory", common_candidate, &e)
    })?;
    let output = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(git_dir_arg(&common))
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .map_err(|e| PorcelainError::io("verify runtime log common directory", &common, &e))?;
    if !output.status.success() {
        return Err(PorcelainError::new(
            "runtime_log_ambiguous",
            format!("{} is not owned by a Git common directory", log.display()),
            "use the repository's canonical Git common-dir runtime log",
        ));
    }
    let reported = absolute_normalized(Path::new(String::from_utf8_lossy(&output.stdout).trim()));
    let reported = std::fs::canonicalize(&reported)
        .map_err(|e| PorcelainError::io("resolve reported Git common directory", &reported, &e))?;
    if reported != common {
        return Err(PorcelainError::new(
            "runtime_log_ambiguous",
            format!(
                "{} does not match its owning Git common directory",
                log.display()
            ),
            "use the repository's canonical Git common-dir runtime log",
        ));
    }
    Ok(Some(RuntimeStore {
        root: common.join(RUNTIME_DIR),
    }))
}

fn legacy_path_for_store(store: &RuntimeStore) -> Result<PathBuf, PorcelainError> {
    let common = store
        .root
        .parent()
        .expect("runtime root has common-dir parent");
    let output = std::process::Command::new("git")
        .arg("--git-dir")
        .arg(git_dir_arg(common))
        .args(["worktree", "list", "--porcelain"])
        .output()
        .map_err(|e| PorcelainError::io("resolve runtime legacy worktree", common, &e))?;
    let worktree = String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("worktree "))
        .map(PathBuf::from)
        .ok_or_else(|| {
            PorcelainError::new(
                "runtime_log_ambiguous",
                "Git returned no worktree for runtime log migration",
                "run from a non-bare repository worktree",
            )
        })?;
    Ok(legacy_path(&worktree))
}

/// Prepare a runtime log before any read-modify-write path can load it.
///
/// This recognizes canonical runtime logs from their owning Git common directory.
/// An explicit `--log` cannot skip legacy migration through CWD or path aliases;
/// unrelated file logs retain their normal file-seam behavior.
pub fn prepare_runtime_log(log_path: &Path) -> Result<(), PorcelainError> {
    let Some(store) = store_for_runtime_log(log_path)? else {
        return Ok(());
    };
    {
        let legacy = legacy_path_for_store(&store)?;
        if store.metadata().exists() {
            let metadata: Value =
                serde_json::from_slice(&std::fs::read(store.metadata()).map_err(|e| {
                    PorcelainError::io("read runtime metadata", &store.metadata(), &e)
                })?)
                .map_err(|e| {
                    PorcelainError::new(
                        "migration_blocked",
                        format!("runtime metadata is invalid: {e}"),
                        "repair runtime.json before appending",
                    )
                })?;
            if legacy.exists()
                && metadata
                    .pointer("/migration/legacy_prefix_sha256")
                    .and_then(Value::as_str)
                    != Some(&sha256_hex(&std::fs::read(&legacy).map_err(|e| {
                        PorcelainError::io("read legacy log", &legacy, &e)
                    })?))
            {
                return Err(PorcelainError::new(
                    "migration_blocked",
                    "runtime metadata differs from legacy source",
                    "preserve evidence and repair runtime migration before appending",
                ));
            }
            return Ok(());
        }
        // Fresh template-installed hooks have no runtime yet. Bootstrap through
        // same migration gate so source validation always precedes every write.
        migrate(&store, &legacy)?;
    }
    Ok(())
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Verify legacy chain before creating any runtime file or installing hooks.
/// The prefix record binds every canonical append path, including porcelain
/// commands that do not pass through hook capture.
pub fn migrate(store: &RuntimeStore, legacy: &Path) -> Result<Value, PorcelainError> {
    let source_exists = legacy.exists();
    let source_bytes = if source_exists {
        let bytes =
            std::fs::read(legacy).map_err(|e| PorcelainError::io("read legacy log", legacy, &e))?;
        crate::checks::load_event_log_from_bytes(&bytes, legacy).map_err(|_| {
            PorcelainError::new(
                "migration_blocked",
                "legacy log chain is invalid",
                "preserve legacy evidence and repair its chain before retrying",
            )
        })?;
        Some(bytes)
    } else {
        None
    };

    if store.root.exists() && !store.root.is_dir() {
        return Err(PorcelainError::new(
            "runtime_store_invalid",
            format!("runtime root {} is not a directory", store.root.display()),
            "move the conflicting path, then re-run `hugit attach`",
        ));
    }
    let copied = store.legacy_log();
    let snapshot = match (source_bytes.as_deref(), copied.exists()) {
        (Some(source), true) => {
            let existing = std::fs::read(&copied)
                .map_err(|e| PorcelainError::io("read immutable legacy copy", &copied, &e))?;
            if existing != source {
                return Err(PorcelainError::new(
                    "migration_blocked",
                    "immutable legacy-log.json differs from source legacy log",
                    "preserve both files and resolve migration state before retrying",
                ));
            }
            Some(existing)
        }
        (Some(source), false) => Some(source.to_vec()),
        (None, true) => Some(
            std::fs::read(&copied)
                .map_err(|e| PorcelainError::io("read immutable legacy copy", &copied, &e))?,
        ),
        (None, false) => None,
    };
    if let Some(bytes) = snapshot.as_deref() {
        crate::checks::load_event_log_from_bytes(bytes, &copied).map_err(|_| {
            PorcelainError::new(
                "migration_blocked",
                "legacy snapshot chain is invalid",
                "preserve legacy evidence and repair its chain before retrying",
            )
        })?;
    }
    let digest = snapshot.as_deref().map(sha256_hex);
    let canonical = store.canonical_log();
    let _lock = FileLock::acquire_unprepared(&canonical).map_err(|e| {
        PorcelainError::new(
            "migration_blocked",
            e.to_string(),
            "retry after active hugit writer exits",
        )
    })?;
    if !copied.exists()
        && let Some(bytes) = snapshot.as_deref()
    {
        atomic_write_unprepared(&copied, bytes).map_err(|e| {
            PorcelainError::new(
                "runtime_write_failed",
                e.to_string(),
                "retry `hugit attach`",
            )
        })?;
    }
    let mut canonical_log = if canonical.exists() {
        let bytes = std::fs::read(&canonical)
            .map_err(|e| PorcelainError::io("read canonical runtime log", &canonical, &e))?;
        crate::checks::load_event_log_from_bytes(&bytes, &canonical)?
    } else {
        hugit_refstore::EventLog::new()
    };
    if let Some(digest) = digest.as_deref() {
        if canonical_log.records().is_empty() {
            canonical_log
                .append_authorized(
                    PrincipalClass::Orchestrator,
                    Endpoint::Push,
                    LEGACY_PREFIX_KIND,
                    vec!["orchestrator:hugit-runtime".to_string()],
                    serde_json::to_string(&json!({"legacy_prefix_sha256": digest})).unwrap(),
                    0,
                )
                .map_err(|denied| {
                    PorcelainError::new(
                        "migration_blocked",
                        format!("prefix authorization denied: {:?}", denied.reason),
                        "repair runtime state before retrying",
                    )
                })?;
        } else {
            let first = &canonical_log.records()[0];
            let bound = first.kind == LEGACY_PREFIX_KIND
                && serde_json::from_str::<Value>(&first.payload)
                    .ok()
                    .and_then(|p| {
                        p.get("legacy_prefix_sha256")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .as_deref()
                    == Some(digest);
            if !bound {
                return Err(PorcelainError::new(
                    "migration_blocked",
                    "canonical log lacks immutable legacy-prefix binding",
                    "preserve evidence and repair incomplete migration before retrying",
                ));
            }
        }
    }
    let canonical_bytes = serde_json::to_vec_pretty(canonical_log.records()).map_err(|e| {
        PorcelainError::new(
            "runtime_write_failed",
            e.to_string(),
            "retry `hugit attach`",
        )
    })?;
    if !canonical.exists() || !canonical_log.records().is_empty() {
        atomic_write_unprepared(&canonical, &canonical_bytes).map_err(|e| {
            PorcelainError::new(
                "runtime_write_failed",
                e.to_string(),
                "retry `hugit attach`",
            )
        })?;
    }

    let metadata_path = store.metadata();
    // A crash after the immutable snapshot or canonical prefix but before
    // metadata leaves enough evidence to resume. Preserve an already-recorded
    // source path when source worktree disappeared between attempts.
    let legacy_source = if source_exists {
        Value::String(legacy.display().to_string())
    } else if metadata_path.exists() {
        serde_json::from_slice::<Value>(
            &std::fs::read(&metadata_path)
                .map_err(|e| PorcelainError::io("read runtime metadata", &metadata_path, &e))?,
        )
        .map_err(|e| {
            PorcelainError::new(
                "runtime_metadata_invalid",
                e.to_string(),
                "repair runtime.json before retrying",
            )
        })?
        .pointer("/migration/legacy_source")
        .cloned()
        .unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let mut metadata = json!({
        "version": 1,
        "migration": {
            "legacy_source": legacy_source,
            "legacy_prefix_sha256": digest,
        }
    });
    // Receipt identity is runtime metadata, not migration evidence. Preserve an
    // already-durable repository UUID when migration recovery replays.
    if metadata_path.exists() {
        let existing: Value = serde_json::from_slice(
            &std::fs::read(&metadata_path)
                .map_err(|e| PorcelainError::io("read runtime metadata", &metadata_path, &e))?,
        )
        .map_err(|e| {
            PorcelainError::new(
                "runtime_metadata_invalid",
                e.to_string(),
                "repair runtime.json before retrying",
            )
        })?;
        if let Some(repository_id) = existing.get("repository_id") {
            if !repository_id
                .as_str()
                .is_some_and(crate::capture::receipt::valid_repo_id)
            {
                return Err(PorcelainError::new(
                    "runtime_metadata_invalid",
                    "runtime repository_id has invalid format",
                    "repair runtime.json before retrying",
                ));
            }
            metadata["repository_id"] = repository_id.clone();
        }
    }
    let metadata_bytes = serde_json::to_vec_pretty(&metadata).map_err(|e| {
        PorcelainError::new(
            "runtime_metadata_invalid",
            e.to_string(),
            "retry `hugit attach`",
        )
    })?;
    if metadata_path.exists() {
        let existing: Value = serde_json::from_slice(
            &std::fs::read(&metadata_path)
                .map_err(|e| PorcelainError::io("read runtime metadata", &metadata_path, &e))?,
        )
        .map_err(|e| {
            PorcelainError::new(
                "runtime_metadata_invalid",
                e.to_string(),
                "repair runtime.json before retrying",
            )
        })?;
        if existing != metadata {
            return Err(PorcelainError::new(
                "migration_blocked",
                "runtime.json migration metadata differs from current legacy source",
                "preserve evidence and resolve migration state before retrying",
            ));
        }
    } else {
        atomic_write_unprepared(&metadata_path, &metadata_bytes).map_err(|e| {
            PorcelainError::new(
                "runtime_write_failed",
                e.to_string(),
                "retry `hugit attach`",
            )
        })?;
    }
    Ok(metadata)
}

/// Bind a runtime legacy snapshot before any caller appends to `log`.
/// Caller holds `log`'s FileLock across this check, its append, and persist.
/// Non-runtime logs have neither runtime metadata nor snapshot and pass through.
pub fn bind_legacy_prefix(
    log_path: &Path,
    log: &mut hugit_refstore::EventLog,
) -> Result<(), String> {
    let Some(root) = log_path.parent() else {
        return Ok(());
    };
    let snapshot_path = root.join(LEGACY_LOG);
    let metadata_path = root.join(RUNTIME_METADATA);
    if !snapshot_path.exists() && !metadata_path.exists() {
        return Ok(());
    }
    let recovered_digest = if metadata_path.exists() {
        None
    } else {
        Some(sha256_hex(&std::fs::read(&snapshot_path).map_err(|e| {
            format!("read immutable legacy snapshot: {e}")
        })?))
    };
    let metadata = if metadata_path.exists() {
        serde_json::from_slice::<Value>(
            &std::fs::read(&metadata_path).map_err(|e| format!("read runtime metadata: {e}"))?,
        )
        .map_err(|e| format!("runtime metadata is invalid: {e}"))?
    } else {
        // Interrupted migration after snapshot creation: recover metadata before
        // canonical append. Source path is unavailable here, snapshot is evidence.
        json!({"version": 1, "migration": {
            "legacy_source": Value::Null,
            "legacy_prefix_sha256": recovered_digest.expect("snapshot was read above"),
        }})
    };
    // Fresh runtime state has metadata but no legacy snapshot. It needs no
    // prefix; only a non-null migration digest makes prefix binding mandatory.
    let expected_digest = metadata
        .pointer("/migration/legacy_prefix_sha256")
        .and_then(Value::as_str)
        .map(str::to_string);
    if expected_digest.is_none() {
        return Ok(());
    }
    let snapshot = std::fs::read(&snapshot_path)
        .map_err(|e| format!("read immutable legacy snapshot: {e}"))?;
    crate::checks::load_event_log_from_bytes(&snapshot, &snapshot_path)
        .map_err(|_| "legacy snapshot chain is invalid".to_string())?;
    let digest = sha256_hex(&snapshot);
    if metadata
        .pointer("/migration/legacy_prefix_sha256")
        .and_then(Value::as_str)
        != Some(&digest)
    {
        return Err("runtime metadata does not bind immutable legacy snapshot".to_string());
    }
    if !metadata_path.exists() {
        let bytes = serde_json::to_vec_pretty(&metadata)
            .map_err(|e| format!("serialize recovered runtime metadata: {e}"))?;
        atomic_write_unprepared(&metadata_path, &bytes)
            .map_err(|e| format!("persist recovered runtime metadata: {e}"))?;
    }
    if log.records().is_empty() {
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Push,
            LEGACY_PREFIX_KIND,
            vec!["orchestrator:hugit-runtime".to_string()],
            serde_json::to_string(&json!({"legacy_prefix_sha256": digest}))
                .map_err(|e| format!("serialize legacy prefix: {e}"))?,
            0,
        )
        .map_err(|denied| format!("prefix authorization denied: {:?}", denied.reason))?;
    } else {
        let first = &log.records()[0];
        let bound = first.kind == LEGACY_PREFIX_KIND
            && serde_json::from_str::<Value>(&first.payload)
                .ok()
                .and_then(|p| {
                    p.get("legacy_prefix_sha256")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .as_deref()
                == Some(&digest);
        if !bound {
            return Err("canonical log lacks immutable legacy-prefix binding".to_string());
        }
    }
    Ok(())
}

/// Central persist gate for canonical runtime logs. Callers already hold the
/// matching FileLock; non-runtime explicit paths retain normal file semantics.
pub fn prepare_runtime_write(log_path: &Path, bytes: &[u8]) -> Result<Vec<u8>, PorcelainError> {
    let Some(_) = store_for_runtime_log(log_path)? else {
        return Ok(bytes.to_vec());
    };

    // FileLock::acquire prepared migration before mkdir/lock creation. Re-run
    // recovery here so a direct atomic writer cannot persist past an interrupted
    // metadata step, then bind prefix into exact bytes being committed.
    prepare_runtime_log(log_path)?;
    let mut log = crate::checks::load_event_log_from_bytes(bytes, log_path)?;
    bind_legacy_prefix(log_path, &mut log).map_err(|e| {
        PorcelainError::new(
            "migration_blocked",
            e,
            "repair runtime migration before appending",
        )
    })?;
    serde_json::to_vec_pretty(log.records()).map_err(|e| {
        PorcelainError::new(
            "runtime_write_failed",
            e.to_string(),
            "retry runtime append",
        )
    })
}
