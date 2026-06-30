//! The git-from-CAS read path + ingest — the **content-addressed object seam**.
//!
//! This module is the ONE place the CoreLink CAS wire contract + auth + env live.
//! Everything else in `hugit-serve` consumes the `(Option<Arc<CasObjectSource>>,
//! git_refs, git_root_tree)` tuple this module produces — exactly the same shape
//! `state::load_git_dir` produces — so the blob/edit/clone call sites never learn
//! whether the objects came from a local `git` dir or the live CoreLink CAS.
//!
//! ## The confirmed CoreLink CAS contract
//!
//! CoreLink's CAS is content-addressed AND content-VERIFIED by **BLAKE3-256, a
//! 64-hex lowercase digest**. The store key is `blake3(bytes)`, NOT the git oid:
//! CoreLink recomputes the digest on write and rejects a mismatch (anti-poisoning).
//! git's SHA-1 oid is preserved via a hugit-side `oid → blake3` index kept in R2.
//!
//! - **Serve GET** `GET {CAS_URL}/v1/cas/{tenant}/{blake3}` → 200 raw bytes /
//!   404 absent / 410 erased (both → `Ok(None)`); other → `Err`. Headers:
//!   `Authorization: Bearer <PAT>` + `x-corelink-scope: cas:r`.
//! - **Ingest PUT** `PUT {CAS_URL}/v1/cas/{tenant}/{blake3}` body = bytes;
//!   201 fresh / 200 idempotent; `x-corelink-scope: cas:rw`.
//!
//! The `{blake3}` segment is guarded `^[0-9a-f]{64}$` BEFORE interpolation (a
//! path-traversal / cross-tenant-escape boundary — the same guard the AC client
//! applies to its memo key). The HTTP transport is behind a [`CasTransport`] trait
//! so tests inject an in-memory map and live uses [`UreqCasTransport`].
//!
//! ## Mutable state (NOT CAS — CAS is immutable)
//!
//! Two small JSON objects per repo live in hugit's R2 (the existing
//! [`crate::state::R2Config`] GET path, extended with [`R2Config::get_object`]):
//! - refs manifest `<tenant>/<repo>/refs.json` = [`RefsManifest`];
//! - oid→blake3 index `<tenant>/<repo>/oid-index.json` = a flat JSON map.

use std::collections::{BTreeMap, VecDeque};
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use hugit_proto::{CasObjectSource, GitObject, ObjectKind, PackError};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// B. The BLAKE3 content-address key.
// ─────────────────────────────────────────────────────────────────────────────

/// The CoreLink CAS key for `bytes`: **BLAKE3-256, 64-hex lowercase** (the
/// confirmed contract). `blake3` produces a 256-bit digest by default and
/// `to_hex()` renders it lowercase, so this is exactly CoreLink's key.
///
/// Computed over the EXACT bytes PUT to / GET from the CAS — i.e. the
/// loose-object framing ([`encode_loose`]), never the bare body.
#[must_use]
pub fn cas_key(bytes: &[u8]) -> String {
    blake3::hash(bytes).to_hex().to_string()
}

/// Whether `key` is a canonical CAS digest: exactly 64 lowercase-hex chars.
/// Guards the path segment before interpolation (traversal / cross-tenant escape).
#[must_use]
pub fn is_valid_cas_key(key: &str) -> bool {
    key.len() == 64
        && key
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

// ─────────────────────────────────────────────────────────────────────────────
// C. Loose-object framing — hugit's envelope (CAS stores opaque bytes verbatim).
// ─────────────────────────────────────────────────────────────────────────────

/// The kind token git uses in the loose-object header (`blob`/`tree`/`commit`/`tag`).
fn kind_token(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Blob => "blob",
        ObjectKind::Tree => "tree",
        ObjectKind::Commit => "commit",
        ObjectKind::Tag => "tag",
    }
}

/// Parse a git loose-object kind token; `None` for an unknown token.
fn kind_from_token(token: &[u8]) -> Option<ObjectKind> {
    match token {
        b"blob" => Some(ObjectKind::Blob),
        b"tree" => Some(ObjectKind::Tree),
        b"commit" => Some(ObjectKind::Commit),
        b"tag" => Some(ObjectKind::Tag),
        _ => None,
    }
}

/// Encode a git object into the **loose-object framing** stored in the CAS:
/// `"<kind> <len>\0"` (ASCII) followed by the raw body. This is git's canonical
/// loose pre-image — the SAME bytes git SHA-1 hashes over — so the framing is
/// self-describing and the reader can recover both the kind and the body length.
/// The CAS key is `cas_key` over exactly these bytes.
#[must_use]
pub fn encode_loose(kind: ObjectKind, body: &[u8]) -> Vec<u8> {
    let header = format!("{} {}\0", kind_token(kind), body.len());
    let mut out = Vec::with_capacity(header.len() + body.len());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(body);
    out
}

/// Errors decoding the loose-object framing. Each variant names a distinct
/// malformation so the boot loader / ingest can fail closed with a clear reason.
/// `Display` is hand-written (hugit-serve does not depend on `thiserror`).
#[derive(Debug, PartialEq, Eq)]
pub enum LooseError {
    /// No NUL byte terminating the header.
    NoNul,
    /// The header is not `"<kind> <len>"` (no space, or non-UTF-8).
    BadHeader,
    /// The kind token is not one of blob/tree/commit/tag.
    UnknownKind(String),
    /// The declared length is not a base-10 integer.
    BadLength,
    /// The declared length does not match the actual body length (corruption).
    LengthMismatch {
        /// The length the header declared.
        declared: usize,
        /// The actual body byte count.
        actual: usize,
    },
}

impl std::fmt::Display for LooseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LooseError::NoNul => write!(f, "loose object: missing NUL header terminator"),
            LooseError::BadHeader => write!(f, "loose object: malformed header"),
            LooseError::UnknownKind(k) => write!(f, "loose object: unknown kind token {k:?}"),
            LooseError::BadLength => write!(f, "loose object: length is not a number"),
            LooseError::LengthMismatch { declared, actual } => write!(
                f,
                "loose object: length mismatch (header says {declared}, body is {actual})"
            ),
        }
    }
}

impl std::error::Error for LooseError {}

/// Decode the loose-object framing produced by [`encode_loose`]: parse the
/// `"<kind> <len>\0"` header, validate the declared length against the actual
/// body, and return `(kind, body)`. Rejects every malformation (fail-closed).
pub fn decode_loose(bytes: &[u8]) -> Result<(ObjectKind, Vec<u8>), LooseError> {
    let nul = bytes
        .iter()
        .position(|&b| b == 0)
        .ok_or(LooseError::NoNul)?;
    let header = &bytes[..nul];
    let body = &bytes[nul + 1..];

    let space = header
        .iter()
        .position(|&b| b == b' ')
        .ok_or(LooseError::BadHeader)?;
    let kind_tok = &header[..space];
    let len_tok = &header[space + 1..];

    let kind = kind_from_token(kind_tok)
        .ok_or_else(|| LooseError::UnknownKind(String::from_utf8_lossy(kind_tok).into_owned()))?;
    let len_str = std::str::from_utf8(len_tok).map_err(|_| LooseError::BadLength)?;
    let declared: usize = len_str.parse().map_err(|_| LooseError::BadLength)?;
    if declared != body.len() {
        return Err(LooseError::LengthMismatch {
            declared,
            actual: body.len(),
        });
    }
    Ok((kind, body.to_vec()))
}

// ─────────────────────────────────────────────────────────────────────────────
// A. The CAS object client — the ONE contract-isolated piece.
// ─────────────────────────────────────────────────────────────────────────────

/// CoreLink scope header (transcribed from CoreLink, fail-closed shape).
const SCOPE_HEADER: &str = "x-corelink-scope";
const SCOPE_READ: &str = "cas:r";
const SCOPE_READ_WRITE: &str = "cas:rw";

/// The FROZEN bulk-CAS content type (the native batch plane envelope; CoreLink
/// answers 415 on any other type — modeled in the test double).
const BATCH_CONTENT_TYPE: &str = "application/x-hugit-cas-batch";

/// FROZEN cap: at most this many objects per batch request (upload / read / exists).
pub const BATCH_MAX_OBJECTS: usize = 2000;

/// FROZEN cap: at most this many bytes of OBJECT payload per batch request (the
/// concatenated bytes for upload; the RETURNED `ok` bytes for read). 8 MiB.
pub const BATCH_MAX_BYTES: usize = 8 * 1024 * 1024;

/// CLIENT request-sizing for the read/exists probes (well UNDER the frozen 2000
/// cap — not a contract change). A 2000-object exists/read makes the server do
/// ~2000 R2 round-trips in one request, which blows past the client read-timeout
/// AND the edge's ~100s request ceiling (observed: 2026-06-20 engine boot +
/// git-ingest dedup both timed out at 2000). 256 keeps each request fast; the
/// boot's full read just becomes a few more sequential round-trips.
pub const BATCH_REQUEST_CHUNK: usize = 256;

/// The production default PAT secret-file path, relative to `$HOME`
/// (`~/.hugit/secrets/corelink/pat` — the same handoff path the AC client uses).
const DEFAULT_PAT_FILE_REL: &str = ".hugit/secrets/corelink/pat";

/// A CAS interaction fault. The transport adds I/O variants; the in-memory test
/// double never errors except where it models CoreLink's verify-on-write.
/// `Display` is hand-written (hugit-serve does not depend on `thiserror`); it
/// NEVER renders the PAT (errors only ever name the missing piece).
#[derive(Debug)]
pub enum CasError {
    /// The blake3 key to interpolate is not canonical `^[0-9a-f]{64}$` — a
    /// request-target guard (path-traversal / cross-tenant escape).
    InvalidKey(String),
    /// A transport/protocol failure (network, TLS, I/O).
    Transport(String),
    /// The server answered with a status the contract does not map to a hit
    /// (200), an absence (404/410), or a write success (200/201).
    Status(u16),
    /// Required runtime config (URL/tenant/PAT) was missing/blank. Fail-closed:
    /// the client refuses to call without a credential rather than degrade.
    NotConfigured(String),
    /// The server rejected the batch envelope with **415** (wrong `Content-Type`)
    /// — a hard contract violation, not retryable.
    BatchUnsupportedMediaType,
    /// The bulk (batch) CAS plane is **NOT deployed** in this environment: the
    /// batch route itself answered **405** (Method Not Allowed) or **404** (Not
    /// Found). This is distinct from a per-object 404 (object absence, handled in
    /// [`CasClient::get`]) and from 413/415/400 (which are real bulk-plane
    /// responses): it signals "no bulk plane here", so the caller must transparently
    /// degrade to the always-live single-object [`CasClient::put`]/[`CasClient::get`]
    /// path rather than fail. Carries the observed status (405 or 404).
    BulkUnsupported(u16),
    /// The server rejected the batch with **400** (framing error: manifest/byte
    /// length mismatch or a truncated body) — a hard, non-retryable bug on our
    /// side, surfaced rather than silently dropped.
    BatchFraming(String),
    /// A batch response body did not parse as the contract shape, or a returned
    /// manifest was internally inconsistent (e.g. byte slice out of range).
    BatchMalformedResponse(String),
}

impl std::fmt::Display for CasError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CasError::InvalidKey(w) => {
                write!(f, "CAS invalid key (refusing to build request): {w}")
            }
            CasError::Transport(e) => write!(f, "CAS transport error: {e}"),
            CasError::Status(c) => write!(f, "CAS server returned unexpected HTTP {c}"),
            CasError::NotConfigured(w) => write!(f, "CAS client not configured: {w}"),
            CasError::BatchUnsupportedMediaType => write!(
                f,
                "CAS batch rejected with 415 (wrong Content-Type; expected {BATCH_CONTENT_TYPE})"
            ),
            CasError::BulkUnsupported(code) => write!(
                f,
                "CAS bulk plane not deployed (HTTP {code} on the batch route) — \
                 the caller should fall back to the single-object path"
            ),
            CasError::BatchFraming(w) => write!(f, "CAS batch framing error (HTTP 400): {w}"),
            CasError::BatchMalformedResponse(w) => {
                write!(f, "CAS batch response malformed: {w}")
            }
        }
    }
}

impl std::error::Error for CasError {}

/// The single HTTP exchange against the CoreLink CAS surface — the ONLY part that
/// touches the network. Behind a trait so the route/header building + response
/// mapping are proven hermetically against an in-memory map.
pub trait CasTransport {
    /// `GET {url}` with a Bearer PAT + `cas:r` scope. Returns `(status, body)`;
    /// `body` is meaningful only on 200.
    fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), CasError>;

    /// `PUT {url}` with a Bearer PAT + `cas:rw` scope and a raw body. Returns the
    /// response status.
    fn put(&self, url: &str, bearer: &str, body: &[u8]) -> Result<u16, CasError>;

    /// `POST {url}` with a Bearer PAT, the given `scope` (`cas:r`/`cas:rw`), the
    /// batch `content_type`, and a raw body. Returns `(status, body)` — the bulk
    /// (batch) CAS plane. `body` is meaningful on 200 (the batch result) AND on
    /// 413 (the `batch_too_large` JSON), so it is always returned verbatim.
    fn post(
        &self,
        url: &str,
        bearer: &str,
        scope: &str,
        content_type: &str,
        body: &[u8],
    ) -> Result<(u16, Vec<u8>), CasError>;
}

/// Runtime config for the CAS client: base URL, tenant, and the secret PAT. The
/// PAT is held privately and only ever placed in the `Authorization` header —
/// never logged, never in `Debug`/`Display`/errors.
#[derive(Clone)]
pub struct CasConfig {
    /// CoreLink CAS base URL (e.g. `https://<your-corelink-url>`).
    base_url: String,
    /// The tenant id — first CAS path segment.
    tenant: String,
    /// The CoreLink PAT (Bearer). SECRET — never rendered.
    pat: String,
}

// Hand-written so the PAT can never leak through `{:?}`.
impl std::fmt::Debug for CasConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CasConfig")
            .field("base_url", &self.base_url)
            .field("tenant", &self.tenant)
            .field("pat", &"<redacted>")
            .finish()
    }
}

impl CasConfig {
    /// Build from injected values. Fail-closed: any blank field → `NotConfigured`
    /// naming the piece (never the PAT value).
    pub fn new(
        base_url: impl Into<String>,
        tenant: impl Into<String>,
        pat: impl Into<String>,
    ) -> Result<Self, CasError> {
        let base_url = base_url.into();
        let tenant = tenant.into();
        let pat = pat.into();
        if base_url.trim().is_empty() {
            return Err(CasError::NotConfigured("base URL is empty".into()));
        }
        if tenant.trim().is_empty() {
            return Err(CasError::NotConfigured("tenant is empty".into()));
        }
        if pat.trim().is_empty() {
            return Err(CasError::NotConfigured("PAT is empty".into()));
        }
        Ok(Self {
            base_url,
            tenant,
            pat,
        })
    }

    /// The `Authorization` header value. Crate-private: the secret never escapes
    /// the transport boundary.
    fn bearer(&self) -> String {
        format!("Bearer {}", self.pat)
    }

    /// The full endpoint URL for a blake3 key. The key is validated
    /// `^[0-9a-f]{64}$` BEFORE interpolation (a key carrying path separators could
    /// escape the `/v1/cas/{tenant}/` prefix), so a non-canonical key is rejected
    /// rather than built into a request.
    fn endpoint(&self, blake3: &str) -> Result<String, CasError> {
        if !is_valid_cas_key(blake3) {
            return Err(CasError::InvalidKey(format!(
                "CAS key must match ^[0-9a-f]{{64}}$ (got {} chars)",
                blake3.len()
            )));
        }
        Ok(format!(
            "{}/v1/cas/{}/{}",
            self.base_url.trim_end_matches('/'),
            self.tenant,
            blake3
        ))
    }

    /// The bulk-upload endpoint: `POST {base}/v1/cas/{tenant}/batch` (`cas:rw`).
    fn batch_endpoint(&self) -> String {
        format!(
            "{}/v1/cas/{}/batch",
            self.base_url.trim_end_matches('/'),
            self.tenant
        )
    }

    /// The bulk-read endpoint: `POST {base}/v1/cas/{tenant}/batch-read` (`cas:r`).
    fn batch_read_endpoint(&self) -> String {
        format!(
            "{}/v1/cas/{}/batch-read",
            self.base_url.trim_end_matches('/'),
            self.tenant
        )
    }

    /// The bulk-exists endpoint: `POST {base}/v1/cas/{tenant}/batch-exists` (`cas:r`).
    fn batch_exists_endpoint(&self) -> String {
        format!(
            "{}/v1/cas/{}/batch-exists",
            self.base_url.trim_end_matches('/'),
            self.tenant
        )
    }
}

/// The CoreLink CAS object client over a pluggable [`CasTransport`].
///
/// The route/header/mapping LOGIC lives here and is proven hermetically (mock
/// transport). A CONFIGURED client speaks the real contract; an un-configured one
/// fails closed (it has no `CasConfig`).
#[derive(Debug, Clone)]
pub struct CasClient<T: CasTransport = UreqCasTransport> {
    config: CasConfig,
    transport: T,
}

/// The result of a bulk read: each requested blake3 paired with its bytes
/// (`Some` for an `ok` object) or `None` (`absent`/`gone`), in request order.
pub type BatchReadResult = Vec<(String, Option<Vec<u8>>)>;

impl<T: CasTransport> CasClient<T> {
    /// Construct over an explicit transport + config. Used in tests to inject a
    /// mock transport; in production `T = UreqCasTransport`.
    pub fn with_transport(config: CasConfig, transport: T) -> Self {
        Self { config, transport }
    }

    /// Fetch a CAS object by its blake3 key. Maps the confirmed contract:
    /// - 200 → `Ok(Some(bytes))`
    /// - 404 (absent) / 410 (erased) → `Ok(None)`
    /// - anything else → `Err(CasError::Status)`.
    pub fn get(&self, blake3: &str) -> Result<Option<Vec<u8>>, CasError> {
        let url = self.config.endpoint(blake3)?;
        let (status, body) = self.transport.get(&url, &self.config.bearer())?;
        match status {
            200 => Ok(Some(body)),
            404 | 410 => Ok(None),
            other => Err(CasError::Status(other)),
        }
    }

    /// PUT a CAS object under its blake3 key (`cas:rw`). 201 fresh / 200
    /// idempotent are both success; anything else is `Err`.
    ///
    /// NOTE: CoreLink recomputes blake3 over the body on write and rejects a
    /// mismatch (anti-poisoning) — so `blake3` MUST equal `cas_key(body)`. The
    /// ingest path always passes the freshly-computed key, so this holds by
    /// construction; the in-memory test double models the rejection.
    pub fn put(&self, blake3: &str, body: &[u8]) -> Result<(), CasError> {
        let url = self.config.endpoint(blake3)?;
        let status = self.transport.put(&url, &self.config.bearer(), body)?;
        match status {
            200 | 201 => Ok(()),
            other => Err(CasError::Status(other)),
        }
    }

    // ── Bulk (batch) CAS plane ────────────────────────────────────────────────

    /// Bulk-upload `objects` (`(blake3, framed-bytes)`) to the CAS in one or more
    /// batches that each respect the FROZEN caps (≤ [`BATCH_MAX_OBJECTS`] objects
    /// AND ≤ [`BATCH_MAX_BYTES`] of object bytes). For each batch:
    /// build the NDJSON manifest + blank line + concatenated bytes, POST it
    /// (`cas:rw`, [`BATCH_CONTENT_TYPE`]), and parse the per-object status array.
    ///
    /// Resilience to the FROZEN contract's edges:
    /// - **413** `batch_too_large` → the chunk is split in HALF (deterministic) and
    ///   each half retried — so a stale local cap never wedges the upload.
    /// - **415** → hard [`CasError::BatchUnsupportedMediaType`] (contract bug).
    /// - **400** → hard [`CasError::BatchFraming`] (our framing is wrong).
    /// - a per-object `error` status is returned to the caller (NOT retried here —
    ///   the ingest layer decides the retry policy); `created`/`exists` are success.
    ///
    /// Returns the per-object `(blake3, UploadStatus)` in INPUT order. Every input
    /// object appears exactly once (statuses are aggregated across chunks).
    pub fn batch_upload(
        &self,
        objects: &[(String, Vec<u8>)],
    ) -> Result<Vec<(String, UploadStatus)>, CasError> {
        let mut out: Vec<(String, UploadStatus)> = Vec::with_capacity(objects.len());
        for chunk in chunk_by_caps(objects, |(_, bytes)| bytes.len()) {
            self.batch_upload_chunk(chunk, &mut out)?;
        }
        Ok(out)
    }

    /// Upload one already-cap-respecting chunk, appending its statuses to `out`.
    /// On a 413 (a stale/over-tight cap raced the server's), split the chunk in
    /// half and recurse — guaranteeing termination because a singleton chunk that
    /// still 413s is a hard server-side rejection surfaced as an `error` status.
    fn batch_upload_chunk(
        &self,
        chunk: &[(String, Vec<u8>)],
        out: &mut Vec<(String, UploadStatus)>,
    ) -> Result<(), CasError> {
        if chunk.is_empty() {
            return Ok(());
        }
        let body = build_upload_body(chunk);
        let (status, resp) = match self.transport.post(
            &self.config.batch_endpoint(),
            &self.config.bearer(),
            SCOPE_READ_WRITE,
            BATCH_CONTENT_TYPE,
            &body,
        ) {
            Ok(pair) => pair,
            // A transport error — typically the server taking longer than the read
            // timeout to process a large batch (write N objects to R2), amplified
            // under local CPU contention. Split like a 413 so the batch adaptively
            // shrinks to fit the timeout window; CAS upload is idempotent
            // (content-addressed), so re-sending a half is safe. A singleton that
            // still fails surfaces as that object's per-object error.
            Err(CasError::Transport(msg)) => {
                if chunk.len() == 1 {
                    out.push((
                        chunk[0].0.clone(),
                        UploadStatus::Error(format!("transport: {msg}")),
                    ));
                    return Ok(());
                }
                let mid = chunk.len() / 2;
                self.batch_upload_chunk(&chunk[..mid], out)?;
                return self.batch_upload_chunk(&chunk[mid..], out);
            }
            Err(e) => return Err(e),
        };
        match status {
            200 => {
                let parsed = parse_upload_response(&resp, chunk)?;
                out.extend(parsed);
                Ok(())
            }
            413 => {
                // Over a cap the server enforces. A singleton cannot be split — a
                // single object over the cap is unservable, so surface it as an
                // `error` for that object (the contract's per-object error lane).
                if chunk.len() == 1 {
                    out.push((
                        chunk[0].0.clone(),
                        UploadStatus::Error("batch_too_large".into()),
                    ));
                    return Ok(());
                }
                let mid = chunk.len() / 2;
                self.batch_upload_chunk(&chunk[..mid], out)?;
                self.batch_upload_chunk(&chunk[mid..], out)
            }
            // The bulk plane is not deployed here (the batch route itself does not
            // exist) — signal the caller to degrade to per-object uploads.
            404 | 405 => Err(CasError::BulkUnsupported(status)),
            415 => Err(CasError::BatchUnsupportedMediaType),
            400 => Err(CasError::BatchFraming(
                "server rejected the upload manifest/byte framing".into(),
            )),
            other => Err(CasError::Status(other)),
        }
    }

