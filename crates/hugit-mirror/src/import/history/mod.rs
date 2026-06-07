//! Commit/tree/blob import with byte-identity materialization (WP-E2a, items ① ⑤).
//!
//! # Import boundary law (⑤)
//! Every imported commit materializes as an **opaque [`EventRecord`]** change-event
//! with `kind = "git.commit"`. A bare commit NEVER produces a synthesized intent
//! (the proposed-intent path is owned by E2b; it is NOT in this module).
//! The no-intent path is the ONLY path in this module.
//!
//! # Byte-identity (①)
//! Object identity is verified by comparing the source object OID (SHA-1 hex
//! for git) against the OID recomputed from the imported bytes. A mismatch is a
//! hard error — `ImportError::ByteIdentityMismatch`.
//!
//! # Disjointness (⑤)
//! This module does NOT reference the PR/issue (E2b) module or any intent-synthesis path.

use hugit_contracts::EventRecord;
use hugit_refstore::{canonical_json, compute_this_hash};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod object_hash;

/// A git object identifier (SHA-1 hex, 40 chars for SHA-1 repos).
pub type Oid = String;

/// The kind string used when projecting a bare commit to an EventRecord.
///
/// MUST remain `"git.commit"` — callers may match on this string to
/// distinguish commit change-events from other event kinds.
pub const COMMIT_EVENT_KIND: &str = "git.commit";

/// Metadata for a single imported commit.
#[derive(Debug, Clone)]
pub struct CommitMeta {
    /// The object OID from the source repository (SHA-1 hex).
    pub oid: String,
    /// Commit author name.
    pub author: String,
    /// Commit message.
    pub message: String,
    /// Unix epoch seconds of the author timestamp.
    pub timestamp: u64,
    /// Parent OIDs (empty for root commits).
    pub parents: Vec<Oid>,
    /// Tree OID.
    pub tree_oid: Oid,
}

/// Raw bytes of a git object (blob/tree/commit).
#[derive(Debug, Clone)]
pub struct GitObject {
    /// Object OID (SHA-1 hex from source).
    pub oid: Oid,
    /// Object kind: "blob", "tree", or "commit".
    pub kind: String,
    /// Raw object bytes (byte-identical to source).
    pub bytes: Vec<u8>,
}

/// Errors from the history importer.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// Byte identity check failed: recomputed OID does not match source OID.
    #[error("byte-identity mismatch: source oid={source_oid}, recomputed oid={recomputed_oid}")]
    ByteIdentityMismatch {
        source_oid: String,
        recomputed_oid: String,
    },

    /// The git shell command failed.
    #[error("git command failed: {0}")]
    GitCommand(String),

    /// The repository could not be cloned or accessed.
    #[error("repository access error: {0}")]
    RepoAccess(String),

    /// An object was not found in the repository.
    #[error("object not found: {oid}")]
    ObjectNotFound { oid: String },

    /// The payload could not be serialized.
    #[error("payload serialization error: {0}")]
    PayloadSerialize(String),
}

