//! Server state + repo→log resolution + the verified-load chokepoint.
//!
//! The `{repo}` slug arrives from the URL, so it is validated as a single safe
//! path segment (no traversal) BEFORE it is used — a hostile
//! `..%2F..%2Fetc%2Fpasswd` can never escape the source. Loading ALWAYS routes
//! through the engine's single verified loader
//! (`hugit_cli::checks::load_event_log[_from_bytes]` → `rehydrate_and_verify` →
//! `verify_chain`, PS-13) — a tampered chain fails CLOSED as 503, never
//! projected, REGARDLESS of source (local file or R2 object).
//!
//! ## Source (engine-storage)
//! - **Local** (`HUGIT_SERVE_LOG_DIR`): `<dir>/<repo>.json` — the dev/test default.
//! - **R2** (`HUGIT_SERVE_R2_*`): `<tenant_id>/<repo>.json` from the dedicated
//!   `corelink-githugr-engine` bucket over the S3 API, SigV4-signed
//!   ([`crate::sigv4`]). The bucket key contract is CoreLink's
//!   (`<tenant_id>/<repo>.json`; tenant = Clerk `publicMetadata.tenant_id`). Until
//!   real Clerk auth (the P2 identity seam) the tenant is the configured
//!   `HUGIT_SERVE_R2_TENANT_ID` (the single dev tenant) — disclosed, not faked.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hugit_refstore::EventLog;

use crate::error::EngineErr;
use crate::sigv4;
use crate::token::{ClerkValidator, TokenConfig, TokenStore};
use crate::writes::CasToken;

/// Where the engine reads canonical event logs from.
#[derive(Clone)]
pub enum LogSource {
    /// Local directory: `<dir>/<repo>.json`.
    Local { dir: PathBuf },
    /// R2 (S3-compatible) bucket: `<tenant_id>/<repo>.json`.
    R2(Box<R2Config>),
}

/// R2 read-source config (engine-storage Option A — the engine reads R2 directly).
#[derive(Clone)]
pub struct R2Config {
    /// `https://<account_id>.r2.cloudflarestorage.com` (no trailing slash).
    pub endpoint: String,
    /// `<account_id>.r2.cloudflarestorage.com` (the signed `host` header).
    pub host: String,
    /// `corelink-githugr-engine`.
    pub bucket: String,
    /// SigV4 region — R2 uses `auto`.
    pub region: String,
    pub key_id: String,
    pub secret: String,
    /// The tenant prefix; the configured dev tenant until the P2 Clerk seam.
    pub tenant_id: String,
    /// Sync HTTP client with a bounded timeout (no hanging reads).
    pub agent: ureq::Agent,
}

/// Immutable server configuration.
#[derive(Clone)]
pub struct AppState {
    /// Where event logs are read from (local dir | R2).
    pub source: LogSource,
    /// The Wave-1 dev Bearer token (the P2-Clerk stub). Fail-closed: required.
    pub dev_token: String,
    /// Clerk JWKS validator for `POST /v1/token` (None → dev-token-only; the
    /// endpoint 404s without it). The P2 Clerk identity seam.
    pub validator: Option<Arc<ClerkValidator>>,
    /// In-process engine-token store. Always present so the lookup gate compiles
    /// uniformly; stays empty until a Clerk exchange mints a token. Single-host
    /// (the multi-instance shared store is the same P2 seam as the idem ledger).
    pub token_store: Arc<TokenStore>,
}

