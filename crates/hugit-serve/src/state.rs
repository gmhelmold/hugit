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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hugit_refstore::EventLog;

use crate::error::EngineErr;
use crate::sigv4;

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

        let source = if std::env::var("HUGIT_SERVE_R2_ACCOUNT_ID").is_ok() {
            Self::r2_from_env()?
        } else {
            let dir = std::env::var("HUGIT_SERVE_LOG_DIR")
                .map_err(|_| "HUGIT_SERVE_LOG_DIR is not set".to_string())?;
            LogSource::Local {
                dir: PathBuf::from(dir),
            }
        };
        Ok(Self { source, dev_token })
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
        if !is_safe_repo_slug(repo) {
            return Err(EngineErr::not_found());
        }
        let (bytes, label) = match self.source.fetch(repo)? {
            Some(b) => b,
            None => return Err(EngineErr::not_found()),
        };
        hugit_cli::checks::load_event_log_from_bytes(&bytes, Path::new(&label)).map_err(|e| {
            EngineErr::unavailable(format!("engine log read/verify failed ({})", e.kind()))
        })
    }
}

impl LogSource {
    /// Fetch a repo's raw event-log bytes. `Ok(None)` = the object does not exist
    /// (→ 404, no existence leak); `Ok(Some((bytes, label)))` = present; `Err` =
    /// a transport/IO fault (→ 503). `label` is the source string for error context.
    fn fetch(&self, repo: &str) -> Result<Option<(Vec<u8>, String)>, EngineErr> {
        match self {
            LogSource::Local { dir } => {
                let path = dir.join(format!("{repo}.json"));
                match std::fs::read(&path) {
                    Ok(b) => Ok(Some((b, path.display().to_string()))),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
                    Err(e) => Err(EngineErr::unavailable(format!(
                        "local log read failed: {e}"
                    ))),
                }
            }
            LogSource::R2(c) => c.fetch(repo),
        }
    }
}

impl R2Config {
    /// Build from the `HUGIT_SERVE_R2_*` env (same vars the read server uses).
    /// `REGION` defaults to `auto` (R2). For the snapshot uploader the `KEY_ID`/
    /// `SECRET` are the one-shot READ+WRITE grant; for the server they are the
    /// standing read-only cred — same shape, different scope.
    pub fn from_env() -> Result<Self, String> {
        let req =
            |k: &str| std::env::var(k).map_err(|_| format!("{k} is not set (R2 source selected)"));
        let account_id = req("HUGIT_SERVE_R2_ACCOUNT_ID")?;
        let host = format!("{account_id}.r2.cloudflarestorage.com");
        let agent = ureq::AgentBuilder::new()
            .timeout(Duration::from_secs(30))
            .build();
        Ok(R2Config {
            endpoint: format!("https://{host}"),
            host,
            bucket: req("HUGIT_SERVE_R2_BUCKET")?,
            region: std::env::var("HUGIT_SERVE_R2_REGION").unwrap_or_else(|_| "auto".to_string()),
            key_id: req("HUGIT_SERVE_R2_KEY_ID")?,
            secret: req("HUGIT_SERVE_R2_SECRET")?,
            tenant_id: req("HUGIT_SERVE_R2_TENANT_ID")?,
            agent,
        })
    }

    fn fetch(&self, repo: &str) -> Result<Option<(Vec<u8>, String)>, EngineErr> {
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
                let mut buf = Vec::new();
                r.into_reader().read_to_end(&mut buf).map_err(|e| {
                    // Detail (which carries the bucket/key `label`) to the SERVER log
                    // only — never echo storage internals to the public client.
                    eprintln!("hugit-serve: R2 body read failed for {label}: {e}");
                    EngineErr::unavailable("engine storage read failed".to_string())
                })?;
                Ok(Some((buf, label)))
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

    /// PUT `body` to `<tenant_id>/<repo>.json` (the one-shot snapshot upload — used
    /// by the `hugit-snapshot` bin, NOT by the read server). The standing engine
    /// credential is read-only by design (a PUT 403s); this path is reached only
    /// when the config is built from a read+WRITE credential. Returns the wire key
    /// on success. A 2xx is success; anything else is an explicit error (never a
    /// silent partial write).
    pub fn put(&self, repo: &str, body: &[u8]) -> Result<String, EngineErr> {
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
        let resp = self
            .agent
            .put(&url)
            .set("Authorization", &signed.authorization)
            .set("x-amz-date", &signed.amz_date)
            .set("x-amz-content-sha256", &signed.content_sha256)
            .send_bytes(body);
        match resp {
            Ok(r) if (200..300).contains(&r.status()) => Ok(format!("r2://{}/{key}", self.bucket)),
            Ok(r) => Err(EngineErr::unavailable(format!(
                "R2 PUT unexpected status {}",
                r.status()
            ))),
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