    /// Bulk-read the objects named by `hashes` from the CAS in cap-respecting
    /// batches. Because the RETURNED byte sizes are unknown a priori, chunks are
    /// formed by ≤ [`BATCH_MAX_OBJECTS`] hashes; a **413** (the server caps on the
    /// RETURNED `ok` bytes) splits that chunk in half and retries — so an
    /// unexpectedly-large set of returned blobs always converges.
    ///
    /// Returns `(blake3, Option<bytes>)` in INPUT order: `Some` for an `ok` object,
    /// `None` for `absent`/`gone`. The caller (the boot loader) fails closed on a
    /// `None` it required. 415/400 are hard errors (contract bugs).
    pub fn batch_read(&self, hashes: &[String]) -> Result<BatchReadResult, CasError> {
        let mut out: BatchReadResult = Vec::with_capacity(hashes.len());
        // Read is bounded by RETURNED bytes (unknown here), so chunk by count only
        // — and by BATCH_REQUEST_CHUNK (256), NOT the frozen 2000 cap: a 2000-object
        // read makes the server do ~2000 R2 fetches in one request → boot timeout.
        // and let the 413-split handle an oversized return.
        for chunk in hashes.chunks(BATCH_REQUEST_CHUNK) {
            self.batch_read_chunk(chunk, &mut out)?;
        }
        Ok(out)
    }

    fn batch_read_chunk(
        &self,
        chunk: &[String],
        out: &mut BatchReadResult,
    ) -> Result<(), CasError> {
        if chunk.is_empty() {
            return Ok(());
        }
        // Guard every requested hash BEFORE building a request (path/cross-tenant
        // discipline parity with the single-object endpoint).
        for h in chunk {
            if !is_valid_cas_key(h) {
                return Err(CasError::InvalidKey(format!(
                    "batch-read hash must match ^[0-9a-f]{{64}}$ (got {} chars)",
                    h.len()
                )));
            }
        }
        let body = build_hash_request(chunk);
        let (status, resp) = self.transport.post(
            &self.config.batch_read_endpoint(),
            &self.config.bearer(),
            SCOPE_READ,
            BATCH_CONTENT_TYPE,
            &body,
        )?;
        match status {
            200 => {
                let parsed = parse_read_response(&resp, chunk)?;
                out.extend(parsed);
                Ok(())
            }
            413 => {
                if chunk.len() == 1 {
                    // A single object whose bytes exceed the cap cannot be batch-read;
                    // fall back to the single-object GET (which streams it whole).
                    let bytes = self.get(&chunk[0])?;
                    out.push((chunk[0].clone(), bytes));
                    return Ok(());
                }
                let mid = chunk.len() / 2;
                self.batch_read_chunk(&chunk[..mid], out)?;
                self.batch_read_chunk(&chunk[mid..], out)
            }
            // The bulk plane is not deployed here — degrade to per-object GET.
            404 | 405 => Err(CasError::BulkUnsupported(status)),
            415 => Err(CasError::BatchUnsupportedMediaType),
            400 => Err(CasError::BatchFraming(
                "server rejected the batch-read hash-list framing".into(),
            )),
            other => Err(CasError::Status(other)),
        }
    }

    /// Bulk-existence probe for `hashes` (the dedup primitive). Chunks by
    /// ≤ [`BATCH_MAX_OBJECTS`] hashes (the request body is tiny — only the count cap
    /// applies). Returns `(blake3, present)` in INPUT order. 415/400 → hard error.
    pub fn batch_exists(&self, hashes: &[String]) -> Result<Vec<(String, bool)>, CasError> {
        let mut out: Vec<(String, bool)> = Vec::with_capacity(hashes.len());
        for chunk in hashes.chunks(BATCH_REQUEST_CHUNK) {
            if chunk.is_empty() {
                continue;
            }
            for h in chunk {
                if !is_valid_cas_key(h) {
                    return Err(CasError::InvalidKey(format!(
                        "batch-exists hash must match ^[0-9a-f]{{64}}$ (got {} chars)",
                        h.len()
                    )));
                }
            }
            let body = build_hash_request(chunk);
            let (status, resp) = self.transport.post(
                &self.config.batch_exists_endpoint(),
                &self.config.bearer(),
                SCOPE_READ,
                BATCH_CONTENT_TYPE,
                &body,
            )?;
            match status {
                200 => out.extend(parse_exists_response(&resp, chunk)?),
                // The bulk plane is not deployed here — signal per-object fallback.
                404 | 405 => return Err(CasError::BulkUnsupported(status)),
                415 => return Err(CasError::BatchUnsupportedMediaType),
                400 => {
                    return Err(CasError::BatchFraming(
                        "server rejected the batch-exists hash-list framing".into(),
                    ));
                }
                other => return Err(CasError::Status(other)),
            }
        }
        Ok(out)
    }
}

/// The per-object outcome of a [`CasClient::batch_upload`]. `Created`/`Exists` are
/// success (the object is in the CAS); `Error` carries the server's message and is
/// the only status the ingest layer retries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadStatus {
    /// The object was freshly written.
    Created,
    /// The object was already present (dedup hit) — success, no write.
    Exists,
    /// The object FAILED (hash mismatch / R2 fault); the message is the server's.
    Error(String),
}

// ── Batch wire framing (NDJSON manifest + blank line + concatenated bytes) ─────

/// One manifest line on the UPLOAD request / READ response: `{"hash","len":…}`.
#[derive(Serialize, Deserialize)]
struct ManifestLine {
    hash: String,
    len: u64,
}

/// One per-hash line on a READ request / EXISTS request: `{"hash":…}`.
#[derive(Serialize, Deserialize)]
struct HashLine {
    hash: String,
}

/// One element of the UPLOAD response array: `{"hash","status","error"}`.
#[derive(Deserialize)]
struct UploadResult {
    hash: String,
    status: String,
    #[serde(default)]
    error: Option<String>,
}

/// One element of the EXISTS response array: `{"hash","present"}`.
#[derive(Deserialize)]
struct ExistsResult {
    hash: String,
    present: bool,
}

/// One manifest line on a READ response: `{"hash","len","status":"ok|absent|gone"}`.
#[derive(Deserialize)]
struct ReadManifestLine {
    hash: String,
    len: u64,
    status: String,
}

/// Split `items` into consecutive chunks that each fit BOTH caps: ≤
/// [`BATCH_MAX_OBJECTS`] items AND ≤ [`BATCH_MAX_BYTES`] of summed `size`. A single
/// item larger than the byte cap is emitted as its own (over-cap) singleton chunk —
/// the caller's 413-split then surfaces it as a per-object error rather than
/// looping. Deterministic: the same input always splits the same way.
fn chunk_by_caps<T>(items: &[T], size: impl Fn(&T) -> usize) -> Vec<&[T]> {
    let mut chunks: Vec<&[T]> = Vec::new();
    let mut start = 0;
    let mut cur_bytes = 0usize;
    let mut i = 0;
    while i < items.len() {
        let sz = size(&items[i]);
        let count = i - start;
        // Would adding item `i` break a cap? (And the chunk is non-empty so we can
        // close it without producing an empty chunk.)
        let breaks_count = count >= BATCH_MAX_OBJECTS;
        let breaks_bytes = count > 0 && cur_bytes + sz > BATCH_MAX_BYTES;
        if breaks_count || breaks_bytes {
            chunks.push(&items[start..i]);
            start = i;
            cur_bytes = 0;
            continue; // re-evaluate item `i` as the head of the new chunk.
        }
        cur_bytes += sz;
        i += 1;
    }
    if start < items.len() {
        chunks.push(&items[start..]);
    }
    chunks
}

/// Build the upload request body: NDJSON manifest (`{"hash","len"}` per object, in
/// order) + a single blank line + the concatenated raw bytes (each exactly `len`).
fn build_upload_body(chunk: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut body = Vec::new();
    for (hash, bytes) in chunk {
        let line = ManifestLine {
            hash: hash.clone(),
            len: bytes.len() as u64,
        };
        // `to_writer`-style: serialize then push a newline (NDJSON).
        body.extend_from_slice(serde_json::to_vec(&line).unwrap_or_default().as_slice());
        body.push(b'\n');
    }
    body.push(b'\n'); // the blank line terminating the manifest.
    for (_, bytes) in chunk {
        body.extend_from_slice(bytes);
    }
    body
}

/// Build a hash-list request body (READ / EXISTS): NDJSON `{"hash":…}` lines.
fn build_hash_request(chunk: &[String]) -> Vec<u8> {
    let mut body = Vec::new();
    for hash in chunk {
        let line = HashLine { hash: hash.clone() };
        body.extend_from_slice(serde_json::to_vec(&line).unwrap_or_default().as_slice());
        body.push(b'\n');
    }
    body
}

/// Parse the UPLOAD response (a JSON array of `{"hash","status","error"}`) into
/// per-object `(blake3, UploadStatus)` in the SAME order as `chunk`. The server is
/// partial-failure tolerant, so the array may interleave; we re-key by hash and
/// emit in input order. A hash present in the request but absent from the response
/// is a malformed response (fail closed).
fn parse_upload_response(
    resp: &[u8],
    chunk: &[(String, Vec<u8>)],
) -> Result<Vec<(String, UploadStatus)>, CasError> {
    let results: Vec<UploadResult> = serde_json::from_slice(resp).map_err(|e| {
        CasError::BatchMalformedResponse(format!("upload response is not a status array: {e}"))
    })?;
    let mut by_hash: BTreeMap<String, UploadStatus> = BTreeMap::new();
    for r in results {
        let status = match r.status.as_str() {
            "created" => UploadStatus::Created,
            "exists" => UploadStatus::Exists,
            "error" => UploadStatus::Error(r.error.unwrap_or_else(|| "unspecified".into())),
            other => {
                return Err(CasError::BatchMalformedResponse(format!(
                    "unknown upload status {other:?} for {}",
                    r.hash
                )));
            }
        };
        by_hash.insert(r.hash, status);
    }
    let mut out = Vec::with_capacity(chunk.len());
    for (hash, _) in chunk {
        let status = by_hash.remove(hash).ok_or_else(|| {
            CasError::BatchMalformedResponse(format!(
                "upload response omitted a status for requested object {hash}"
            ))
        })?;
        out.push((hash.clone(), status));
    }
    Ok(out)
}

/// Parse the READ response (manifest NDJSON + blank line + concatenated bytes) into
/// `(blake3, Option<bytes>)` in the SAME order as `chunk`. `ok` → `Some(bytes)`
/// sliced by `len`; `absent`/`gone` → `None` (no bytes). Fails closed on a manifest
/// that does not split cleanly, a byte slice out of range, or an unknown status.
fn parse_read_response(resp: &[u8], chunk: &[String]) -> Result<BatchReadResult, CasError> {
    // Split on the FIRST blank line (`\n\n`): manifest, then the byte region.
    let sep = find_blank_line(resp).ok_or_else(|| {
        CasError::BatchMalformedResponse(
            "batch-read response has no manifest/body separator".into(),
        )
    })?;
    let manifest_region = &resp[..sep];
    let bytes_region = &resp[sep + 2..]; // skip the "\n\n".

    let mut manifest: Vec<ReadManifestLine> = Vec::new();
    for line in manifest_region.split(|&b| b == b'\n') {
        if line.is_empty() {
            continue;
        }
        let parsed: ReadManifestLine = serde_json::from_slice(line).map_err(|e| {
            CasError::BatchMalformedResponse(format!("batch-read manifest line invalid: {e}"))
        })?;
        manifest.push(parsed);
    }

    // Walk the manifest in order, slicing the concatenated bytes for `ok` entries.
    let mut by_hash: BTreeMap<String, Option<Vec<u8>>> = BTreeMap::new();
    let mut cursor = 0usize;
    for line in &manifest {
        match line.status.as_str() {
            "ok" => {
                let len = line.len as usize;
                let end = cursor
                    .checked_add(len)
                    .filter(|&e| e <= bytes_region.len())
                    .ok_or_else(|| {
                        CasError::BatchMalformedResponse(format!(
                            "batch-read byte slice for {} (len {len}) runs past the body \
                             (cursor {cursor}, body {})",
                            line.hash,
                            bytes_region.len()
                        ))
                    })?;
                by_hash.insert(line.hash.clone(), Some(bytes_region[cursor..end].to_vec()));
                cursor = end;
            }
            "absent" | "gone" => {
                by_hash.insert(line.hash.clone(), None);
            }
            other => {
                return Err(CasError::BatchMalformedResponse(format!(
                    "unknown batch-read status {other:?} for {}",
                    line.hash
                )));
            }
        }
    }

    let mut out = Vec::with_capacity(chunk.len());
    for hash in chunk {
        let entry = by_hash.remove(hash).ok_or_else(|| {
            CasError::BatchMalformedResponse(format!(
                "batch-read response omitted requested hash {hash}"
            ))
        })?;
        out.push((hash.clone(), entry));
    }
    Ok(out)
}

/// Parse the EXISTS response (JSON array of `{"hash","present"}`) into
/// `(blake3, present)` in the SAME order as `chunk`.
fn parse_exists_response(resp: &[u8], chunk: &[String]) -> Result<Vec<(String, bool)>, CasError> {
    let results: Vec<ExistsResult> = serde_json::from_slice(resp).map_err(|e| {
        CasError::BatchMalformedResponse(format!("exists response is not a present array: {e}"))
    })?;
    let mut by_hash: BTreeMap<String, bool> = BTreeMap::new();
    for r in results {
        by_hash.insert(r.hash, r.present);
    }
    let mut out = Vec::with_capacity(chunk.len());
    for hash in chunk {
        let present = by_hash.remove(hash).ok_or_else(|| {
            CasError::BatchMalformedResponse(format!(
                "exists response omitted requested hash {hash}"
            ))
        })?;
        out.push((hash.clone(), present));
    }
    Ok(out)
}

/// Find the byte offset of the FIRST blank line (`\n\n`) in `buf` — the
/// manifest/body separator. Returns the index of the first `\n` of the pair.
fn find_blank_line(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

impl CasClient<UreqCasTransport> {
    /// Build a CONFIGURED client over the real `ureq` transport from explicit
    /// values (URL/tenant/PAT).
    pub fn configured(
        base_url: impl Into<String>,
        tenant: impl Into<String>,
        pat: impl Into<String>,
    ) -> Result<Self, CasError> {
        Ok(Self {
            config: CasConfig::new(base_url, tenant, pat)?,
            transport: UreqCasTransport::new(),
        })
    }

    /// Build a CONFIGURED client from the env (the deploy plug-and-play path):
    /// - `HUGIT_SERVE_CAS_URL` — the CAS base URL (= `https://<your-corelink-url>`),
    /// - `HUGIT_SERVE_CAS_TENANT_ID` — the tenant id (first CAS path segment),
    /// - the PAT from `HUGIT_SERVE_CAS_PAT_FILE` (preferred; default
    ///   `~/.hugit/secrets/corelink/pat`, trailing newline trimmed), falling back
    ///   to `HUGIT_SERVE_CAS_PAT` ONLY if the file is absent.
    ///
    /// Fail-closed: a missing/blank piece → `NotConfigured` naming it (never the
    /// PAT value); the PAT is held only inside `CasConfig`.
    pub fn from_env() -> Result<Self, CasError> {
        let base_url = read_required_env("HUGIT_SERVE_CAS_URL")?;
        let tenant = read_required_env("HUGIT_SERVE_CAS_TENANT_ID")?;
        let pat = read_cas_pat()?;
        Self::configured(base_url, tenant, pat)
    }
}

/// Read a required, non-blank env var, else `NotConfigured` naming it.
fn read_required_env(name: &str) -> Result<String, CasError> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Ok(v),
        _ => Err(CasError::NotConfigured(format!("{name} unset/empty"))),
    }
}

/// Resolve the CAS PAT: file-preferred (`HUGIT_SERVE_CAS_PAT_FILE` override, else
/// `~/.hugit/secrets/corelink/pat`), env-fallback (`HUGIT_SERVE_CAS_PAT`) only if
/// the file is absent. Mirrors the AC client's secret-file discipline (mode-guard
/// on Unix, blank-is-missing). Never returns the value in an error.
fn read_cas_pat() -> Result<String, CasError> {
    let pat_file = resolve_cas_pat_file()?;
    match std::fs::read_to_string(&pat_file) {
        Ok(contents) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let meta = std::fs::metadata(&pat_file).map_err(|e| {
                    CasError::NotConfigured(format!(
                        "PAT: cannot stat secret file {}: {}",
                        pat_file.display(),
                        e.kind()
                    ))
                })?;
                let mode = meta.permissions().mode();
                if mode & 0o077 != 0 {
                    return Err(CasError::NotConfigured(format!(
                        "PAT: secret file {} has insecure permissions {:o} \
                         (group/other access); chmod 600 it",
                        pat_file.display(),
                        mode & 0o777
                    )));
                }
            }
            let pat = contents.trim_end().to_string();
            if pat.is_empty() {
                return Err(CasError::NotConfigured(format!(
                    "PAT: secret file {} is empty",
                    pat_file.display()
                )));
            }
            Ok(pat)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            match std::env::var("HUGIT_SERVE_CAS_PAT") {
                Ok(v) if !v.trim().is_empty() => Ok(v),
                _ => Err(CasError::NotConfigured(format!(
                    "PAT: secret file {} absent and HUGIT_SERVE_CAS_PAT unset/empty",
                    pat_file.display()
                ))),
            }
        }
        Err(e) => Err(CasError::NotConfigured(format!(
            "PAT: cannot read secret file {}: {}",
            pat_file.display(),
            e.kind()
        ))),
    }
}

/// Resolve the CAS PAT secret-file path: an explicit `HUGIT_SERVE_CAS_PAT_FILE`
/// override wins; else `~/.hugit/secrets/corelink/pat` against `$HOME`.
fn resolve_cas_pat_file() -> Result<PathBuf, CasError> {
    if let Ok(p) = std::env::var("HUGIT_SERVE_CAS_PAT_FILE")
        && !p.trim().is_empty()
    {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var("HOME").map_err(|_| {
        CasError::NotConfigured(
            "PAT: no HUGIT_SERVE_CAS_PAT_FILE override and $HOME is unset (cannot locate \
             the default ~/.hugit/secrets/corelink/pat)"
                .to_string(),
        )
    })?;
    Ok(Path::new(&home).join(DEFAULT_PAT_FILE_REL))
}

/// The real `ureq`-backed CAS transport — the thin network seam, the ONLY code
/// here that opens a socket. `ureq` is already in the workspace lock.
#[derive(Debug, Clone)]
pub struct UreqCasTransport {
    agent: ureq::Agent,
}

impl Default for UreqCasTransport {
    fn default() -> Self {
        Self::new()
    }
}

impl UreqCasTransport {
    /// A transport with a bounded timeout (no hanging reads).
    #[must_use]
    pub fn new() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(90))
                .build(),
        }
    }
}

/// Retry a ureq call expression on a throttle status (`429`/`503`) with exponential
/// backoff (200ms · 2^n, capped; up to 6 attempts ≈ ~13s). The CAS rate-limits a
/// burst — e.g. a `git-ingest` dedup/upload of thousands of objects — so every CAS
/// call backs off instead of failing the whole operation on the first 429. Written
/// as a macro (not a fn) so the large `ureq::Error` is never exposed in a signature
/// (clippy::result_large_err); the result lands in a local binding. ureq surfaces
/// non-2xx as `Err(Status(code, _))`, so a throttle lands in the retry arm.
macro_rules! with_throttle_retry {
    ($call:expr) => {{
        let mut attempt: u32 = 0;
        loop {
            match $call {
                Err(ureq::Error::Status(code, resp))
                    if (code == 429 || code == 503) && attempt < 6 =>
                {
                    let _ = resp; // drop the throttle response (Retry-After not surfaced here)
                    std::thread::sleep(Duration::from_millis(200u64 * (1u64 << attempt.min(5))));
                    attempt += 1;
                }
                other => break other,
            }
        }
    }};
}

impl CasTransport for UreqCasTransport {
    fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), CasError> {
        let resp = with_throttle_retry!(
            self.agent
                .get(url)
                .set("Authorization", bearer)
                .set(SCOPE_HEADER, SCOPE_READ)
                .call()
        );
        match resp {
            Ok(r) => {
                let status = r.status();
                let mut buf = Vec::new();
                r.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|e| CasError::Transport(e.to_string()))?;
                Ok((status, buf))
            }
            // ureq surfaces non-2xx as `Error::Status(code, resp)`; map it so the
            // protocol logic can branch (esp. 404/410 → absent).
            Err(ureq::Error::Status(code, _resp)) => Ok((code, Vec::new())),
            Err(e) => Err(CasError::Transport(e.to_string())),
        }
    }

    fn put(&self, url: &str, bearer: &str, body: &[u8]) -> Result<u16, CasError> {
        let resp = with_throttle_retry!(
            self.agent
                .put(url)
                .set("Authorization", bearer)
                .set(SCOPE_HEADER, SCOPE_READ_WRITE)
                .set("Content-Type", "application/octet-stream")
                .send_bytes(body)
        );
        match resp {
            Ok(r) => Ok(r.status()),
            Err(ureq::Error::Status(code, _resp)) => Ok(code),
            Err(e) => Err(CasError::Transport(e.to_string())),
        }
    }

    fn post(
        &self,
        url: &str,
        bearer: &str,
        scope: &str,
        content_type: &str,
        body: &[u8],
    ) -> Result<(u16, Vec<u8>), CasError> {
        let resp = with_throttle_retry!(
            self.agent
                .post(url)
                .set("Authorization", bearer)
                .set(SCOPE_HEADER, scope)
                .set("Content-Type", content_type)
                .send_bytes(body)
        );
        match resp {
            Ok(r) => {
                let status = r.status();
                let mut buf = Vec::new();
                r.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|e| CasError::Transport(e.to_string()))?;
                Ok((status, buf))
            }
            // Non-2xx surfaces as `Error::Status(code, resp)`; capture the body too
            // (413 carries the `batch_too_large` JSON the splitter needs to see —
            // though the splitter only branches on the code, the body is preserved).
            //
            // SECURITY: cap the error body read at 64 KiB. A hostile or buggy CAS
            // returning a multi-megabyte error body must not OOM the engine.
            Err(ureq::Error::Status(code, resp)) => {
                const MAX_ERR_BODY: u64 = 64 * 1024;
                let mut buf = Vec::new();
                let _ = resp.into_reader().take(MAX_ERR_BODY).read_to_end(&mut buf);
                Ok((code, buf))
            }
            Err(e) => Err(CasError::Transport(e.to_string())),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// D. Mutable state in hugit's R2 — refs.json + oid-index.json.
// ─────────────────────────────────────────────────────────────────────────────

/// The refs manifest stored at `<tenant>/<repo>/refs.json`: HEAD's symbolic ref
/// name plus the full `refname → git-sha1-oid` map. The `git_refs` AppState field
/// is `manifest.refs` verbatim.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefsManifest {
    /// The symbolic HEAD ref name (e.g. `refs/heads/main`).
    pub head: String,
    /// `refname → git-sha1-oid` (hex), e.g. `refs/heads/main → <40-hex>`.
    pub refs: BTreeMap<String, String>,
}

