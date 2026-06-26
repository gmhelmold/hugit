//! The runner LEASE-ACQUIRE client (WP-#2-PR1).
//!
//! The write-side analogue of the live CoreLink AC client
//! ([`crate::client::ac::HttpAcClient`]): a thin, mockable HTTP client that
//! acquires a runner lease, dispatches an exec, polls the result envelope
//! (meta + events), and closes the lease against `HUGIT_RUNNER_HOST`. It mirrors
//! the AC client EXACTLY — the same secret discipline (the PAT is held private
//! and only ever placed in the `Authorization` header; never logged, never in
//! `Debug`/`Display`/errors/url strings), the same fail-closed
//! `NotConfigured` loader, and the same trait/seam split so the request building
//! and response parsing are proven hermetically against a FAKE transport with
//! ZERO network calls.
//!
//! ## Scope of THIS PR (PR1)
//!
//! This is the transport + config + DTOs + `from_runtime` loader + the hermetic
//! tests ONLY. It is NOT wired into [`crate::runner::lease_exec::LiveBoxRunnerExecutor`]
//! yet (that is PR3) and it NEVER makes a live network call here (the only code
//! that opens a socket is [`UreqRunnerTransport`], which is exercised solely in
//! production, never in a test). The §13.1 `IntentMetrics` DTO + its mapping are
//! deliberately deferred to PR2 — `poll_meta` here returns a raw [`RawMeta`]
//! (an un-interpreted JSON value), and `capture_on_land` is NOT wired.

use std::path::{Path, PathBuf};

use hugit_contracts::{CheckDef, RunnerLease};
use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Errors — fail-closed, secret-free (mirrors `AcError`).
// ─────────────────────────────────────────────────────────────────────────────

/// Errors from a runner-lease interaction. The PAT NEVER appears in any variant:
/// every message names the piece/endpoint/status, never a credential value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerError {
    /// A transport/protocol failure from the live client (network, TLS, I/O).
    Transport(String),
    /// A RETRYABLE server-busy / contention condition: an HTTP 429
    /// (rate-limited) or 503 (service unavailable) from the runner fabric.
    /// Carried as a TYPED variant — never an ad-hoc string — so a caller can
    /// match it structurally and retry instead of collapsing to a terminal
    /// error (mirrors `AcError::Busy`).
    Busy {
        /// A short, secret-free description of what was busy (the HTTP status).
        detail: String,
    },
    /// The required runtime configuration (runner host and/or PAT) was not
    /// supplied. Fail-closed: the client refuses to make a network call without
    /// a credential rather than silently degrading. Names the missing piece,
    /// never the value.
    NotConfigured(String),
    /// The server answered with an HTTP status the protocol does not map to a
    /// success (e.g. 401 bad PAT, 403 cross-tenant, 5xx server fault). Carries
    /// the status for the operator.
    Status(u16),
    /// A success body could not be decoded into the expected DTO.
    Decode(String),
    /// The `lease_id` to be interpolated into the request URL is not a canonical
    /// safe identifier (`^[A-Za-z0-9_-]{1,128}$`). This is a request-target
    /// trust-boundary guard (defense-in-depth against path traversal /
    /// cross-tenant escape via the lease-id path segment), distinct from any
    /// response guard. Carries a short description of the violation; never a PAT.
    InvalidLeaseId(String),
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerError::Transport(e) => write!(f, "runner transport error: {e}"),
            RunnerError::Busy { detail } => write!(f, "runner busy (retryable): {detail}"),
            RunnerError::NotConfigured(what) => {
                write!(f, "runner client not configured: {what}")
            }
            RunnerError::Status(code) => {
                write!(f, "runner server returned unexpected HTTP {code}")
            }
            RunnerError::Decode(e) => write!(f, "runner response decode failed: {e}"),
            RunnerError::InvalidLeaseId(what) => {
                write!(
                    f,
                    "runner invalid lease id (refusing to build request): {what}"
                )
            }
        }
    }
}

impl std::error::Error for RunnerError {}

