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
use crate::token::{SessionExchangeClient, SessionExchangeConfig, TokenStore};
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
    /// CoreLink session-exchange client for `POST /v1/token` (Option B). `None` →
    /// dev-token-only; the endpoint 404s without it (presence not disclosed). The
    /// P2 Clerk identity seam: hugit forwards the Clerk JWT, CoreLink verifies it.
    pub exchange: Option<Arc<SessionExchangeClient>>,
    /// In-process engine-token store. Always present so the lookup gate compiles
    /// uniformly; stays empty until a Clerk exchange mints a token. Single-host
    /// (the multi-instance shared store is the same P2 seam as the idem ledger).
    pub token_store: Arc<TokenStore>,
    /// The git object source for the file-content reads (`blob`/`edit`). `None` =
    /// the content seam is not wired (no `HUGIT_SERVE_GIT_DIR`) → those reads 404
    /// honestly (NOT a fake blank file). When `Some`, paired with `git_root_tree`.
    pub git_source: Option<Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    /// The oid of HEAD's root tree in `git_source`, resolved once at boot. `None`
    /// in lock-step with `git_source` (both present or both absent).
    pub git_root_tree: Option<gix_hash::ObjectId>,
    /// The git refs (`ref name → tip oid hex`) for the smart-HTTP wire serving
    /// (`git clone`/`git fetch`). Populated from `for-each-ref` over the SAME
    /// `HUGIT_SERVE_GIT_DIR` the objects were enumerated from — refs and objects
    /// MUST be consistent (reading refs from a different projection could advertise
    /// a tip whose closure is not in the CAS). Empty when no git dir is wired → the
    /// git wire routes 404 honestly (git serving not live). This map IS the
    /// `hugit_proto::RefView` source for the clone advertisement.
    pub git_refs: std::collections::BTreeMap<String, String>,
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
        let exchange = SessionExchangeConfig::from_env()?.map(|cfg| Arc::new(cfg.into_client()));
        let token_store = Arc::new(TokenStore::new());

        // The git content seam (`blob`/`edit` reads + the clone/fetch wire).
        // Source precedence (F): `HUGIT_SERVE_CAS_URL` set → the live CoreLink CAS
        // (mutable refs/oid-index manifests from hugit's R2, immutable objects from
        // the CAS); else `HUGIT_SERVE_GIT_DIR` → a local git dir; else none → the
        // content reads 404 honestly (NOT a fake blank file). Loaded once at boot.
        let (git_source, git_root_tree, git_refs) = if std::env::var("HUGIT_SERVE_CAS_URL")
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
        {
            let (cas, root, refs) = load_from_cas_env()?;
            let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(cas);
            (Some(src), Some(root), refs)
        } else {
            match std::env::var("HUGIT_SERVE_GIT_DIR") {
                Ok(dir) if !dir.trim().is_empty() => {
                    let (cas, root, refs) = load_git_dir(&dir)?;
                    let src: Arc<dyn hugit_proto::ObjectSource + Send + Sync> = Arc::new(cas);
                    (Some(src), Some(root), refs)
                }
                _ => (None, None, std::collections::BTreeMap::new()),
            }
        };

        Ok(Self {
            source,
            dev_token,
            exchange,
            token_store,
            git_source,
            git_root_tree,
            git_refs,
        })
    }

    fn r2_from_env() -> Result<LogSource, String> {
        Ok(LogSource::R2(Box::new(R2Config::from_env()?)))
    }

    /// Explicit Local constructor (tests). No git content seam (both `None`) —
    /// blob/edit reads 404 honestly until a deploy sets `HUGIT_SERVE_GIT_DIR`.
    #[must_use]
    pub fn new(log_dir: PathBuf, dev_token: String) -> Self {
        Self {
            source: LogSource::Local { dir: log_dir },
            dev_token,
            exchange: None,
            token_store: Arc::new(TokenStore::new()),
            git_source: None,
            git_root_tree: None,
            git_refs: std::collections::BTreeMap::new(),
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
            LogSource::R2(c) => {
                // FAIL-CLOSED (audit hardening): an `Unsupported` token on the R2
                // write path means `fetch` got no ETag for an existing object — a
                // PUT would then be UNCONDITIONAL (last-writer-wins), silently
                // bypassing the CAS. R2 always returns an ETag, so this is
                // defense-in-depth: refuse the non-CAS write rather than degrade
                // silently. (The snapshot uploader's intentional unconditional
                // create uses `put`, never this engine write path.)
                if matches!(expected, CasToken::Unsupported) {
                    return Err(EngineErr::unavailable(
                        "engine storage returned no version token; refusing a non-CAS write",
                    ));
                }
                c.put_conditional(repo, bytes, expected).map(|_| ())
            }
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

    /// Fetch an ARBITRARY R2 object by its full key (no `<repo>.json` shaping) —
    /// the small generic GET the git-from-CAS loader needs for the mutable
    /// `<tenant>/<repo>/refs.json` + `<tenant>/<repo>/oid-index.json` objects.
    /// `Ok(None)` = absent (404); `Ok(Some(bytes))` = present; `Err` = a
    /// transport/non-404 fault. Mirrors [`fetch`]'s SigV4 + generic-503 discipline
    /// (no storage-topology leak to the client; specifics to the server log).
    pub fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, EngineErr> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let signed = sigv4::sign_s3_get(
            &self.host,
            &self.bucket,
            key,
            &self.key_id,
            &self.secret,
            &self.region,
            now,
        );
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
                    eprintln!("hugit-serve: R2 body read failed for {label}: {e}");
                    EngineErr::unavailable("engine storage read failed".to_string())
                })?;
                Ok(Some(buf))
            }
            Err(ureq::Error::Status(404, _)) => Ok(None),
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
    ///
    /// ⚠️ Unconditional by intent: this OVERWRITES the whole object (initial seeding).
    /// It deliberately does NOT compare-and-swap, so running it against a repo that is
    /// taking LIVE engine writes can clobber them — it is a seeding/operator tool, not
    /// a steady-state writer. Steady-state writes go through `LogSink::persist` (CAS).
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
    ///
    /// THREAT-MODEL NOTE (audit): because `If-Match` is unsigned, an on-path attacker
    /// who can rewrite the request (a MITM or a malicious/buggy intermediary) could
    /// STRIP it, downgrading the CAS to an unconditional overwrite. This is bounded by
    /// the fact that the engine talks to R2 over TLS DIRECTLY (no intermediary), so it
    /// is not exploitable in the deployed topology — it is NOT a general "tamper-proof"
    /// guarantee. Signing the header (adding it to `SignedHeaders`) would close even
    /// the intermediary case; deferred as the threat is out of the direct-TLS model.
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
            Ok(r) => {
                eprintln!("[hugit-serve] R2 PUT unexpected status {}", r.status());
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            // The precondition failed: a concurrent writer moved the head. This is
            // the CAS-conflict retry signal, NOT a hard failure.
            Err(ureq::Error::Status(412, _)) => Err(EngineErr::cas_conflict()),
            Err(ureq::Error::Status(403, r)) => {
                eprintln!(
                    "[hugit-serve] R2 PUT 403 — credential is not write-scoped: {:?}",
                    r.status()
                );
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            Err(ureq::Error::Status(s, _)) => {
                eprintln!("[hugit-serve] R2 PUT status {s}");
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            Err(e) => {
                eprintln!("[hugit-serve] R2 PUT transport error: {e}");
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
        }
    }

    /// PUT `body` to an ARBITRARY R2 key UNCONDITIONALLY — the generic write the
    /// git ingest bin uses to publish `<tenant>/<repo>/refs.json` +
    /// `<tenant>/<repo>/oid-index.json` (the mutable manifests; the immutable git
    /// objects go to the CAS, not R2). Like [`put`], this needs the one-shot RW
    /// grant (the standing engine cred is read-only → 403). Returns the wire key.
    pub fn put_object(&self, key: &str, body: &[u8]) -> Result<String, EngineErr> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let signed = sigv4::sign_s3_put(
            &self.host,
            &self.bucket,
            key,
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
            Ok(r) => {
                eprintln!("[hugit-serve] R2 PUT (object) unexpected status {}", r.status());
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            Err(ureq::Error::Status(403, r)) => {
                eprintln!(
                    "[hugit-serve] R2 PUT (object) 403 — credential is not write-scoped: {:?}",
                    r.status()
                );
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            Err(ureq::Error::Status(s, _)) => {
                eprintln!("[hugit-serve] R2 PUT (object) status {s}");
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
            Err(e) => {
                eprintln!("[hugit-serve] R2 PUT (object) transport error: {e}");
                Err(EngineErr::unavailable("engine storage write unavailable"))
            }
        }
    }
}

/// hugit's R2 as the mutable-manifest source for the git-from-CAS loader: maps
/// the [`R2Config::get_object`] `EngineErr` to the loader's `String` error.
impl crate::cas::R2Get for R2Config {
    fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
        R2Config::get_object(self, key).map_err(|e| format!("R2 get {key}: {}", e.reason))
    }
}

/// hugit's R2 as the mutable-manifest write target for the git-ingest bin: maps
/// the [`R2Config::put_object`] `EngineErr` to the ingest's `String` error.
impl crate::cas::R2Put for R2Config {
    fn put_object(&self, key: &str, body: &[u8]) -> Result<(), String> {
        R2Config::put_object(self, key, body)
            .map(|_| ())
            .map_err(|e| format!("R2 put {key}: {}", e.reason))
    }
}

/// Build the git content seam from the live CoreLink CAS (the F-switch target):
/// `HUGIT_SERVE_CAS_*` → a configured [`crate::cas::CasClient`]; the mutable
/// `refs.json`/`oid-index.json` manifests from hugit's R2 (the same
/// `HUGIT_SERVE_R2_*` config the log source uses); the single launch repo slug
/// from `HUGIT_SERVE_CAS_REPO`. Fail-closed (→ a fatal boot error) on any missing
/// piece or integrity violation. Tenant for the R2 manifest keys is
/// `HUGIT_SERVE_CAS_TENANT_ID` (the CAS path tenant). Not exercised by the
/// hermetic handler tests (those call [`crate::cas::load_from_cas`] with doubles).
fn load_from_cas_env() -> Result<crate::cas::LazyCasLoad, String> {
    let cas = crate::cas::CasClient::from_env()
        .map_err(|e| format!("HUGIT_SERVE_CAS_*: CAS client not configured: {e}"))?;
    let tenant = std::env::var("HUGIT_SERVE_CAS_TENANT_ID")
        .map_err(|_| "HUGIT_SERVE_CAS_TENANT_ID is not set (CAS source selected)".to_string())?;
    let repo = std::env::var("HUGIT_SERVE_CAS_REPO")
        .map_err(|_| "HUGIT_SERVE_CAS_REPO is not set (CAS source selected)".to_string())?;
    if !is_safe_repo_slug(&repo) {
        return Err(format!(
            "HUGIT_SERVE_CAS_REPO is not a safe repo slug: {repo:?}"
        ));
    }
    // The mutable manifests live in hugit's R2 (the same dedicated bucket the log
    // source reads). The CAS source requires the R2 cred to be present too.
    let r2 =
        R2Config::from_env().map_err(|e| format!("CAS source needs the R2 manifest store: {e}"))?;
    // LAZY boot: read only the manifests + resolve HEAD's tree; objects are
    // fetched from the CAS on demand at serve time (the eager full-closure load
    // doesn't fit the container start window — 2026-06-20 incident).
    crate::cas::load_manifests_from_cas(cas, &r2, &tenant, &repo)
}

/// Load a git directory into a [`CasObjectSource`] and resolve HEAD's root-tree
/// oid — the **live-infra seam** for the `blob`/`edit` file-content reads. Only
/// invoked when `HUGIT_SERVE_GIT_DIR` is set; not exercised by the hermetic
/// handler tests (those seed a `CasObjectSource` directly).
///
/// ## Why a `git` subprocess (not a reused hugit-proto reader)
/// hugit-proto exposes the projection/pack-assembly read path, but no public
/// entrypoint that ingests an on-disk git dir into a `CasObjectSource`; the only
/// such reader in the workspace lives in `hugit-mirror` (`import::history`,
/// `git cat-file`-backed), which is NOT a dependency of this serve crate. Rather
/// than fork that logic OR add a heavyweight new dependency, this loader shells
/// out to the local `git` binary — the same `cat-file`/`rev-list` plumbing
/// hugit-mirror uses — to enumerate every object reachable from HEAD and stream
/// it into the content-addressed store. The store verifies each object against
/// its oid on insert ([`CasObjectSource`]'s content-addressing invariant), so a
/// tampered git dir cannot smuggle mislabeled bytes into a served blob.
///
/// Returns `Err(String)` (→ a fatal boot error) if `git` is absent, the dir is
/// not a repo, HEAD does not resolve, or an object fails to parse — fail-closed:
/// a misconfigured content seam refuses to start rather than silently serving 404.
fn load_git_dir(
    git_dir: &str,
) -> Result<
    (
        hugit_proto::CasObjectSource,
        gix_hash::ObjectId,
        std::collections::BTreeMap<String, String>,
    ),
    String,
> {
    use hugit_proto::{CasObjectSource, ObjectKind};
    use std::process::Command;

    // Run `git -C <dir> <args...>`, capturing stdout bytes; map any failure to a
    // boot error string (no secret content — only the git dir path + git stderr).
    let git = |args: &[&str]| -> Result<Vec<u8>, String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(git_dir)
            .args(args)
            .output()
            .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: failed to spawn `git`: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "HUGIT_SERVE_GIT_DIR={git_dir}: `git {}` failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(out.stdout)
    };

    // Resolve HEAD's root tree oid.
    let root_hex = String::from_utf8(git(&["rev-parse", "HEAD^{tree}"])?)
        .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: HEAD tree oid is not UTF-8: {e}"))?;
    let root_hex = root_hex.trim();
    let root_tree = gix_hash::ObjectId::from_hex(root_hex.as_bytes())
        .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: HEAD tree oid {root_hex:?} invalid: {e}"))?;

    // Enumerate every object reachable from ANY ref (`--all`, not just HEAD) so a
    // clone of any branch resolves its full closure from the CAS — the git wire
    // advertisement lists every ref, so every ref's reachable objects must be
    // present (a clone of a branch whose objects were not enumerated would 404
    // mid-stream). `<oid> [path]` per line.
    let listing = String::from_utf8(git(&["rev-list", "--objects", "--all"])?)
        .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: rev-list output is not UTF-8: {e}"))?;

    // The reachable oid set (`rev-list --objects` emits `<oid> [path]` per line).
    let oids: Vec<&str> = listing
        .lines()
        .filter_map(|l| l.split_whitespace().next())
        .filter(|o| !o.is_empty())
        .collect();

    // Stream EVERY reachable object through ONE `git cat-file --batch` process
    // instead of spawning two `git` subprocesses per object. The launch repo has
    // thousands of objects (6.8k+ at this writing); per-object spawns made boot
    // take MINUTES — past the container healthcheck window, and boot is fail-closed
    // (a slow boot ⇒ an unhealthy container ⇒ a broken deploy). `--batch` reads
    // oids on stdin and emits, per object, a header line `<oid> SP <type> SP
    // <size> LF` followed by exactly `<size>` raw body bytes and a trailing LF.
    // The body is the loose-header-less object bytes — exactly what
    // `CasObjectSource` re-hashes against the oid.
    let mut cas = CasObjectSource::new();
    if !oids.is_empty() {
        use std::io::{Read, Write};

        let mut child = Command::new("git")
            .arg("-C")
            .arg(git_dir)
            .args(["cat-file", "--batch"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| {
                format!("HUGIT_SERVE_GIT_DIR: failed to spawn `git cat-file --batch`: {e}")
            })?;

        // Feed oids on a writer thread while we drain stdout on this thread, and
        // drain stderr on a third — writing all oids before reading stdout would
        // deadlock once the OS pipe buffer fills (object bodies can be MBs).
        let oids_owned: Vec<String> = oids.iter().map(|s| s.to_string()).collect();
        let mut stdin = child.stdin.take().expect("piped stdin");
        let writer = std::thread::spawn(move || -> std::io::Result<()> {
            for oid in &oids_owned {
                stdin.write_all(oid.as_bytes())?;
                stdin.write_all(b"\n")?;
            }
            Ok(()) // drop(stdin) closes the pipe so cat-file finishes
        });
        let mut stderr_pipe = child.stderr.take().expect("piped stderr");
        let errs = std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stderr_pipe.read_to_string(&mut s);
            s
        });

        let mut out = Vec::new();
        child
            .stdout
            .take()
            .expect("piped stdout")
            .read_to_end(&mut out)
            .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: reading `git cat-file --batch`: {e}"))?;

        let writer_res = writer.join();
        let stderr_text = errs.join().unwrap_or_default();
        let status = child
            .wait()
            .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: `git cat-file --batch` wait: {e}"))?;
        match writer_res {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                return Err(format!(
                    "HUGIT_SERVE_GIT_DIR: writing oids to `git cat-file --batch`: {e}"
                ));
            }
            Err(_) => {
                return Err("HUGIT_SERVE_GIT_DIR: the cat-file writer thread panicked".to_string());
            }
        }
        if !status.success() {
            return Err(format!(
                "HUGIT_SERVE_GIT_DIR: `git cat-file --batch` failed: {}",
                stderr_text.trim()
            ));
        }

        // Parse the stream: repeated `<oid> SP <type> SP <size> LF <body> LF`. A
        // `<oid> SP missing LF` line (no body) cannot occur for a rev-list oid but
        // is handled defensively.
        let mut i = 0usize;
        while i < out.len() {
            let nl = match out[i..].iter().position(|&b| b == b'\n') {
                Some(p) => i + p,
                None => break, // no trailing header — stop
            };
            let header = std::str::from_utf8(&out[i..nl])
                .map_err(|_| "HUGIT_SERVE_GIT_DIR: non-UTF-8 cat-file header".to_string())?;
            i = nl + 1;
            let mut parts = header.split(' ');
            let _oid = parts.next().unwrap_or("");
            let type_str = parts.next().unwrap_or("");
            if type_str == "missing" {
                continue; // no body follows
            }
            let size: usize = parts
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| format!("HUGIT_SERVE_GIT_DIR: bad cat-file header {header:?}"))?;
            let kind = match type_str {
                "blob" => ObjectKind::Blob,
                "tree" => ObjectKind::Tree,
                "commit" => ObjectKind::Commit,
                "tag" => ObjectKind::Tag,
                // A reachable object of an unknown type cannot occur from a healthy
                // git; skip its body rather than abort.
                _ => {
                    i += size + 1; // body + trailing LF
                    continue;
                }
            };
            if i + size > out.len() {
                return Err("HUGIT_SERVE_GIT_DIR: truncated cat-file object body".to_string());
            }
            // Insert under the oid the bytes hash to; the store rejects a mismatch
            // on the later `get`, so byte-identity to git is preserved.
            cas.insert_raw(kind, out[i..i + size].to_vec());
            i += size;
            if i < out.len() && out[i] == b'\n' {
                i += 1; // trailing LF after the body
            }
        }
    }

    // The refs for the git wire advertisement (`git clone`/`git fetch`). Read from
    // the SAME git dir as the objects so the two are consistent (the launch repo's
    // refs, not the event-log projection — they must agree with the CAS closure).
    // `for-each-ref` emits `<refname> <objectname>` per line (our chosen format).
    let refs_listing =
        String::from_utf8(git(&["for-each-ref", "--format=%(refname) %(objectname)"])?)
            .map_err(|e| format!("HUGIT_SERVE_GIT_DIR: for-each-ref output is not UTF-8: {e}"))?;
    let mut refs = std::collections::BTreeMap::new();
    for line in refs_listing.lines() {
        let mut it = line.split_whitespace();
        if let (Some(name), Some(oid)) = (it.next(), it.next()) {
            refs.insert(name.to_string(), oid.to_string());
        }
    }

    Ok((cas, root_tree, refs))
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
            ("HUGIT_SERVE_R2_TENANT_ID", "test-tenant-1"),
        ]))
        .expect("native names");
        assert_eq!(c.host, "acct123.r2.cloudflarestorage.com");
        assert_eq!(c.endpoint, "https://acct123.r2.cloudflarestorage.com");
        assert_eq!(c.tenant_id, "test-tenant-1");
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
            ("HUGIT_SERVE_R2_TENANT_ID", "test-tenant-1"),
        ]))
        .expect("S3-standard names");
        assert_eq!(c.host, "acct123.r2.cloudflarestorage.com");
        assert_eq!(c.endpoint, "https://acct123.r2.cloudflarestorage.com");
        assert_eq!(c.key_id, "k");
        assert_eq!(c.secret, "s");
    }

    #[test]
    fn r2_persist_with_unsupported_token_fails_closed_not_unconditional() {
        // Audit hardening: an `Unsupported` token on the R2 write path (fetch got no
        // ETag) must REFUSE the write (no silent last-writer-wins), short-circuiting
        // BEFORE any network PUT. Build an R2 source with dummy creds — the guard
        // returns first, so no request is ever sent.
        let cfg = R2Config::from_vars(vars(&[
            ("HUGIT_SERVE_R2_ACCOUNT_ID", "acct123"),
            ("HUGIT_SERVE_R2_BUCKET", "corelink-githugr-engine"),
            ("HUGIT_SERVE_R2_KEY_ID", "k"),
            ("HUGIT_SERVE_R2_SECRET", "s"),
            ("HUGIT_SERVE_R2_TENANT_ID", "test-tenant-1"),
        ]))
        .expect("config");
        let source = LogSource::R2(Box::new(cfg));
        let err = source
            .persist("hugit", b"{}", &CasToken::Unsupported)
            .expect_err("an Unsupported token on R2 must fail-closed, not write unconditionally");
        assert_eq!(err.status, 503);
        assert_eq!(err.code, "ENGINE_UNAVAILABLE");
        assert!(
            err.reason.contains("no version token"),
            "the reason must name the refused non-CAS write, got: {}",
            err.reason
        );
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