/// The `git-sha1-oid → blake3-64hex` index stored at `<tenant>/<repo>/oid-index.json`.
pub type OidIndex = BTreeMap<String, String>;

/// Parse a refs manifest from raw JSON bytes.
pub fn parse_refs_manifest(bytes: &[u8]) -> Result<RefsManifest, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("refs.json parse failed: {e}"))
}

/// Parse the oid→blake3 index from raw JSON bytes.
pub fn parse_oid_index(bytes: &[u8]) -> Result<OidIndex, String> {
    serde_json::from_slice(bytes).map_err(|e| format!("oid-index.json parse failed: {e}"))
}

/// The R2 key for a repo's refs manifest: `<tenant>/<repo>/refs.json`.
#[must_use]
pub fn refs_manifest_key(tenant: &str, repo: &str) -> String {
    format!("{tenant}/{repo}/refs.json")
}

/// The R2 key for a repo's oid→blake3 index: `<tenant>/<repo>/oid-index.json`.
#[must_use]
pub fn oid_index_key(tenant: &str, repo: &str) -> String {
    format!("{tenant}/{repo}/oid-index.json")
}

// ─────────────────────────────────────────────────────────────────────────────
// E. The boot loader — `load_from_cas` (DOUBLE integrity).
// ─────────────────────────────────────────────────────────────────────────────

/// The mutable-state source the loader reads `refs.json` + `oid-index.json` from:
/// hugit's R2 (the existing `R2Config::get_object`). Behind a trait so the loader
/// is proven hermetically against an in-memory double — `Ok(None)` = absent.
pub trait R2Get {
    /// Fetch an R2 object by its full key. `Ok(None)` = absent; `Err` = transport.
    fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, String>;
}

/// The output of [`load_from_cas`] — IDENTICAL in shape to `state::load_git_dir`,
/// so the blob/edit/clone call sites are untouched regardless of source.
pub type CasLoad = (
    CasObjectSource,
    gix_hash::ObjectId,
    BTreeMap<String, String>,
);

/// The output of [`load_manifests_from_cas`] — the LAZY counterpart of [`CasLoad`]:
/// a [`LazyCasObjectSource`] (objects fetched on demand) plus the same root-tree +
/// refs. Same shape as `CasLoad`, so the call sites are untouched.
pub type LazyCasLoad<T = UreqCasTransport> = (
    LazyCasObjectSource<T>,
    gix_hash::ObjectId,
    BTreeMap<String, String>,
);

/// Boot-load a repo's git objects from hugit's R2 (mutable manifests) + the
/// CoreLink CAS (immutable objects), with **DOUBLE integrity**:
///
/// 1. fetch `<tenant>/<repo>/refs.json` + `oid-index.json` from R2 (`r2`);
/// 2. for each `(git_oid, blake3)` in the index: `cas.get(blake3)` →
///    [`decode_loose`] → `CasObjectSource::insert_raw(kind, body)`, which
///    re-derives the **git SHA-1** and (via the store's content-address invariant)
///    will reject a mismatch on the later `get`;
/// 3. ALSO assert here that the re-derived git SHA-1 equals the `git_oid` the index
///    requested — so **both** the CoreLink BLAKE3 (the CAS verified on its read)
///    AND git's SHA-1 (verified here) are checked (defense in depth);
/// 4. `git_refs = manifest.refs`; `git_root_tree` = the root-tree oid of the commit
///    at `manifest.refs[manifest.head]` (parsed via `gix_object::CommitRefIter`).
///
/// Fail-closed on a missing manifest/index/object, a malformed loose object, or
/// ANY oid mismatch — a misconfigured content seam refuses to start rather than
/// silently serving 404s. Produces the SAME tuple as `load_git_dir`.
pub fn load_from_cas<T: CasTransport, R: R2Get>(
    cas: &CasClient<T>,
    r2: &R,
    tenant: &str,
    repo: &str,
) -> Result<CasLoad, String> {
    // 1. The mutable manifests from R2 (fail-closed on absence).
    let refs_bytes = r2
        .get_object(&refs_manifest_key(tenant, repo))?
        .ok_or_else(|| format!("refs.json absent for {tenant}/{repo} (content seam not seeded)"))?;
    let manifest = parse_refs_manifest(&refs_bytes)?;

    let index_bytes = r2
        .get_object(&oid_index_key(tenant, repo))?
        .ok_or_else(|| {
            format!("oid-index.json absent for {tenant}/{repo} (content seam not seeded)")
        })?;
    let index = parse_oid_index(&index_bytes)?;

    // 2+3. BATCH-read every object from the CAS by its blake3 (one round-trip per
    // chunk instead of one per object), decode the loose framing, insert under its
    // re-derived git oid, and assert that oid == the index oid (DOUBLE integrity).
    //
    // The index can map several git oids to the SAME blake3 (identical content), so
    // read the DISTINCT blake3s once and fan the bytes back out to each git oid.
    let distinct_hashes: Vec<String> = index
        .values()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    // If the bulk plane is NOT deployed (405/404 on the batch-read route), degrade
    // to the always-live per-object `get` loop over the distinct hashes. The
    // double-integrity (blake3==key below + git-SHA-1==index-oid in the insert loop)
    // is identical on either path — a per-object GET 404/410 (absent) is corruption
    // for an INDEXED object and still fails closed.
    let read: BatchReadResult = match cas.batch_read(&distinct_hashes) {
        Ok(read) => read,
        Err(CasError::BulkUnsupported(code)) => {
            eprintln!(
                "load_from_cas: bulk CAS plane absent (HTTP {code}) on batch-read — \
                 falling back to per-object read of {} object(s)",
                distinct_hashes.len()
            );
            let mut out: BatchReadResult = Vec::with_capacity(distinct_hashes.len());
            for h in &distinct_hashes {
                let bytes = cas
                    .get(h)
                    .map_err(|e| format!("CAS per-object read failed for blake3 {h}: {e}"))?;
                out.push((h.clone(), bytes));
            }
            out
        }
        Err(e) => return Err(format!("CAS batch-read failed: {e}")),
    };
    let mut bytes_by_blake3: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (blake3, maybe) in read {
        // Fail closed on any absent/gone object — a misconfigured seam must not boot.
        let framed = maybe.ok_or_else(|| {
            format!("CAS object absent/gone for blake3 {blake3} (content seam incomplete)")
        })?;
        bytes_by_blake3.insert(blake3, framed);
    }

    let mut store = CasObjectSource::new();
    for (git_oid_hex, blake3) in &index {
        let want_oid = gix_hash::ObjectId::from_hex(git_oid_hex.as_bytes())
            .map_err(|e| format!("oid-index: {git_oid_hex:?} is not a valid git oid: {e}"))?;
        let framed = bytes_by_blake3.get(blake3).ok_or_else(|| {
            format!("CAS object absent for git oid {git_oid_hex} (blake3 {blake3})")
        })?;
        // Defense in depth: the bytes the CAS handed back MUST also blake3-hash to
        // the key we asked for. CoreLink verifies this on write; we re-verify on
        // read so a mis-serving CAS cannot smuggle bytes past the SHA-1 check.
        let actual_blake3 = cas_key(framed);
        if &actual_blake3 != blake3 {
            return Err(format!(
                "CAS content-address violation for git oid {git_oid_hex}: asked for blake3 \
                 {blake3} but the bytes hash to {actual_blake3}"
            ));
        }
        let (kind, body) = decode_loose(framed)
            .map_err(|e| format!("CAS object for git oid {git_oid_hex} is malformed: {e}"))?;
        // `insert_raw` re-derives the git SHA-1 and files the object under it.
        let derived = store.insert_raw(kind, body);
        // The SHA-1 git the index claims MUST match the SHA-1 the bytes derive to.
        if derived != want_oid {
            return Err(format!(
                "git oid mismatch: oid-index claims {want_oid} but the CAS bytes (blake3 \
                 {blake3}) derive to git oid {derived} (double-integrity check failed)"
            ));
        }
    }

    // 4. The root tree of the commit at HEAD.
    let head_oid_hex = manifest.refs.get(&manifest.head).ok_or_else(|| {
        format!(
            "refs.json head {:?} has no entry in refs (manifest is inconsistent)",
            manifest.head
        )
    })?;
    let head_oid = gix_hash::ObjectId::from_hex(head_oid_hex.as_bytes())
        .map_err(|e| format!("head oid {head_oid_hex:?} is not a valid git oid: {e}"))?;
    let root_tree = root_tree_of_commit(&store, &head_oid)?;

    Ok((store, root_tree, manifest.refs))
}

/// A **lazy, on-demand** [`hugit_proto::ObjectSource`] backed by the CoreLink CAS.
///
/// Boot loads ONLY the oid→blake3 index (a few KB of `oid-index.json`); each git
/// object is fetched from the CAS the first time it is served (clone / blob /
/// outline), verified (blake3 + git-SHA-1 double-integrity, same as the eager
/// path), and cached so repeated reads don't re-hit R2. This keeps boot O(1) in
/// the object count — the eager [`load_from_cas`] is O(objects) (one CAS read
/// each) and blew past the container start window for a real repo (~200s for the
/// 6862-object hugit closure → CF kills the container; 2026-06-20 incident). The
/// distroless / no-git-binary invariant (#151) is preserved.
/// Default byte budget for the decoded-object first-fetch cache (64 MiB of object
/// BODIES). Chosen generous enough to retain a hot working set for serving a clone
/// without re-decoding, yet small relative to the engine container's memory so the
/// cache can never be the cause of an OOM on a long-lived single instance. Without
/// a bound the cache retained every decoded body forever → unbounded RSS growth
/// (the write-path-hardening audit's #5 finding); a miss-after-evict simply
/// re-fetches from CAS (content-addressing keeps correctness intact).
pub const DEFAULT_OBJECT_CACHE_BYTES: usize = 64 * 1024 * 1024;

/// A byte-bounded, FIFO-eviction cache of decoded git objects for the lazy CAS
/// read path.
///
/// Content-addressing makes a hit never stale — an oid maps to the same bytes
/// forever — so eviction only ever costs a re-fetch, never correctness. The budget
/// is on the summed object-body bytes (not entry count) so a handful of large
/// blobs cannot blow RSS. Eviction is insertion-order (FIFO): a cheap
/// approximate-LRU, sufficient because the accept loop is single-threaded so this
/// cache is never contended.
struct BoundedObjectCache {
    /// oid → decoded object.
    map: BTreeMap<gix_hash::ObjectId, GitObject>,
    /// Insertion order, oldest at the front — the FIFO eviction queue.
    order: VecDeque<gix_hash::ObjectId>,
    /// Running sum of cached object body bytes (what the budget bounds).
    bytes: usize,
    /// The body-byte ceiling; an insertion past it evicts oldest entries first.
    max_bytes: usize,
}

impl BoundedObjectCache {
    /// A fresh cache bounded to `max_bytes` of object-body bytes.
    fn new(max_bytes: usize) -> Self {
        Self {
            map: BTreeMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            max_bytes,
        }
    }

    /// A clone of the cached object for `oid`, if present.
    fn get(&self, oid: &gix_hash::ObjectId) -> Option<GitObject> {
        self.map.get(oid).cloned()
    }

    /// Insert `obj` under `oid`, then evict oldest entries until within the byte
    /// budget. A duplicate oid (same bytes by content-addressing) is a no-op on the
    /// accounting. If a single object exceeds the whole budget it is retained
    /// transiently (the cache never drops the just-inserted entry, so it is never
    /// empty mid-serve and the bound is best-effort — at most one oversized object
    /// over).
    fn insert(&mut self, oid: gix_hash::ObjectId, obj: GitObject) {
        let sz = obj.data.len();
        if self.map.insert(oid, obj).is_none() {
            self.order.push_back(oid);
            self.bytes = self.bytes.saturating_add(sz);
        }
        // Evict FIFO from the front (oldest) while over budget, but never the last
        // entry — `order.len() > 1` keeps the just-inserted object resident.
        while self.bytes > self.max_bytes && self.order.len() > 1 {
            if let Some(old) = self.order.pop_front()
                && let Some(removed) = self.map.remove(&old)
            {
                self.bytes = self.bytes.saturating_sub(removed.data.len());
            }
        }
    }

    /// The number of resident objects (test-visibility).
    #[cfg(test)]
    fn len(&self) -> usize {
        self.map.len()
    }
}

pub struct LazyCasObjectSource<T: CasTransport = UreqCasTransport> {
    /// git oid → its blake3 CAS key (the whole `oid-index.json`, parsed once).
    /// INTERIOR-MUTABLE + shared: a successful CAS-mode `git push` merges the
    /// just-pushed `oid → blake3` entries into this same cell via a [`LiveOidIndex`]
    /// handle held by the repo's `RepoState`, so a freshly-pushed object resolves on
    /// the read path with NO engine reboot. The accept loop is single-threaded, so
    /// the `RwLock` is uncontended (the design's "contention-free either way").
    index: LiveOidIndex,
    /// The CAS client used to fetch object bytes on demand.
    cas: CasClient<T>,
    /// First-fetch cache: a served object is decoded + verified once, then reused.
    /// Content-addressing makes this safe across a hot-swap: an oid maps to the same
    /// bytes forever, so a cache hit is never stale and the new index entries simply
    /// take the cold path on their first read. BYTE-BOUNDED with FIFO eviction
    /// ([`BoundedObjectCache`]) so a long-lived instance cannot grow RSS without
    /// bound; a miss-after-evict re-fetches (content-addressing keeps it correct).
    cache: std::sync::Mutex<BoundedObjectCache>,
}

impl<T: CasTransport> LazyCasObjectSource<T> {
    /// Build a lazy source over an oid→blake3 `index` and an owned CAS client.
    #[must_use]
    pub fn new(index: BTreeMap<gix_hash::ObjectId, String>, cas: CasClient<T>) -> Self {
        Self {
            index: LiveOidIndex::new(index),
            cas,
            cache: std::sync::Mutex::new(BoundedObjectCache::new(DEFAULT_OBJECT_CACHE_BYTES)),
        }
    }

    /// Like [`Self::new`] but with an explicit decoded-object cache byte budget —
    /// test-only, to exercise eviction without allocating 64 MiB of fixtures.
    #[cfg(test)]
    fn new_with_cache_budget(
        index: BTreeMap<gix_hash::ObjectId, String>,
        cas: CasClient<T>,
        cache_max_bytes: usize,
    ) -> Self {
        Self {
            index: LiveOidIndex::new(index),
            cas,
            cache: std::sync::Mutex::new(BoundedObjectCache::new(cache_max_bytes)),
        }
    }

    /// The number of objects currently resident in the decoded-object cache
    /// (test-visibility, for the eviction bound assertion).
    #[cfg(test)]
    fn cache_len(&self) -> usize {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// A shared handle to this source's live oid→blake3 index, so the push-finalize
    /// path can merge new entries into the SAME cell this source reads. Cloning the
    /// handle shares the `Arc` — it does NOT copy the index.
    #[must_use]
    pub fn live_index_handle(&self) -> LiveOidIndex {
        self.index.clone()
    }

    /// The number of objects the index references (the boot footprint — what the
    /// eager path would have pre-loaded).
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// Whether the index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }
}

/// An interior-mutable, shared `git-oid → blake3` index — the live counterpart of
/// the boot-loaded `oid-index.json`.
///
/// Threaded between the read path (a [`LazyCasObjectSource`] resolving an oid →
/// blake3 → CAS bytes) and the CAS-mode push finalize (which merges the pushed
/// objects' `oid → blake3` entries). The handle is `Clone` and shares one `Arc`,
/// so an update through any clone is observed by every reader — no reboot. The
/// accept loop is single-threaded (`server::serve_on`), so the `RwLock` is
/// uncontended.
#[derive(Clone)]
pub struct LiveOidIndex(Arc<RwLock<BTreeMap<gix_hash::ObjectId, String>>>);

impl LiveOidIndex {
    /// Wrap a boot-loaded oid→blake3 map.
    #[must_use]
    pub fn new(index: BTreeMap<gix_hash::ObjectId, String>) -> Self {
        Self(Arc::new(RwLock::new(index)))
    }

    /// The blake3 CAS key for `oid` in the LIVE index, or `None` if not indexed.
    #[must_use]
    pub fn get(&self, oid: &gix_hash::ObjectId) -> Option<String> {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(oid)
            .cloned()
    }

    /// Whether `oid` is in the LIVE index.
    #[must_use]
    pub fn contains(&self, oid: &gix_hash::ObjectId) -> bool {
        self.0
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(oid)
    }

    /// The number of indexed oids in the LIVE index.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    /// Whether the LIVE index is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.read().unwrap_or_else(|e| e.into_inner()).is_empty()
    }

    /// Merge `additions` (`oid → blake3`) into the LIVE index. Idempotent: a
    /// re-pushed oid simply re-asserts the same blake3 (content addressing). Called
    /// AFTER the durable manifests committed, so the in-memory index is never ahead
    /// of the durable `oid-index.json`.
    pub fn merge(&self, additions: impl IntoIterator<Item = (gix_hash::ObjectId, String)>) {
        let mut guard = self.0.write().unwrap_or_else(|e| e.into_inner());
        for (oid, blake3) in additions {
            guard.insert(oid, blake3);
        }
    }
}

impl<T: CasTransport + Send + Sync> hugit_proto::ObjectSource for LazyCasObjectSource<T> {
    fn get(&self, oid: &gix_hash::ObjectId) -> Result<Option<GitObject>, PackError> {
        // Served before? Return the decoded+verified object from the cache.
        if let Some(obj) = self
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(oid)
        {
            return Ok(Some(obj));
        }
        // Not in the LIVE index → a CLEAN absence (Ok(None)), exactly like the eager
        // store. The live read picks up oids merged by a just-completed CAS push.
        let Some(blake3) = self.index.get(oid) else {
            return Ok(None);
        };
        let blake3 = blake3.as_str();
        // Indexed → the object MUST be fetchable. A transport error or an absent
        // object is a broken seam, surfaced as PackError::Source (fail-closed) —
        // never a silent empty/partial serve.
        let framed = self
            .cas
            .get(blake3)
            .map_err(|e| {
                PackError::Source(format!("CAS fetch failed for git oid {oid} (blake3 {blake3}): {e}"))
            })?
            .ok_or_else(|| {
                PackError::Source(format!(
                    "CAS object absent for indexed git oid {oid} (blake3 {blake3}) — content seam incomplete"
                ))
            })?;
        // Defense in depth: the returned bytes must hash to the blake3 we asked for.
        let actual_blake3 = cas_key(&framed);
        if actual_blake3 != *blake3 {
            return Err(PackError::Source(format!(
                "CAS content-address violation for git oid {oid}: asked blake3 {blake3}, bytes hash to {actual_blake3}"
            )));
        }
        let (kind, body) = decode_loose(&framed).map_err(|e| {
            PackError::Source(format!(
                "CAS object for git oid {oid} is malformed loose framing: {e}"
            ))
        })?;
        let obj = GitObject::new(kind, body);
        // Double-integrity: the git SHA-1 the bytes derive to MUST equal the asked oid.
        let derived = obj.try_oid()?;
        if &derived != oid {
            return Err(PackError::AddressMismatch {
                stored: *oid,
                actual: derived,
            });
        }
        self.cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(*oid, obj.clone());
        Ok(Some(obj))
    }

    fn contains(&self, oid: &gix_hash::ObjectId) -> bool {
        self.index.contains(oid)
    }
}

/// Manifest-only boot loader: the LAZY counterpart of [`load_from_cas`].
///
/// Reads ONLY `refs.json` + `oid-index.json` from R2 and resolves HEAD's root
/// tree (a couple of on-demand fetches — the commit + its tree), then hands back
/// a [`LazyCasObjectSource`] that fetches the rest on demand. Boot cost is the
/// index size + 2 objects, NOT the whole closure.
pub fn load_manifests_from_cas<T: CasTransport + Send + Sync, R: R2Get>(
    cas: CasClient<T>,
    r2: &R,
    tenant: &str,
    repo: &str,
) -> Result<LazyCasLoad<T>, String> {
    // The mutable manifests from R2 (fail-closed on absence — same as the eager path).
    let refs_bytes = r2
        .get_object(&refs_manifest_key(tenant, repo))?
        .ok_or_else(|| format!("refs.json absent for {tenant}/{repo} (content seam not seeded)"))?;
    let manifest = parse_refs_manifest(&refs_bytes)?;

    let index_bytes = r2
        .get_object(&oid_index_key(tenant, repo))?
        .ok_or_else(|| {
            format!("oid-index.json absent for {tenant}/{repo} (content seam not seeded)")
        })?;
    let index_raw = parse_oid_index(&index_bytes)?;

    // Re-key the oid→blake3 index by ObjectId (O(log n) get on the serve path).
    let mut index: BTreeMap<gix_hash::ObjectId, String> = BTreeMap::new();
    for (oid_hex, blake3) in &index_raw {
        let oid = gix_hash::ObjectId::from_hex(oid_hex.as_bytes())
            .map_err(|e| format!("oid-index: {oid_hex:?} is not a valid git oid: {e}"))?;
        index.insert(oid, blake3.clone());
    }

    let source = LazyCasObjectSource::new(index, cas);

    // Resolve HEAD's root tree — two on-demand fetches (the HEAD commit + its
    // tree id is read from the commit), NOT the full closure.
    let head_oid_hex = manifest.refs.get(&manifest.head).ok_or_else(|| {
        format!(
            "refs.json head {:?} has no entry in refs (manifest is inconsistent)",
            manifest.head
        )
    })?;
    let head_oid = gix_hash::ObjectId::from_hex(head_oid_hex.as_bytes())
        .map_err(|e| format!("head oid {head_oid_hex:?} is not a valid git oid: {e}"))?;
    let root_tree = root_tree_of_commit(&source, &head_oid)?;

    Ok((source, root_tree, manifest.refs))
}

// ─────────────────────────────────────────────────────────────────────────────
// G. Ingest core — enumerate → encode_loose → cas_key → PUT → manifests.
// ─────────────────────────────────────────────────────────────────────────────

/// One enumerated git object to ingest: its git oid (hex), kind, and raw body
/// (NO loose header — the framing is added by [`encode_loose`] here).
#[derive(Debug, Clone)]
pub struct IngestObject {
    /// The git SHA-1 oid (40-hex lowercase).
    pub git_oid: String,
    /// The object kind.
    pub kind: ObjectKind,
    /// The raw, unframed body bytes (canonical git encoding for the kind).
    pub body: Vec<u8>,
}

/// The R2 writes the ingest produces, after all objects are in the CAS: publish
/// `refs.json` + `oid-index.json` under the repo's tenant/repo prefix. Behind a
/// trait so the ingest is proven hermetically against an in-memory double.
pub trait R2Put {
    /// PUT an R2 object by its full key. `Err` = transport.
    fn put_object(&self, key: &str, body: &[u8]) -> Result<(), String>;
}

