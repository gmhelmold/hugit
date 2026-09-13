//! Deterministic receipt drain. Source remains until canonical append, or until
//! a durable dead-letter copy records why it cannot be appended.

use super::receipt::{ClaimedReceipt, claim_receipt, ensure_private_dir, read_receipt_bytes};
use crate::{
    checks::load_event_log,
    pr::filelock::{FileLock, atomic_write},
    runtime_store,
};
use hugit_refstore::authz::{Endpoint, PrincipalClass};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Bound recovery work per drain. A large completed-marker directory must not
/// turn one detached hook into an unbounded filesystem sweep.
const COMPLETION_CLEANUP_LIMIT: usize = 64;

#[derive(Debug, Deserialize, Serialize)]
struct CompletionMarkerV1 {
    version: u8,
    receipt_id: String,
    receipt_sha256: String,
}

#[derive(Default, Debug, Clone)]
pub struct DrainStatus {
    pub drained: usize,
    pub pending: usize,
    pub dead_letters: usize,
    pub last_error: Option<DrainFailure>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DrainFailure {
    pub code: String,
    pub message: String,
}

#[derive(Debug)]
enum DrainError {
    Invalid(String),
    Retry(String),
}

/// Typed forensic record for rejected receipt. Source bytes never persist here.
#[derive(Serialize)]
struct DeadLetterV1 {
    version: u8,
    receipt_sha256: String,
    reason: String,
    receipt: DeadLetterReceiptMetadataV1,
}

#[derive(Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum DeadLetterReceiptMetadataV1 {
    Parsed {
        version: u8,
        repo_id: String,
        invocation_id: String,
        receipt_id: String,
        kind: String,
        principal: String,
        payload: serde_json::Value,
        recorded_at: u64,
    },
    InvalidJson {
        byte_len: usize,
        parse_error: String,
    },
}

pub fn spawn_worker(log_path: &Path, hook_log: Option<&Path>) -> Result<(), String> {
    let exe =
        std::env::current_exe().map_err(|e| format!("resolve receipt worker executable: {e}"))?;
    let top_level = log_path.parent().ok_or("runtime log has no parent")?;
    let mut command = Command::new(exe);
    command
        .args(["capture", "--kind", "worker", "--top-level"])
        .arg(top_level)
        .args(["--log"])
        .arg(log_path)
        .arg("--drain-worker")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Some(hook_log) = hook_log {
        command.arg("--hook-log").arg(hook_log);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("spawn detached receipt worker: {e}"))
}

pub fn drain(log_path: &Path, limit: usize) -> Result<DrainStatus, String> {
    runtime_store::prepare_runtime_log(log_path).map_err(|e| e.to_json())?;
    let root = log_path.parent().ok_or("runtime log has no parent")?;
    let receipts = root.join(runtime_store::RECEIPTS_DIR);
    let completed = root.join(runtime_store::COMPLETED_RECEIPTS_DIR);
    let dead = root.join(runtime_store::DEAD_LETTER_DIR);
    ensure_private_dir(root)?;
    ensure_private_dir(&receipts)?;
    ensure_private_dir(&completed)?;
    ensure_private_dir(&dead)?;
    let lock = match acquire_drain_lock(log_path) {
        Ok(lock) => lock,
        // Another hook's worker owns canonical projection. Source receipt remains
        // queued for next hook or health recovery attempt.
        Err(error) => {
            return write_current_status(
                &receipts,
                &dead,
                &completed,
                root,
                DrainStatus {
                    last_error: Some(DrainFailure {
                        code: "log_busy".into(),
                        message: error.to_string(),
                    }),
                    ..DrainStatus::default()
                },
            );
        }
    };
    cleanup_completed(&receipts, &completed)?;
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&receipts)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| super::receipt::is_receipt_entry(p))
        .filter(|p| !is_completed(&completed, p))
        .collect();
    paths.sort();
    let expected_repo_id = runtime_store::repository_id(log_path)?;
    let mut status = DrainStatus::default();
    for path in paths.into_iter().take(limit) {
        let claim = match claim_receipt(&path) {
            Ok(claim) => claim,
            Err(error) => {
                status.last_error = Some(DrainFailure {
                    code: "receipt_claim_failed".into(),
                    message: error,
                });
                continue;
            }
        };
        match drain_one(log_path, &claim, &expected_repo_id) {
            Ok(()) => match mark_completed(&claim, &completed) {
                Ok(()) => status.drained += 1,
                Err(error) => {
                    status.last_error = Some(DrainFailure {
                        code: "drained_receipt_retained".into(),
                        message: error,
                    });
                }
            },
            Err(DrainError::Invalid(reason)) => {
                if let Err(error) = dead_letter(&claim, &dead, &reason) {
                    status.last_error = Some(DrainFailure {
                        code: "invalid_receipt_retained".into(),
                        message: error,
                    });
                } else if let Err(error) = claim.remove_if_unchanged() {
                    status.last_error = Some(DrainFailure {
                        code: "dead_lettered_receipt_retained".into(),
                        message: error,
                    });
                }
            }
            Err(DrainError::Retry(reason)) => {
                // Do not convert I/O, lock, or corrupt-log outages into data loss.
                status.last_error = Some(DrainFailure {
                    code: "retryable_drain_failure".into(),
                    message: reason,
                });
            }
        }
    }
    // Completed source receipts may already be cleaned up when a prior worker
    // crashes before this derived projection. Reconcile canonical evidence even
    // with an empty queue or zero drain limit.
    reconcile_pairing_projection(log_path)?;
    // Canonical durability and source removal are complete. Status is only a
    // projection, so never hold canonical writer lock across its I/O.
    cleanup_completed(&receipts, &completed)?;
    drop(lock);
    write_current_status(&receipts, &dead, &completed, root, status)
}