impl AppState {
    /// Build from env. `HUGIT_ENGINE_DEV_TOKEN` is always required (fail-closed).
    /// If `HUGIT_SERVE_R2_ACCOUNT_ID` is set → the R2 source (all `R2_*` required);
    /// else the Local source (`HUGIT_SERVE_LOG_DIR` required).
    pub fn from_env() -> Result<Self, String> {
        let dev_token = std::env::var("HUGIT_ENGINE_DEV_TOKEN").map_err(|_| {
            "HUGIT_ENGINE_DEV_TOKEN is not set (fail-closed: refusing to start without an auth token)"
                .to_string()
        })?;
        if dev_token.trim().is_empty() {
            return Err("HUGIT_ENGINE_DEV_TOKEN is empty (fail-closed)".to_string());
        }

        // R2 is selected by EITHER the native ACCOUNT_ID or the S3-standard ENDPOINT
        // (so a standard cred file, which carries _ENDPOINT not _ACCOUNT_ID, selects R2).
        let r2_selected = std::env::var("HUGIT_SERVE_R2_ACCOUNT_ID").is_ok()
            || std::env::var("HUGIT_SERVE_R2_ENDPOINT").is_ok();
        let source = if r2_selected {
            Self::r2_from_env()?
        } else {
            let dir = std::env::var("HUGIT_SERVE_LOG_DIR")
                .map_err(|_| "HUGIT_SERVE_LOG_DIR is not set".to_string())?;
            LogSource::Local {
                dir: PathBuf::from(dir),
            }
        };
        let validator = TokenConfig::from_env()?.map(|cfg| Arc::new(cfg.into_validator()));
        let token_store = Arc::new(TokenStore::new());
        Ok(Self {
            source,
            dev_token,
            validator,
            token_store,
        })
    }

    fn r2_from_env() -> Result<LogSource, String> {
        Ok(LogSource::R2(Box::new(R2Config::from_env()?)))
    }

    /// Explicit Local constructor (tests).
    #[must_use]
    pub fn new(log_dir: PathBuf, dev_token: String) -> Self {
        Self {
            source: LogSource::Local { dir: log_dir },
            dev_token,
            validator: None,
            token_store: Arc::new(TokenStore::new()),
        }
    }

    /// A short label of the active source (for the boot log; no secrets).
    #[must_use]
    pub fn source_label(&self) -> String {
        match &self.source {
            LogSource::Local { dir } => format!("local:{}", dir.display()),
            LogSource::R2(c) => format!("r2:{}/{}/<tenant>", c.host, c.bucket),
        }
    }

    /// Load + chain-verify a repo's event log. A non-existent log (or an unsafe
    /// slug) → 404 (no existence leak). Any parse / tamper / transport fault →
    /// 503 ENGINE_UNAVAILABLE (fail-honest — never a fake-empty VM). The verify
    /// is identical for both sources (the PS-13 chokepoint).
    pub fn load_verified(&self, repo: &str) -> Result<EventLog, EngineErr> {
        self.load_verified_with_token(repo).map(|(log, _)| log)
    }

    /// As [`load_verified`], but ALSO returns the [`CasToken`] for the head (the R2
    /// object ETag, or a local content hash) — the version the write-door's
    /// compare-and-swap persists against. The chain verify is the SAME single
    /// PS-13 chokepoint (this method IS the body; `load_verified` drops the token),
    /// so a write can never skip the verification a read performs.
    pub fn load_verified_with_token(&self, repo: &str) -> Result<(EventLog, CasToken), EngineErr> {
        if !is_safe_repo_slug(repo) {
            return Err(EngineErr::not_found());
        }
        let (bytes, label, token) = match self.source.fetch(repo)? {
            Some(b) => b,
            None => return Err(EngineErr::not_found()),
        };
        let log = hugit_cli::checks::load_event_log_from_bytes(&bytes, Path::new(&label)).map_err(
            |e| EngineErr::unavailable(format!("engine log read/verify failed ({})", e.kind())),
        )?;
        Ok((log, token))
    }
}