/// Projects a `CommitMeta` to an opaque `EventRecord` change-event.
///
/// # Import boundary law (⑤)
/// The returned event has `kind = COMMIT_EVENT_KIND = "git.commit"`.
/// This function NEVER produces an intent from a bare commit.
/// The no-intent path is the ONLY path — proposed intents are E2b's domain.
///
/// # Byte-identity (①)
/// The caller must have already verified byte-identity (oid compare) before
/// calling this function. The `oid` field in `CommitMeta` is included in the
/// payload as the authoritative source-side hash.
pub fn project_commit_to_event(
    commit: &CommitMeta,
    seq: u64,
    prev_hash: &str,
    principal: &str,
) -> Result<EventRecord, ImportError> {
    // Opaque payload: the commit metadata as JSON. No intent fields.
    let payload_obj = serde_json::json!({
        "oid": commit.oid,
        "author": commit.author,
        "message": commit.message,
        "timestamp": commit.timestamp,
        "parents": commit.parents,
        "tree_oid": commit.tree_oid,
    });
    let serialized = serde_json::to_string(&payload_obj)
        .map_err(|e| ImportError::PayloadSerialize(e.to_string()))?;
    // Chain canonical-JSON bytes (sorted keys, no insignificant whitespace) so
    // the producer hashes exactly what a verifier re-canonicalises.
    let payload = canonical_json(&serialized)
        .ok_or_else(|| ImportError::PayloadSerialize("payload is not valid JSON".to_string()))?;

    let principal_chain = vec![principal.to_string()];

    // Single-source the chain hash via the canonical formula
    // (hugit_refstore::compute_this_hash) — never re-transcribe the byte-format.
    let this_hash = compute_this_hash(
        prev_hash,
        COMMIT_EVENT_KIND,
        &principal_chain,
        &payload,
        seq,
    );

    let recorded_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    Ok(EventRecord {
        seq,
        prev_hash: prev_hash.to_string(),
        this_hash,
        kind: COMMIT_EVENT_KIND.to_string(),
        principal_chain,
        payload,
        recorded_at,
    })
}

/// Verify byte identity: recompute the git object hash from `raw_bytes` and
/// compare against `expected_oid`.
///
/// Returns `Ok(())` on match; `Err(ImportError::ByteIdentityMismatch)` on mismatch.
pub fn verify_byte_identity(
    kind: &str,
    raw_bytes: &[u8],
    expected_oid: &str,
) -> Result<(), ImportError> {
    let recomputed = object_hash::compute_git_oid(kind, raw_bytes);
    if recomputed != expected_oid {
        return Err(ImportError::ByteIdentityMismatch {
            source_oid: expected_oid.to_string(),
            recomputed_oid: recomputed,
        });
    }
    Ok(())
}

/// Import a batch of commits from a local git repository path, yielding
/// `EventRecord` change-events.
///
/// Uses `git cat-file` shell-out to read object bytes, verifies byte-identity
/// for each, and projects each commit to an opaque change-event (⑤: no intents).
///
/// `repo_path` — absolute path to the (cloned) local repository.
/// `oids` — ordered list of commit OIDs to import.
/// `start_seq` — starting sequence number for the first event.
/// `start_prev_hash` — `prev_hash` for the first event.
/// `principal` — importing principal name (e.g. `"hugit-mirror/e2a"`).
pub fn import_commits(
    repo_path: &std::path::Path,
    oids: &[Oid],
    start_seq: u64,
    start_prev_hash: &str,
    principal: &str,
) -> Result<Vec<EventRecord>, ImportError> {
    let mut events = Vec::with_capacity(oids.len());
    let mut prev_hash = start_prev_hash.to_string();

    for (i, oid) in oids.iter().enumerate() {
        let seq = start_seq + i as u64;
        // Read commit object bytes via git cat-file shell-out.
        let obj = read_git_object(repo_path, oid, "commit")?;

        // Verify byte identity: recompute OID from raw bytes, compare to source.
        verify_byte_identity("commit", &obj.bytes, oid)?;

        // Parse commit metadata from raw bytes.
        let meta = parse_commit_meta(oid, &obj.bytes)?;

        // Project to EventRecord change-event (import boundary ⑤: no intent).
        let event = project_commit_to_event(&meta, seq, &prev_hash, principal)?;

        prev_hash = event.this_hash.clone();
        events.push(event);
    }

    Ok(events)
}