fn acquire_drain_lock(log_path: &Path) -> Result<FileLock, String> {
    // One worker owns projection. Completion cleanup fsyncs both directories,
    // so detached workers wait through a bounded 5 s handoff instead of leaving
    // a receipt stranded after a near-simultaneous hook.
    let mut last = String::new();
    for _ in 0..500 {
        match FileLock::acquire(log_path) {
            Ok(lock) => return Ok(lock),
            Err(error) => {
                last = error.to_string();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }
    Err(format!("canonical log busy: {last}"))
}

fn drain_one(
    log_path: &Path,
    claim: &ClaimedReceipt,
    expected_repo_id: &str,
) -> Result<(), DrainError> {
    let receipt = claim.receipt().map_err(DrainError::Invalid)?;
    if receipt.version != 1
        || receipt.repo_id != expected_repo_id
        || !super::receipt::valid_repo_id(&receipt.repo_id)
        || !super::receipt::valid_repo_id(&receipt.invocation_id)
        || receipt.receipt_id
            != super::receipt::receipt_id(&receipt.repo_id, &receipt.invocation_id)
        || !claim.has_original_name(&format!("{}.json", receipt.receipt_id))
    {
        return Err(DrainError::Invalid("invalid receipt identity".into()));
    }
    let mut log = load_event_log(log_path).map_err(|e| DrainError::Retry(e.to_json()))?;
    runtime_store::bind_legacy_prefix(log_path, &mut log).map_err(DrainError::Retry)?;
    let duplicate = log.records().iter().any(|record| {
        serde_json::from_str::<serde_json::Value>(&record.payload)
            .ok()
            .and_then(|v| {
                v.get("receipt_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            })
            == Some(receipt.receipt_id.clone())
    });
    if !duplicate {
        let mut payload = receipt.payload.clone();
        crate::porcelain::scrub_payload(&mut payload);
        for key in [
            "target",
            "from",
            "to",
            "old",
            "new",
            "ref",
            "branch",
            "shas",
            "merged_from",
        ] {
            if let Some(value) = receipt.payload.get(key).and_then(serde_json::Value::as_str) {
                payload[key] = json!(crate::porcelain::structural_secret_scrub(value));
            }
        }
        // Receipt identity is local-generated and must survive payload scrub for
        // canonical dedupe. Raw receipt payload never controls this field.
        payload["receipt_id"] = json!(receipt.receipt_id);
        let text = hugit_refstore::canonical_json(&payload.to_string())
            .unwrap_or_else(|| payload.to_string());
        let principal = crate::porcelain::structural_secret_scrub(&receipt.principal);
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Push,
            receipt.kind,
            vec![principal],
            text,
            receipt.recorded_at,
        )
        .map_err(|e| DrainError::Invalid(format!("authz denied: {:?}", e.reason)))?;
        let bytes = serde_json::to_vec_pretty(log.records())
            .map_err(|e| DrainError::Retry(e.to_string()))?;
        atomic_write(log_path, &bytes).map_err(|e| DrainError::Retry(e.to_string()))?;
    }
    Ok(())
}

fn reconcile_pairing_projection(log_path: &Path) -> Result<(), String> {
    let mut log = load_event_log(log_path).map_err(|error| error.to_json())?;
    runtime_store::bind_legacy_prefix(log_path, &mut log)?;
    append_pairing_projection(log_path, &mut log)
}

/// Persist reducer output in canonical log. Source receipt IDs make every
/// pending/unpaired/paired/ambiguous label auditable and deterministic.
fn append_pairing_projection(
    log_path: &Path,
    log: &mut hugit_refstore::EventLog,
) -> Result<(), String> {
    let projections = reference_transaction_projections(log.records());
    for projection in projections {
        let payload = json!({"reference_transaction_pairing": projection});
        let text = hugit_refstore::canonical_json(&payload.to_string())
            .unwrap_or_else(|| payload.to_string());
        let exists = log.records().iter().any(|record| record.payload == text);
        if !exists {
            log.append_authorized(
                PrincipalClass::Orchestrator,
                Endpoint::Push,
                "ref.update".to_string(),
                vec!["orchestrator:hugit-hook".to_string()],
                text,
                0,
            )
            .map_err(|error| format!("append transaction pairing: {:?}", error.reason))?;
            let bytes = serde_json::to_vec_pretty(log.records())
                .map_err(|error| format!("serialize transaction pairing: {error}"))?;
            atomic_write(log_path, &bytes).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// Pair reference transactions from whole canonical log, never append order.
///
/// Receipts can drain terminal-first after a crash. Labeling either receipt at
/// append time would make result depend on that race and would guess among equal
/// concurrent fingerprints. Consumers must reduce full evidence instead.
pub fn reference_transaction_pairing(
    records: &[hugit_contracts::event_record::EventRecord],
    fingerprint: &str,
) -> &'static str {
    let mut prepared = 0usize;
    let mut terminal = 0usize;
    for record in records {
        let Ok(record_payload) = serde_json::from_str::<serde_json::Value>(&record.payload) else {
            continue;
        };
        let Some(prior) = record_payload.get("reference_transaction") else {
            continue;
        };
        if prior.get("fingerprint").and_then(serde_json::Value::as_str) != Some(fingerprint) {
            continue;
        }
        match prior.get("phase").and_then(serde_json::Value::as_str) {
            Some("prepared") => prepared += 1,
            Some("committed" | "aborted") => terminal += 1,
            _ => {}
        }
    }
    match (prepared, terminal) {
        (1, 1) => "paired",
        (0, 0) | (1, 0) => "pending",
        (0, 1) => "unpaired",
        _ => "ambiguous",
    }
}

pub fn reference_transaction_projections(
    records: &[hugit_contracts::event_record::EventRecord],
) -> Vec<serde_json::Value> {
    let mut fingerprints = std::collections::BTreeSet::new();
    for record in records {
        if let Ok(payload) = serde_json::from_str::<serde_json::Value>(&record.payload)
            && let Some(fingerprint) = payload["reference_transaction"]["fingerprint"].as_str()
        {
            fingerprints.insert(fingerprint.to_owned());
        }
    }
    fingerprints
        .into_iter()
        .map(|fingerprint| {
            let mut sources = Vec::new();
            for record in records {
                let Ok(payload) = serde_json::from_str::<serde_json::Value>(&record.payload) else {
                    continue;
                };
                if payload["reference_transaction"]["fingerprint"].as_str() == Some(&fingerprint)
                    && let Some(receipt_id) = payload["receipt_id"].as_str()
                {
                    sources.push(receipt_id.to_owned());
                }
            }
            sources.sort();
            json!({
                "fingerprint": fingerprint.clone(),
                "state": reference_transaction_pairing(records, &fingerprint),
                "source_receipt_ids": sources,
            })
        })
        .collect()
}

fn dead_letter(claim: &ClaimedReceipt, dead: &Path, reason: &str) -> Result<(), String> {
    let raw = claim.bytes();
    // Exact source hash preserves forensic binding without persisting potentially
    // secret-bearing bytes. Every parsed user field crosses capture scrub law.
    let receipt_sha256 = crate::runtime_store::sha256_hex(raw);
    let receipt = match claim.receipt() {
        Ok(receipt) => {
            let mut payload = receipt.payload;
            crate::porcelain::scrub_payload(&mut payload);
            crate::porcelain::structural_scrub_json(&mut payload);
            DeadLetterReceiptMetadataV1::Parsed {
                version: receipt.version,
                repo_id: crate::porcelain::structural_secret_scrub(&receipt.repo_id),
                invocation_id: crate::porcelain::structural_secret_scrub(&receipt.invocation_id),
                receipt_id: crate::porcelain::structural_secret_scrub(&receipt.receipt_id),
                kind: crate::porcelain::structural_secret_scrub(&receipt.kind),
                principal: crate::porcelain::structural_secret_scrub(&receipt.principal),
                payload,
                recorded_at: receipt.recorded_at,
            }
        }
        Err(error) => DeadLetterReceiptMetadataV1::InvalidJson {
            byte_len: raw.len(),
            parse_error: crate::redaction::scrub(&error),
        },
    };
    let letter = DeadLetterV1 {
        version: 1,
        receipt_sha256: receipt_sha256.clone(),
        reason: crate::redaction::scrub(reason),
        receipt,
    };
    // Preserve only a receipt-ID-shaped source name. Claimed source names are
    // untrusted and can otherwise persist a token in the dead-letter pathname.
    let target = dead.join(dead_letter_name(claim, &receipt_sha256));
    let bytes = serde_json::to_vec(&letter).map_err(|e| format!("serialize dead letter: {e}"))?;
    write_immutable(dead, &target, &bytes)?;
    Ok(())
}

fn dead_letter_name(claim: &ClaimedReceipt, receipt_sha256: &str) -> String {
    let name = claim.original_name();
    if name
        .strip_suffix(".json")
        .is_some_and(super::receipt::valid_repo_id)
    {
        name.to_owned()
    } else {
        format!("{receipt_sha256}.json")
    }
}

fn completed_name(path: &Path) -> Option<String> {
    path.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(".json"))
        .map(|id| format!("{id}.done"))
}

fn is_completed(completed: &Path, path: &Path) -> bool {
    let Some(name) = completed_name(path) else {
        return false;
    };
    let Ok(receipt_bytes) = super::receipt::read_receipt_bytes(path) else {
        return false;
    };
    let Ok(receipt) = serde_json::from_slice::<super::receipt::ReceiptV1>(&receipt_bytes) else {
        return false;
    };
    if receipt.version != 1
        || !super::receipt::valid_repo_id(&receipt.repo_id)
        || !super::receipt::valid_repo_id(&receipt.invocation_id)
        || receipt.receipt_id
            != super::receipt::receipt_id(&receipt.repo_id, &receipt.invocation_id)
        || completed_name(path) != Some(format!("{}.done", receipt.receipt_id))
    {
        return false;
    }
    marker_matches(
        &super::receipt::read_receipt_bytes(&completed.join(name)).unwrap_or_default(),
        &receipt.receipt_id,
        &receipt_bytes,
    )
}

/// Completion marker follows canonical append. Source receipt stays immutable,
/// so a path swap cannot make drain rename or delete replacement bytes.
fn mark_completed(claim: &ClaimedReceipt, completed: &Path) -> Result<(), String> {
    let receipt = claim.receipt()?;
    let name = format!("{}.done", receipt.receipt_id);
    let bytes = marker_bytes(&receipt.receipt_id, claim.bytes())?;
    match super::receipt::atomic_create_immutable(completed, &name, &bytes) {
        Ok(()) => Ok(()),
        Err(error) if error == "already exists" => {
            if super::receipt::read_receipt_bytes(&completed.join(name))? == bytes {
                Ok(())
            } else {
                Err("completed receipt id collision has different immutable bytes".into())
            }
        }
        Err(error) => Err(format!("mark receipt complete: {error}")),
    }
}

fn marker_bytes(receipt_id: &str, receipt_bytes: &[u8]) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&CompletionMarkerV1 {
        version: 1,
        receipt_id: receipt_id.to_owned(),
        receipt_sha256: runtime_store::sha256_hex(receipt_bytes),
    })
    .map_err(|e| format!("serialize completion marker: {e}"))
}

/// Markers are accepted only when their exact canonical bytes bind current
/// receipt ID and bytes. A forged or replaced marker therefore cannot skip drain.
fn marker_matches(marker: &[u8], receipt_id: &str, receipt_bytes: &[u8]) -> bool {
    marker_bytes(receipt_id, receipt_bytes).is_ok_and(|expected| marker == expected)
}

/// Bound policy: retire completed source plus marker on each drain. Canonical
/// receipt-ID dedupe survives any crash between marker creation and either unlink.
fn cleanup_completed(receipts: &Path, completed: &Path) -> Result<(), String> {
    let markers: Vec<_> = std::fs::read_dir(completed)
        .map_err(|e| format!("read completed receipts: {e}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("done"))
        .take(COMPLETION_CLEANUP_LIMIT)
        .collect();
    for marker_path in markers {
        let marker_name = marker_path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("completed receipt has invalid name")?;
        let marker = super::receipt::read_receipt_bytes(&marker_path)?;
        let Some(receipt_id) = marker_name.strip_suffix(".done") else {
            continue;
        };
        let source = receipts.join(format!("{receipt_id}.json"));
        if let Ok(claim) = claim_receipt(&source)
            && claim.has_original_name(&format!("{receipt_id}.json"))
            && claim
                .receipt()
                .is_ok_and(|receipt| receipt.receipt_id == receipt_id)
            && marker_matches(&marker, receipt_id, claim.bytes())
        {
            // Recheck exact opened bytes immediately before unlink. A source
            // pathname swapped after inventory remains for normal drain.
            claim.remove_if_unchanged()?;
        }
        // Remove marker after source: crash before this unlink leaves a marker
        // that next drain can safely finish; crash after it leaves canonical dedupe.
        remove_immutable_if_matches(completed, marker_name, &marker)?;
    }
    Ok(())
}

fn remove_immutable_if_matches(dir: &Path, name: &str, expected: &[u8]) -> Result<(), String> {
    let path = dir.join(name);
    if super::receipt::read_receipt_bytes(&path)? != expected {
        return Err("completed receipt marker was replaced; retained for recovery".into());
    }
    #[cfg(unix)]
    {
        use std::io::Read;
        use std::os::unix::{ffi::OsStrExt, fs::MetadataExt, io::AsRawFd};
        let dir_file = super::receipt::open_private_dir(dir)?;
        let name_c = std::ffi::CString::new(name.as_bytes())
            .map_err(|_| "completed receipt has invalid name")?;
        let mut file = super::receipt::open_private_regular_at(
            &dir_file,
            std::ffi::OsStr::from_bytes(name.as_bytes()),
        )?;
        let mut opened_bytes = Vec::new();
        file.read_to_end(&mut opened_bytes)
            .map_err(|e| format!("read completion marker: {e}"))?;
        if opened_bytes != expected {
            return Err("completed receipt marker was replaced; retained for recovery".into());
        }
        let opened = file
            .metadata()
            .map_err(|e| format!("stat completion marker: {e}"))?;
        let mut current = std::mem::MaybeUninit::<libc::stat>::uninit();
        if unsafe {
            libc::fstatat(
                dir_file.as_raw_fd(),
                name_c.as_ptr(),
                current.as_mut_ptr(),
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            return Err(format!(
                "revalidate completion marker: {}",
                std::io::Error::last_os_error()
            ));
        }
        let current = unsafe { current.assume_init() };
        if u64::try_from(current.st_dev).ok() != Some(opened.dev())
            || current.st_ino != opened.ino()
        {
            return Err("completed receipt marker was replaced; retained for recovery".into());
        }
        if unsafe { libc::unlinkat(dir_file.as_raw_fd(), name_c.as_ptr(), 0) } != 0 {
            return Err(format!(
                "remove completion marker: {}",
                std::io::Error::last_os_error()
            ));
        }
        dir_file
            .sync_all()
            .map_err(|e| format!("fsync completed receipt directory: {e}"))
    }
    #[cfg(not(unix))]
    {
        let _ = (dir, name, expected);
        Err("safe completion marker removal is unavailable on this platform".into())
    }
}

fn write_immutable(dir: &Path, target: &Path, bytes: &[u8]) -> Result<(), String> {
    ensure_private_dir(dir)?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("dead letter has invalid name")?;
    match super::receipt::atomic_create_immutable(dir, name, bytes) {
        Ok(()) => Ok(()),
        Err(error) if error == "already exists" => {
            if read_receipt_bytes(target)? == bytes {
                Ok(())
            } else {
                Err("dead-letter id collision has different immutable bytes".into())
            }
        }
        Err(error) => Err(format!("create dead letter: {error}")),
    }
}

fn pending(dir: &Path, completed: &Path) -> Result<usize, String> {
    Ok(std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter(|e| super::receipt::is_receipt_entry(&e.path()))
        .filter(|e| !is_completed(completed, &e.path()))
        .count())
}

fn receipt_entries(dir: &Path) -> Result<usize, String> {
    Ok(std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .filter(|e| super::receipt::is_receipt_entry(&e.path()))
        .count())
}

fn write_current_status(
    receipts: &Path,
    dead: &Path,
    completed: &Path,
    root: &Path,
    mut status: DrainStatus,
) -> Result<DrainStatus, String> {
    status.pending = pending(receipts, completed)?;
    status.dead_letters = receipt_entries(dead)?;
    let bytes = serde_json::to_vec_pretty(&json!({"version":1,"pending":status.pending,"dead_letters":status.dead_letters,"drained":status.drained,"last_error":status.last_error})).map_err(|e| e.to_string())?;
    super::receipt::atomic_replace_private(root, runtime_store::STATUS, &bytes).map(|()| status)
}

pub fn read_status(root: &Path) -> Option<DrainStatus> {
    #[derive(Deserialize)]
    struct StoredStatus {
        pending: usize,
        dead_letters: usize,
        drained: usize,
        last_error: Option<DrainFailure>,
    }
    let bytes = super::receipt::read_receipt_bytes(&root.join(runtime_store::STATUS)).ok()?;
    let stored: StoredStatus = serde_json::from_slice(&bytes).ok()?;
    Some(DrainStatus {
        pending: stored.pending,
        dead_letters: stored.dead_letters,
        drained: stored.drained,
        last_error: stored.last_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::receipt::{ReceiptV1, receipt_id, write_receipt};

    fn scratch(tag: &str) -> (PathBuf, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "hugit-receipt-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let log = root.join("event-log.json");
        std::fs::write(&log, b"[]").unwrap();
        std::fs::write(
            root.join("runtime.json"),
            format!(r#"{{"repository_id":"{}"}}"#, "a".repeat(64)),
        )
        .unwrap();
        (root, log)
    }
    fn receipt(repo: &str, invocation: &str, target: &str) -> ReceiptV1 {
        let repo = if repo == "repo" {
            "a".repeat(64)
        } else {
            repo.to_string()
        };
        let invocation = crate::runtime_store::sha256_hex(invocation.as_bytes());
        ReceiptV1 {
            version: 1,
            repo_id: repo.clone(),
            invocation_id: invocation.clone(),
            receipt_id: receipt_id(&repo, &invocation),
            kind: "ref.update".into(),
            principal: "orchestrator:hugit-hook".into(),
            payload: json!({"target":target}),
            recorded_at: 1,
        }
    }

    fn transaction_receipt(invocation: &str, phase: &str, from: &str, to: &str) -> ReceiptV1 {
        let mut receipt = receipt("repo", invocation, "");
        receipt.payload = json!({"reference_transaction": {
            "phase": phase,
            "updates": [{"from": from, "to": to, "ref": "refs/heads/topic"}],
            "fingerprint": "same-fingerprint"
        }});
        receipt
    }

    fn transaction_state(log: &Path) -> &'static str {
        let records = load_event_log(log).unwrap();
        reference_transaction_pairing(records.records(), "same-fingerprint")
    }

    #[test]
    fn transaction_pairing_matrix_is_order_independent_and_never_guesses() {
        let oid = "a".repeat(40);
        let zero = "0".repeat(40);
        for (tag, phases, expected) in [
            // Prepared persists through worker crash: state remains pending.
            (
                "crash",
                vec![("prepared", oid.as_str(), oid.as_str())],
                "pending",
            ),
            (
                "commit",
                vec![
                    ("prepared", oid.as_str(), oid.as_str()),
                    ("committed", oid.as_str(), oid.as_str()),
                ],
                "paired",
            ),
            // Terminal receipt can drain first; later prepared converges identically.
            (
                "terminal-first",
                vec![
                    ("committed", oid.as_str(), oid.as_str()),
                    ("prepared", oid.as_str(), oid.as_str()),
                ],
                "paired",
            ),
            (
                "abort",
                vec![
                    ("prepared", oid.as_str(), oid.as_str()),
                    ("aborted", oid.as_str(), oid.as_str()),
                ],
                "paired",
            ),
            // Deletion is a zero new OID, still one exact transaction fact.
            (
                "delete",
                vec![
                    ("prepared", oid.as_str(), zero.as_str()),
                    ("committed", oid.as_str(), zero.as_str()),
                ],
                "paired",
            ),
            // Equal concurrent fingerprints have no correlation key: ambiguous.
            (
                "ambiguous",
                vec![
                    ("prepared", oid.as_str(), oid.as_str()),
                    ("prepared", oid.as_str(), oid.as_str()),
                    ("committed", oid.as_str(), oid.as_str()),
                ],
                "ambiguous",
            ),
        ] {
            let (root, log) = scratch(tag);
            let source_count = phases.len();
            for (number, (phase, from, to)) in phases.into_iter().enumerate() {
                write_receipt(
                    &root.join("receipts"),
                    &transaction_receipt(&format!("{tag}-{number}"), phase, from, to),
                )
                .unwrap();
                drain(&log, 8).unwrap();
            }
            assert_eq!(transaction_state(&log), expected, "{tag}");
            let records = load_event_log(&log).unwrap();
            let projections = reference_transaction_projections(records.records());
            assert_eq!(projections.len(), 1, "{tag}");
            assert_eq!(projections[0]["state"], expected, "{tag}");
            assert_eq!(
                projections[0]["source_receipt_ids"]
                    .as_array()
                    .map(Vec::len),
                Some(source_count),
                "{tag}: every source receipt stays attributable"
            );
            assert!(
                records.records().iter().any(|record| {
                    serde_json::from_str::<serde_json::Value>(&record.payload)
                        .ok()
                        .is_some_and(|payload| {
                            payload["reference_transaction_pairing"]["state"] == expected
                        })
                }),
                "{tag}: reducer label persists in canonical log"
            );
        }
    }

    #[test]
    fn empty_completed_queue_reconciles_missing_pairing_projection_once() {
        let (root, log_path) = scratch("completed-empty-reconcile");
        let mut log = load_event_log(&log_path).unwrap();
        for phase in ["prepared", "committed"] {
            let payload = json!({"reference_transaction": {
                "phase": phase,
                "fingerprint": "completed-fingerprint"
            }});
            log.append_authorized(
                PrincipalClass::Orchestrator,
                Endpoint::Push,
                "ref.update".to_string(),
                vec!["orchestrator:hugit-hook".to_string()],
                hugit_refstore::canonical_json(&payload.to_string()).unwrap(),
                0,
            )
            .unwrap();
        }
        atomic_write(
            &log_path,
            &serde_json::to_vec_pretty(log.records()).unwrap(),
        )
        .unwrap();
        ensure_private_dir(&root.join("receipts")).unwrap();

        drain(&log_path, 0).unwrap();
        let after_first = load_event_log(&log_path).unwrap();
        assert!(after_first.records().iter().any(|record| {
            serde_json::from_str::<serde_json::Value>(&record.payload)
                .ok()
                .is_some_and(|payload| {
                    payload["reference_transaction_pairing"]["fingerprint"]
                        == "completed-fingerprint"
                        && payload["reference_transaction_pairing"]["state"] == "paired"
                })
        }));
        let count = after_first.len();

        drain(&log_path, 0).unwrap();
        assert_eq!(
            load_event_log(&log_path).unwrap().len(),
            count,
            "reconcile is idempotent"
        );
    }

    #[test]
    fn retry_same_receipt_appends_once() {
        let (root, log) = scratch("dedupe");
        let r = receipt("repo", "one", "a");
        write_receipt(&root.join("receipts"), &r).unwrap();
        drain(&log, 8).unwrap();
        write_receipt(&root.join("receipts"), &r).unwrap();
        drain(&log, 8).unwrap();
        assert_eq!(
            load_event_log(&log).unwrap().len(),
            1,
            "receipt_id is canonical dedupe key"
        );
    }

    #[test]
    fn durable_receipt_survives_worker_death_then_drains() {
        let (root, log) = scratch("kill");
        let r = receipt("repo", "dead-worker", "a");
        let source = write_receipt(&root.join("receipts"), &r).unwrap();
        assert!(source.exists(), "receipt durable before worker starts");
        drain(&log, 8).unwrap();
        assert_eq!(load_event_log(&log).unwrap().len(), 1);
        assert!(
            !source.exists(),
            "completed source retires after canonical receipt-ID durability"
        );
        assert!(
            !root
                .join("completed-receipts")
                .join(format!("{}.done", r.receipt_id))
                .exists(),
            "completed marker retires with its source"
        );
    }

    #[test]
    fn forged_completion_marker_cannot_skip_drain() {
        let (root, log) = scratch("forged-marker");
        let receipt = receipt("repo", "forged-marker", "a");
        let source = write_receipt(&root.join("receipts"), &receipt).unwrap();
        let completed = root.join("completed-receipts");
        ensure_private_dir(&completed).unwrap();
        super::super::receipt::atomic_create_immutable(
            &completed,
            &format!("{}.done", receipt.receipt_id),
            b"forged marker",
        )
        .unwrap();

        let status = drain(&log, 8).unwrap();
        assert_eq!(status.drained, 1);
        assert_eq!(load_event_log(&log).unwrap().len(), 1);
        assert!(!source.exists());
        assert!(std::fs::read_dir(completed).unwrap().next().is_none());
    }

    #[test]
    fn completed_receipts_cleanup_is_bounded() {
        let (root, log) = scratch("completed-cleanup");
        for n in 0..=COMPLETION_CLEANUP_LIMIT {
            write_receipt(
                &root.join("receipts"),
                &receipt("repo", &format!("completed-{n}"), "a"),
            )
            .unwrap();
        }
        let status = drain(&log, COMPLETION_CLEANUP_LIMIT + 1).unwrap();
        assert_eq!(status.drained, COMPLETION_CLEANUP_LIMIT + 1);
        assert_eq!(
            load_event_log(&log).unwrap().len(),
            COMPLETION_CLEANUP_LIMIT + 1
        );
        assert_eq!(
            std::fs::read_dir(root.join("receipts")).unwrap().count(),
            1,
            "one completed receipt remains for next bounded cleanup pass"
        );
        assert_eq!(
            std::fs::read_dir(root.join("completed-receipts"))
                .unwrap()
                .count(),
            1,
            "one marker remains for next bounded cleanup pass"
        );
        drain(&log, 0).unwrap();
        assert_eq!(std::fs::read_dir(root.join("receipts")).unwrap().count(), 0);
        assert_eq!(
            std::fs::read_dir(root.join("completed-receipts"))
                .unwrap()
                .count(),
            0,
            "later bounded pass retires remaining completion state"
        );
    }

    #[test]
    fn concurrent_receipts_drain_without_lost_events() {
        let (root, log) = scratch("race");
        for n in 0..8 {
            write_receipt(
                &root.join("receipts"),
                &receipt("repo", &format!("i{n}"), &format!("t{n}")),
            )
            .unwrap();
        }
        let mut workers = Vec::new();
        for _ in 0..4 {
            let log = log.clone();
            workers.push(std::thread::spawn(move || {
                let _ = drain(&log, 32);
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }
        drain(&log, 32).unwrap();
        assert_eq!(
            load_event_log(&log).unwrap().len(),
            8,
            "racing drainers retain every invocation"
        );
    }

    #[test]
    fn busy_drain_retains_receipt_for_recovery() {
        let (root, log) = scratch("busy");
        let source = write_receipt(&root.join("receipts"), &receipt("repo", "busy", "a")).unwrap();
        let _lock = FileLock::acquire(&log).unwrap();
        let status = drain(&log, 8).unwrap();
        assert_eq!(status.pending, 1);
        assert!(
            source.exists(),
            "temporary lock contention must not dead-letter receipt"
        );
        assert_eq!(
            status.last_error.as_ref().map(|error| error.code.as_str()),
            Some("log_busy")
        );
        let persisted = read_status(&root).expect("failure status is durable");
        assert_eq!(persisted.last_error.unwrap().code, "log_busy");
    }

    #[test]
    fn temporary_receipt_is_never_a_drain_input() {
        let (root, log) = scratch("partial");
        let receipts = root.join("receipts");
        ensure_private_dir(&receipts).unwrap();
        std::fs::write(receipts.join(".partial.json.tmp"), b"{").unwrap();
        let status = drain(&log, 8).unwrap();
        assert_eq!(status.drained, 0);
        assert!(receipts.join(".partial.json.tmp").exists());
        assert_eq!(load_event_log(&log).unwrap().len(), 0);
    }

    #[test]
    fn cross_repo_receipt_never_reaches_canonical_log() {
        let (root, log) = scratch("cross-repo");
        let foreign = receipt(&"b".repeat(64), "foreign", "a");
        write_receipt(&root.join("receipts"), &foreign).unwrap();
        let status = drain(&log, 8).unwrap();
        assert_eq!(status.dead_letters, 1);
        assert_eq!(load_event_log(&log).unwrap().len(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_receipt_is_not_followed() {
        let (root, log) = scratch("symlink");
        let receipts = root.join("receipts");
        ensure_private_dir(&receipts).unwrap();
        let victim = root.join("victim");
        std::fs::write(&victim, b"secret").unwrap();
        std::os::unix::fs::symlink(&victim, receipts.join("forged.json")).unwrap();
        assert!(read_receipt_bytes(&receipts.join("forged.json")).is_err());
        let status = drain(&log, 8).unwrap();
        assert_eq!(status.drained, 0);
        assert_eq!(std::fs::read(&victim).unwrap(), b"secret");
    }

    #[test]
    fn mutated_receipt_identity_moves_to_durable_dead_letter() {
        let (root, log) = scratch("mutation");
        let receipt = receipt("repo", "original", "a");
        let source = write_receipt(&root.join("receipts"), &receipt).unwrap();
        let mut mutated: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&source).unwrap()).unwrap();
        mutated["invocation_id"] = json!("changed");
        let mutated_bytes = serde_json::to_vec(&mutated).unwrap();
        std::fs::write(&source, &mutated_bytes).unwrap();

        let status = drain(&log, 8).unwrap();
        assert_eq!(status.pending, 0);
        assert_eq!(status.dead_letters, 1);
        assert!(
            !source.exists(),
            "durable dead letter permits bounded completed-source cleanup"
        );
        let letters: Vec<_> = std::fs::read_dir(root.join("dead-letter"))
            .unwrap()
            .collect();
        assert_eq!(letters.len(), 1);
        assert_eq!(
            letters[0].as_ref().unwrap().file_name().to_string_lossy(),
            format!("{}.json", receipt.receipt_id)
        );
        let letter: serde_json::Value =
            serde_json::from_slice(&std::fs::read(letters[0].as_ref().unwrap().path()).unwrap())
                .unwrap();
        assert!(
            letter["reason"]
                .as_str()
                .unwrap()
                .contains("invalid receipt identity")
        );
        assert_eq!(letter["receipt"]["state"], "parsed");
        assert_eq!(letter["receipt"]["invocation_id"], "changed");
        assert_eq!(
            letter["receipt_sha256"],
            crate::runtime_store::sha256_hex(&mutated_bytes)
        );
        assert!(letter.get("raw_receipt").is_none());
    }

    #[test]
    fn token_shaped_keys_and_values_dead_letter_without_status_or_export_leaks() {
        let (root, log) = scratch("dead-letter-secret-scan");
        let secret = ["gh", "p_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJ"].concat();
        let receipt = receipt("repo", "secret", &secret);
        let source = write_receipt(&root.join("receipts"), &receipt).unwrap();
        let mut mutated: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&source).unwrap()).unwrap();
        mutated["invocation_id"] = json!(secret);
        mutated["payload"] = json!({
            (secret.clone()): secret.clone(),
            "nested": { (secret.clone()): [secret.clone(), { (secret.clone()): secret.clone() }] },
        });
        let raw = serde_json::to_vec(&mutated).unwrap();
        std::fs::write(&source, &raw).unwrap();

        let status = drain(&log, 8).unwrap();
        assert_eq!(status.dead_letters, 1);
        assert_eq!(status.pending, 0, "dead-lettered malformed source retires");
        let dead_letter = std::fs::read_dir(root.join("dead-letter"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let dead_bytes = std::fs::read(&dead_letter).unwrap();
        let status_bytes = std::fs::read(root.join(crate::runtime_store::STATUS)).unwrap();
        let dead_text = String::from_utf8(dead_bytes).unwrap();
        let status_text = String::from_utf8(status_bytes).unwrap();

        assert!(!dead_text.contains(&secret));
        assert!(!status_text.contains(&secret));
        let letter: serde_json::Value = serde_json::from_str(&dead_text).unwrap();
        assert!(letter["receipt"]["payload"].get(secret).is_none());
        assert!(
            !crate::export::redaction::contains_secret(&dead_text),
            "export residual scanner must reject any dead-letter secret"
        );
        assert!(
            !crate::export::redaction::contains_secret(&status_text),
            "export residual scanner must reject any status secret"
        );
    }

    #[test]
    fn malformed_receipt_dead_letter_stores_only_typed_invalid_metadata() {
        let (root, log) = scratch("dead-letter-malformed");
        let secret = ["gh", "p_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJ"].concat();
        let receipts = root.join("receipts");
        ensure_private_dir(&receipts).unwrap();
        super::super::receipt::atomic_create_immutable(
            &receipts,
            "forged.json",
            format!("{{bad:{secret}").as_bytes(),
        )
        .unwrap();

        let status = drain(&log, 8).unwrap();
        assert_eq!(status.dead_letters, 1);
        assert_eq!(status.pending, 0, "dead-lettered malformed source retires");
        let letter = std::fs::read_dir(root.join("dead-letter"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let text = std::fs::read_to_string(letter).unwrap();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["receipt"]["state"], "invalid_json");
        assert!(value["receipt"].get("byte_len").is_some());
        assert!(value["receipt"].get("parse_error").is_some());
        assert!(!text.contains(&secret));
    }

    #[test]
    fn token_shaped_source_filename_uses_sha256_dead_letter_fallback() {
        let (root, log) = scratch("dead-letter-filename-secret-scan");
        let secret = ["gh", "p_abcdefghijklmnopqrstuvwxyz0123456789ABCDEFGHIJ"].concat();
        let receipts = root.join("receipts");
        let source_name = format!("{secret}.json");
        let source_bytes = b"{invalid json";
        ensure_private_dir(&receipts).unwrap();
        super::super::receipt::atomic_create_immutable(&receipts, &source_name, source_bytes)
            .unwrap();

        let status = drain(&log, 8).unwrap();
        assert_eq!(status.dead_letters, 1);
        let letter = std::fs::read_dir(root.join("dead-letter"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap();
        let name = letter.file_name().into_string().unwrap();
        assert_eq!(
            name,
            format!("{}.json", crate::runtime_store::sha256_hex(source_bytes))
        );
        assert!(!name.contains(&secret));
        assert!(
            !crate::export::redaction::contains_secret(&name),
            "filename secret scanner must reject token-shaped source names"
        );
    }

    #[test]
    fn dead_letter_collision_restores_claimed_receipt() {
        let (root, log) = scratch("dead-letter-collision");
        let receipt = receipt("repo", "collision", "a");
        let source = write_receipt(&root.join("receipts"), &receipt).unwrap();
        let mut mutated: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&source).unwrap()).unwrap();
        mutated["invocation_id"] = json!("changed");
        std::fs::write(&source, serde_json::to_vec(&mutated).unwrap()).unwrap();
        let dead = root.join("dead-letter");
        ensure_private_dir(&dead).unwrap();
        super::super::receipt::atomic_create_immutable(
            &dead,
            &format!("{}.json", receipt.receipt_id),
            b"different immutable letter",
        )
        .unwrap();

        let status = drain(&log, 8).unwrap();
        assert_eq!(
            status.pending, 1,
            "failed dead letter restores public receipt"
        );
        assert_eq!(status.dead_letters, 1);
        assert!(
            source.exists(),
            "failed dead letter never strands hidden claim"
        );
        assert_eq!(
            status.last_error.as_ref().map(|error| error.code.as_str()),
            Some("invalid_receipt_retained")
        );
    }

    #[cfg(unix)]
    #[test]
    fn public_drain_rejects_replaced_completed_source() {
        let (root, log) = scratch("source-swap");
        let receipt = receipt("repo", "source-swap", "a");
        let source = write_receipt(&root.join("receipts"), &receipt).unwrap();
        let original = std::fs::read(&source).unwrap();
        let claim = claim_receipt(&source).unwrap();

        // Build crash-recovery state: canonical append and marker durable, source
        // still present. Replace pathname before public inventory resumes.
        drain_one(&log, &claim, &"a".repeat(64)).unwrap();
        let completed = root.join("completed-receipts");
        ensure_private_dir(&completed).unwrap();
        mark_completed(&claim, &completed).unwrap();
        let held = source.with_extension("held");
        std::fs::rename(&source, &held).unwrap();
        let mut replacement = receipt.clone();
        replacement.payload = json!({"target":"replacement"});
        std::fs::write(&source, serde_json::to_vec(&replacement).unwrap()).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&source, std::fs::Permissions::from_mode(0o600)).unwrap();

        let status = drain(&log, 8).unwrap();
        assert_eq!(claim.bytes(), original.as_slice());
        assert_eq!(status.drained, 1);
        assert!(
            !source.exists(),
            "public inventory claimed replacement instead of trusting old marker"
        );
        assert_eq!(std::fs::read(&held).unwrap(), original);
        assert_eq!(load_event_log(&log).unwrap().len(), 1);
    }
}