impl LogSource {
    /// Fetch a repo's raw event-log bytes + the head [`CasToken`] (R2 ETag, or a
    /// local content hash). `Ok(None)` = the object does not exist (→ 404, no
    /// existence leak); `Ok(Some((bytes, label, token)))` = present; `Err` = a
    /// transport/IO fault (→ 503). `label` is the source string for error context.
    fn fetch(&self, repo: &str) -> Result<Option<(Vec<u8>, String, CasToken)>, EngineErr> {
        match self {
            LogSource::Local { dir } => {
                let path = dir.join(format!("{repo}.json"));
                match std::fs::read(&path) {
                    Ok(b) => {
                        let token = CasToken::Version(content_hash(&b));
                        Ok(Some((b, path.display().to_string(), token)))
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(EngineErr::unavailable(format!(
                        "local log read failed: {e}"
                    ))),
                }
            }
            LogSource::R2(c) => c.fetch(repo),
        }
    }

    /// Durably write a repo's raw event-log bytes back to the source, as a
    /// COMPARE-AND-SWAP against `expected` (the token the matching [`fetch`]
    /// returned). A concurrent head move → [`EngineErr::cas_conflict`] (the
    /// write-door reloads + retries), NEVER last-writer-wins.
    /// - **Local**: re-read + content-hash compare, then atomic temp-write + rename.
    /// - **R2**: a conditional signed PUT (`If-Match`/`If-None-Match`) — REQUIRES a
    ///   write-scoped credential; the standing engine cred is read-only by design,
    ///   so an R2 write fail-honestly returns 503 until the scoped RW grant is wired.
    fn persist(&self, repo: &str, bytes: &[u8], expected: &CasToken) -> Result<(), EngineErr> {
        match self {
            LogSource::Local { dir } => {
                std::fs::create_dir_all(dir).map_err(|e| {
                    EngineErr::unavailable(format!("local log dir create failed: {e}"))
                })?;
                let path = dir.join(format!("{repo}.json"));
                // CAS check: the on-disk head must still equal `expected` (a local
                // analogue of R2 `If-Match`). A residual TOCTOU remains between this
                // compare and the rename below — acceptable because Local is the
                // single-box dev/test source; the production multi-writer source is
                // R2, whose conditional PUT is atomic at the store.
                let current = match std::fs::read(&path) {
                    Ok(b) => CasToken::Version(content_hash(&b)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => CasToken::Absent,
                    Err(e) => {
                        return Err(EngineErr::unavailable(format!(
                            "local log re-read failed: {e}"
                        )));
                    }
                };
                if !cas_matches(expected, &current) {
                    return Err(EngineErr::cas_conflict());
                }
                // Atomic: write a unique temp then rename over the target, so a
                // crash mid-write never leaves a torn `<repo>.json`.
                let tmp = dir.join(format!("{repo}.json.tmp.{}", std::process::id()));
                std::fs::write(&tmp, bytes)
                    .map_err(|e| EngineErr::unavailable(format!("local log write failed: {e}")))?;
                std::fs::rename(&tmp, &path).map_err(|e| {
                    let _ = std::fs::remove_file(&tmp);
                    EngineErr::unavailable(format!("local log rename failed: {e}"))
                })
            }
            LogSource::R2(c) => c.put_conditional(repo, bytes, expected).map(|_| ()),
        }
    }
}

/// The content-hash version of a raw log object — the local CAS token (a stand-in
/// for the R2 ETag). Hex SHA-256 of the exact bytes.
fn content_hash(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    hex::encode(h.finalize())
}

/// Whether the head a write swaps against (`expected`) still equals what is durably
/// present now (`current`). `Unsupported` always matches (the sink opts out of CAS).
fn cas_matches(expected: &CasToken, current: &CasToken) -> bool {
    match (expected, current) {
        (CasToken::Unsupported, _) => true,
        (CasToken::Absent, CasToken::Absent) => true,
        (CasToken::Version(a), CasToken::Version(b)) => a == b,
        _ => false,
    }
}

impl crate::writes::LogSink for AppState {
    /// Load + chain-verify (same gate as the read path; absent → 404) AND capture
    /// the head [`CasToken`] for the write-door's compare-and-swap.
    fn load(&self, repo: &str) -> Result<(EventLog, CasToken), EngineErr> {
        self.load_verified_with_token(repo)
    }

    /// Serialize the mutated log + durably persist it back to the source as a
    /// COMPARE-AND-SWAP against `expected` (the head this request loaded). A
    /// concurrent head move → [`EngineErr::cas_conflict`], which `with_write`
    /// catches to reload + retry — so a multi-writer / R2 deployment never drops a
    /// concurrent request's records (the load→persist gap holds no lock; the CAS,
    /// not a lock, is what makes the cycle safe).
    fn persist(&self, repo: &str, log: &EventLog, expected: &CasToken) -> Result<(), EngineErr> {
        if !is_safe_repo_slug(repo) {
            return Err(EngineErr::not_found());
        }
        let bytes = serde_json::to_vec(log.records())
            .map_err(|e| EngineErr::unavailable(format!("log serialize failed: {e}")))?;
        self.source.persist(repo, &bytes, expected)
    }
}

impl R2Config {
    /// Build from the `HUGIT_SERVE_R2_*` env (same vars the read server uses).
    /// `REGION` defaults to `auto` (R2). For the snapshot uploader the `KEY_ID`/
    /// `SECRET` are the one-shot READ+WRITE grant; for the server they are the
    /// standing read-only cred — same shape, different scope.
    pub fn from_env() -> Result<Self, String> {
        Self::from_vars(|k| std::env::var(k).ok())
    }

    /// Pure core of [`from_env`] — resolves the config from a `get(name)` lookup so
    /// it is testable without mutating global process env.
    ///
    /// Accepts BOTH the engine-native names AND the S3-standard names a CoreLink/AWS
    /// credential file ships with, so such a file can be `source`d verbatim (only
    /// `HUGIT_SERVE_R2_TENANT_ID` must be supplied separately — it is not part of a
    /// generic cred):
    /// - host: `HUGIT_SERVE_R2_ACCOUNT_ID` (native) **or** `…_ENDPOINT` (S3-standard).
    /// - key:  `HUGIT_SERVE_R2_KEY_ID` (native) **or** `…_ACCESS_KEY_ID`.
    /// - secret: `HUGIT_SERVE_R2_SECRET` (native) **or** `…_SECRET_ACCESS_KEY`.
    fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, String> {
        let req = |k: &str| get(k).ok_or_else(|| format!("{k} is not set (R2 source selected)"));
        let either = |primary: &str, alias: &str| get(primary).or_else(|| get(alias));

        // Host from the native ACCOUNT_ID, else parsed from the S3-standard ENDPOINT.
        let (endpoint, host) = match get("HUGIT_SERVE_R2_ACCOUNT_ID") {
            Some(account_id) => {
                let host = format!("{account_id}.r2.cloudflarestorage.com");
                (format!("https://{host}"), host)
            }
            None => {
                let endpoint = get("HUGIT_SERVE_R2_ENDPOINT").ok_or_else(|| {
                    "neither HUGIT_SERVE_R2_ACCOUNT_ID nor HUGIT_SERVE_R2_ENDPOINT is set \
                     (R2 source selected)"
                        .to_string()
                })?;
                let host = endpoint
                    .trim_start_matches("https://")
                    .trim_start_matches("http://")
                    .split('/')
                    .next()
                    .unwrap_or("")
                    .to_string();
                if host.is_empty() {
                    return Err(format!("HUGIT_SERVE_R2_ENDPOINT is malformed: {endpoint}"));
                }
                (endpoint, host)
            }
        };
        let key_id =
            either("HUGIT_SERVE_R2_KEY_ID", "HUGIT_SERVE_R2_ACCESS_KEY_ID").ok_or_else(|| {
                "HUGIT_SERVE_R2_KEY_ID (or _ACCESS_KEY_ID) is not set (R2 source selected)"
                    .to_string()
            })?;
        let secret = either("HUGIT_SERVE_R2_SECRET", "HUGIT_SERVE_R2_SECRET_ACCESS_KEY")
            .ok_or_else(|| {
                "HUGIT_SERVE_R2_SECRET (or _SECRET_ACCESS_KEY) is not set (R2 source selected)"
                    .to_string()
            })?;
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .build();
        Ok(R2Config {
            endpoint,
            host,
            bucket: req("HUGIT_SERVE_R2_BUCKET")?,
            region: get("HUGIT_SERVE_R2_REGION").unwrap_or_else(|| "auto".to_string()),
            key_id,
            secret,
            tenant_id: req("HUGIT_SERVE_R2_TENANT_ID")?,
            agent,
        })
    }

    /// Fetch the raw object + head [`CasToken`] (the GET ETag). `pub` so the live
    /// R2 CAS round-trip proof (`tests/r2_cas_live.rs`, `#[ignore]`) can exercise
    /// the real fetch→put_conditional path.
    pub fn fetch(&self, repo: &str) -> Result<Option<(Vec<u8>, String, CasToken)>, EngineErr> {
        let key = format!("{}/{repo}.json", self.tenant_id);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let signed = sigv4::sign_s3_get(
            &self.host,
            &self.bucket,
            &key,
            &self.key_id,
            &self.secret,
            &self.region,
            now,
        );
        // Path-style URL; tenant (UUID) + repo (validated slug) + ".json" are all
        // RFC-3986-unreserved, so the wire path equals the signed canonical URI.
        let url = format!("{}/{}/{key}", self.endpoint, self.bucket);
        let label = format!("r2://{}/{key}", self.bucket);
        let resp = self
            .agent
            .get(&url)
            .set("Authorization", &signed.authorization)
            .set("x-amz-date", &signed.amz_date)
            .set("x-amz-content-sha256", &signed.content_sha256)
            .call();
        match resp {
            Ok(r) => {
                // Capture the ETag (the CAS version) BEFORE consuming the body. R2
                // returns it quoted (e.g. `"abc…"`); preserve it verbatim so the
                // `If-Match` we send back round-trips byte-identically. A response
                // without an ETag (should not happen for a real object) opts out of
                // CAS for this head rather than failing the read.
                let token = match r.header("etag") {
                    Some(e) if !e.is_empty() => CasToken::Version(e.to_string()),
                    _ => CasToken::Unsupported,
                };
                let mut buf = Vec::new();
                r.into_reader().read_to_end(&mut buf).map_err(|e| {
                    // Detail (which carries the bucket/key `label`) to the SERVER log
                    // only — never echo storage internals to the public client.
                    eprintln!("hugit-serve: R2 body read failed for {label}: {e}");
                    EngineErr::unavailable("engine storage read failed".to_string())
                })?;
                Ok(Some((buf, label, token)))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
            // A non-404 status OR a transport fault. `ureq`'s error Display embeds the
            // request URL (R2 host + bucket + tenant + key); on the PUBLIC read path
            // that would leak the storage topology in a 503 body. Log the specifics
            // server-side; return a GENERIC reason to the client (no existence/topology
            // oracle). Still fail-honest (503), never a fake-empty VM.
            Err(e) => {
                eprintln!("hugit-serve: R2 GET failed for {label}: {e}");
                Err(EngineErr::unavailable(
                    "engine storage temporarily unavailable".to_string(),
                ))
            }
        }
    }

    /// PUT `body` to `<tenant_id>/<repo>.json` UNCONDITIONALLY (the one-shot
    /// snapshot upload — used by the `hugit-snapshot` bin, NOT the engine write
    /// path). The standing engine credential is read-only by design (a PUT 403s);
    /// this path is reached only with a read+WRITE credential. Returns the wire key.
    pub fn put(&self, repo: &str, body: &[u8]) -> Result<String, EngineErr> {
        self.put_conditional(repo, body, &CasToken::Unsupported)
    }

    /// PUT `body` as a COMPARE-AND-SWAP against `expected` (the head the matching
    /// [`fetch`] returned), via the S3/R2 conditional headers:
    /// - [`CasToken::Version(etag)`] → `If-Match: <etag>` (overwrite only if unchanged),
    /// - [`CasToken::Absent`] → `If-None-Match: *` (create only if still absent),
    /// - [`CasToken::Unsupported`] → no conditional header (unconditional PUT).
    ///
    /// R2 returns **412 Precondition Failed** when the precondition is not met
    /// (verified against the Cloudflare S3-compat docs); that maps to
    /// [`EngineErr::cas_conflict`] — the write-door's reload-and-retry signal. The
    /// conditional header is a standard (non-`x-amz`) HTTP header, so per the SigV4
    /// spec it need not be in `SignedHeaders`; it is sent UNSIGNED (keeping the
    /// proven signer untouched) and the live round-trip proves R2 honors it.
    /// A 2xx is success; anything else is an explicit error (never a silent partial).
    pub fn put_conditional(
        &self,
        repo: &str,
        body: &[u8],
        expected: &CasToken,
    ) -> Result<String, EngineErr> {
        let key = format!("{}/{repo}.json", self.tenant_id);
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let signed = sigv4::sign_s3_put(
            &self.host,
            &self.bucket,
            &key,
            body,
            &self.key_id,
            &self.secret,
            &self.region,
            now,
        );
        let url = format!("{}/{}/{key}", self.endpoint, self.bucket);
        let mut req = self
            .agent
            .put(&url)
            .set("Authorization", &signed.authorization)
            .set("x-amz-date", &signed.amz_date)
            .set("x-amz-content-sha256", &signed.content_sha256);
        // The conditional header that turns this PUT into a compare-and-swap.
        match expected {
            CasToken::Version(etag) => req = req.set("If-Match", etag),
            CasToken::Absent => req = req.set("If-None-Match", "*"),
            CasToken::Unsupported => {}
        }
        let resp = req.send_bytes(body);
        match resp {
            Ok(r) if (200..300).contains(&r.status()) => Ok(format!("r2://{}/{key}", self.bucket)),
            Ok(r) => Err(EngineErr::unavailable(format!(
                "R2 PUT unexpected status {}",
                r.status()
            ))),
            // The precondition failed: a concurrent writer moved the head. This is
            // the CAS-conflict retry signal, NOT a hard failure.
            Err(ureq::Error::Status(412, _)) => Err(EngineErr::cas_conflict()),
            Err(ureq::Error::Status(403, _)) => Err(EngineErr::unavailable(
                "R2 PUT 403 — the credential is not write-scoped (need the one-shot RW grant)"
                    .to_string(),
            )),
            Err(ureq::Error::Status(s, _)) => {
                Err(EngineErr::unavailable(format!("R2 PUT status {s}")))
            }
            Err(e) => Err(EngineErr::unavailable(format!("R2 PUT transport: {e}"))),
        }
    }
}

/// A repo slug is a single safe path segment: non-empty, ≤100 chars, ASCII
/// alnum + `-_.`, never `.`/`..`/containing `..` or a path separator. Blocks URL
/// path-traversal into arbitrary files / R2 keys.
#[must_use]
pub fn is_safe_repo_slug(repo: &str) -> bool {
    !repo.is_empty()
        && repo.len() <= 100
        && repo != "."
        && repo != ".."
        && !repo.contains("..")
        && !repo.contains('/')
        && !repo.contains('\\')
        && repo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Build a `get`-style lookup over a fixed map (no global env mutation).
    fn vars(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let m: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| m.get(k).cloned()
    }

    #[test]
    fn r2_config_accepts_engine_native_names() {
        let c = R2Config::from_vars(vars(&[
            ("HUGIT_SERVE_R2_ACCOUNT_ID", "acct123"),
            ("HUGIT_SERVE_R2_BUCKET", "corelink-githugr-engine"),
            ("HUGIT_SERVE_R2_KEY_ID", "k"),
            ("HUGIT_SERVE_R2_SECRET", "s"),
            ("HUGIT_SERVE_R2_TENANT_ID", "ee30f7ba"),
        ]))
        .expect("native names");
        assert_eq!(c.host, "acct123.r2.cloudflarestorage.com");
        assert_eq!(c.endpoint, "https://acct123.r2.cloudflarestorage.com");
        assert_eq!(c.tenant_id, "ee30f7ba");
        assert_eq!(c.region, "auto"); // default
    }

    #[test]
    fn r2_config_accepts_s3_standard_cred_names() {
        // A CoreLink/AWS cred file (sourced verbatim) + the separately-set tenant.
        let c = R2Config::from_vars(vars(&[
            (
                "HUGIT_SERVE_R2_ENDPOINT",
                "https://acct123.r2.cloudflarestorage.com",
            ),
            ("HUGIT_SERVE_R2_ACCESS_KEY_ID", "k"),
            ("HUGIT_SERVE_R2_SECRET_ACCESS_KEY", "s"),
            ("HUGIT_SERVE_R2_BUCKET", "corelink-githugr-engine"),
            ("HUGIT_SERVE_R2_REGION", "auto"),
            ("HUGIT_SERVE_R2_TENANT_ID", "ee30f7ba"),
        ]))
        .expect("S3-standard names");
        assert_eq!(c.host, "acct123.r2.cloudflarestorage.com");
        assert_eq!(c.endpoint, "https://acct123.r2.cloudflarestorage.com");
        assert_eq!(c.key_id, "k");
        assert_eq!(c.secret, "s");
    }

    #[test]
    fn r2_config_missing_host_source_is_an_error() {
        let err = match R2Config::from_vars(vars(&[
            ("HUGIT_SERVE_R2_KEY_ID", "k"),
            ("HUGIT_SERVE_R2_SECRET", "s"),
            ("HUGIT_SERVE_R2_BUCKET", "b"),
            ("HUGIT_SERVE_R2_TENANT_ID", "t"),
        ])) {
            Ok(_) => panic!("expected an error when neither ACCOUNT_ID nor ENDPOINT is set"),
            Err(e) => e,
        };
        assert!(
            err.contains("ACCOUNT_ID") && err.contains("ENDPOINT"),
            "{err}"
        );
    }

    #[test]
    fn safe_slugs_accept_normal_repos() {
        for ok in ["hugit", "my-repo", "repo_1", "a.b", "HuGR"] {
            assert!(is_safe_repo_slug(ok), "{ok} should be safe");
        }
    }

    #[test]
    fn unsafe_slugs_block_traversal() {
        for bad in [
            "",
            ".",
            "..",
            "../etc",
            "a/b",
            "a\\b",
            "a..b",
            "../../etc/passwd",
        ] {
            assert!(!is_safe_repo_slug(bad), "{bad} must be rejected");
        }
    }

    #[test]
    fn unsafe_slug_is_404_before_any_fetch() {
        let st = AppState::new(PathBuf::from("/nonexistent"), "tok".to_string());
        assert_eq!(st.load_verified("../etc/passwd").unwrap_err().status, 404);
    }

    #[test]
    fn absent_local_log_is_404() {
        let st = AppState::new(PathBuf::from("/nonexistent-dir-xyz"), "tok".to_string());
        assert_eq!(st.load_verified("hugit").unwrap_err().status, 404);
    }

    #[test]
    fn source_label_reflects_local() {
        let st = AppState::new(PathBuf::from("/tmp/logs"), "tok".to_string());
        assert!(st.source_label().starts_with("local:"));
    }
}