/// Batch-upload `objects`, then RETRY (once) any object the server marked `error`.
/// A `created`/`exists` status is success. After the single retry, ANY still-`error`
/// object fails the whole ingest (fail-closed — a half-uploaded closure is unservable).
fn upload_with_retry<T: CasTransport>(
    cas: &CasClient<T>,
    objects: &[(String, Vec<u8>)],
) -> Result<(), CasError> {
    if objects.is_empty() {
        return Ok(());
    }
    let statuses = cas.batch_upload(objects)?;
    let mut failed: Vec<(String, Vec<u8>)> = Vec::new();
    for (hash, status) in &statuses {
        if let UploadStatus::Error(_) = status {
            // Re-pair the failed hash with its bytes for the retry.
            if let Some((_, bytes)) = objects.iter().find(|(h, _)| h == hash) {
                failed.push((hash.clone(), bytes.clone()));
            }
        }
    }
    if failed.is_empty() {
        return Ok(());
    }
    // One retry of just the errored objects.
    let retried = cas.batch_upload(&failed)?;
    let still: Vec<&str> = retried
        .iter()
        .filter_map(|(h, s)| matches!(s, UploadStatus::Error(_)).then_some(h.as_str()))
        .collect();
    if !still.is_empty() {
        return Err(CasError::Transport(format!(
            "{} object(s) still failed after one retry (e.g. {})",
            still.len(),
            still.first().copied().unwrap_or("?")
        )));
    }
    Ok(())
}

/// Upload `objects` (`(blake3, framed-bytes)`) via the always-live, idempotent
/// single-object [`CasClient::put`] — the per-object fallback used when the bulk
/// plane is absent (405/404 on the batch route). PUT is implicitly deduped
/// server-side (201 created / 200 exists), so no separate exists-probe is needed.
/// Fail-closed on the FIRST object that cannot be PUT (a half-uploaded closure is
/// unservable, exactly as the bulk path's persistent-error rule).
fn put_objects_individually<T: CasTransport>(
    cas: &CasClient<T>,
    objects: &[(String, Vec<u8>)],
) -> Result<(), CasError> {
    for (blake3, framed) in objects {
        cas.put(blake3, framed)?;
    }
    Ok(())
}

/// Ingest a repo's enumerated object closure into the CoreLink CAS, then publish
/// the mutable manifests to R2 — the transport-injected core of the `git-ingest`
/// bin (the bin only does the `git` enumeration). DEDUP + BATCH path:
/// 1. [`encode_loose`] + [`cas_key`] every object → `(git_oid, blake3, framed)`;
/// 2. [`CasClient::batch_exists`] over ALL blake3s → upload ONLY the missing
///    (the launch closure overlaps massively across re-ingests);
/// 3. [`CasClient::batch_upload`] the missing (chunked to the FROZEN caps); a
///    per-object `error` status is RETRIED once, then fails if it persists;
/// 4. write `<tenant>/<repo>/refs.json` + `oid-index.json` (UNCHANGED).
///
/// Returns the total object count. Fail-closed on any batch error, a persistent
/// per-object upload error, an unsafe slug, or a `head` not present in `refs` (so
/// the loader's `manifest.refs[head]` lookup cannot fail at boot).
pub fn ingest_repo<T: CasTransport, W: R2Put>(
    cas: &CasClient<T>,
    r2: &W,
    tenant: &str,
    repo: &str,
    head: &str,
    refs: &BTreeMap<String, String>,
    objects: &[IngestObject],
) -> Result<usize, String> {
    if !refs.contains_key(head) {
        return Err(format!(
            "ingest: head {head:?} has no entry in refs — the manifest would be \
             inconsistent and the loader would fail at boot"
        ));
    }

    // 1. Frame + key every object; build the git_oid → blake3 index up front.
    let mut index: OidIndex = BTreeMap::new();
    // blake3 → framed bytes (the upload payload, keyed for dedup lookup). A single
    // blake3 can back several git oids (identical content); store the bytes once.
    let mut framed_by_blake3: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for obj in objects {
        let framed = encode_loose(obj.kind, &obj.body);
        let blake3 = cas_key(&framed);
        index.insert(obj.git_oid.clone(), blake3.clone());
        framed_by_blake3.entry(blake3).or_insert(framed);
    }

    // 2. Dedup: probe which distinct blake3s the CAS already holds, upload only the
    // missing. Probe order is deterministic (BTreeMap iteration).
    //
    // The FIRST bulk call is `batch_exists`. If it answers `BulkUnsupported` (the
    // bulk plane is NOT deployed: 405/404 on the batch route), we cannot dedup-probe
    // in bulk — degrade the WHOLE ingest to the per-object path: every framed object
    // is PUT via the always-live, idempotent single-object `put_object` (the server
    // dedups implicitly: 201 fresh / 200 exists), and we skip the dedup probe.
    let all_hashes: Vec<String> = framed_by_blake3.keys().cloned().collect();
    let total_distinct = framed_by_blake3.len();
    match cas.batch_exists(&all_hashes) {
        Ok(present) => {
            let mut present_set: std::collections::BTreeSet<String> =
                std::collections::BTreeSet::new();
            for (hash, is_present) in present {
                if is_present {
                    present_set.insert(hash);
                }
            }
            let missing: Vec<(String, Vec<u8>)> = framed_by_blake3
                .iter()
                .filter(|(h, _)| !present_set.contains(*h))
                .map(|(h, b)| (h.clone(), b.clone()))
                .collect();
            let already = present_set.len();

            // 3. Upload the missing (chunked) — then retry any per-object `error`
            // ONCE. If the UPLOAD discovers the bulk plane is absent mid-stream
            // (batch_exists worked but batch_upload 405s — a partially-deployed
            // server), degrade the remaining uploads to per-object too.
            match upload_with_retry(cas, &missing) {
                Ok(()) => {}
                Err(CasError::BulkUnsupported(code)) => {
                    eprintln!(
                        "git-ingest: bulk CAS plane absent (HTTP {code}) on upload — \
                         falling back to per-object ingest of {} object(s)",
                        missing.len()
                    );
                    put_objects_individually(cas, &missing)
                        .map_err(|e| format!("ingest: per-object upload fallback failed: {e}"))?;
                }
                Err(e) => return Err(format!("ingest: batch-upload failed: {e}")),
            }

            let uploaded = missing.len();
            eprintln!(
                "git-ingest: {} objects ({total_distinct} distinct blobs), {already} already \
                 present (deduped), {uploaded} uploaded",
                objects.len()
            );
        }
        Err(CasError::BulkUnsupported(code)) => {
            // The bulk plane is not deployed. Skip the dedup probe entirely and PUT
            // every distinct object via the idempotent single-object path.
            let all_objects: Vec<(String, Vec<u8>)> = framed_by_blake3
                .iter()
                .map(|(h, b)| (h.clone(), b.clone()))
                .collect();
            eprintln!(
                "git-ingest: bulk CAS plane absent (HTTP {code}) — falling back to per-object \
                 ingest of {total_distinct} object(s)"
            );
            put_objects_individually(cas, &all_objects)
                .map_err(|e| format!("ingest: per-object upload fallback failed: {e}"))?;
            eprintln!(
                "git-ingest: {} objects ({total_distinct} distinct blobs), per-object ingest \
                 (bulk plane absent), {total_distinct} uploaded",
                objects.len()
            );
        }
        Err(e) => return Err(format!("ingest: batch-exists (dedup probe) failed: {e}")),
    }

    // 4. Publish the mutable manifests (UNCHANGED).
    let manifest = RefsManifest {
        head: head.to_string(),
        refs: refs.clone(),
    };
    let refs_bytes =
        serde_json::to_vec(&manifest).map_err(|e| format!("ingest: refs.json serialize: {e}"))?;
    r2.put_object(&refs_manifest_key(tenant, repo), &refs_bytes)
        .map_err(|e| format!("ingest: refs.json PUT: {e}"))?;

    let index_bytes =
        serde_json::to_vec(&index).map_err(|e| format!("ingest: oid-index.json serialize: {e}"))?;
    r2.put_object(&oid_index_key(tenant, repo), &index_bytes)
        .map_err(|e| format!("ingest: oid-index.json PUT: {e}"))?;

    Ok(objects.len())
}

/// Resolve a commit oid to its root-tree oid by parsing the commit object held in
/// `store` with `gix_object::CommitRefIter` (the canonical decoder — the tree id
/// is never re-derived by hand). Fail-closed if the commit is absent, is not a
/// commit, or does not decode.
fn root_tree_of_commit(
    store: &dyn hugit_proto::ObjectSource,
    head: &gix_hash::ObjectId,
) -> Result<gix_hash::ObjectId, String> {
    let obj = store
        .get(head)
        .map_err(|e| format!("loading HEAD commit {head} failed: {e}"))?
        .ok_or_else(|| format!("HEAD commit {head} is not present in the CAS closure"))?;
    if obj.kind != ObjectKind::Commit {
        return Err(format!("HEAD {head} is a {:?}, not a commit", obj.kind));
    }
    gix_object::CommitRefIter::from_bytes(&obj.data)
        .tree_id()
        .map_err(|e| format!("HEAD commit {head} does not decode (no tree id): {e}"))
}

// ─────────────────────────────────────────────────────────────────────────────
// H. CasRw — the buffering receive→CAS write adapter (the CAS-mode push sink).
// ─────────────────────────────────────────────────────────────────────────────

use hugit_proto::write::store::{Cas, CasObject, inflate_loose_framing};
use std::collections::HashMap;

/// A buffering [`Cas`] adapter that turns `receive_pack`'s verbatim loose objects
/// into CoreLink-CAS-shaped, content-addressed writes — the write-side analogue of
/// [`LazyCasObjectSource`] and the foundation of CAS-mode `git push`.
///
/// # The load-bearing correctness invariant
///
/// `receive_pack` produces [`CasObject`]s whose `.bytes` are git's verbatim
/// **zlib-compressed** loose bytes. The CoreLink CAS, however, stores the
/// **uncompressed** `encode_loose` framing (`<kind> <len>\0<body>`) keyed by
/// `blake3(framing)` — exactly what [`LazyCasObjectSource`] reads back. So
/// [`Cas::put`] here **inflates** each object to its framing
/// ([`inflate_loose_framing`]) and keys it with [`cas_key`] — the SAME helper the
/// read side hashes with — BEFORE buffering. An object pushed through `CasRw` is
/// therefore byte-identical to one the CAS read side would serve; storing the
/// compressed bytes (or a mis-computed key) would make every pushed object
/// un-cloneable.
///
/// # Buffer-then-flush
///
/// [`Cas::put`] is **infallible** (the trait contract) and does NO network: it
/// inflates + buffers into `pending` (keyed by blake3, deduped) and records the
/// `git_oid → blake3` mapping in `index_add`. [`Cas::contains`] / [`Cas::get`]
/// consult the pending push and the repo's existing `oid → blake3` index FIRST,
/// then fall through to the wrapped CAS read source — so the receive-pack anchor
/// check (the pushed tip must be delivered OR already live) sees both the
/// in-flight objects and the prior closure. The explicit, fallible [`flush`] is
/// the ONLY network step: it uploads the buffered set (idempotent — content
/// addressing means a re-flush re-PUTs the same bytes the server already holds).
/// The receive-pack handler calls `flush` AFTER a successful `receive_pack`
/// (PR2); a buffered object that failed to inflate fails the flush closed.
///
/// [`flush`]: CasRw::flush
pub struct CasRw<T: CasTransport = UreqCasTransport> {
    /// The wrapped CoreLink CAS client — the read fall-through AND the flush sink.
    cas: CasClient<T>,
    /// The repo's pre-push closure: `git_oid (40-hex) → blake3 (64-hex)` — the
    /// same `oid-index.json` shape the loader reads. Authoritative for "does the
    /// repo already hold this oid" (the anchor's already-live-in-CAS leg).
    existing: OidIndex,
    /// Buffered new objects: `blake3 → uncompressed loose framing`. Deduped by
    /// content address (`or_insert`), so identical content is buffered once.
    pending: HashMap<String, Vec<u8>>,
    /// This push's new objects: `git_oid (40-hex) → blake3 (64-hex)`. The write
    /// analogue of `existing`; consulted first by `contains`/`get`.
    index_add: HashMap<String, String>,
    /// Inflate failures deferred from the infallible `put`. A non-empty list fails
    /// the explicit [`flush`](CasRw::flush) closed (a corrupt object must never be
    /// silently dropped from the closure).
    inflate_errors: Vec<String>,
    /// The existing-closure oids THIS push resolved a delta base from (a thin-pack /
    /// REF_DELTA base that lives only in the repo's prior closure, not in the pushed
    /// pack). Recorded on every `get` that hits the `existing` index — the set the
    /// in-memory hot-swap re-asserts against `live_oid_index` before advancing the
    /// tip (defence-in-depth: refuse to advertise a tip whose base the read-path
    /// index can't resolve). Interior mutability is sound because a `CasRw` is a
    /// per-push LOCAL value, never shared across threads.
    consumed_existing: std::cell::RefCell<std::collections::HashSet<String>>,
}

impl<T: CasTransport> CasRw<T> {
    /// Wrap a CAS client + the repo's existing `oid → blake3` index (its pre-push
    /// closure). The buffer starts empty; objects accrue via [`Cas::put`].
    #[must_use]
    pub fn new(cas: CasClient<T>, existing: OidIndex) -> Self {
        Self {
            cas,
            existing,
            pending: HashMap::new(),
            index_add: HashMap::new(),
            inflate_errors: Vec::new(),
            consumed_existing: std::cell::RefCell::new(std::collections::HashSet::new()),
        }
    }

    /// The number of DISTINCT objects buffered (deduped by blake3) — the set a
    /// [`flush`](CasRw::flush) would upload.
    #[must_use]
    pub fn pending_len(&self) -> usize {
        self.pending.len()
    }

    /// Whether nothing is buffered (a [`flush`](CasRw::flush) would be a no-op).
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// The wrapped CAS client (read fall-through + flush sink).
    #[must_use]
    pub fn cas_client(&self) -> &CasClient<T> {
        &self.cas
    }

    /// This push's new `git_oid → blake3` additions — the entries
    /// [`commit_cas_push_manifests`] merges into `oid-index.json` after the objects
    /// flush. Empty until objects are buffered via [`Cas::put`].
    #[must_use]
    pub fn index_additions(&self) -> &HashMap<String, String> {
        &self.index_add
    }

    /// The existing-closure oids THIS push resolved a delta base from (each is an
    /// object the pushed pack referenced but did NOT carry — a thin-pack / REF_DELTA
    /// base served from the repo's prior closure). The in-memory hot-swap
    /// re-asserts every one is present in the read-path `live_oid_index` before it
    /// advances the ref tip, so a divergence (HA / stale live index) can never
    /// advertise a tip whose base the read path can't resolve.
    #[must_use]
    pub fn consumed_existing_bases(&self) -> Vec<String> {
        self.consumed_existing.borrow().iter().cloned().collect()
    }

    /// Upload the buffered objects to the CoreLink CAS — the ONLY network step.
    ///
    /// Fails closed if any buffered object could not be inflated (an unservable
    /// closure must not be half-committed). An empty buffer is a no-op. Uploads
    /// via the idempotent batch path ([`CasClient::batch_upload`] + a single
    /// per-object error retry); if the bulk plane is absent (405/404 on the batch
    /// route) it degrades to the always-live single-object PUT — exactly the
    /// `ingest_repo` policy. Idempotent: content addressing makes a re-flush a
    /// no-write 200-`exists` on every object.
    pub fn flush(&mut self) -> Result<(), CasError> {
        if !self.inflate_errors.is_empty() {
            return Err(CasError::Transport(format!(
                "CasRw: {} buffered object(s) could not be inflated for CAS storage \
                 (the closure is unservable; refusing to flush) — e.g. {}",
                self.inflate_errors.len(),
                self.inflate_errors
                    .first()
                    .map(String::as_str)
                    .unwrap_or("?"),
            )));
        }
        if self.pending.is_empty() {
            return Ok(());
        }
        // Deterministic order (BTreeMap iteration would; HashMap does not) so the
        // batching is reproducible. The set, not the order, is what matters for
        // correctness — but determinism keeps tests + logs stable.
        let mut objects: Vec<(String, Vec<u8>)> = self
            .pending
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        objects.sort_by(|a, b| a.0.cmp(&b.0));
        match upload_with_retry(&self.cas, &objects) {
            Ok(()) => Ok(()),
            // The bulk plane is not deployed — degrade to the always-live per-object
            // PUT path (the server dedups implicitly: 201 fresh / 200 exists).
            Err(CasError::BulkUnsupported(code)) => {
                eprintln!(
                    "CasRw::flush: bulk CAS plane absent (HTTP {code}) on batch-upload — \
                     falling back to per-object PUT of {} object(s)",
                    objects.len()
                );
                put_objects_individually(&self.cas, &objects)
            }
            Err(e) => Err(e),
        }
    }
}

impl<T: CasTransport> Cas for CasRw<T> {
    /// INFALLIBLE per the trait: inflate the verbatim zlib loose bytes to the CAS
    /// framing, key it with [`cas_key`], and buffer — NO network. A rare inflate
    /// failure is deferred to [`flush`](CasRw::flush) (which fails closed) rather
    /// than panicking or silently dropping the object.
    fn put(&mut self, obj: &CasObject) {
        match inflate_loose_framing(obj) {
            Ok(framing) => {
                let key = cas_key(&framing);
                self.index_add.insert(obj.oid.clone(), key.clone());
                // Content-addressed ⇒ idempotent: identical content buffers once.
                self.pending.entry(key).or_insert(framing);
            }
            Err(e) => self
                .inflate_errors
                .push(format!("git oid {}: zlib inflate failed: {e}", obj.oid)),
        }
    }

    /// The CAS framing for `oid`: the buffered push FIRST (returns the exact bytes
    /// the read side would serve), then the repo's existing closure via the wrapped
    /// CAS (`oid → blake3 → cas.get`). `None` if the oid is in neither.
    fn get(&self, oid: &str) -> Option<Vec<u8>> {
        if let Some(blake3) = self.index_add.get(oid)
            && let Some(framing) = self.pending.get(blake3)
        {
            return Some(framing.clone());
        }
        if let Some(blake3) = self.existing.get(oid) {
            // This push referenced a base from the repo's PRIOR closure (a thin-pack
            // / REF_DELTA base the pushed pack omitted). Record it on the
            // resolution-ATTEMPT (regardless of whether the wrapped `cas.get` then
            // succeeds — the base was referenced) so the in-memory hot-swap can
            // re-assert it is resolvable on the read path before advancing the tip.
            self.consumed_existing.borrow_mut().insert(oid.to_string());
            // Fall through to the wrapped CAS read source. A transport error reads
            // as absent here (the anchor's `contains` is the fail-closed gate).
            return self.cas.get(blake3).ok().flatten();
        }
        None
    }

    /// Whether `oid` is in the in-flight push OR the repo's existing closure —
    /// the anchor's "delivered now OR already live in CAS" gate. Consults the
    /// indices only (no network): the `oid → blake3` index IS the authoritative
    /// read-source answer for repo membership.
    fn contains(&self, oid: &str) -> bool {
        self.index_add.contains_key(oid) || self.existing.contains_key(oid)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// I. CAS-mode push finalize — objects → log → manifests (the fail-closed order).
// ─────────────────────────────────────────────────────────────────────────────

/// A CAS-mode push finalize fault, tagged by WHICH ordered step failed so the
/// receive-pack handler maps it to the right `report-status` `ng`. The order the
/// steps run — objects → log → manifests — is the load-bearing invariant: a later
/// step never runs if an earlier one failed, so a manifest never advertises a tip
/// whose object closure was not uploaded AND whose `ref.update` event was not
/// durably recorded. `Display` is hand-written (no `thiserror` dep here).
#[derive(Debug)]
pub enum CasPushError {
    /// [`CasRw::flush`] failed — the pushed object closure is NOT fully in the CAS.
    /// Aborts BEFORE the log + manifests (no tip advertised without its closure).
    Flush(CasError),
    /// The caller's log-persist (the compare-and-swap append of the `ref.update`
    /// event) failed — a CAS conflict or a transport fault. Objects DID upload
    /// (idempotent), but the manifests are NOT advanced (the tip must not advertise
    /// without the recorded event). Carries the caller's short reason.
    Persist(String),
    /// The manifest rewrite (oid-index then refs) failed AFTER objects + log
    /// committed. The closure + event are durable; the tip is simply not yet
    /// advertised — the client sees an `ng` and retries (idempotent).
    Manifest(String),
}

impl std::fmt::Display for CasPushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CasPushError::Flush(e) => write!(f, "CAS push: object flush failed: {e}"),
            CasPushError::Persist(r) => write!(f, "CAS push: log persist failed: {r}"),
            CasPushError::Manifest(r) => write!(f, "CAS push: manifest rewrite failed: {r}"),
        }
    }
}

impl std::error::Error for CasPushError {}

/// Finalize a CAS-mode push after `receive_pack` accepted it. Runs the three
/// durable steps IN ORDER, each fail-closed:
///
/// 1. **objects** — [`CasRw::flush`] uploads the buffered closure to the CoreLink
///    CAS (idempotent). A failure aborts here, before any tip is advertised.
/// 2. **log** — `persist_log` (the caller's compare-and-swap append of the
///    `ref.update` event). Runs ONLY after the objects are durably in the CAS.
/// 3. **manifests** — [`commit_cas_push_manifests`]: merge the push's
///    `git_oid → blake3` additions into `oid-index.json`, THEN set
///    `refs[ref_name] = new_oid` in `refs.json`, PUTting both to hugit's R2. The
///    oid-index advances BEFORE the ref tip, so refs never names a tip whose
///    objects are unindexed.
///
/// The order is the invariant: only when the full object closure is uploaded AND
/// the event is recorded does any manifest advertise the new tip. (The handler
/// supplies `persist_log` so the log's compare-and-swap stays its responsibility;
/// this fn owns only the CAS-specific object + manifest steps + the ordering.)
pub fn finalize_cas_push<T, R, P>(
    cas: &mut CasRw<T>,
    r2: &R,
    tenant: &str,
    repo: &str,
    ref_name: &str,
    new_oid: &str,
    persist_log: P,
) -> Result<(), CasPushError>
where
    T: CasTransport,
    R: R2Get + R2Put,
    P: FnOnce() -> Result<(), String>,
{
    // 1. objects FIRST — fail-closed before the log or any manifest.
    cas.flush().map_err(CasPushError::Flush)?;
    // 2. log — the compare-and-swap append; only after the closure is durable.
    persist_log().map_err(CasPushError::Persist)?;
    // 3. manifests LAST — oid-index then the ref tip.
    commit_cas_push_manifests(r2, tenant, repo, ref_name, new_oid, cas.index_additions())
        .map_err(CasPushError::Manifest)
}