/// A lease id is interpolated into the request URL, so validate it
/// `^[A-Za-z0-9_-]{1,128}$` BEFORE interpolation: an id carrying path separators
/// (or any other byte) could escape the `/v1/leases/{lease_id}/` prefix and
/// target a different route. Returns [`RunnerError::InvalidLeaseId`] on any
/// violation.
fn validate_lease_id(lease_id: &str) -> Result<(), RunnerError> {
    let ok = (1..=128).contains(&lease_id.len())
        && lease_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(RunnerError::InvalidLeaseId(format!(
            "lease id must match ^[A-Za-z0-9_-]{{1,128}}$ (got {} chars)",
            lease_id.len()
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DTOs — minimal serde request/response shapes.
// ─────────────────────────────────────────────────────────────────────────────

/// Request body for `POST /v1/leases` — acquire a runner lease.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquireLeaseRequest {
    /// Ordered chain of principals (agent ids / user ids) requesting the lease.
    pub principal_chain: Vec<String>,
    /// Filesystem paths the lease should grant access to.
    pub path_set: Vec<String>,
    /// Network policy name / ref governing the runner's outbound access.
    pub net_policy: String,
    /// Requested lease time-to-live, in milliseconds.
    pub ttl_ms: u64,
}

/// Acknowledgement returned by `POST /v1/leases/{lease_id}/exec` — the runner
/// accepted the [`CheckDef`] for execution. The result itself is fetched later
/// via [`LeaseClient::poll_meta`] / [`LeaseClient::poll_events`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecAck {
    /// The lease the exec was dispatched under.
    pub lease_id: String,
    /// Whether the runner accepted the exec request.
    pub accepted: bool,
}

/// The raw, un-interpreted result-envelope META as returned by
/// `GET /v1/leases/{lease_id}/envelope/meta`.
///
/// PR1 keeps this a raw JSON value ON PURPOSE: the §13.1 `IntentMetrics` DTO and
/// its typed mapping (cost/duration capture) are PR2's job. Wrapping the value
/// (rather than aliasing `serde_json::Value`) keeps the public surface stable
/// when PR2 adds typed accessors.
#[derive(Debug, Clone, PartialEq)]
pub struct RawMeta(pub serde_json::Value);

/// The raw result-envelope EVENT bytes as returned by
/// `GET /v1/leases/{lease_id}/envelope/events`.
///
/// Held as opaque bytes (the stream may be NDJSON / SSE, not a single JSON
/// document), so PR1 does not impose a decode that PR2 may need to change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEvents(pub Vec<u8>);

// ─────────────────────────────────────────────────────────────────────────────
// Config — the PAT is private, never rendered (mirrors `AcConfig`).
// ─────────────────────────────────────────────────────────────────────────────

/// Runtime configuration for the live lease client. Injected at runtime (NOT
/// hardcoded, NOT compiled in): the runner host base URL and the secret PAT. The
/// PAT is held privately and only ever placed in the `Authorization` header — it
/// is never logged, never put in an error/`Display`, never in the endpoint
/// string. `Debug` is hand-written so the PAT can NEVER leak through `{:?}`.
#[derive(Clone)]
pub struct RunnerConfig {
    /// Runner fabric base URL (e.g. `https://<your-runner-host>`).
    host: String,
    /// The runner PAT (Bearer). SECRET — never rendered.
    pat: String,
}

impl RunnerConfig {
    /// Build the config from injected values. Returns
    /// [`RunnerError::NotConfigured`] (fail-closed) if any field is blank, so an
    /// unset deployment surfaces a clear error and can never silently degrade.
    pub fn new(host: impl Into<String>, pat: impl Into<String>) -> Result<Self, RunnerError> {
        let host = host.into();
        let pat = pat.into();
        if host.trim().is_empty() {
            return Err(RunnerError::NotConfigured("runner host is empty".into()));
        }
        if pat.trim().is_empty() {
            return Err(RunnerError::NotConfigured("PAT is empty".into()));
        }
        Ok(Self { host, pat })
    }

    /// The `Authorization` header value. Crate-private so the secret never
    /// escapes the transport boundary.
    fn bearer(&self) -> String {
        format!("Bearer {}", self.pat)
    }

    /// The host base, trailing-slash-trimmed, for URL building.
    fn base(&self) -> &str {
        self.host.trim_end_matches('/')
    }
}

// `Debug` is hand-written so the PAT can NEVER leak through `{:?}`.
impl std::fmt::Debug for RunnerConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunnerConfig")
            .field("host", &self.host)
            .field("pat", &"<redacted>")
            .finish()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Transport — the ONLY part that touches the network (mockable).
// ─────────────────────────────────────────────────────────────────────────────

/// A single HTTP exchange against the runner fabric — the ONLY part that touches
/// the network. Splitting it behind a trait lets the request building and
/// response parsing be proven hermetically with a fake transport (no live call),
/// while the real `ureq` transport is the thin seam. The runner client needs
/// both `POST` (acquire/exec/close) and `GET` (poll meta/events), so the trait
/// carries both (the AC client only needed `get`/`put`).
pub trait RunnerTransport {
    /// `POST {url}` with a Bearer PAT and a JSON body. Returns the decoded
    /// `(status, body_bytes)`.
    fn post(&self, url: &str, bearer: &str, body: &[u8]) -> Result<(u16, Vec<u8>), RunnerError>;

    /// `GET {url}` with a Bearer PAT. Returns the decoded `(status, body_bytes)`.
    fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), RunnerError>;
}

/// The real `ureq`-backed transport — the thin network seam. This is the ONLY
/// code that opens a socket; everything around it is proven without it. `ureq`
/// is already in the workspace lock (used by the AC client), so no new
/// transitive surface is added.
#[derive(Debug, Clone, Copy, Default)]
pub struct UreqRunnerTransport;

impl RunnerTransport for UreqRunnerTransport {
    fn post(&self, url: &str, bearer: &str, body: &[u8]) -> Result<(u16, Vec<u8>), RunnerError> {
        let resp = ureq::post(url)
            .set("Authorization", bearer)
            .set("Content-Type", "application/json")
            .send_bytes(body);
        match resp {
            Ok(r) => {
                let status = r.status();
                let mut buf = Vec::new();
                use std::io::Read;
                r.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|e| RunnerError::Transport(e.to_string()))?;
                Ok((status, buf))
            }
            // ureq surfaces non-2xx as `Error::Status(code, resp)`; map it to a
            // status the protocol logic can branch on.
            Err(ureq::Error::Status(code, _resp)) => Ok((code, Vec::new())),
            Err(e) => Err(RunnerError::Transport(e.to_string())),
        }
    }

    fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), RunnerError> {
        let resp = ureq::get(url).set("Authorization", bearer).call();
        match resp {
            Ok(r) => {
                let status = r.status();
                let mut buf = Vec::new();
                use std::io::Read;
                r.into_reader()
                    .read_to_end(&mut buf)
                    .map_err(|e| RunnerError::Transport(e.to_string()))?;
                Ok((status, buf))
            }
            Err(ureq::Error::Status(code, _resp)) => Ok((code, Vec::new())),
            Err(e) => Err(RunnerError::Transport(e.to_string())),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The client.
// ─────────────────────────────────────────────────────────────────────────────

/// The runner lease client over a pluggable [`RunnerTransport`].
///
/// The acquire/exec/poll/close LOGIC lives here and is proven hermetically (fake
/// transport). The live wiring is supplying a CONFIGURED client (host + secret
/// PAT) at runtime via [`LeaseClient::from_runtime`].
#[derive(Debug, Clone)]
pub struct LeaseClient<T: RunnerTransport = UreqRunnerTransport> {
    /// Runtime config (host + PAT): present once injected.
    config: RunnerConfig,
    /// The HTTP transport (real `ureq` by default; a fake in tests).
    transport: T,
}

impl LeaseClient<UreqRunnerTransport> {
    /// Construct a CONFIGURED client over the real `ureq` transport: inject the
    /// host and the secret PAT.
    pub fn configured(
        host: impl Into<String>,
        pat: impl Into<String>,
    ) -> Result<Self, RunnerError> {
        Ok(Self {
            config: RunnerConfig::new(host, pat)?,
            transport: UreqRunnerTransport,
        })
    }

    /// Build a CONFIGURED client from the runtime environment — the
    /// plug-and-play entry point. Reads EXACTLY:
    ///
    /// - `HUGIT_RUNNER_HOST` — the runner fabric base URL,
    /// - the PAT from `~/.hugit/secrets/runner/pat` (preferred; trailing newline
    ///   trimmed), falling back to the `HUGIT_RUNNER_PAT` env var ONLY if that
    ///   file is absent. The file path is overridable via
    ///   `HUGIT_RUNNER_PAT_FILE` (for hermetic tests).
    ///
    /// Returns a configured [`LeaseClient`] over the real [`UreqRunnerTransport`]
    /// when both pieces are present and non-empty; otherwise returns
    /// [`RunnerError::NotConfigured`] whose message NAMES the missing piece
    /// (never the value). Fail-closed, and the PAT is held only inside the
    /// private [`RunnerConfig`] — never logged, never in Debug/Display/errors.
    pub fn from_runtime() -> Result<Self, RunnerError> {
        runner_from_env()
    }
}

impl<T: RunnerTransport> LeaseClient<T> {
    /// Construct a client over an explicit transport + config. Used in tests to
    /// inject a fake transport; in production `T = UreqRunnerTransport`.
    pub fn with_transport(config: RunnerConfig, transport: T) -> Self {
        Self { config, transport }
    }

    /// Acquire a runner lease: `POST {host}/v1/leases`.
    pub fn acquire(&self, req: &AcquireLeaseRequest) -> Result<RunnerLease, RunnerError> {
        let url = format!("{}/v1/leases", self.config.base());
        let body = serde_json::to_vec(req).map_err(|e| RunnerError::Decode(e.to_string()))?;
        let (status, resp) = self.transport.post(&url, &self.config.bearer(), &body)?;
        match status {
            // 200 (returned existing) | 201 (fresh) both carry the lease body.
            200 | 201 => {
                serde_json::from_slice(&resp).map_err(|e| RunnerError::Decode(e.to_string()))
            }
            429 | 503 => Err(RunnerError::Busy {
                detail: format!("acquire returned HTTP {status}"),
            }),
            other => Err(RunnerError::Status(other)),
        }
    }

    /// Dispatch a check exec under a held lease:
    /// `POST {host}/v1/leases/{lease_id}/exec`.
    pub fn exec(&self, lease_id: &str, def: &CheckDef) -> Result<ExecAck, RunnerError> {
        validate_lease_id(lease_id)?;
        let url = format!("{}/v1/leases/{}/exec", self.config.base(), lease_id);
        let body = serde_json::to_vec(def).map_err(|e| RunnerError::Decode(e.to_string()))?;
        let (status, resp) = self.transport.post(&url, &self.config.bearer(), &body)?;
        match status {
            // 200 (ran) | 202 (accepted for async execution) both ack.
            200 | 202 => {
                serde_json::from_slice(&resp).map_err(|e| RunnerError::Decode(e.to_string()))
            }
            429 | 503 => Err(RunnerError::Busy {
                detail: format!("exec returned HTTP {status}"),
            }),
            other => Err(RunnerError::Status(other)),
        }
    }

    /// Poll the result-envelope META:
    /// `GET {host}/v1/leases/{lease_id}/envelope/meta`. Returns the raw,
    /// un-interpreted value (PR2 maps it to §13.1 `IntentMetrics`).
    pub fn poll_meta(&self, lease_id: &str) -> Result<RawMeta, RunnerError> {
        validate_lease_id(lease_id)?;
        let url = format!(
            "{}/v1/leases/{}/envelope/meta",
            self.config.base(),
            lease_id
        );
        let (status, resp) = self.transport.get(&url, &self.config.bearer())?;
        match status {
            200 => {
                let value = serde_json::from_slice(&resp)
                    .map_err(|e| RunnerError::Decode(e.to_string()))?;
                Ok(RawMeta(value))
            }
            429 | 503 => Err(RunnerError::Busy {
                detail: format!("poll_meta returned HTTP {status}"),
            }),
            other => Err(RunnerError::Status(other)),
        }
    }

    /// Poll the result-envelope EVENT stream:
    /// `GET {host}/v1/leases/{lease_id}/envelope/events`. Returns the raw bytes
    /// (the stream may be NDJSON / SSE; PR2+ decides the decode).
    pub fn poll_events(&self, lease_id: &str) -> Result<RawEvents, RunnerError> {
        validate_lease_id(lease_id)?;
        let url = format!(
            "{}/v1/leases/{}/envelope/events",
            self.config.base(),
            lease_id
        );
        let (status, resp) = self.transport.get(&url, &self.config.bearer())?;
        match status {
            200 => Ok(RawEvents(resp)),
            429 | 503 => Err(RunnerError::Busy {
                detail: format!("poll_events returned HTTP {status}"),
            }),
            other => Err(RunnerError::Status(other)),
        }
    }

    /// Close (release) a lease: `POST {host}/v1/leases/{lease_id}/close`.
    pub fn close(&self, lease_id: &str) -> Result<(), RunnerError> {
        validate_lease_id(lease_id)?;
        let url = format!("{}/v1/leases/{}/close", self.config.base(), lease_id);
        let (status, _resp) = self.transport.post(&url, &self.config.bearer(), &[])?;
        match status {
            // 200 (closed) | 204 (no content) both succeed.
            200 | 204 => Ok(()),
            429 | 503 => Err(RunnerError::Busy {
                detail: format!("close returned HTTP {status}"),
            }),
            other => Err(RunnerError::Status(other)),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// The runtime config LOADER (fail-closed) — mirrors `corelink_ac_from_env`.
//
// Names:
//   - HUGIT_RUNNER_HOST  — the runner fabric base URL
//   - the PAT, read from the FILE `~/.hugit/secrets/runner/pat` (preferred),
//     falling back to the `HUGIT_RUNNER_PAT` env var ONLY if that file is absent.
//
// Fail-closed is LAW: a missing/blank piece returns `NotConfigured` NAMING the
// piece — never the value. The PAT is read into the private `RunnerConfig` and
// from there only ever placed in the `Authorization` header.
// ─────────────────────────────────────────────────────────────────────────────

/// Env var holding the runner fabric base URL.
pub const ENV_RUNNER_HOST: &str = "HUGIT_RUNNER_HOST";
/// Env var holding the runner PAT — the FALLBACK source, used only when the
/// secret file is absent.
pub const ENV_RUNNER_PAT: &str = "HUGIT_RUNNER_PAT";
/// Env var overriding the PAT secret-file path (for hermetic tests; production
/// uses the default handoff path under `~/.hugit`).
pub const ENV_RUNNER_PAT_FILE: &str = "HUGIT_RUNNER_PAT_FILE";

/// The production default PAT secret-file path, relative to `$HOME`.
/// Joined with `$HOME` so the absolute path is `~/.hugit/secrets/runner/pat`.
const DEFAULT_PAT_FILE_REL: &str = ".hugit/secrets/runner/pat";

/// Resolve the PAT secret-file path: an explicit `HUGIT_RUNNER_PAT_FILE`
/// override (tests point this at a temp file) wins; otherwise the production
/// default `~/.hugit/secrets/runner/pat` resolved against `$HOME`.
fn resolve_pat_file() -> Result<PathBuf, RunnerError> {
    if let Ok(p) = std::env::var(ENV_RUNNER_PAT_FILE)
        && !p.trim().is_empty()
    {
        return Ok(PathBuf::from(p));
    }
    let home = std::env::var("HOME").map_err(|_| {
        RunnerError::NotConfigured(format!(
            "PAT: no {ENV_RUNNER_PAT_FILE} override and $HOME is unset (cannot locate the \
             default ~/{DEFAULT_PAT_FILE_REL})"
        ))
    })?;
    Ok(Path::new(&home).join(DEFAULT_PAT_FILE_REL))
}

/// Read the PAT, preferring the secret file at `pat_file` (trimming a trailing
/// newline) and falling back to the `HUGIT_RUNNER_PAT` env var ONLY if the file
/// is absent. On every failure path returns `NotConfigured` whose message names
/// the missing piece but NEVER the value. A present-but-blank file/env is treated
/// as missing (fail-closed).
fn read_pat(pat_file: &Path) -> Result<String, RunnerError> {
    match std::fs::read_to_string(pat_file) {
        Ok(contents) => {
            // A secret file must not be readable/writable by group or other.
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let meta = std::fs::metadata(pat_file).map_err(|e| {
                    RunnerError::NotConfigured(format!(
                        "PAT: cannot stat secret file {}: {}",
                        pat_file.display(),
                        e.kind()
                    ))
                })?;
                let mode = meta.permissions().mode();
                if mode & 0o077 != 0 {
                    return Err(RunnerError::NotConfigured(format!(
                        "PAT: secret file {} has insecure permissions {:o} \
                         (group/other access); chmod 600 it",
                        pat_file.display(),
                        mode & 0o777
                    )));
                }
            }
            let pat = contents.trim_end().to_string();
            if pat.is_empty() {
                return Err(RunnerError::NotConfigured(format!(
                    "PAT: secret file {} is empty",
                    pat_file.display()
                )));
            }
            Ok(pat)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File absent → fall back to the env var (the ONLY fallback).
            match std::env::var(ENV_RUNNER_PAT) {
                Ok(v) if !v.trim().is_empty() => Ok(v),
                _ => Err(RunnerError::NotConfigured(format!(
                    "PAT: secret file {} absent and {ENV_RUNNER_PAT} unset/empty",
                    pat_file.display()
                ))),
            }
        }
        Err(e) => Err(RunnerError::NotConfigured(format!(
            "PAT: cannot read secret file {}: {}",
            pat_file.display(),
            e.kind()
        ))),
    }
}

/// Free-function form of [`LeaseClient::from_runtime`] — reads the runner runtime
/// config from the environment and returns a configured client (real `ureq`
/// transport) or [`RunnerError::NotConfigured`] naming the missing piece.
pub fn runner_from_env() -> Result<LeaseClient<UreqRunnerTransport>, RunnerError> {
    // Host — present + non-empty, else NotConfigured naming it.
    let host = match std::env::var(ENV_RUNNER_HOST) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            return Err(RunnerError::NotConfigured(format!(
                "host: {ENV_RUNNER_HOST} unset/empty"
            )));
        }
    };
    // PAT — file-preferred, env-fallback; NotConfigured naming it on any miss.
    let pat_file = resolve_pat_file()?;
    let pat = read_pat(&pat_file)?;

    LeaseClient::configured(host, pat)
}

// ─────────────────────────────────────────────────────────────────────────────
// Hermetic tests — FAKE transport, ZERO network.
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// A sentinel PAT used across the secret-discipline assertions. If this ever
    /// appears in a `Debug`/`Display`/error string, the redaction is broken.
    const SENTINEL_PAT: &str = "pat-SUPER-SECRET-do-not-leak-7f3a";

    /// A recorded request made through the fake transport.
    #[derive(Debug, Clone)]
    struct FakeCall {
        method: &'static str,
        url: String,
        bearer: String,
        body: Vec<u8>,
    }

    /// A fully in-memory transport: returns queued `(status, body)` responses
    /// FIFO and records every call for assertion. NEVER opens a socket.
    #[derive(Debug, Default)]
    struct FakeTransport {
        responses: Mutex<VecDeque<(u16, Vec<u8>)>>,
        calls: Mutex<Vec<FakeCall>>,
    }

    impl FakeTransport {
        fn with_responses(responses: Vec<(u16, Vec<u8>)>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn next(&self) -> Result<(u16, Vec<u8>), RunnerError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| RunnerError::Transport("fake: no queued response".into()))
        }

        fn calls(&self) -> Vec<FakeCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl RunnerTransport for FakeTransport {
        fn post(
            &self,
            url: &str,
            bearer: &str,
            body: &[u8],
        ) -> Result<(u16, Vec<u8>), RunnerError> {
            self.calls.lock().unwrap().push(FakeCall {
                method: "POST",
                url: url.to_string(),
                bearer: bearer.to_string(),
                body: body.to_vec(),
            });
            self.next()
        }

        fn get(&self, url: &str, bearer: &str) -> Result<(u16, Vec<u8>), RunnerError> {
            self.calls.lock().unwrap().push(FakeCall {
                method: "GET",
                url: url.to_string(),
                bearer: bearer.to_string(),
                body: Vec::new(),
            });
            self.next()
        }
    }

    fn sample_lease(lease_id: &str) -> RunnerLease {
        RunnerLease {
            lease_id: lease_id.to_string(),
            principal_chain: vec!["agent:tester".to_string()],
            path_set: vec!["/work".to_string()],
            expiry: 1_000_000,
            net_policy: "deny-all".to_string(),
            tmp_root: "/tmp/runner".to_string(),
            state: hugit_contracts::RunnerState::Held,
        }
    }

    fn sample_def() -> CheckDef {
        CheckDef {
            def_digest: "a".repeat(64),
            command: "cargo test".to_string(),
            inputs: vec!["src/**".to_string()],
            toolchain_ref: "rust-1.96".to_string(),
            env_manifest: "blob:abc".to_string(),
            glob_set: vec!["**/*.rs".to_string()],
        }
    }

    #[test]
    fn happy_path_acquire_exec_poll_close() {
        let lease_id = "lease-abc123";
        let lease = sample_lease(lease_id);
        let ack = ExecAck {
            lease_id: lease_id.to_string(),
            accepted: true,
        };
        let transport = FakeTransport::with_responses(vec![
            (201, serde_json::to_vec(&lease).unwrap()),
            (202, serde_json::to_vec(&ack).unwrap()),
            (200, br#"{"status":"running","cost_usd_micros":0}"#.to_vec()),
            (200, b"event: started\nevent: done\n".to_vec()),
            (204, Vec::new()),
        ]);
        let config = RunnerConfig::new("https://runner.example/", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);

        // acquire
        let req = AcquireLeaseRequest {
            principal_chain: vec!["agent:tester".to_string()],
            path_set: vec!["/work".to_string()],
            net_policy: "deny-all".to_string(),
            ttl_ms: 60_000,
        };
        let got = client.acquire(&req).unwrap();
        assert_eq!(got, lease);

        // exec
        let got_ack = client.exec(lease_id, &sample_def()).unwrap();
        assert!(got_ack.accepted);
        assert_eq!(got_ack.lease_id, lease_id);

        // poll meta (raw)
        let meta = client.poll_meta(lease_id).unwrap();
        assert_eq!(meta.0["status"], "running");

        // poll events (raw bytes)
        let events = client.poll_events(lease_id).unwrap();
        assert!(events.0.starts_with(b"event: started"));

        // close
        client.close(lease_id).unwrap();

        // Verify the URLs were strung correctly (trailing slash trimmed) and the
        // bearer carried the PAT on every call.
        let calls = client.transport.calls();
        assert_eq!(calls.len(), 5);
        assert_eq!(calls[0].method, "POST");
        assert_eq!(calls[0].url, "https://runner.example/v1/leases");
        assert_eq!(
            calls[1].url,
            "https://runner.example/v1/leases/lease-abc123/exec"
        );
        assert_eq!(calls[1].method, "POST");
        assert_eq!(
            calls[2].url,
            "https://runner.example/v1/leases/lease-abc123/envelope/meta"
        );
        assert_eq!(calls[2].method, "GET");
        assert_eq!(
            calls[3].url,
            "https://runner.example/v1/leases/lease-abc123/envelope/events"
        );
        assert_eq!(
            calls[4].url,
            "https://runner.example/v1/leases/lease-abc123/close"
        );
        for c in &calls {
            assert_eq!(c.bearer, format!("Bearer {SENTINEL_PAT}"));
        }

        // The acquire body is the serialized request; the close body is empty.
        let sent_req: AcquireLeaseRequest = serde_json::from_slice(&calls[0].body).unwrap();
        assert_eq!(sent_req, req);
        assert!(calls[4].body.is_empty());
    }

    #[test]
    fn busy_status_is_typed_retryable() {
        let transport = FakeTransport::with_responses(vec![(503, Vec::new())]);
        let config = RunnerConfig::new("https://runner.example", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);
        let req = AcquireLeaseRequest {
            principal_chain: vec![],
            path_set: vec![],
            net_policy: "deny-all".to_string(),
            ttl_ms: 1,
        };
        match client.acquire(&req) {
            Err(RunnerError::Busy { .. }) => {}
            other => panic!("expected Busy, got {other:?}"),
        }
    }

    #[test]
    fn unexpected_status_is_terminal() {
        let transport = FakeTransport::with_responses(vec![(401, Vec::new())]);
        let config = RunnerConfig::new("https://runner.example", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);
        match client.poll_meta("lease-1") {
            Err(RunnerError::Status(401)) => {}
            other => panic!("expected Status(401), got {other:?}"),
        }
    }

    #[test]
    fn invalid_lease_id_refused_before_request() {
        let transport = FakeTransport::with_responses(vec![]);
        let config = RunnerConfig::new("https://runner.example", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);
        // A path-traversal lease id must be refused BEFORE any transport call.
        match client.exec("../../etc/passwd", &sample_def()) {
            Err(RunnerError::InvalidLeaseId(_)) => {}
            other => panic!("expected InvalidLeaseId, got {other:?}"),
        }
        assert!(
            client.transport.calls().is_empty(),
            "no request should be made for an invalid lease id"
        );
    }

    #[test]
    fn blank_config_is_not_configured() {
        assert!(matches!(
            RunnerConfig::new("", SENTINEL_PAT),
            Err(RunnerError::NotConfigured(_))
        ));
        assert!(matches!(
            RunnerConfig::new("https://runner.example", "  "),
            Err(RunnerError::NotConfigured(_))
        ));
    }

    #[test]
    fn from_runtime_not_configured_when_host_unset() {
        // Serialize env access within this test only (no other test in this file
        // touches the env), so this is race-free in-process.
        unsafe {
            std::env::remove_var(ENV_RUNNER_HOST);
            std::env::remove_var(ENV_RUNNER_PAT);
            // Point the PAT file at a guaranteed-absent path so the loader cannot
            // pick up a real ~/.hugit secret.
            std::env::set_var(ENV_RUNNER_PAT_FILE, "/nonexistent/hugit/runner/pat");
        }
        match LeaseClient::from_runtime() {
            Err(RunnerError::NotConfigured(msg)) => {
                assert!(
                    msg.contains(ENV_RUNNER_HOST),
                    "should name the missing host: {msg}"
                );
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
        unsafe {
            std::env::remove_var(ENV_RUNNER_PAT_FILE);
        }
    }

    /// The PAT must NEVER appear in any `Debug`/`Display`/error rendering — not
    /// of the config, not of the client, not of an error surfaced from a call.
    #[test]
    fn pat_never_leaks_in_debug_or_errors() {
        let config = RunnerConfig::new("https://runner.example", SENTINEL_PAT).unwrap();

        // Config Debug is redacted.
        let cfg_dbg = format!("{config:?}");
        assert!(
            !cfg_dbg.contains(SENTINEL_PAT),
            "config Debug leaked PAT: {cfg_dbg}"
        );
        assert!(cfg_dbg.contains("<redacted>"));

        // Client Debug is redacted (it embeds the config).
        let transport = FakeTransport::with_responses(vec![(500, Vec::new())]);
        let client = LeaseClient::with_transport(config, transport);
        let client_dbg = format!("{client:?}");
        assert!(
            !client_dbg.contains(SENTINEL_PAT),
            "client Debug leaked PAT: {client_dbg}"
        );

        // An error surfaced from a failing call carries no PAT in Debug or Display.
        let err = client.poll_meta("lease-1").unwrap_err();
        assert!(!format!("{err:?}").contains(SENTINEL_PAT));
        assert!(!format!("{err}").contains(SENTINEL_PAT));

        // The NotConfigured path likewise never echoes a value.
        let nc = RunnerConfig::new("https://h", "").unwrap_err();
        assert!(!format!("{nc}").contains(SENTINEL_PAT));
    }
}
