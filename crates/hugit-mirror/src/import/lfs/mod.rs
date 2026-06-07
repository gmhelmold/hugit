//! LFS object materialization (WP-E2a, item ④).
//!
//! Resolves Git LFS pointer files to their actual object bytes, verifying the
//! SHA-256 hash of the materialized content. The imported bytes are stored
//! content (not the pointer) — the import boundary law applies here too.
//!
//! # LFS pointer format (RFC)
//! ```text
//! version https://git-lfs.github.com/spec/v1
//! oid sha256:<hex-sha256>
//! size <bytes>
//! ```
//!
//! # Materialization
//! 1. Detect whether a blob is an LFS pointer (starts with version line).
//! 2. Parse the OID and size from the pointer.
//! 3. Fetch the actual object bytes from the LFS server.
//! 4. Verify SHA-256 of fetched bytes matches the pointer OID.
//! 5. Return the materialized bytes (NOT the pointer).

/// An LFS pointer parsed from a blob.
#[derive(Debug, Clone, PartialEq)]
pub struct LfsPointer {
    /// The LFS OID string, e.g. `"sha256:abcdef..."`.
    pub oid: String,
    /// The expected byte size of the LFS object.
    pub size: u64,
    /// The SHA-256 hex digest (without the `sha256:` prefix).
    pub sha256: String,
}

/// Errors from LFS materialization.
#[derive(Debug, thiserror::Error)]
pub enum LfsError {
    /// The blob is not an LFS pointer.
    #[error("not an LFS pointer")]
    NotAPointer,

    /// The LFS pointer could not be parsed.
    #[error("malformed LFS pointer: {0}")]
    MalformedPointer(String),

    /// Fetching the LFS object failed.
    #[error("LFS fetch failed for oid {oid}: {message}")]
    FetchFailed { oid: String, message: String },

    /// The fetched bytes do not match the pointer SHA-256.
    #[error("LFS sha256 mismatch: pointer={pointer_sha256}, actual={actual_sha256}")]
    Sha256Mismatch {
        pointer_sha256: String,
        actual_sha256: String,
    },

    /// The fetched size does not match the pointer size.
    #[error("LFS size mismatch: pointer={pointer_size}, actual={actual_size}")]
    SizeMismatch { pointer_size: u64, actual_size: u64 },

    /// LFS credentials absent (token required but not set).
    #[error("LFS credentials absent for repo {repo}")]
    CredentialsAbsent { repo: String },
}

/// Returns `true` if `blob_bytes` starts with the LFS pointer version line.
pub fn is_lfs_pointer(blob_bytes: &[u8]) -> bool {
    blob_bytes.starts_with(b"version https://git-lfs.github.com/spec/v1")
}

/// Parse an LFS pointer from blob bytes.
///
/// Returns `Err(LfsError::NotAPointer)` if the bytes are not an LFS pointer.
pub fn parse_lfs_pointer(blob_bytes: &[u8]) -> Result<LfsPointer, LfsError> {
    if !is_lfs_pointer(blob_bytes) {
        return Err(LfsError::NotAPointer);
    }

    let text = std::str::from_utf8(blob_bytes)
        .map_err(|e| LfsError::MalformedPointer(format!("invalid UTF-8: {e}")))?;

    let mut oid: Option<String> = None;
    let mut size: Option<u64> = None;

    for line in text.lines() {
        if let Some(o) = line.strip_prefix("oid ") {
            oid = Some(o.trim().to_string());
        } else if let Some(s) = line.strip_prefix("size ") {
            size = Some(
                s.trim()
                    .parse::<u64>()
                    .map_err(|e| LfsError::MalformedPointer(format!("invalid size: {e}")))?,
            );
        }
    }

    let oid = oid.ok_or_else(|| LfsError::MalformedPointer("missing oid field".to_string()))?;
    let size = size.ok_or_else(|| LfsError::MalformedPointer("missing size field".to_string()))?;

    let sha256 = oid
        .strip_prefix("sha256:")
        .ok_or_else(|| LfsError::MalformedPointer("oid must have sha256: prefix".to_string()))?
        .to_string();

    if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(LfsError::MalformedPointer(format!(
            "invalid sha256 hex: {sha256}"
        )));
    }

    Ok(LfsPointer { oid, size, sha256 })
}

/// Verify that `materialized_bytes` matches the LFS pointer's SHA-256 and size.
///
/// Returns `Ok(())` on match; returns the appropriate `LfsError` on mismatch.
pub fn verify_lfs_object(pointer: &LfsPointer, materialized_bytes: &[u8]) -> Result<(), LfsError> {
    // Size check.
    let actual_size = materialized_bytes.len() as u64;
    if actual_size != pointer.size {
        return Err(LfsError::SizeMismatch {
            pointer_size: pointer.size,
            actual_size,
        });
    }

    // SHA-256 check.
    use sha2::{Digest, Sha256};
    let actual_sha256 = hex::encode(Sha256::digest(materialized_bytes));
    if actual_sha256 != pointer.sha256 {
        return Err(LfsError::Sha256Mismatch {
            pointer_sha256: pointer.sha256.clone(),
            actual_sha256,
        });
    }

    Ok(())
}