/// Read a git object's raw bytes from a local repository using `git cat-file`.
pub fn read_git_object(
    repo_path: &std::path::Path,
    oid: &str,
    expected_kind: &str,
) -> Result<GitObject, ImportError> {
    // git cat-file -t <oid> to verify kind.
    let type_out = std::process::Command::new("git")
        .args([
            "-C",
            repo_path.to_str().unwrap_or("."),
            "cat-file",
            "-t",
            oid,
        ])
        .output()
        .map_err(|e| ImportError::GitCommand(format!("cat-file -t: {e}")))?;

    if !type_out.status.success() {
        return Err(ImportError::ObjectNotFound {
            oid: oid.to_string(),
        });
    }

    let kind = String::from_utf8_lossy(&type_out.stdout).trim().to_string();
    if kind != expected_kind {
        return Err(ImportError::GitCommand(format!(
            "expected object kind {expected_kind}, got {kind} for {oid}"
        )));
    }

    // Use `git cat-file --batch` for raw object bytes.
    use std::io::Write as IoWrite;
    let mut child = std::process::Command::new("git")
        .args([
            "-C",
            repo_path.to_str().unwrap_or("."),
            "cat-file",
            "--batch",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| ImportError::GitCommand(format!("spawn cat-file --batch: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(format!("{oid}\n").as_bytes())
            .map_err(|e| ImportError::GitCommand(format!("write to cat-file stdin: {e}")))?;
    }

    let batch_out = child
        .wait_with_output()
        .map_err(|e| ImportError::GitCommand(format!("cat-file --batch output: {e}")))?;

    if !batch_out.status.success() {
        return Err(ImportError::GitCommand(format!(
            "cat-file --batch failed for {oid}"
        )));
    }

    // Parse batch output: "<oid> <type> <size>\n<bytes>\n".
    // The header is VALIDATED — `<oid>` must match the requested oid, `<type>`
    // must match the verified kind, `<size>` must parse — and the body is sliced
    // to EXACTLY `<size>` bytes (never "everything after the first newline",
    // which would over-read past the object into the trailing framing).
    let output = &batch_out.stdout;
    let header_end = output
        .iter()
        .position(|&b| b == b'\n')
        .ok_or_else(|| ImportError::GitCommand(format!("cat-file --batch: no header for {oid}")))?;
    let header = std::str::from_utf8(&output[..header_end]).map_err(|e| {
        ImportError::GitCommand(format!("cat-file --batch: non-UTF8 header for {oid}: {e}"))
    })?;

    // A missing object yields "<oid> missing" — surface as ObjectNotFound.
    if header.ends_with(" missing") || header == "missing" {
        return Err(ImportError::ObjectNotFound {
            oid: oid.to_string(),
        });
    }

    let mut parts = header.split(' ');
    let hdr_oid = parts.next().unwrap_or_default();
    let hdr_type = parts.next().ok_or_else(|| {
        ImportError::GitCommand(format!("cat-file --batch: malformed header {header:?}"))
    })?;
    let hdr_size_str = parts.next().ok_or_else(|| {
        ImportError::GitCommand(format!("cat-file --batch: header missing size {header:?}"))
    })?;
    if parts.next().is_some() {
        return Err(ImportError::GitCommand(format!(
            "cat-file --batch: header has trailing fields {header:?}"
        )));
    }

    if hdr_oid != oid {
        return Err(ImportError::GitCommand(format!(
            "cat-file --batch: header oid {hdr_oid} != requested {oid}"
        )));
    }
    if hdr_type != kind {
        return Err(ImportError::GitCommand(format!(
            "cat-file --batch: header type {hdr_type} != verified kind {kind} for {oid}"
        )));
    }
    let size: usize = hdr_size_str.parse().map_err(|e| {
        ImportError::GitCommand(format!(
            "cat-file --batch: invalid size {hdr_size_str:?}: {e}"
        ))
    })?;

    // Slice EXACTLY `size` bytes of body, starting right after the header
    // newline. The body must be present in full (git appends a trailing '\n'
    // after it, so there must be at least `size + 1` bytes available).
    let body_start = header_end + 1;
    let body_end = body_start.checked_add(size).ok_or_else(|| {
        ImportError::GitCommand(format!("cat-file --batch: size overflow for {oid}"))
    })?;
    if output.len() < body_end {
        return Err(ImportError::GitCommand(format!(
            "cat-file --batch: truncated body for {oid} (have {}, need {body_end})",
            output.len()
        )));
    }
    let object_bytes = output[body_start..body_end].to_vec();

    Ok(GitObject {
        oid: oid.to_string(),
        kind,
        bytes: object_bytes,
    })
}

/// Parse commit metadata from raw commit object bytes.
///
/// Git commit objects:
/// ```text
/// tree <tree-oid>\n
/// parent <parent-oid>\n   (zero or more)
/// author <name> <email> <timestamp> <tz>\n
/// committer <name> <email> <timestamp> <tz>\n
/// \n
/// <message>
/// ```
pub fn parse_commit_meta(oid: &str, bytes: &[u8]) -> Result<CommitMeta, ImportError> {
    let text = String::from_utf8_lossy(bytes);
    let mut tree_oid = String::new();
    let mut parents = Vec::new();
    let mut author = String::new();
    let mut timestamp = 0u64;
    let mut message = String::new();
    let mut in_header = true;

    for line in text.lines() {
        if in_header && line.is_empty() {
            in_header = false;
            continue;
        }
        if in_header {
            if let Some(t) = line.strip_prefix("tree ") {
                tree_oid = t.trim().to_string();
            } else if let Some(p) = line.strip_prefix("parent ") {
                parents.push(p.trim().to_string());
            } else if let Some(a) = line.strip_prefix("author ") {
                author = a.to_string();
                let parts: Vec<&str> = a.split_whitespace().collect();
                if parts.len() >= 2
                    && let Ok(ts) = parts[parts.len() - 2].parse::<u64>()
                {
                    timestamp = ts;
                }
            }
        } else {
            if !message.is_empty() {
                message.push('\n');
            }
            message.push_str(line);
        }
    }

    Ok(CommitMeta {
        oid: oid.to_string(),
        author,
        message,
        timestamp,
        parents,
        tree_oid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn commit_projects_to_event_record_not_intent() {
        let meta = CommitMeta {
            oid: "a".repeat(40),
            author: "Alice <alice@example.com>".to_string(),
            message: "initial commit".to_string(),
            timestamp: 1_700_000_000,
            parents: vec![],
            tree_oid: "b".repeat(40),
        };
        let genesis = "0000000000000000000000000000000000000000000000000000000000000000";
        let event = project_commit_to_event(&meta, 0, genesis, "hugit-mirror/e2a").unwrap();

        // Must be a change-event, not an intent.
        assert_eq!(event.kind, COMMIT_EVENT_KIND);
        assert_eq!(event.kind, "git.commit");
        assert!(event.payload.contains(&meta.oid));

        let payload: serde_json::Value = serde_json::from_str(&event.payload).unwrap();
        assert_eq!(payload["oid"].as_str().unwrap(), meta.oid);
        // No intent synthesized: no "intent_id", no "charter", no "acceptance" fields.
        assert!(payload.get("intent_id").is_none());
        assert!(payload.get("charter").is_none());
    }

    #[test]
    fn byte_identity_mismatch_is_hard_error() {
        let bytes = b"hello world";
        let correct_oid = object_hash::compute_git_oid("blob", bytes);
        let wrong_oid = "deadbeef".repeat(5);
        let err = verify_byte_identity("blob", bytes, &wrong_oid).unwrap_err();
        assert!(matches!(err, ImportError::ByteIdentityMismatch { .. }));
        // Correct OID must pass.
        verify_byte_identity("blob", bytes, &correct_oid).unwrap();
    }

    #[test]
    fn parse_commit_meta_extracts_fields() {
        let raw = b"tree aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nparent bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\nauthor Alice <alice@example.com> 1700000000 +0000\ncommitter Alice <alice@example.com> 1700000000 +0000\n\nInitial commit\n";
        let meta = parse_commit_meta("cccc", raw).unwrap();
        assert_eq!(meta.tree_oid, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(meta.parents.len(), 1);
        assert_eq!(meta.timestamp, 1_700_000_000);
        assert!(meta.message.contains("Initial commit"));
    }
}