/// Read-merge-write the mutable manifests for a CAS-mode push: merge `index_add`
/// (`git_oid → blake3`, the push's new objects) into `<tenant>/<repo>/oid-index.json`
/// and set `refs[ref_name] = new_oid` in `<tenant>/<repo>/refs.json`, then PUT both
/// back to hugit's R2 — **oid-index FIRST, the ref tip LAST**.
///
/// ## Concurrency — SINGLE-WRITER assumption (documented deviation)
///
/// hugit's R2 surface ([`R2Put`]) exposes only an UNCONDITIONAL `put_object` for an
/// arbitrary key; a true `If-Match` conditional PUT for the manifest keys is not in
/// the surface ([`R2Get::get_object`] does not surface the ETag). Per the WP spec's
/// fallback, this is the **read-modify-CAS** variant: it RE-READS each manifest
/// immediately before merging, so a concurrent push whose write already landed is
/// preserved. Two pushes racing INSIDE the read→PUT gap can still lose a ref tip —
/// acceptable under the `self-hosted-alpha` deploy (single-tenant, single-writer,
/// `HUGIT_SERVE_RECEIVE_PACK`-gated). No OBJECT closure is ever lost (the CAS is
/// content-addressed + idempotent); at worst a ref tip regresses, surfaced as a
/// non-fast-forward on the next push. Wiring a signed conditional PUT (the true
/// store-side CAS) is the tracked follow-up.
pub fn commit_cas_push_manifests<R: R2Get + R2Put>(
    r2: &R,
    tenant: &str,
    repo: &str,
    ref_name: &str,
    new_oid: &str,
    index_add: &HashMap<String, String>,
) -> Result<(), String> {
    // 1. oid-index.json FIRST: read-merge-write the new objects' oid→blake3 map, so
    //    the index always covers every object the ref tip (written next) can name.
    let index_key = oid_index_key(tenant, repo);
    let mut index: OidIndex = match r2.get_object(&index_key)? {
        Some(bytes) => parse_oid_index(&bytes)?,
        None => OidIndex::new(),
    };
    for (oid, blake3) in index_add {
        index.insert(oid.clone(), blake3.clone());
    }
    let index_bytes =
        serde_json::to_vec(&index).map_err(|e| format!("push: oid-index.json serialize: {e}"))?;
    r2.put_object(&index_key, &index_bytes)?;

    // 2. refs.json LAST: read-merge-write the new tip. Writing it AFTER the index
    //    guarantees the advertised tip's objects are already indexed (+ uploaded).
    let refs_key = refs_manifest_key(tenant, repo);
    let mut manifest = match r2.get_object(&refs_key)? {
        Some(bytes) => parse_refs_manifest(&bytes)?,
        // A repo with no refs.json yet: seed it with this ref as HEAD (defensive —
        // CAS-mode repos are seeded at ingest, so this is the empty-repo edge).
        None => RefsManifest {
            head: ref_name.to_string(),
            refs: BTreeMap::new(),
        },
    };
    manifest
        .refs
        .insert(ref_name.to_string(), new_oid.to_string());
    let refs_bytes =
        serde_json::to_vec(&manifest).map_err(|e| format!("push: refs.json serialize: {e}"))?;
    r2.put_object(&refs_key, &refs_bytes)?;
    Ok(())
}

/// Finalize a CAS-mode **delete-ref** push (`git push --delete <branch>`) after the
/// serve handler validated it (deploy-gate + write-authz + the default-branch guard +
/// the stale-check). The MIRROR of [`finalize_cas_push`], but it REMOVES the ref
/// instead of advancing a tip — there is no object closure to upload (a delete carries
/// no pack), so the only durable steps are:
///
/// 1. **log** — `persist_log` (the caller's compare-and-swap append of the `ref.delete`
///    event). Runs FIRST: the removal must be recorded before the manifest drops the
///    pointer, so the ref is never gone from refs.json without the event on the log.
/// 2. **manifest** — [`remove_cas_ref_from_manifest`]: read `refs.json`, drop
///    `refs[ref_name]`, PUT it back to hugit's R2. `oid-index.json` is UNCHANGED — a
///    git delete-ref drops only the ref pointer, never the objects (no GC), so the
///    deleted tip's closure stays resolvable for any other ref that names it.
///
/// Each step is fail-closed → a distinct [`CasPushError`] on fault. Returns `Ok(())`
/// ONLY when the event is durably appended AND `refs.json` is durably rewritten without
/// the ref. NO objects are flushed (a delete uploads nothing).
pub fn finalize_cas_delete<R, P>(
    r2: &R,
    tenant: &str,
    repo: &str,
    ref_name: &str,
    persist_log: P,
) -> Result<(), CasPushError>
where
    R: R2Get + R2Put,
    P: FnOnce() -> Result<(), String>,
{
    // 1. log FIRST — the compare-and-swap append of the `ref.delete` event. The ref is
    //    never dropped from refs.json without the removal recorded on the log.
    persist_log().map_err(CasPushError::Persist)?;
    // 2. manifest — drop the ref pointer; oid-index untouched (objects stay in CAS).
    remove_cas_ref_from_manifest(r2, tenant, repo, ref_name).map_err(CasPushError::Manifest)
}