/// A materialized LFS object: the actual bytes (not the pointer).
#[derive(Debug, Clone)]
pub struct MaterializedLfsObject {
    /// The LFS pointer that was resolved.
    pub pointer: LfsPointer,
    /// The actual object bytes (byte-identical to the LFS server's content).
    pub bytes: Vec<u8>,
}

/// Materialize an LFS pointer using an in-memory blob (test/fixture path).
///
/// In production this would call the GitHub LFS batch API. In tests, a
/// pre-loaded map of sha256 → bytes is used to avoid network calls.
pub fn materialize_lfs_from_fixture(
    pointer: &LfsPointer,
    fixture_store: &std::collections::HashMap<String, Vec<u8>>,
) -> Result<MaterializedLfsObject, LfsError> {
    let bytes = fixture_store
        .get(&pointer.sha256)
        .ok_or_else(|| LfsError::FetchFailed {
            oid: pointer.oid.clone(),
            message: format!("sha256 {} not found in fixture store", pointer.sha256),
        })?
        .clone();

    verify_lfs_object(pointer, &bytes)?;

    Ok(MaterializedLfsObject {
        pointer: pointer.clone(),
        bytes,
    })
}

// ════════════════════════ Real LFS batch-API fetch ══════════════════════════
//
// The Git LFS batch API (git-lfs/docs/api/batch.md):
//   POST {lfs-endpoint}/objects/batch
//   { "operation":"download", "transfers":["basic"],
//     "objects":[ {"oid":"<sha256>","size":<n>} ] }
// → { "transfer":"basic",
//     "objects":[ {"oid":"<sha256>","size":<n>,
//                  "actions":{"download":{"href":"<url>","header":{...}}}} ] }
// then GET the `download.href` to retrieve the actual object bytes.
//
// This module speaks that REAL protocol: it serializes a real batch request,
// parses a real batch response, follows the real `download` href, and
// SHA-256-verifies the fetched bytes. The HTTP itself is abstracted behind
// [`LfsTransport`] so the gate lane can drive it against a real on-disk LFS
// server fixture (NOT an in-memory echo): the fixture parses the JSON request,
// emits a real batch JSON response pointing at hrefs, and serves the object
// bytes from disk. Live GitHub LFS plugs the same trait with HTTPS.

/// HTTP-shaped transport for the LFS batch protocol.
///
/// - `post_batch`: POST a JSON batch request to `{endpoint}/objects/batch`,
///   returning the JSON response body bytes.
/// - `get`: GET a download `href`, returning the raw object bytes.
///
/// Implementations may be live HTTPS (GitHub LFS) or a real on-disk fixture
/// server. They MUST NOT echo the request as the response — they must speak the
/// real protocol (parse request → produce a conformant response / serve bytes).
pub trait LfsTransport {
    /// POST a batch request body to the endpoint; return the response body.
    fn post_batch(&self, endpoint: &str, request_json: &[u8]) -> Result<Vec<u8>, LfsError>;
    /// GET an object by its download href; return the raw bytes.
    fn get(&self, href: &str) -> Result<Vec<u8>, LfsError>;
}

/// Build the real LFS batch *download* request JSON for a set of pointers.
pub fn build_batch_download_request(pointers: &[LfsPointer]) -> Vec<u8> {
    let objects: Vec<serde_json::Value> = pointers
        .iter()
        .map(|p| serde_json::json!({ "oid": p.sha256, "size": p.size }))
        .collect();
    let body = serde_json::json!({
        "operation": "download",
        "transfers": ["basic"],
        "objects": objects,
    });
    serde_json::to_vec(&body).expect("batch request is always serializable")
}

/// A single object's download action parsed from a batch response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchDownloadAction {
    /// The object sha256 (oid without prefix).
    pub sha256: String,
    /// Declared size from the response.
    pub size: u64,
    /// The download href to GET the bytes from.
    pub href: String,
}

/// Parse a real LFS batch response, extracting the download action per object.
///
/// Fails closed: a per-object `error` block, a missing `download` action, or a
/// malformed body is a hard [`LfsError`], never a silent skip.
pub fn parse_batch_response(response_json: &[u8]) -> Result<Vec<BatchDownloadAction>, LfsError> {
    let v: serde_json::Value =
        serde_json::from_slice(response_json).map_err(|e| LfsError::FetchFailed {
            oid: "<batch>".to_string(),
            message: format!("malformed batch response JSON: {e}"),
        })?;
    let objects =
        v.get("objects")
            .and_then(|o| o.as_array())
            .ok_or_else(|| LfsError::FetchFailed {
                oid: "<batch>".to_string(),
                message: "batch response missing `objects` array".to_string(),
            })?;

    let mut actions = Vec::with_capacity(objects.len());
    for obj in objects {
        let oid = obj
            .get("oid")
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string();
        // Fail-closed on a per-object error block.
        if let Some(err) = obj.get("error") {
            let msg = err
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("unknown LFS batch error");
            return Err(LfsError::FetchFailed {
                oid,
                message: format!("batch object error: {msg}"),
            });
        }
        let size =
            obj.get("size")
                .and_then(|s| s.as_u64())
                .ok_or_else(|| LfsError::FetchFailed {
                    oid: oid.clone(),
                    message: "batch object missing size".to_string(),
                })?;
        let href = obj
            .get("actions")
            .and_then(|a| a.get("download"))
            .and_then(|d| d.get("href"))
            .and_then(|h| h.as_str())
            .ok_or_else(|| LfsError::FetchFailed {
                oid: oid.clone(),
                message: "batch object missing download action href".to_string(),
            })?
            .to_string();
        actions.push(BatchDownloadAction {
            sha256: oid,
            size,
            href,
        });
    }
    Ok(actions)
}