/// Read-modify-write `<tenant>/<repo>/refs.json` to REMOVE `ref_name`, then PUT it back
/// to hugit's R2. `oid-index.json` is deliberately NOT touched (a delete drops only the
/// ref pointer, never the objects). Fail-closed on any R2 read/parse/serialize/PUT fault.
///
/// Same SINGLE-WRITER read-modify-CAS caveat as [`commit_cas_push_manifests`] (the
/// `self-hosted-alpha` single-tenant, `HUGIT_SERVE_RECEIVE_PACK`-gated deploy): it
/// RE-READS refs.json immediately before the removal so a concurrent push's landed
/// write is preserved; the unconditional PUT is safe only under the single-writer
/// invariant. A missing refs.json is treated as nothing-to-remove (idempotent — the
/// handler already validated presence against the live snapshot).
pub fn remove_cas_ref_from_manifest<R: R2Get + R2Put>(
    r2: &R,
    tenant: &str,
    repo: &str,
    ref_name: &str,
) -> Result<(), String> {
    let refs_key = refs_manifest_key(tenant, repo);
    let mut manifest = match r2.get_object(&refs_key)? {
        Some(bytes) => parse_refs_manifest(&bytes)?,
        // No refs.json yet ⇒ nothing to remove (idempotent). Don't fabricate one.
        None => return Ok(()),
    };
    manifest.refs.remove(ref_name);
    let refs_bytes =
        serde_json::to_vec(&manifest).map_err(|e| format!("delete: refs.json serialize: {e}"))?;
    r2.put_object(&refs_key, &refs_bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── B. blake3 key ───────────────────────────────────────────────────────

    #[test]
    fn cas_key_is_64_lowercase_hex_and_deterministic() {
        let a = cas_key(b"hello CAS");
        let b = cas_key(b"hello CAS");
        assert_eq!(a, b, "same bytes → same key");
        assert_eq!(a.len(), 64, "blake3-256 renders to 64 hex chars");
        assert!(is_valid_cas_key(&a), "key passes the path-segment guard");
        // A different input yields a different key (sanity, not a collision proof).
        assert_ne!(a, cas_key(b"hello cas"));
    }

    #[test]
    fn cas_key_matches_blake3_known_vector_empty() {
        // BLAKE3 of the empty input — the published canonical test vector.
        assert_eq!(
            cas_key(b""),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn invalid_keys_are_rejected() {
        assert!(!is_valid_cas_key(""));
        assert!(!is_valid_cas_key("xyz"));
        assert!(!is_valid_cas_key(&"a".repeat(63)));
        assert!(!is_valid_cas_key(&"a".repeat(65)));
        // Uppercase hex is rejected (CoreLink keys are lowercase).
        assert!(!is_valid_cas_key(&"A".repeat(64)));
        // A path separator must never pass the guard.
        assert!(!is_valid_cas_key(
            "../../etc/passwd00000000000000000000000000000000000000000000"
        ));
    }

    // ── C. loose framing round-trip + malformed reject ───────────────────────

    #[test]
    fn loose_round_trips_all_kinds() {
        for (kind, body) in [
            (ObjectKind::Blob, b"file contents".to_vec()),
            (ObjectKind::Tree, vec![0u8, 1, 2, 3, 0, 255]),
            (ObjectKind::Commit, b"tree abc\nauthor x\n\nmsg".to_vec()),
            (ObjectKind::Tag, b"object abc\ntype commit\n".to_vec()),
        ] {
            let framed = encode_loose(kind, &body);
            let (k, b) = decode_loose(&framed).expect("round-trips");
            assert_eq!(k, kind);
            assert_eq!(b, body);
        }
    }

    #[test]
    fn loose_header_shape_is_git_canonical() {
        // The framing is git's loose pre-image: "<kind> <len>\0<body>".
        let framed = encode_loose(ObjectKind::Blob, b"abc");
        assert_eq!(&framed[..7], b"blob 3\0");
        assert_eq!(&framed[7..], b"abc");
    }

    #[test]
    fn decode_rejects_malformed() {
        // No NUL.
        assert_eq!(decode_loose(b"blob 3 abc").unwrap_err(), LooseError::NoNul);
        // No space in header.
        assert!(matches!(
            decode_loose(b"blob3\0abc").unwrap_err(),
            LooseError::BadHeader
        ));
        // Unknown kind.
        assert!(matches!(
            decode_loose(b"widget 3\0abc").unwrap_err(),
            LooseError::UnknownKind(_)
        ));
        // Non-numeric length.
        assert!(matches!(
            decode_loose(b"blob xx\0abc").unwrap_err(),
            LooseError::BadLength
        ));
        // Length mismatch (header says 99, body is 3).
        assert!(matches!(
            decode_loose(b"blob 99\0abc").unwrap_err(),
            LooseError::LengthMismatch {
                declared: 99,
                actual: 3
            }
        ));
    }

    // ── A. CasClient route/auth/mapping ───────────────────────────────────────

    /// An in-memory CAS double that models CoreLink: keyed by blake3, and
    /// VERIFIES the key == blake3(body) on PUT (anti-poisoning) — a mismatch is a
    /// rejected write (HTTP 400-class), exactly like the live store.
    #[derive(Default)]
    struct MapCasTransport {
        objects: std::sync::Mutex<BTreeMap<String, Vec<u8>>>,
        /// Records the last scope header seen on GET/PUT for the auth assertion.
        last_get_bearer: std::sync::Mutex<Option<String>>,
        /// If set, batch UPLOAD marks this many of the first objects in each batch
        /// as `status:"error"` (partial-failure modeling). Decremented per object
        /// failed so a RETRY of just the failed ones eventually succeeds.
        fail_uploads: std::sync::Mutex<usize>,
        /// If `Some(n)`, batch UPLOAD/READ enforces a HARD per-request cap of `n`
        /// objects, answering 413 `batch_too_large` above it (to test the client's
        /// split-and-retry without needing a 2000-object fixture).
        small_cap: Option<usize>,
        /// If set, the batch endpoints (`/batch`, `/batch-read`, `/batch-exists`)
        /// answer this status (405 or 404) — modeling an environment where the bulk
        /// plane is NOT deployed — while single-object GET/PUT keep serving. Drives
        /// the per-object auto-fallback.
        batch_absent_status: Option<u16>,
    }

    impl MapCasTransport {
        fn key_from_url(url: &str) -> String {
            url.rsplit('/').next().unwrap_or("").to_string()
        }

        /// Serve `POST /batch` per the FROZEN contract: parse the NDJSON manifest +
        /// blank line + concatenated bytes, verify framing (400) + caps (413), and
        /// per object verify `blake3(bytes)==hash` (else that object → `error`).
        fn serve_batch_upload(&self, body: &[u8]) -> Result<(u16, Vec<u8>), CasError> {
            let sep = match find_blank_line(body) {
                Some(s) => s,
                None => return Ok((400, b"{\"error\":\"no manifest separator\"}".to_vec())),
            };
            let manifest = &body[..sep];
            let bytes_region = &body[sep + 2..];
            let mut entries: Vec<(String, usize)> = Vec::new();
            let mut total_len = 0usize;
            for line in manifest.split(|&b| b == b'\n') {
                if line.is_empty() {
                    continue;
                }
                let ml: ManifestLine = match serde_json::from_slice(line) {
                    Ok(m) => m,
                    Err(_) => return Ok((400, b"{\"error\":\"bad manifest line\"}".to_vec())),
                };
                total_len += ml.len as usize;
                entries.push((ml.hash, ml.len as usize));
            }
            // Framing: Σlen MUST equal the byte region exactly (no truncation/excess).
            if total_len != bytes_region.len() {
                return Ok((400, b"{\"error\":\"byte length mismatch\"}".to_vec()));
            }
            // Cap (413).
            if let Some(cap) = self.small_cap
                && entries.len() > cap
            {
                return Ok((413, batch_too_large_body()));
            }
            // Walk the bytes, verify each object, optionally inject failures.
            let mut cursor = 0usize;
            let mut results = Vec::new();
            let mut to_fail = *self.fail_uploads.lock().unwrap();
            for (hash, len) in entries {
                let bytes = &bytes_region[cursor..cursor + len];
                cursor += len;
                if to_fail > 0 {
                    to_fail -= 1;
                    *self.fail_uploads.lock().unwrap() -= 1;
                    results.push(serde_json::json!({
                        "hash": hash, "status": "error", "error": "injected fault"
                    }));
                    continue;
                }
                // Content-verify (anti-poisoning), exactly like the single PUT.
                if cas_key(bytes) != hash {
                    results.push(serde_json::json!({
                        "hash": hash, "status": "error", "error": "hash mismatch"
                    }));
                    continue;
                }
                let fresh = self
                    .objects
                    .lock()
                    .unwrap()
                    .insert(hash.clone(), bytes.to_vec())
                    .is_none();
                results.push(serde_json::json!({
                    "hash": hash,
                    "status": if fresh { "created" } else { "exists" },
                    "error": serde_json::Value::Null,
                }));
            }
            Ok((200, serde_json::to_vec(&results).unwrap()))
        }

        /// Serve `POST /batch-read`: parse the NDJSON hash list, enforce the cap on
        /// RETURNED bytes (413), and emit the manifest + blank line + concatenated
        /// `ok` bytes (absent ones → `status:"absent"`, no bytes).
        fn serve_batch_read(&self, body: &[u8]) -> Result<(u16, Vec<u8>), CasError> {
            let mut hashes: Vec<String> = Vec::new();
            for line in body.split(|&b| b == b'\n') {
                if line.is_empty() {
                    continue;
                }
                let hl: HashLine = match serde_json::from_slice(line) {
                    Ok(h) => h,
                    Err(_) => return Ok((400, b"{\"error\":\"bad hash line\"}".to_vec())),
                };
                hashes.push(hl.hash);
            }
            let store = self.objects.lock().unwrap();
            // The cap is on RETURNED ok bytes: compute it, answer 413 if over.
            let returned: usize = hashes
                .iter()
                .filter_map(|h| store.get(h))
                .map(Vec::len)
                .sum();
            if let Some(cap) = self.small_cap
                && hashes.len() > cap
            {
                return Ok((413, batch_too_large_body()));
            }
            // Also model the real byte cap (returned bytes), so the byte-driven split
            // is exercised even without small_cap when a single read is huge.
            if returned > BATCH_MAX_BYTES && hashes.len() > 1 {
                return Ok((413, batch_too_large_body()));
            }
            let mut manifest = Vec::new();
            let mut bytes_region = Vec::new();
            for h in &hashes {
                match store.get(h) {
                    Some(bytes) => {
                        manifest.extend_from_slice(
                            serde_json::to_vec(&serde_json::json!({
                                "hash": h, "len": bytes.len() as u64, "status": "ok"
                            }))
                            .unwrap()
                            .as_slice(),
                        );
                        manifest.push(b'\n');
                        bytes_region.extend_from_slice(bytes);
                    }
                    None => {
                        manifest.extend_from_slice(
                            serde_json::to_vec(&serde_json::json!({
                                "hash": h, "len": 0u64, "status": "absent"
                            }))
                            .unwrap()
                            .as_slice(),
                        );
                        manifest.push(b'\n');
                    }
                }
            }
            let mut out = manifest;
            out.push(b'\n'); // blank-line separator.
            out.extend_from_slice(&bytes_region);
            Ok((200, out))
        }

        /// Serve `POST /batch-exists`: NDJSON hash list → `[{"hash","present"}]`.
        fn serve_batch_exists(&self, body: &[u8]) -> Result<(u16, Vec<u8>), CasError> {
            let mut results = Vec::new();
            let store = self.objects.lock().unwrap();
            for line in body.split(|&b| b == b'\n') {
                if line.is_empty() {
                    continue;
                }
                let hl: HashLine = match serde_json::from_slice(line) {
                    Ok(h) => h,
                    Err(_) => return Ok((400, b"{\"error\":\"bad hash line\"}".to_vec())),
                };
                let present = store.contains_key(&hl.hash);
                results.push(serde_json::json!({ "hash": hl.hash, "present": present }));
            }
            Ok((200, serde_json::to_vec(&results).unwrap()))
        }
    }

    /// The FROZEN 413 body shape.
    fn batch_too_large_body() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "error": "batch_too_large",
            "limit_objects": BATCH_MAX_OBJECTS,
            "limit_bytes": BATCH_MAX_BYTES,
        }))
        .unwrap()
    }

    impl CasTransport for MapCasTransport {
        fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), CasError> {
            *self.last_get_bearer.lock().unwrap() = Some(bearer.to_string());
            let key = Self::key_from_url(url);
            match self.objects.lock().unwrap().get(&key) {
                Some(bytes) => Ok((200, bytes.clone())),
                None => Ok((404, Vec::new())),
            }
        }

        fn put(&self, url: &str, _bearer: &str, body: &[u8]) -> Result<u16, CasError> {
            let key = Self::key_from_url(url);
            // Model CoreLink's verify-on-write: refuse a key that is not
            // blake3(body) (anti-poisoning) with a 400-class status.
            if key != cas_key(body) {
                return Ok(400);
            }
            let fresh = self
                .objects
                .lock()
                .unwrap()
                .insert(key, body.to_vec())
                .is_none();
            Ok(if fresh { 201 } else { 200 })
        }

        fn post(
            &self,
            url: &str,
            _bearer: &str,
            _scope: &str,
            content_type: &str,
            body: &[u8],
        ) -> Result<(u16, Vec<u8>), CasError> {
            // Bulk plane NOT deployed: the batch route answers 405/404 (regardless
            // of content type — the route simply does not exist).
            if let Some(code) = self.batch_absent_status {
                return Ok((code, Vec::new()));
            }
            // 415: the contract's wrong-Content-Type rejection.
            if content_type != BATCH_CONTENT_TYPE {
                return Ok((415, Vec::new()));
            }
            if url.ends_with("/batch") {
                self.serve_batch_upload(body)
            } else if url.ends_with("/batch-read") {
                self.serve_batch_read(body)
            } else if url.ends_with("/batch-exists") {
                self.serve_batch_exists(body)
            } else {
                Ok((404, Vec::new()))
            }
        }
    }

    fn client_with(transport: MapCasTransport) -> CasClient<MapCasTransport> {
        let cfg = CasConfig::new("https://cas.example", "tenant-1", "secret-pat").unwrap();
        CasClient::with_transport(cfg, transport)
    }

    #[test]
    fn endpoint_route_and_auth_are_the_confirmed_contract() {
        let cfg = CasConfig::new("https://cas.example/", "test-tenant-1", "p").unwrap();
        let key = "a".repeat(64);
        let url = cfg.endpoint(&key).unwrap();
        assert_eq!(
            url,
            format!("https://cas.example/v1/cas/test-tenant-1/{key}")
        );
        assert_eq!(cfg.bearer(), "Bearer p");
    }

    #[test]
    fn get_put_round_trip_and_status_mapping() {
        let client = client_with(MapCasTransport::default());
        let body = encode_loose(ObjectKind::Blob, b"hi");
        let key = cas_key(&body);

        // Absent → Ok(None).
        assert!(client.get(&key).unwrap().is_none());
        // PUT fresh → 201 → Ok.
        client.put(&key, &body).unwrap();
        // Now present → Ok(Some(bytes)).
        assert_eq!(client.get(&key).unwrap().unwrap(), body);
        // Idempotent re-PUT → 200 → Ok.
        client.put(&key, &body).unwrap();
        // The GET carried the Bearer PAT.
        assert_eq!(
            client.transport.last_get_bearer.lock().unwrap().as_deref(),
            Some("Bearer secret-pat")
        );
    }

    #[test]
    fn put_under_wrong_key_is_refused_by_the_store_double() {
        // Anti-poisoning: PUT bytes under a key that is NOT their blake3 → the
        // store (modeling CoreLink) refuses → CasError::Status.
        let client = client_with(MapCasTransport::default());
        let wrong = "b".repeat(64);
        let err = client.put(&wrong, b"some bytes").unwrap_err();
        assert!(matches!(err, CasError::Status(400)), "{err:?}");
    }

    #[test]
    fn get_with_non_hex_key_fails_before_any_request() {
        let client = client_with(MapCasTransport::default());
        let err = client.get("not-a-valid-key").unwrap_err();
        assert!(matches!(err, CasError::InvalidKey(_)), "{err:?}");
    }

    #[test]
    fn config_fails_closed_on_blank() {
        assert!(CasConfig::new("", "t", "p").is_err());
        assert!(CasConfig::new("u", "", "p").is_err());
        assert!(CasConfig::new("u", "t", "").is_err());
    }

    #[test]
    fn config_debug_redacts_pat() {
        let cfg = CasConfig::new("https://cas", "t", "super-secret-pat").unwrap();
        let dbg = format!("{cfg:?}");
        assert!(dbg.contains("<redacted>"));
        assert!(!dbg.contains("super-secret-pat"));
    }

    // ── D. R2 manifest + index shapes ─────────────────────────────────────────

    #[test]
    fn refs_manifest_round_trips_json() {
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), "a".repeat(40));
        let m = RefsManifest {
            head: "refs/heads/main".to_string(),
            refs,
        };
        let bytes = serde_json::to_vec(&m).unwrap();
        assert_eq!(parse_refs_manifest(&bytes).unwrap(), m);
    }

    #[test]
    fn oid_index_round_trips_json() {
        let mut idx: OidIndex = BTreeMap::new();
        idx.insert("a".repeat(40), "b".repeat(64));
        let bytes = serde_json::to_vec(&idx).unwrap();
        assert_eq!(parse_oid_index(&bytes).unwrap(), idx);
    }

    #[test]
    fn r2_keys_are_tenant_repo_scoped() {
        assert_eq!(refs_manifest_key("t", "hugit"), "t/hugit/refs.json");
        assert_eq!(oid_index_key("t", "hugit"), "t/hugit/oid-index.json");
    }

    // ── E + G. load_from_cas + ingest, against in-memory CAS+R2 doubles ────────

    /// An in-memory R2 double for the manifests (refs.json / oid-index.json).
    #[derive(Default, Clone)]
    struct MapR2 {
        objects: std::sync::Arc<std::sync::Mutex<BTreeMap<String, Vec<u8>>>>,
    }

    impl R2Get for MapR2 {
        fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
            Ok(self.objects.lock().unwrap().get(key).cloned())
        }
    }

    impl R2Put for MapR2 {
        fn put_object(&self, key: &str, body: &[u8]) -> Result<(), String> {
            self.objects
                .lock()
                .unwrap()
                .insert(key.to_string(), body.to_vec());
            Ok(())
        }
    }

    /// Build the raw bytes of a single-entry git tree: `<mode> <name>\0<20-byte oid>`.
    fn build_tree(mode: &str, name: &str, oid: gix_hash::ObjectId) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(mode.as_bytes());
        out.push(b' ');
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        out.extend_from_slice(oid.as_bytes());
        out
    }

    /// Build a minimal git commit object body pointing at `tree`.
    fn build_commit(tree: gix_hash::ObjectId) -> Vec<u8> {
        format!(
            "tree {tree}\n\
             author Test <t@e> 0 +0000\n\
             committer Test <t@e> 0 +0000\n\
             \n\
             seed\n"
        )
        .into_bytes()
    }

    /// Compute a git object's SHA-1 oid the SAME way `CasObjectSource` does.
    fn git_oid(kind: ObjectKind, body: &[u8]) -> gix_hash::ObjectId {
        hugit_proto::GitObject::new(kind, body.to_vec()).oid()
    }

    /// Seed a complete `(blob → tree → commit)` repo into a CAS double + an R2
    /// double (refs.json + oid-index.json). Returns the doubles, the head oid hex,
    /// and the blob oid + content for resolution assertions.
    #[allow(clippy::type_complexity)]
    fn seed_repo() -> (
        CasClient<MapCasTransport>,
        MapR2,
        String,
        gix_hash::ObjectId,
        Vec<u8>,
    ) {
        let blob_body = b"hello from CAS\n".to_vec();
        let blob_oid = git_oid(ObjectKind::Blob, &blob_body);
        let tree_body = build_tree("100644", "README.md", blob_oid);
        let tree_oid = git_oid(ObjectKind::Tree, &tree_body);
        let commit_body = build_commit(tree_oid);
        let commit_oid = git_oid(ObjectKind::Commit, &commit_body);

        let objects = [
            IngestObject {
                git_oid: blob_oid.to_string(),
                kind: ObjectKind::Blob,
                body: blob_body.clone(),
            },
            IngestObject {
                git_oid: tree_oid.to_string(),
                kind: ObjectKind::Tree,
                body: tree_body,
            },
            IngestObject {
                git_oid: commit_oid.to_string(),
                kind: ObjectKind::Commit,
                body: commit_body,
            },
        ];

        let cas = client_with(MapCasTransport::default());
        let r2 = MapR2::default();
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), commit_oid.to_string());

        let n = ingest_repo(
            &cas,
            &r2,
            "tenant-1",
            "hugit",
            "refs/heads/main",
            &refs,
            &objects,
        )
        .expect("ingest succeeds");
        assert_eq!(n, 3);

        (cas, r2, commit_oid.to_string(), blob_oid, blob_body)
    }

    #[test]
    fn ingest_then_load_then_resolve_blob_round_trips() {
        let (cas, r2, commit_hex, blob_oid, blob_body) = seed_repo();

        // Load the closure back from the doubles.
        let (store, root_tree, refs) =
            load_from_cas(&cas, &r2, "tenant-1", "hugit").expect("load succeeds");

        // Refs + head root tree are set; the closure resolves the seeded file.
        assert_eq!(refs.get("refs/heads/main").unwrap(), &commit_hex);
        let (resolved_oid, bytes) =
            hugit_proto::resolve_blob_at_path(&store, &root_tree, "README.md")
                .unwrap()
                .expect("README.md resolves from the CAS-loaded tree");
        assert_eq!(resolved_oid, blob_oid);
        assert_eq!(bytes, blob_body);
    }

    #[test]
    fn ingest_puts_every_object_under_its_blake3_with_a_correct_index() {
        let (cas, r2, _commit_hex, _blob_oid, _blob_body) = seed_repo();

        // The oid-index maps every git oid to the blake3 of its loose framing, and
        // each blake3 is actually present in the CAS.
        let index_bytes = r2
            .get_object(&oid_index_key("tenant-1", "hugit"))
            .unwrap()
            .expect("oid-index.json present");
        let index = parse_oid_index(&index_bytes).unwrap();
        assert_eq!(index.len(), 3);
        for blake3 in index.values() {
            assert!(is_valid_cas_key(blake3));
            assert!(
                cas.get(blake3).unwrap().is_some(),
                "every indexed blake3 is present in the CAS"
            );
        }
    }

    #[test]
    fn load_fails_closed_when_an_object_is_missing() {
        let (cas, r2, _, _, _) = seed_repo();
        // Corrupt the index to reference a blake3 the CAS does not hold.
        let mut index: OidIndex = parse_oid_index(
            &r2.get_object(&oid_index_key("tenant-1", "hugit"))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        // Replace one entry's blake3 with a valid-shaped but absent key.
        let some_git_oid = index.keys().next().unwrap().clone();
        index.insert(some_git_oid, "c".repeat(64));
        r2.put_object(
            &oid_index_key("tenant-1", "hugit"),
            &serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();

        let err = load_from_cas(&cas, &r2, "tenant-1", "hugit").unwrap_err();
        assert!(err.contains("absent"), "{err}");
    }

    #[test]
    fn load_fails_closed_on_git_oid_mismatch_double_integrity() {
        // Double-integrity: a body whose git-oid ≠ the index oid is rejected even
        // though the blake3 matches the key. Seed an object under a WRONG git oid.
        let cas = client_with(MapCasTransport::default());
        let r2 = MapR2::default();

        let body = b"some blob".to_vec();
        let framed = encode_loose(ObjectKind::Blob, &body);
        let blake3 = cas_key(&framed);
        cas.put(&blake3, &framed).unwrap(); // blake3 is correct → CAS accepts.

        // The index claims this blake3 belongs to a DIFFERENT git oid.
        let wrong_git_oid = "0".repeat(40);
        let mut index: OidIndex = BTreeMap::new();
        index.insert(wrong_git_oid.clone(), blake3);
        r2.put_object(
            &oid_index_key("tenant-1", "hugit"),
            &serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), wrong_git_oid);
        let manifest = RefsManifest {
            head: "refs/heads/main".to_string(),
            refs,
        };
        r2.put_object(
            &refs_manifest_key("tenant-1", "hugit"),
            &serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let err = load_from_cas(&cas, &r2, "tenant-1", "hugit").unwrap_err();
        assert!(
            err.contains("git oid mismatch") || err.contains("double-integrity"),
            "{err}"
        );
    }

    #[test]
    fn load_fails_closed_when_cas_serves_blake3_mismatched_bytes() {
        // Model a mis-serving CAS: the index asks for blake3 K, but the bytes the
        // CAS returns hash to something else. The loader's read-side blake3
        // re-verify must reject it (CoreLink-verify modeled on our side too).
        //
        // We use a transport whose GET returns fixed bytes regardless of the key.
        struct MisservingTransport {
            bytes: Vec<u8>,
        }
        impl CasTransport for MisservingTransport {
            fn get(&self, _url: &str, _bearer: &str) -> Result<(u16, Vec<u8>), CasError> {
                Ok((200, self.bytes.clone()))
            }
            fn put(&self, _url: &str, _bearer: &str, _body: &[u8]) -> Result<u16, CasError> {
                Ok(201)
            }
            fn post(
                &self,
                _url: &str,
                _bearer: &str,
                _scope: &str,
                _content_type: &str,
                body: &[u8],
            ) -> Result<(u16, Vec<u8>), CasError> {
                // batch-read: serve the SAME fixed bytes for whatever hash is asked
                // (the misserving model). One hash requested → manifest ok + bytes.
                let mut hash = String::new();
                for line in body.split(|&b| b == b'\n') {
                    if line.is_empty() {
                        continue;
                    }
                    if let Ok(h) = serde_json::from_slice::<HashLine>(line) {
                        hash = h.hash;
                        break;
                    }
                }
                let mut out = serde_json::to_vec(&serde_json::json!({
                    "hash": hash, "len": self.bytes.len() as u64, "status": "ok"
                }))
                .unwrap();
                out.push(b'\n');
                out.push(b'\n');
                out.extend_from_slice(&self.bytes);
                Ok((200, out))
            }
        }
        let body = encode_loose(ObjectKind::Blob, b"the real bytes");
        let real_key = cas_key(&body);
        let cfg = CasConfig::new("https://cas", "t", "p").unwrap();
        // The transport serves `body` but the index will ask for a DIFFERENT key.
        let cas = CasClient::with_transport(cfg, MisservingTransport { bytes: body });
        let r2 = MapR2::default();
        let asked_key = "d".repeat(64); // valid shape, ≠ real_key.
        assert_ne!(asked_key, real_key);
        let git_oid_hex = "1".repeat(40);
        let mut index: OidIndex = BTreeMap::new();
        index.insert(git_oid_hex.clone(), asked_key);
        r2.put_object(
            &oid_index_key("t", "hugit"),
            &serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), git_oid_hex);
        let manifest = RefsManifest {
            head: "refs/heads/main".to_string(),
            refs,
        };
        r2.put_object(
            &refs_manifest_key("t", "hugit"),
            &serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let err = load_from_cas(&cas, &r2, "t", "hugit").unwrap_err();
        assert!(err.contains("content-address violation"), "{err}");
    }

    #[test]
    fn ingest_refuses_head_not_in_refs() {
        let cas = client_with(MapCasTransport::default());
        let r2 = MapR2::default();
        let refs = BTreeMap::new(); // empty → head not present.
        let err = ingest_repo(&cas, &r2, "t", "hugit", "refs/heads/main", &refs, &[])
            .expect_err("head must be in refs");
        assert!(err.contains("no entry in refs"), "{err}");
    }

    #[test]
    fn load_fails_closed_when_manifest_absent() {
        let cas = client_with(MapCasTransport::default());
        let r2 = MapR2::default(); // nothing seeded.
        let err = load_from_cas(&cas, &r2, "t", "hugit").unwrap_err();
        assert!(err.contains("refs.json absent"), "{err}");
    }

    // ── Batch ops: chunker, upload, read, exists ──────────────────────────────

    #[test]
    fn chunker_splits_by_object_count() {
        // 6000 tiny items → ceil(6000/2000) = 3 chunks of ≤2000.
        let items: Vec<(String, Vec<u8>)> = (0..6000)
            .map(|i| (format!("{i:064x}"), vec![0u8; 4]))
            .collect();
        let chunks = chunk_by_caps(&items, |(_, b)| b.len());
        assert_eq!(chunks.len(), 3);
        for c in &chunks {
            assert!(c.len() <= BATCH_MAX_OBJECTS);
            let bytes: usize = c.iter().map(|(_, b)| b.len()).sum();
            assert!(bytes <= BATCH_MAX_BYTES);
        }
        // No item lost and order preserved.
        let total: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(total, 6000);
    }

    #[test]
    fn chunker_splits_by_byte_cap() {
        // 5 items of 3 MiB each = 15 MiB → ≤8 MiB chunks → 2 fit per chunk → 3 chunks.
        let three_mib = 3 * 1024 * 1024;
        let items: Vec<(String, Vec<u8>)> = (0..5)
            .map(|i| (format!("{i:064x}"), vec![0u8; three_mib]))
            .collect();
        let chunks = chunk_by_caps(&items, |(_, b)| b.len());
        for c in &chunks {
            let bytes: usize = c.iter().map(|(_, b)| b.len()).sum();
            assert!(bytes <= BATCH_MAX_BYTES, "chunk over byte cap: {bytes}");
        }
        // 3 MiB items: 2 per 8 MiB chunk → [2,2,1].
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].len(), 2);
        assert_eq!(chunks[2].len(), 1);
    }

    #[test]
    fn chunker_emits_oversized_singleton() {
        // A single item over the byte cap is its own chunk (the 413-split surfaces it).
        let items: Vec<(String, Vec<u8>)> = vec![("a".repeat(64), vec![0u8; BATCH_MAX_BYTES + 1])];
        let chunks = chunk_by_caps(&items, |(_, b)| b.len());
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].len(), 1);
    }

    /// Build N distinct framed blob objects + their blake3 keys.
    fn framed_blobs(n: usize) -> Vec<(String, Vec<u8>)> {
        (0..n)
            .map(|i| {
                let framed = encode_loose(ObjectKind::Blob, format!("object-{i}").as_bytes());
                (cas_key(&framed), framed)
            })
            .collect()
    }

    #[test]
    fn batch_upload_round_trips_and_dedups() {
        let client = client_with(MapCasTransport::default());
        let objs = framed_blobs(5);
        let statuses = client.batch_upload(&objs).unwrap();
        assert_eq!(statuses.len(), 5);
        assert!(statuses.iter().all(|(_, s)| *s == UploadStatus::Created));
        // Re-upload → all `exists` (dedup hit), order preserved.
        let again = client.batch_upload(&objs).unwrap();
        assert!(again.iter().all(|(_, s)| *s == UploadStatus::Exists));
        for ((h1, _), (h2, _)) in objs.iter().zip(again.iter()) {
            assert_eq!(h1, h2);
        }
    }

    #[test]
    fn batch_upload_surfaces_partial_failure() {
        // Inject 2 failures; the first 2 objects come back `error`.
        let t = MapCasTransport::default();
        *t.fail_uploads.lock().unwrap() = 2;
        let client = client_with(t);
        let objs = framed_blobs(4);
        let statuses = client.batch_upload(&objs).unwrap();
        let errs = statuses
            .iter()
            .filter(|(_, s)| matches!(s, UploadStatus::Error(_)))
            .count();
        assert_eq!(errs, 2, "two injected failures surface as error");
    }

    #[test]
    fn upload_with_retry_recovers_injected_failures() {
        // 2 injected failures; the single retry of just those 2 finds the counter
        // exhausted → succeeds. ingest's upload_with_retry must not return an error.
        let t = MapCasTransport::default();
        *t.fail_uploads.lock().unwrap() = 2;
        let client = client_with(t);
        let objs = framed_blobs(4);
        upload_with_retry(&client, &objs).expect("retry recovers the transient failures");
        // All 4 are now present.
        for (h, _) in &objs {
            assert!(client.get(h).unwrap().is_some());
        }
    }

    #[test]
    fn batch_upload_413_triggers_client_split_and_succeeds() {
        // A hard cap of 2 objects/request → uploading 5 must split (5→2+2+1, then
        // each half ≤2) and ultimately commit all 5.
        let t = MapCasTransport {
            small_cap: Some(2),
            ..Default::default()
        };
        let client = client_with(t);
        let objs = framed_blobs(5);
        let statuses = client.batch_upload(&objs).unwrap();
        // The chunker makes ONE chunk of 5 (under the FROZEN 2000 cap); the server's
        // small_cap=2 forces 413 → the client halves until ≤2 → all created.
        assert_eq!(statuses.len(), 5);
        assert!(statuses.iter().all(|(_, s)| *s == UploadStatus::Created));
        for (h, _) in &objs {
            assert!(client.get(h).unwrap().is_some());
        }
    }

    #[test]
    fn batch_upload_415_on_wrong_content_type_is_hard_error() {
        // Drive the transport's post directly with a wrong content type → 415.
        let t = MapCasTransport::default();
        let (status, _) = t
            .post(
                "https://x/v1/cas/t/batch",
                "b",
                SCOPE_READ_WRITE,
                "application/json",
                b"",
            )
            .unwrap();
        assert_eq!(status, 415);
    }

    #[test]
    fn batch_upload_400_on_truncated_body_is_hard_error() {
        // A manifest claiming len=10 but only 3 body bytes → framing 400.
        let t = MapCasTransport::default();
        let mut body =
            serde_json::to_vec(&serde_json::json!({"hash":"a".repeat(64),"len":10})).unwrap();
        body.push(b'\n');
        body.push(b'\n');
        body.extend_from_slice(b"abc"); // only 3 bytes, manifest said 10.
        let (status, _) = t
            .post(
                "https://x/v1/cas/t/batch",
                "b",
                SCOPE_READ_WRITE,
                BATCH_CONTENT_TYPE,
                &body,
            )
            .unwrap();
        assert_eq!(status, 400);
    }

    #[test]
    fn batch_read_reconstructs_bytes_by_len() {
        let client = client_with(MapCasTransport::default());
        let objs = framed_blobs(3);
        client.batch_upload(&objs).unwrap();
        let hashes: Vec<String> = objs.iter().map(|(h, _)| h.clone()).collect();
        let read = client.batch_read(&hashes).unwrap();
        assert_eq!(read.len(), 3);
        for ((h, expect), (rh, got)) in objs.iter().zip(read.iter()) {
            assert_eq!(h, rh, "order preserved");
            assert_eq!(got.as_ref().unwrap(), expect, "bytes reconstructed by len");
        }
    }

    #[test]
    fn batch_read_absent_is_none() {
        let client = client_with(MapCasTransport::default());
        let absent = vec!["e".repeat(64)];
        let read = client.batch_read(&absent).unwrap();
        assert_eq!(read.len(), 1);
        assert!(read[0].1.is_none(), "absent → None");
    }

    #[test]
    fn batch_exists_correctness() {
        let client = client_with(MapCasTransport::default());
        let objs = framed_blobs(3);
        client.batch_upload(&objs[..2]).unwrap(); // upload only the first 2.
        let hashes: Vec<String> = objs.iter().map(|(h, _)| h.clone()).collect();
        let exists = client.batch_exists(&hashes).unwrap();
        assert_eq!(exists.len(), 3);
        assert!(exists[0].1 && exists[1].1, "first two present");
        assert!(!exists[2].1, "third absent");
    }

    #[test]
    fn ingest_dedups_only_missing_uploaded() {
        // Seed one repo; re-ingest the SAME closure → batch-exists finds all present
        // → zero uploads. We assert via a transport that counts writes implicitly:
        // a second ingest succeeds and the object set is unchanged.
        let (cas, r2, _commit_hex, _blob_oid, _blob_body) = seed_repo();
        let before = cas.transport.objects.lock().unwrap().len();
        // Re-run ingest with the same objects → all already present.
        let blob_body = b"hello from CAS\n".to_vec();
        let blob_oid = git_oid(ObjectKind::Blob, &blob_body);
        let tree_body = build_tree("100644", "README.md", blob_oid);
        let tree_oid = git_oid(ObjectKind::Tree, &tree_body);
        let commit_body = build_commit(tree_oid);
        let commit_oid = git_oid(ObjectKind::Commit, &commit_body);
        let objects = [
            IngestObject {
                git_oid: blob_oid.to_string(),
                kind: ObjectKind::Blob,
                body: blob_body,
            },
            IngestObject {
                git_oid: tree_oid.to_string(),
                kind: ObjectKind::Tree,
                body: tree_body,
            },
            IngestObject {
                git_oid: commit_oid.to_string(),
                kind: ObjectKind::Commit,
                body: commit_body,
            },
        ];
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), commit_oid.to_string());
        ingest_repo(
            &cas,
            &r2,
            "tenant-1",
            "hugit",
            "refs/heads/main",
            &refs,
            &objects,
        )
        .unwrap();
        let after = cas.transport.objects.lock().unwrap().len();
        assert_eq!(
            before, after,
            "re-ingest of identical closure uploads nothing new"
        );
    }

    #[test]
    fn ingest_then_load_batched_resolves_blob_with_413_split() {
        // End-to-end under a small server cap so BOTH ingest upload and load read
        // exercise the 413 split, then resolve the seeded file.
        let blob_body = b"hello batched CAS\n".to_vec();
        let blob_oid = git_oid(ObjectKind::Blob, &blob_body);
        let tree_body = build_tree("100644", "README.md", blob_oid);
        let tree_oid = git_oid(ObjectKind::Tree, &tree_body);
        let commit_body = build_commit(tree_oid);
        let commit_oid = git_oid(ObjectKind::Commit, &commit_body);
        let objects = [
            IngestObject {
                git_oid: blob_oid.to_string(),
                kind: ObjectKind::Blob,
                body: blob_body.clone(),
            },
            IngestObject {
                git_oid: tree_oid.to_string(),
                kind: ObjectKind::Tree,
                body: tree_body,
            },
            IngestObject {
                git_oid: commit_oid.to_string(),
                kind: ObjectKind::Commit,
                body: commit_body,
            },
        ];
        let cfg = CasConfig::new("https://cas.example", "tenant-1", "secret-pat").unwrap();
        let cas = CasClient::with_transport(
            cfg,
            MapCasTransport {
                small_cap: Some(1), // force a 413 split on every multi-object batch.
                ..Default::default()
            },
        );
        let r2 = MapR2::default();
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), commit_oid.to_string());
        ingest_repo(
            &cas,
            &r2,
            "tenant-1",
            "hugit",
            "refs/heads/main",
            &refs,
            &objects,
        )
        .unwrap();

        let (store, root_tree, _refs) =
            load_from_cas(&cas, &r2, "tenant-1", "hugit").expect("batched load succeeds");
        let (resolved_oid, bytes) =
            hugit_proto::resolve_blob_at_path(&store, &root_tree, "README.md")
                .unwrap()
                .expect("README.md resolves from the batch-loaded tree");
        assert_eq!(resolved_oid, blob_oid);
        assert_eq!(bytes, blob_body);
    }

    // ── Bulk-plane-absent auto-fallback (405/404 on the batch route) ───────────

    /// The three batch ops surface a 405/404 on the batch route as
    /// `BulkUnsupported`, distinct from 413/415/400 and a per-object 404.
    #[test]
    fn batch_ops_report_bulk_unsupported_on_405_and_404() {
        for code in [405u16, 404] {
            let t = MapCasTransport {
                batch_absent_status: Some(code),
                ..Default::default()
            };
            let client = client_with(t);
            let objs = framed_blobs(3);
            let hashes: Vec<String> = objs.iter().map(|(h, _)| h.clone()).collect();

            let up = client.batch_upload(&objs).unwrap_err();
            assert!(
                matches!(up, CasError::BulkUnsupported(c) if c == code),
                "{up:?}"
            );
            let rd = client.batch_read(&hashes).unwrap_err();
            assert!(
                matches!(rd, CasError::BulkUnsupported(c) if c == code),
                "{rd:?}"
            );
            let ex = client.batch_exists(&hashes).unwrap_err();
            assert!(
                matches!(ex, CasError::BulkUnsupported(c) if c == code),
                "{ex:?}"
            );
        }
    }

    /// `ingest_repo` against a bulk-absent (405) double succeeds via per-object PUT;
    /// every object lands; refs/index are written; it round-trips through
    /// `load_from_cas` (which ALSO falls back to per-object GET here).
    #[test]
    fn ingest_falls_back_to_per_object_when_bulk_absent_405() {
        let blob_body = b"hello no-bulk CAS\n".to_vec();
        let blob_oid = git_oid(ObjectKind::Blob, &blob_body);
        let tree_body = build_tree("100644", "README.md", blob_oid);
        let tree_oid = git_oid(ObjectKind::Tree, &tree_body);
        let commit_body = build_commit(tree_oid);
        let commit_oid = git_oid(ObjectKind::Commit, &commit_body);
        let objects = [
            IngestObject {
                git_oid: blob_oid.to_string(),
                kind: ObjectKind::Blob,
                body: blob_body.clone(),
            },
            IngestObject {
                git_oid: tree_oid.to_string(),
                kind: ObjectKind::Tree,
                body: tree_body,
            },
            IngestObject {
                git_oid: commit_oid.to_string(),
                kind: ObjectKind::Commit,
                body: commit_body,
            },
        ];
        let cfg = CasConfig::new("https://cas.example", "tenant-1", "secret-pat").unwrap();
        let cas = CasClient::with_transport(
            cfg,
            MapCasTransport {
                batch_absent_status: Some(405), // bulk plane not deployed.
                ..Default::default()
            },
        );
        let r2 = MapR2::default();
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), commit_oid.to_string());
        let n = ingest_repo(
            &cas,
            &r2,
            "tenant-1",
            "hugit",
            "refs/heads/main",
            &refs,
            &objects,
        )
        .expect("ingest succeeds via per-object fallback");
        assert_eq!(n, 3);

        // Every object landed in the CAS (PUT'd individually).
        assert_eq!(cas.transport.objects.lock().unwrap().len(), 3);
        // refs.json + oid-index.json were published.
        assert!(
            r2.get_object(&refs_manifest_key("tenant-1", "hugit"))
                .unwrap()
                .is_some()
        );
        assert!(
            r2.get_object(&oid_index_key("tenant-1", "hugit"))
                .unwrap()
                .is_some()
        );

        // Round-trip back through load_from_cas (also bulk-absent → per-object GET).
        let (store, root_tree, refs) =
            load_from_cas(&cas, &r2, "tenant-1", "hugit").expect("load via per-object GET");
        assert_eq!(
            refs.get("refs/heads/main").unwrap(),
            &commit_oid.to_string()
        );
        let (resolved_oid, bytes) =
            hugit_proto::resolve_blob_at_path(&store, &root_tree, "README.md")
                .unwrap()
                .expect("README.md resolves from the per-object-loaded tree");
        assert_eq!(resolved_oid, blob_oid);
        assert_eq!(bytes, blob_body);
    }

    /// `load_from_cas` against a bulk-absent (404) double succeeds via per-object
    /// GET. (Seed via the normal bulk path, then re-point the client at a
    /// bulk-absent transport that shares the same object store.)
    #[test]
    fn load_falls_back_to_per_object_get_when_bulk_absent_404() {
        // Seed normally (bulk plane present).
        let (cas, r2, commit_hex, blob_oid, blob_body) = seed_repo();
        // Move the seeded objects into a bulk-absent transport.
        let seeded = cas.transport.objects.lock().unwrap().clone();
        let cfg = CasConfig::new("https://cas.example", "tenant-1", "secret-pat").unwrap();
        let no_bulk = MapCasTransport {
            objects: std::sync::Mutex::new(seeded),
            batch_absent_status: Some(404),
            ..Default::default()
        };
        let cas = CasClient::with_transport(cfg, no_bulk);

        let (store, root_tree, refs) =
            load_from_cas(&cas, &r2, "tenant-1", "hugit").expect("load via per-object GET");
        assert_eq!(refs.get("refs/heads/main").unwrap(), &commit_hex);
        let (resolved_oid, bytes) =
            hugit_proto::resolve_blob_at_path(&store, &root_tree, "README.md")
                .unwrap()
                .expect("README.md resolves via the per-object GET fallback");
        assert_eq!(resolved_oid, blob_oid);
        assert_eq!(bytes, blob_body);
    }

    /// LAZY load: boot reads only the manifests (no full-closure pre-load); a blob
    /// resolves by fetching its objects from the CAS ON DEMAND, byte-identical to
    /// the eager path, and a second read serves from the cache.
    #[test]
    fn lazy_source_resolves_blob_on_demand_and_caches() {
        let (cas, r2, commit_hex, blob_oid, blob_body) = seed_repo();
        let (lazy, root_tree, refs) =
            load_manifests_from_cas(cas, &r2, "tenant-1", "hugit").expect("manifests load lazily");
        assert_eq!(refs.get("refs/heads/main").unwrap(), &commit_hex);
        // The blob is fetched on demand (it was never pre-loaded at boot).
        let (resolved_oid, bytes) =
            hugit_proto::resolve_blob_at_path(&lazy, &root_tree, "README.md")
                .unwrap()
                .expect("README.md resolves on demand");
        assert_eq!(resolved_oid, blob_oid);
        assert_eq!(bytes, blob_body);
        // Second resolve serves from the cache → identical bytes.
        let again = hugit_proto::resolve_blob_at_path(&lazy, &root_tree, "README.md")
            .unwrap()
            .expect("second resolve from cache");
        assert_eq!(again.1, blob_body);
        // contains() reflects the index without a fetch.
        use hugit_proto::ObjectSource as _;
        assert!(lazy.contains(&blob_oid));
    }

    /// Fail-closed: an oid the index references but the CAS no longer holds fails as
    /// a `PackError` (content seam incomplete) — never a silent empty serve. An oid
    /// NOT in the index is a clean absence (`Ok(None)`).
    #[test]
    fn lazy_source_fails_closed_when_indexed_object_absent() {
        use hugit_proto::ObjectSource as _;
        let cfg = CasConfig::new("https://cas.example", "tenant-1", "secret-pat").unwrap();
        // Empty store → every get is a 404 (absent).
        let cas = CasClient::with_transport(cfg, MapCasTransport::default());
        let body = b"missing object\n";
        let oid = git_oid(ObjectKind::Blob, body);
        let blake3 = cas_key(&encode_loose(ObjectKind::Blob, body));
        let mut index = BTreeMap::new();
        index.insert(oid, blake3);
        let lazy = LazyCasObjectSource::new(index, cas);
        // Indexed but absent in the CAS → fail closed.
        let err = lazy.get(&oid).unwrap_err();
        assert!(
            matches!(err, PackError::Source(_)),
            "indexed-but-absent must fail closed, got {err:?}"
        );
        // Not in the index → clean absence, not an error.
        let other = git_oid(ObjectKind::Blob, b"not indexed\n");
        assert!(lazy.get(&other).unwrap().is_none());
    }

    /// Live ref hot-swap, READ leg (frozen design §3): an oid the BOOT index did
    /// not know resolves through the SAME source — no new source, no reboot — once
    /// its `oid → blake3` is merged via the shared [`LiveOidIndex`] handle (exactly
    /// what `RepoState::apply_cas_push_inmemory` does after a push). Before the
    /// merge it is a CLEAN absence (`Ok(None)`); after, it resolves byte-identically.
    #[test]
    fn lazy_source_resolves_oid_merged_after_boot() {
        use hugit_proto::ObjectSource as _;
        // A real pushed object: body → loose framing → blake3 key + git SHA-1 oid.
        let body = b"freshly pushed blob\n";
        let framing = encode_loose(ObjectKind::Blob, body);
        let blake3 = cas_key(&framing);
        let oid = git_oid(ObjectKind::Blob, body);
        // The CAS already HOLDS the object (a push flushes objects BEFORE the
        // manifests + the in-memory merge), but the boot index does not know its oid.
        let mut seeded = BTreeMap::new();
        seeded.insert(blake3.clone(), framing.clone());
        let transport = MapCasTransport {
            objects: std::sync::Mutex::new(seeded),
            ..MapCasTransport::default()
        };
        let source = LazyCasObjectSource::new(BTreeMap::new(), client_with(transport));
        let handle = source.live_index_handle();

        // Before the merge: a clean absence (not in the index), never an error.
        assert!(!source.contains(&oid), "unknown oid is absent pre-merge");
        assert!(source.get(&oid).unwrap().is_none());

        // Merge the pushed oid → blake3 (the object-source leg of the hot-swap).
        handle.merge([(oid, blake3.clone())]);

        // After the merge, with NO new source: the SAME source resolves the oid,
        // double-integrity verified (re-derives to the asked oid).
        assert!(
            source.contains(&oid),
            "merged oid resolvable with no reboot"
        );
        let obj = source
            .get(&oid)
            .unwrap()
            .expect("oid resolves after the live merge");
        assert_eq!(
            obj.oid(),
            oid,
            "resolved bytes re-derive to the asked git oid (byte-identical)"
        );
    }

    /// Write-path-hardening #5: the decoded-object cache is BYTE-BOUNDED and evicts
    /// FIFO past its budget, and a miss-after-evict re-fetches byte-identically from
    /// CAS (content-addressing → eviction is free correctness-wise). Without the
    /// bound the cache grew unboundedly (RSS → OOM). git-free: all fixtures in-mem.
    #[test]
    fn lazy_cache_evicts_past_budget_and_refetches_correctly() {
        use hugit_proto::ObjectSource as _;
        // Build N distinct blobs, each ~1 KiB; seed the CAS + the index with all.
        const N: usize = 8;
        const BLOB_BYTES: usize = 1024;
        let mut seeded: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        let mut index: BTreeMap<gix_hash::ObjectId, String> = BTreeMap::new();
        let mut oids: Vec<gix_hash::ObjectId> = Vec::new();
        for i in 0..N {
            let body = vec![i as u8; BLOB_BYTES];
            let framing = encode_loose(ObjectKind::Blob, &body);
            let blake3 = cas_key(&framing);
            let oid = git_oid(ObjectKind::Blob, &body);
            seeded.insert(blake3.clone(), framing);
            index.insert(oid, blake3);
            oids.push(oid);
        }
        let transport = MapCasTransport {
            objects: std::sync::Mutex::new(seeded),
            ..MapCasTransport::default()
        };
        // Budget ≈ 2 blob bodies, so resident entries stay bounded ≤ 3 across N gets.
        let budget = 2 * BLOB_BYTES + BLOB_BYTES / 2;
        let source =
            LazyCasObjectSource::new_with_cache_budget(index, client_with(transport), budget);

        // Fetch every object once (each a cold fetch + cache insert).
        for oid in &oids {
            let got = source.get(oid).unwrap().expect("indexed object resolves");
            assert_eq!(got.oid(), *oid, "fetched bytes re-derive to the asked oid");
        }
        // The cache is BOUNDED — it did not retain all N objects.
        let resident = source.cache_len();
        assert!(
            resident <= 3,
            "cache bounded by budget: {resident} resident of {N} (≈2-blob budget)"
        );

        // A miss-after-evict: the FIRST oid was evicted long ago, yet re-getting it
        // re-fetches from CAS and serves byte-identically (correctness unaffected).
        let again = source
            .get(&oids[0])
            .unwrap()
            .expect("evicted object re-fetches from CAS");
        assert_eq!(
            again.oid(),
            oids[0],
            "miss-after-evict re-serves the exact bytes (byte-identical)"
        );
    }

    /// Unit: [`BoundedObjectCache`] keeps its byte sum within budget and evicts the
    /// OLDEST entry first (FIFO), but never drops the just-inserted entry even if it
    /// alone exceeds the budget (best-effort bound, never empty mid-serve).
    #[test]
    fn bounded_object_cache_evicts_fifo_within_budget() {
        let mk = |b: u8, n: usize| GitObject::new(ObjectKind::Blob, vec![b; n]);
        let oid_of = |b: u8, n: usize| git_oid(ObjectKind::Blob, &vec![b; n]);
        let mut cache = BoundedObjectCache::new(250);
        let (o1, o2, o3) = (oid_of(1, 100), oid_of(2, 100), oid_of(3, 100));
        cache.insert(o1, mk(1, 100));
        cache.insert(o2, mk(2, 100));
        // 200 ≤ 250: both resident.
        assert_eq!(cache.len(), 2);
        // Third pushes to 300 > 250 → evict the oldest (o1).
        cache.insert(o3, mk(3, 100));
        assert!(cache.bytes <= 250, "byte sum within budget after eviction");
        assert!(cache.get(&o1).is_none(), "oldest (o1) was evicted FIFO");
        assert!(cache.get(&o2).is_some(), "o2 still resident");
        assert!(cache.get(&o3).is_some(), "just-inserted o3 resident");

        // An object larger than the WHOLE budget is retained transiently (never
        // dropped on its own insert → cache never empty mid-serve).
        let big = oid_of(9, 1000);
        cache.insert(big, mk(9, 1000));
        assert!(
            cache.get(&big).is_some(),
            "oversized entry kept (best-effort)"
        );
        assert_eq!(cache.len(), 1, "everything else evicted to make room");
    }

    /// Double-integrity is STILL enforced on the per-object GET fallback path: a
    /// blake3 mismatch (the CAS serves bytes that don't hash to the asked key) fails
    /// closed even when the bulk plane is absent.
    #[test]
    fn load_per_object_fallback_still_enforces_blake3_integrity() {
        // A misserving transport that ALSO 405s the batch route, forcing per-object
        // GET — which returns bytes that don't hash to the requested key.
        struct NoBulkMisservingTransport {
            bytes: Vec<u8>,
        }
        impl CasTransport for NoBulkMisservingTransport {
            fn get(&self, _url: &str, _bearer: &str) -> Result<(u16, Vec<u8>), CasError> {
                Ok((200, self.bytes.clone()))
            }
            fn put(&self, _url: &str, _bearer: &str, _body: &[u8]) -> Result<u16, CasError> {
                Ok(201)
            }
            fn post(
                &self,
                _url: &str,
                _bearer: &str,
                _scope: &str,
                _content_type: &str,
                _body: &[u8],
            ) -> Result<(u16, Vec<u8>), CasError> {
                Ok((405, Vec::new())) // bulk plane absent → forces per-object GET.
            }
        }
        let body = encode_loose(ObjectKind::Blob, b"the real bytes");
        let real_key = cas_key(&body);
        let cfg = CasConfig::new("https://cas", "t", "p").unwrap();
        let cas = CasClient::with_transport(cfg, NoBulkMisservingTransport { bytes: body });
        let r2 = MapR2::default();
        let asked_key = "d".repeat(64); // valid shape, ≠ real_key.
        assert_ne!(asked_key, real_key);
        let git_oid_hex = "1".repeat(40);
        let mut index: OidIndex = BTreeMap::new();
        index.insert(git_oid_hex.clone(), asked_key);
        r2.put_object(
            &oid_index_key("t", "hugit"),
            &serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), git_oid_hex);
        let manifest = RefsManifest {
            head: "refs/heads/main".to_string(),
            refs,
        };
        r2.put_object(
            &refs_manifest_key("t", "hugit"),
            &serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();

        let err = load_from_cas(&cas, &r2, "t", "hugit").unwrap_err();
        assert!(err.contains("content-address violation"), "{err}");
    }

    /// A per-object GET 404 during the bulk-absent fallback load (a missing INDEXED
    /// object) is corruption and fails closed.
    #[test]
    fn load_per_object_fallback_fails_closed_on_missing_object() {
        // Seed normally then re-point at a bulk-absent transport with an EMPTY store
        // (the indexed object is gone) → per-object GET 404 → Ok(None) → fail closed.
        let (_cas, r2, _commit_hex, _blob_oid, _blob_body) = seed_repo();
        let cfg = CasConfig::new("https://cas.example", "tenant-1", "secret-pat").unwrap();
        let empty_no_bulk = MapCasTransport {
            batch_absent_status: Some(405),
            ..Default::default()
        };
        let cas = CasClient::with_transport(cfg, empty_no_bulk);
        let err = load_from_cas(&cas, &r2, "tenant-1", "hugit").unwrap_err();
        assert!(err.contains("absent") || err.contains("gone"), "{err}");
    }

    /// The normal bulk path still takes precedence when the double is NOT bulk-absent
    /// (a re-assertion that the default path is unchanged).
    #[test]
    fn normal_bulk_path_is_the_default_when_not_absent() {
        let (cas, r2, commit_hex, blob_oid, blob_body) = seed_repo();
        // No batch_absent_status → bulk ops serve normally.
        let (store, root_tree, refs) =
            load_from_cas(&cas, &r2, "tenant-1", "hugit").expect("bulk load succeeds");
        assert_eq!(refs.get("refs/heads/main").unwrap(), &commit_hex);
        let (resolved_oid, bytes) =
            hugit_proto::resolve_blob_at_path(&store, &root_tree, "README.md")
                .unwrap()
                .expect("README.md resolves via the bulk path");
        assert_eq!(resolved_oid, blob_oid);
        assert_eq!(bytes, blob_body);
    }

    /// 405 MID-UPLOAD: `batch_exists` succeeds (bulk present) but `batch_upload`
    /// then answers 405 (a partially-deployed server) → the remaining uploads
    /// degrade to per-object PUT and the ingest still succeeds.
    #[test]
    fn ingest_degrades_when_upload_405s_after_exists_ok() {
        /// A transport that serves `batch-exists` (all absent) + single-object
        /// GET/PUT normally, but answers 405 on `/batch` (upload) — modeling a server
        /// where exists is deployed but the upload route is not.
        #[derive(Default)]
        struct ExistsOkUpload405 {
            inner: MapCasTransport,
        }
        impl CasTransport for ExistsOkUpload405 {
            fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), CasError> {
                self.inner.get(url, bearer)
            }
            fn put(&self, url: &str, bearer: &str, body: &[u8]) -> Result<u16, CasError> {
                self.inner.put(url, bearer, body)
            }
            fn post(
                &self,
                url: &str,
                bearer: &str,
                scope: &str,
                content_type: &str,
                body: &[u8],
            ) -> Result<(u16, Vec<u8>), CasError> {
                if url.ends_with("/batch") {
                    return Ok((405, Vec::new())); // upload route not deployed.
                }
                self.inner.post(url, bearer, scope, content_type, body)
            }
        }
        let blob_body = b"mid-upload-405 blob\n".to_vec();
        let blob_oid = git_oid(ObjectKind::Blob, &blob_body);
        let tree_body = build_tree("100644", "README.md", blob_oid);
        let tree_oid = git_oid(ObjectKind::Tree, &tree_body);
        let commit_body = build_commit(tree_oid);
        let commit_oid = git_oid(ObjectKind::Commit, &commit_body);
        let objects = [
            IngestObject {
                git_oid: blob_oid.to_string(),
                kind: ObjectKind::Blob,
                body: blob_body.clone(),
            },
            IngestObject {
                git_oid: tree_oid.to_string(),
                kind: ObjectKind::Tree,
                body: tree_body,
            },
            IngestObject {
                git_oid: commit_oid.to_string(),
                kind: ObjectKind::Commit,
                body: commit_body,
            },
        ];
        let cfg = CasConfig::new("https://cas.example", "tenant-1", "secret-pat").unwrap();
        let cas = CasClient::with_transport(cfg, ExistsOkUpload405::default());
        let r2 = MapR2::default();
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), commit_oid.to_string());
        let n = ingest_repo(
            &cas,
            &r2,
            "tenant-1",
            "hugit",
            "refs/heads/main",
            &refs,
            &objects,
        )
        .expect("ingest succeeds despite upload-405 (per-object fallback)");
        assert_eq!(n, 3);
        assert_eq!(
            cas.transport.inner.objects.lock().unwrap().len(),
            3,
            "all objects PUT individually after the upload-405"
        );
    }

    // ── CAS error-body cap ────────────────────────────────────────────────────

    /// Verify that the MAX_ERR_BODY cap (64 KiB) works correctly by exercising
    /// the same `Read::take` logic used in `UreqCasTransport`.
    ///
    /// We can't easily spin a ureq HTTP server in a unit test, so we prove the
    /// cap by testing the `Read::take` mechanic directly on a large in-memory
    /// reader — this is the exact code path that `resp.into_reader().take(...)
    /// .read_to_end(&mut buf)` follows.
    #[test]
    fn cas_error_body_read_is_capped_at_64_kib() {
        use std::io::Read as _;
        const MAX_ERR_BODY: u64 = 64 * 1024;
        // Simulate a hostile 512 KiB error body.
        let large_body = vec![b'X'; 512 * 1024];
        let cursor = std::io::Cursor::new(&large_body);
        let mut buf = Vec::new();
        cursor
            .take(MAX_ERR_BODY)
            .read_to_end(&mut buf)
            .expect("take read must not error");
        assert_eq!(
            buf.len(),
            64 * 1024,
            "capped read must stop at 64 KiB, got {} bytes",
            buf.len()
        );
    }

    // ── H. CasRw — the buffering receive→CAS write adapter ────────────────────

    /// Build a git object's VERBATIM zlib-compressed loose bytes — the exact
    /// `CasObject.bytes` shape `receive_pack` reads off disk: `zlib(<kind> <len>\0
    /// <body>)`. The uncompressed pre-image is `encode_loose(kind, body)`.
    fn verbatim_zlib_loose(kind: ObjectKind, body: &[u8]) -> Vec<u8> {
        use std::io::Write as _;
        let framing = encode_loose(kind, body);
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&framing).expect("deflate");
        enc.finish().expect("finish")
    }

    /// THE load-bearing test: an object PUT through `CasRw` (from its verbatim zlib
    /// loose bytes) round-trips to the UNCOMPRESSED framing the CAS read side
    /// serves, AND is keyed by exactly the read-side `blake3(encode_loose(...))`.
    /// I.e. a pushed object is byte-identical to a served one — never the
    /// compressed bytes, never a mis-computed key.
    #[test]
    fn casrw_put_inflates_to_the_read_side_framing_and_key() {
        let body = b"fn main() { println!(\"a real pushed file\"); }";
        // What the CAS read side stores + hashes (the truth we must match).
        let read_side_framing = encode_loose(ObjectKind::Blob, body);
        let read_side_key = cas_key(&read_side_framing);

        // What receive_pack hands the adapter: the COMPRESSED loose bytes.
        let compressed = verbatim_zlib_loose(ObjectKind::Blob, body);
        assert_ne!(
            compressed, read_side_framing,
            "the adapter input is genuinely compressed, not the framing"
        );

        let oid = "fe8f5f1e013d57d0629ff3999a71986ffc2b05fb";
        let mut casrw = CasRw::new(client_with(MapCasTransport::default()), OidIndex::new());
        casrw.put(&CasObject {
            oid: oid.into(),
            bytes: compressed,
        });

        // get() returns the UNCOMPRESSED framing (what the read side would serve).
        assert_eq!(
            casrw.get(oid).as_deref(),
            Some(read_side_framing.as_slice()),
            "get() returns the inflated framing, byte-identical to the served form"
        );
        // …and it is buffered under the read side's blake3 key — NOT the zlib bytes'.
        assert_eq!(
            casrw.index_add.get(oid),
            Some(&read_side_key),
            "keyed by blake3(encode_loose), the SAME key the CAS read side computes"
        );
        assert!(casrw.contains(oid));
        assert_eq!(casrw.pending_len(), 1);
    }

    #[test]
    fn casrw_put_is_infallible_and_dedupes_identical_content() {
        let body = b"identical content";
        let mut casrw = CasRw::new(client_with(MapCasTransport::default()), OidIndex::new());
        // Two distinct git oids carrying byte-identical content (e.g. the same blob
        // referenced twice) → ONE buffered object (content-addressed dedup).
        casrw.put(&CasObject {
            oid: "a".repeat(40),
            bytes: verbatim_zlib_loose(ObjectKind::Blob, body),
        });
        casrw.put(&CasObject {
            oid: "b".repeat(40),
            bytes: verbatim_zlib_loose(ObjectKind::Blob, body),
        });
        assert_eq!(casrw.pending_len(), 1, "identical content buffers once");
        // Both oids resolve to the same framing.
        let framing = encode_loose(ObjectKind::Blob, body);
        assert_eq!(
            casrw.get(&"a".repeat(40)).as_deref(),
            Some(framing.as_slice())
        );
        assert_eq!(
            casrw.get(&"b".repeat(40)).as_deref(),
            Some(framing.as_slice())
        );
    }

    #[test]
    fn casrw_flush_uploads_exactly_the_buffered_set_byte_identical() {
        let mut casrw = CasRw::new(client_with(MapCasTransport::default()), OidIndex::new());
        // Buffer three distinct objects across kinds.
        let inputs: [(ObjectKind, &[u8]); 3] = [
            (ObjectKind::Blob, b"blob one"),
            (ObjectKind::Blob, b"blob two - longer contents here"),
            (ObjectKind::Tree, &[0u8, 1, 2, 3, 0, 255]),
        ];
        let mut want: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for (i, (kind, body)) in inputs.iter().enumerate() {
            let framing = encode_loose(*kind, body);
            want.insert(cas_key(&framing), framing);
            casrw.put(&CasObject {
                oid: format!("{i:040x}"),
                bytes: verbatim_zlib_loose(*kind, body),
            });
        }
        assert_eq!(casrw.pending_len(), 3);

        casrw.flush().expect("flush uploads the buffered set");

        // The mock CAS now holds EXACTLY the buffered set, keyed by blake3, with the
        // framing bytes (the mock's batch_upload verifies blake3(bytes)==hash, so a
        // store at all proves byte-identity to the read side).
        let stored = casrw.cas_client().transport.objects.lock().unwrap().clone();
        assert_eq!(
            stored.len(),
            want.len(),
            "exactly the buffered set, no more"
        );
        for (key, framing) in &want {
            assert_eq!(
                stored.get(key),
                Some(framing),
                "byte-identical under its key"
            );
        }
    }

    #[test]
    fn casrw_flush_empty_buffer_is_a_no_op() {
        let mut casrw = CasRw::new(client_with(MapCasTransport::default()), OidIndex::new());
        assert!(casrw.is_empty());
        casrw.flush().expect("empty flush is Ok");
        assert!(
            casrw
                .cas_client()
                .transport
                .objects
                .lock()
                .unwrap()
                .is_empty(),
            "no objects uploaded"
        );
    }

    #[test]
    fn casrw_flush_is_idempotent() {
        let mut casrw = CasRw::new(client_with(MapCasTransport::default()), OidIndex::new());
        casrw.put(&CasObject {
            oid: "c".repeat(40),
            bytes: verbatim_zlib_loose(ObjectKind::Blob, b"once"),
        });
        casrw.flush().expect("first flush");
        // A second flush re-PUTs the same content-addressed bytes (200 exists) — no
        // error, the store unchanged in size.
        casrw.flush().expect("re-flush is idempotent");
        assert_eq!(
            casrw.cas_client().transport.objects.lock().unwrap().len(),
            1
        );
    }

    #[test]
    fn casrw_reads_fall_through_to_the_existing_repo_closure() {
        // An object NOT in this push but in the repo's pre-push closure: it lives in
        // the CAS already, reachable via the existing oid→blake3 index.
        let body = b"a prior-tip object already in CAS";
        let framing = encode_loose(ObjectKind::Commit, body);
        let blake3 = cas_key(&framing);
        let prior_oid = "dd".repeat(20); // a 40-hex git oid

        let transport = MapCasTransport::default();
        transport
            .objects
            .lock()
            .unwrap()
            .insert(blake3.clone(), framing.clone());

        let mut existing = OidIndex::new();
        existing.insert(prior_oid.clone(), blake3);
        let casrw = CasRw::new(client_with(transport), existing);

        // contains() sees it (the anchor's already-live-in-CAS leg) with no buffer.
        assert!(casrw.contains(&prior_oid));
        // get() fetches it through the wrapped CAS read source.
        assert_eq!(casrw.get(&prior_oid).as_deref(), Some(framing.as_slice()));
        // An oid in neither the push nor the closure is cleanly absent.
        assert!(!casrw.contains(&"ee".repeat(20)));
        assert_eq!(casrw.get(&"ee".repeat(20)), None);
    }

    /// THE thin-pack / CAS-base-reachability seam: a REF_DELTA base (or an
    /// incremental-push ancestor) that lives ONLY in SERVER-SIDE history is resolved
    /// by `receive_pack` via `cas.get`/`cas.contains` on the `CasRw` adapter — which
    /// resolves a git-oid through the persisted `oid-index → blake3 → R2 (read
    /// source)` path, NOT just the in-push buffer. This proves (a) `contains`/`get`
    /// see server-side history, and (b) the bytes `get` returns are exactly what the
    /// receive-pack thin-base resolver decodes (the loose framing — `decode_loose`
    /// parity), so the proto seam consumes them directly.
    #[test]
    fn casrw_resolves_server_history_oid_for_thin_pack_base() {
        // A tree object that is part of the repo's server-side history (NOT in this
        // push): only in the persisted oid-index + the CAS, never buffered.
        let body: &[u8] = b"\x00\x01\x02tree-ish body bytes for a server-side base";
        let framing = encode_loose(ObjectKind::Tree, body);
        let blake3 = cas_key(&framing);
        let history_oid = "ab".repeat(20); // a 40-hex git oid in server history

        // Seed the CAS read source (the same git-oid→blake3→R2 path the read side uses).
        let transport = MapCasTransport::default();
        transport
            .objects
            .lock()
            .unwrap()
            .insert(blake3.clone(), framing.clone());

        // The persisted oid-index (re-read by `open_writer` at push time) maps the
        // git oid to its blake3 — server-side history, NOT an in-push write.
        let mut existing = OidIndex::new();
        existing.insert(history_oid.clone(), blake3);
        // An EMPTY push (no buffered objects): `get`/`contains` answer purely from
        // server-side history, exactly the thin-pack-base case.
        let casrw = CasRw::new(client_with(transport), existing);

        // (a) the anchor / reachability `contains` sees server history.
        assert!(
            casrw.contains(&history_oid),
            "contains resolves a server-side-history oid via the oid-index"
        );
        // (b) `get` returns the loose framing the proto thin-base resolver decodes.
        let got = casrw
            .get(&history_oid)
            .expect("get resolves the server-history object via oid-index→R2");
        let (kind, decoded) =
            decode_loose(&got).expect("get returns proto-decodable loose framing");
        assert_eq!(kind, ObjectKind::Tree);
        assert_eq!(
            decoded.as_slice(),
            body,
            "byte-identical to the server-side base"
        );
    }

    #[test]
    fn casrw_put_buffers_over_an_existing_object_then_serves_the_push() {
        // The pushed object's oid is also in the existing closure (a re-push): the
        // in-flight buffer is consulted FIRST, so get() serves the freshly-inflated
        // framing without a network read.
        let body = b"re-pushed object";
        let framing = encode_loose(ObjectKind::Blob, body);
        let blake3 = cas_key(&framing);
        let oid = "ab".repeat(20);

        let mut existing = OidIndex::new();
        existing.insert(oid.clone(), blake3);
        // Note: the mock store is EMPTY — so a fall-through would return None. The
        // buffer-first rule means get() still succeeds.
        let mut casrw = CasRw::new(client_with(MapCasTransport::default()), existing);
        casrw.put(&CasObject {
            oid: oid.clone(),
            bytes: verbatim_zlib_loose(ObjectKind::Blob, body),
        });
        assert_eq!(casrw.get(&oid).as_deref(), Some(framing.as_slice()));
    }

    #[test]
    fn casrw_flush_fails_closed_on_a_non_inflatable_object() {
        let mut casrw = CasRw::new(client_with(MapCasTransport::default()), OidIndex::new());
        // A garbage (non-zlib) object: put() cannot inflate it. put() is infallible,
        // so the error is deferred — and flush() then refuses (fail-closed: a
        // corrupt object must never be silently dropped from the closure).
        casrw.put(&CasObject {
            oid: "f".repeat(40),
            bytes: vec![0xde, 0xad, 0xbe, 0xef],
        });
        let err = casrw
            .flush()
            .expect_err("a non-inflatable object fails the flush");
        assert!(matches!(err, CasError::Transport(_)), "{err:?}");
        // Nothing was uploaded.
        assert!(
            casrw
                .cas_client()
                .transport
                .objects
                .lock()
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn casrw_flush_degrades_to_per_object_when_bulk_plane_absent() {
        // Bulk plane not deployed (batch route 405s) → flush degrades to per-object
        // PUT, which is always live + idempotent. The object still lands.
        let transport = MapCasTransport {
            batch_absent_status: Some(405),
            ..MapCasTransport::default()
        };
        let mut casrw = CasRw::new(client_with(transport), OidIndex::new());
        let body = b"object via per-object fallback";
        let framing = encode_loose(ObjectKind::Blob, body);
        let key = cas_key(&framing);
        casrw.put(&CasObject {
            oid: "1".repeat(40),
            bytes: verbatim_zlib_loose(ObjectKind::Blob, body),
        });
        casrw.flush().expect("flush degrades to per-object PUT");
        assert_eq!(
            casrw
                .cas_client()
                .transport
                .objects
                .lock()
                .unwrap()
                .get(&key),
            Some(&framing),
            "the object landed via the single-object PUT path"
        );
    }

    // ── I. CAS-mode push finalize (objects → log → manifests, fail-closed) ─────

    /// Seed a `MapR2` with a repo's pre-push manifests: `refs.json`
    /// (HEAD = `refs/heads/main` at `old_tip`) + an `oid-index.json` holding
    /// `existing` (the pre-push `git_oid → blake3` closure).
    fn seed_manifests(r2: &MapR2, tenant: &str, repo: &str, old_tip: &str, existing: &OidIndex) {
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), old_tip.to_string());
        let manifest = RefsManifest {
            head: "refs/heads/main".to_string(),
            refs,
        };
        r2.put_object(
            &refs_manifest_key(tenant, repo),
            &serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        r2.put_object(
            &oid_index_key(tenant, repo),
            &serde_json::to_vec(existing).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn finalize_cas_push_uploads_objects_then_rewrites_manifests() {
        let (tenant, repo) = ("t", "hugit");
        let r2 = MapR2::default();
        // Pre-push closure: one prior object already indexed under the old tip.
        let old_tip = "aa".repeat(20);
        let mut existing = OidIndex::new();
        existing.insert(
            old_tip.clone(),
            cas_key(&encode_loose(ObjectKind::Commit, b"old tip body")),
        );
        seed_manifests(&r2, tenant, repo, &old_tip, &existing);

        let mut casrw = CasRw::new(client_with(MapCasTransport::default()), existing);
        // The push delivers two NEW objects (a blob + the new tip commit).
        let blob = b"the newly pushed file body";
        let new_tip = "bb".repeat(20);
        let blob_oid = "cc".repeat(20);
        casrw.put(&CasObject {
            oid: blob_oid.clone(),
            bytes: verbatim_zlib_loose(ObjectKind::Blob, blob),
        });
        casrw.put(&CasObject {
            oid: new_tip.clone(),
            bytes: verbatim_zlib_loose(ObjectKind::Commit, b"new tip commit body"),
        });
        let want_keys: Vec<String> = casrw.index_additions().values().cloned().collect();

        let mut persisted = false;
        finalize_cas_push(
            &mut casrw,
            &r2,
            tenant,
            repo,
            "refs/heads/pushed",
            &new_tip,
            || {
                persisted = true;
                Ok(())
            },
        )
        .expect("finalize succeeds");

        assert!(
            persisted,
            "the log-persist step ran (objects → LOG → manifests)"
        );

        // 1. Objects uploaded to the CAS (byte-identical framing under blake3).
        {
            let stored = casrw.cas_client().transport.objects.lock().unwrap();
            assert_eq!(stored.len(), want_keys.len(), "exactly the pushed set");
            for key in &want_keys {
                assert!(stored.contains_key(key), "pushed object {key} uploaded");
            }
        }
        // 2. oid-index.json merged: prior entry PRESERVED + new oids added.
        let idx_bytes = r2
            .get_object(&oid_index_key(tenant, repo))
            .unwrap()
            .unwrap();
        let idx = parse_oid_index(&idx_bytes).unwrap();
        assert!(idx.contains_key(&old_tip), "prior closure entry preserved");
        assert!(idx.contains_key(&blob_oid), "new blob oid indexed");
        assert!(idx.contains_key(&new_tip), "new tip oid indexed");
        // 3. refs.json: the new tip is advertised; the untouched ref is preserved.
        let refs_bytes = r2
            .get_object(&refs_manifest_key(tenant, repo))
            .unwrap()
            .unwrap();
        let m = parse_refs_manifest(&refs_bytes).unwrap();
        assert_eq!(m.refs.get("refs/heads/pushed"), Some(&new_tip));
        assert_eq!(
            m.refs.get("refs/heads/main"),
            Some(&old_tip),
            "an untouched ref is preserved by the read-merge-write"
        );
    }

    #[test]
    fn finalize_cas_push_flush_failure_aborts_before_log_and_manifests() {
        // THE fail-closed invariant: a flush/upload error aborts BEFORE the log AND
        // the manifests — the tip is never advertised without its uploaded closure.
        let (tenant, repo) = ("t", "hugit");
        let r2 = MapR2::default();
        let old_tip = "aa".repeat(20);
        let mut existing = OidIndex::new();
        existing.insert(
            old_tip.clone(),
            cas_key(&encode_loose(ObjectKind::Commit, b"old")),
        );
        seed_manifests(&r2, tenant, repo, &old_tip, &existing);

        let mut casrw = CasRw::new(client_with(MapCasTransport::default()), existing);
        // A non-inflatable (garbage) object: put() defers an inflate error; flush()
        // then FAILS closed → finalize must abort at step 1 (objects).
        casrw.put(&CasObject {
            oid: "ff".repeat(20),
            bytes: vec![0xde, 0xad, 0xbe, 0xef],
        });

        let mut persisted = false;
        let err = finalize_cas_push(
            &mut casrw,
            &r2,
            tenant,
            repo,
            "refs/heads/pushed",
            &"bb".repeat(20),
            || {
                persisted = true;
                Ok(())
            },
        )
        .expect_err("a flush failure fails the finalize");
        assert!(matches!(err, CasPushError::Flush(_)), "{err:?}");
        assert!(
            !persisted,
            "the log-persist step NEVER ran (aborted at objects)"
        );

        // Nothing uploaded.
        assert!(
            casrw
                .cas_client()
                .transport
                .objects
                .lock()
                .unwrap()
                .is_empty(),
            "no objects reached the CAS"
        );
        // The manifests are UNTOUCHED — the tip was never advertised.
        let refs_bytes = r2
            .get_object(&refs_manifest_key(tenant, repo))
            .unwrap()
            .unwrap();
        let m = parse_refs_manifest(&refs_bytes).unwrap();
        assert_eq!(m.refs.get("refs/heads/main"), Some(&old_tip));
        assert!(
            !m.refs.contains_key("refs/heads/pushed"),
            "no new tip advertised on a failed push"
        );
        let idx = parse_oid_index(
            &r2.get_object(&oid_index_key(tenant, repo))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(idx.len(), 1, "oid-index unchanged (only the prior entry)");
    }

    #[test]
    fn finalize_cas_push_persist_failure_does_not_advance_manifests() {
        // Objects flushed, but the LOG persist (compare-and-swap) failed → the ref
        // tip must NOT advance (the manifest must not advertise a tip whose
        // ref.update event was never recorded). Objects are durable + idempotent.
        let (tenant, repo) = ("t", "hugit");
        let r2 = MapR2::default();
        let old_tip = "aa".repeat(20);
        seed_manifests(&r2, tenant, repo, &old_tip, &OidIndex::new());

        let mut casrw = CasRw::new(client_with(MapCasTransport::default()), OidIndex::new());
        let new_tip = "bb".repeat(20);
        casrw.put(&CasObject {
            oid: new_tip.clone(),
            bytes: verbatim_zlib_loose(ObjectKind::Commit, b"tip body"),
        });

        let err = finalize_cas_push(
            &mut casrw,
            &r2,
            tenant,
            repo,
            "refs/heads/pushed",
            &new_tip,
            || Err("cas-conflict".to_string()),
        )
        .expect_err("a persist failure fails the finalize");
        assert!(matches!(err, CasPushError::Persist(_)), "{err:?}");

        // Objects DID upload (idempotent), but the tip did NOT advance.
        assert!(
            !casrw
                .cas_client()
                .transport
                .objects
                .lock()
                .unwrap()
                .is_empty(),
            "objects uploaded before the persist step"
        );
        let m = parse_refs_manifest(
            &r2.get_object(&refs_manifest_key(tenant, repo))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            !m.refs.contains_key("refs/heads/pushed"),
            "tip not advertised without the recorded event"
        );
    }

    #[test]
    fn cas_push_write_authz_rejects_non_owner_without_a_read_predicate_leak() {
        // The receive-pack handler gates a CAS push on `authorize_write` (OWNERSHIP),
        // NEVER the read-visibility predicate. Assert that gate directly: a PUBLIC
        // repo opens READS to any tenant, but a non-owner is DENIED the push.
        use crate::authz::{RepoMeta, Visibility, authorize_read, authorize_write};
        let meta = RepoMeta {
            visibility: Visibility::Public,
            owner_tenant: Some("org-a".to_string()),
        };
        let non_owner = vec!["clerk:org-b:user-9".to_string()];
        assert!(
            authorize_read(&non_owner, &meta),
            "public opens the clone (read) to any tenant"
        );
        assert!(
            !authorize_write(&non_owner, &meta),
            "but a non-owner is denied the push — write is ownership, not the read predicate"
        );
        // The owning tenant + the operator may push.
        assert!(authorize_write(&["clerk:org-a:user-1".to_string()], &meta));
        assert!(authorize_write(&["orchestrator:hugit".to_string()], &meta));
    }

    // ── J. CAS-mode delete-ref finalize (log → manifest, fail-closed) ──────────

    /// Seed a `MapR2` with a two-ref refs.json (HEAD = `refs/heads/main` at `main_tip`,
    /// plus a deletable `refs/heads/stale` at `stale_tip`) and an oid-index holding
    /// `existing`. Returns nothing — the caller reads the doubles back.
    fn seed_two_ref_manifests(
        r2: &MapR2,
        tenant: &str,
        repo: &str,
        main_tip: &str,
        stale_tip: &str,
        existing: &OidIndex,
    ) {
        let mut refs = BTreeMap::new();
        refs.insert("refs/heads/main".to_string(), main_tip.to_string());
        refs.insert("refs/heads/stale".to_string(), stale_tip.to_string());
        let manifest = RefsManifest {
            head: "refs/heads/main".to_string(),
            refs,
        };
        r2.put_object(
            &refs_manifest_key(tenant, repo),
            &serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        r2.put_object(
            &oid_index_key(tenant, repo),
            &serde_json::to_vec(existing).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn finalize_cas_delete_removes_ref_keeps_objects_and_other_refs() {
        let (tenant, repo) = ("t", "hugit");
        let r2 = MapR2::default();
        let main_tip = "aa".repeat(20);
        let stale_tip = "bb".repeat(20);
        let mut existing = OidIndex::new();
        existing.insert(
            stale_tip.clone(),
            cas_key(&encode_loose(ObjectKind::Commit, b"stale tip body")),
        );
        seed_two_ref_manifests(&r2, tenant, repo, &main_tip, &stale_tip, &existing);

        let mut persisted = false;
        finalize_cas_delete(&r2, tenant, repo, "refs/heads/stale", || {
            persisted = true;
            Ok(())
        })
        .expect("delete finalize succeeds");
        assert!(persisted, "the `ref.delete` log-persist step ran");

        // refs.json: the deleted ref is GONE, the untouched ref preserved.
        let m = parse_refs_manifest(
            &r2.get_object(&refs_manifest_key(tenant, repo))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            !m.refs.contains_key("refs/heads/stale"),
            "the deleted ref is removed from refs.json"
        );
        assert_eq!(
            m.refs.get("refs/heads/main"),
            Some(&main_tip),
            "an untouched ref survives the delete"
        );

        // oid-index UNCHANGED — a delete drops only the ref pointer, never the objects.
        let idx = parse_oid_index(
            &r2.get_object(&oid_index_key(tenant, repo))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            idx.contains_key(&stale_tip),
            "the deleted tip's object stays indexed (no GC)"
        );
    }

    #[test]
    fn finalize_cas_delete_persist_failure_does_not_remove_ref() {
        // A log-persist (compare-and-swap) fault aborts BEFORE the manifest rewrite —
        // the ref must stay advertised (fail-closed: the removal event is the gate).
        let (tenant, repo) = ("t", "hugit");
        let r2 = MapR2::default();
        let main_tip = "aa".repeat(20);
        let stale_tip = "bb".repeat(20);
        seed_two_ref_manifests(&r2, tenant, repo, &main_tip, &stale_tip, &OidIndex::new());

        let err = finalize_cas_delete(&r2, tenant, repo, "refs/heads/stale", || {
            Err("cas-conflict".to_string())
        })
        .expect_err("a persist fault fails the finalize");
        assert!(matches!(err, CasPushError::Persist(_)), "{err:?}");

        let m = parse_refs_manifest(
            &r2.get_object(&refs_manifest_key(tenant, repo))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            m.refs.get("refs/heads/stale"),
            Some(&stale_tip),
            "the ref is STILL advertised — never removed without the recorded event"
        );
    }

    #[test]
    fn finalize_cas_delete_manifest_failure_aborts_not_ok() {
        // THE fail-closed invariant for §5: an R2 refs.json PUT fault → a Manifest
        // error (NOT Ok). The handler maps it to an `ng`, never an `ok`.
        #[derive(Default, Clone)]
        struct PutFailsR2 {
            inner: MapR2,
        }
        impl R2Get for PutFailsR2 {
            fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
                self.inner.get_object(key)
            }
        }
        impl R2Put for PutFailsR2 {
            fn put_object(&self, _key: &str, _body: &[u8]) -> Result<(), String> {
                Err("injected R2 PUT fault".to_string())
            }
        }

        let (tenant, repo) = ("t", "hugit");
        let r2 = PutFailsR2::default();
        let main_tip = "aa".repeat(20);
        let stale_tip = "bb".repeat(20);
        seed_two_ref_manifests(
            &r2.inner,
            tenant,
            repo,
            &main_tip,
            &stale_tip,
            &OidIndex::new(),
        );

        let mut persisted = false;
        let err = finalize_cas_delete(&r2, tenant, repo, "refs/heads/stale", || {
            persisted = true;
            Ok(())
        })
        .expect_err("a manifest PUT fault fails the finalize");
        assert!(matches!(err, CasPushError::Manifest(_)), "{err:?}");
        assert!(
            persisted,
            "the persist ran (the fault is downstream, at the manifest PUT)"
        );

        // The ref is STILL present in the (un-PUT) refs.json — never `ok`, never removed.
        let m = parse_refs_manifest(
            &r2.inner
                .get_object(&refs_manifest_key(tenant, repo))
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            m.refs.get("refs/heads/stale"),
            Some(&stale_tip),
            "refs.json never lost the ref (the PUT was rejected)"
        );
    }
}