/// Materialize an LFS pointer via the **real batch API path**: POST a batch
/// download request, parse the response, GET the download href, and SHA-256-
/// verify the fetched bytes (fail-closed on any mismatch).
///
/// `endpoint` is the LFS endpoint base (e.g.
/// `https://github.com/owner/repo.git/info/lfs`). `transport` performs the
/// actual POST/GET — live HTTPS in production, a real on-disk LFS server
/// fixture in the gate lane.
pub fn materialize_lfs_via_batch<T: LfsTransport>(
    pointer: &LfsPointer,
    endpoint: &str,
    transport: &T,
) -> Result<MaterializedLfsObject, LfsError> {
    let request = build_batch_download_request(std::slice::from_ref(pointer));
    let response = transport.post_batch(endpoint, &request)?;
    let actions = parse_batch_response(&response)?;

    let action = actions
        .into_iter()
        .find(|a| a.sha256 == pointer.sha256)
        .ok_or_else(|| LfsError::FetchFailed {
            oid: pointer.oid.clone(),
            message: format!("batch response had no action for {}", pointer.sha256),
        })?;

    // The fetched bytes are the ACTUAL object, never the pointer.
    let bytes = transport.get(&action.href)?;

    // Fail-closed verification: size + sha256 must match the pointer.
    verify_lfs_object(pointer, &bytes)?;

    Ok(MaterializedLfsObject {
        pointer: pointer.clone(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};

    fn make_fixture_pointer(content: &[u8]) -> (LfsPointer, Vec<u8>) {
        let sha256 = hex::encode(Sha256::digest(content));
        let oid = format!("sha256:{sha256}");
        let size = content.len() as u64;
        let pointer_text =
            format!("version https://git-lfs.github.com/spec/v1\noid {oid}\nsize {size}\n");
        let pointer = LfsPointer { oid, size, sha256 };
        (pointer, pointer_text.into_bytes())
    }

    #[test]
    fn is_lfs_pointer_detects_lfs_blobs() {
        let lfs_blob = b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 100\n";
        assert!(is_lfs_pointer(lfs_blob));
        let regular_blob = b"hello world";
        assert!(!is_lfs_pointer(regular_blob));
    }

    #[test]
    fn parse_lfs_pointer_round_trip() {
        let content = b"actual lfs content bytes";
        let (expected, pointer_bytes) = make_fixture_pointer(content);
        let parsed = parse_lfs_pointer(&pointer_bytes).unwrap();
        assert_eq!(parsed.sha256, expected.sha256);
        assert_eq!(parsed.size, expected.size);
    }

    #[test]
    fn verify_lfs_object_passes_on_correct_bytes() {
        let content = b"lfs object content";
        let (pointer, _) = make_fixture_pointer(content);
        verify_lfs_object(&pointer, content).unwrap();
    }

    #[test]
    fn verify_lfs_object_fails_on_wrong_bytes() {
        let content = b"lfs object content";
        let (pointer, _) = make_fixture_pointer(content);
        let err = verify_lfs_object(&pointer, b"wrong content").unwrap_err();
        // Size mismatch fires first.
        assert!(matches!(err, LfsError::SizeMismatch { .. }));
    }

    #[test]
    fn materialize_lfs_from_fixture_succeeds() {
        let content = b"real lfs object bytes here";
        let (pointer, _) = make_fixture_pointer(content);

        let mut fixture_store = std::collections::HashMap::new();
        fixture_store.insert(pointer.sha256.clone(), content.to_vec());

        let materialized = materialize_lfs_from_fixture(&pointer, &fixture_store).unwrap();
        assert_eq!(materialized.bytes, content);
        assert_eq!(materialized.pointer.sha256, pointer.sha256);
    }

    #[test]
    fn lfs_pointer_not_returned_as_materialized() {
        // The contract: materialized bytes must NOT be the pointer text.
        let content = b"actual binary data that is not a pointer";
        let (pointer, pointer_text) = make_fixture_pointer(content);

        let mut fixture_store = std::collections::HashMap::new();
        fixture_store.insert(pointer.sha256.clone(), content.to_vec());

        let materialized = materialize_lfs_from_fixture(&pointer, &fixture_store).unwrap();
        // The materialized bytes are the actual content, not the pointer.
        assert_ne!(materialized.bytes, pointer_text);
        assert_eq!(materialized.bytes, content.to_vec());
    }
}
