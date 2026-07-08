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
//! tests. It NEVER makes a live network call here (the only code that opens a
//! socket is [`UreqRunnerTransport`], which is exercised solely in production,
//! never in a test). `poll_meta` here returns a raw [`RawMeta`] (an
//! un-interpreted JSON value); the typed §13.1 `IntentMetrics` mapping + the
//! `LiveBoxRunnerExecutor` wiring + the acquire→exec→poll→close orchestration
//! that consume it live in [`crate::runner::metrics`] +
//! [`crate::runner::dispatch`] (WP-Wave-E-PR3).

use std::path::{Path, PathBuf};

use hugit_contracts::{CheckDef, RunnerLease};
use serde::{Deserialize, Serialize};

use crate::runner::metrics::RunnerJobMetrics;

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
    /// The server-supplied §13.2 `ingest_path` is not a safe absolute path, so
    /// concatenating it onto the base could redirect the request (and the SCOPED
    /// ingest CREDENTIAL sent with it) to an attacker-chosen host. A
    /// RESPONSE-trust-boundary guard: the acquire response comes from the fabric,
    /// but a buggy/compromised producer must NOT be able to exfiltrate the scoped
    /// credential. Carries a short description; never the credential.
    InvalidIngestPath(String),
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
            RunnerError::InvalidIngestPath(what) => {
                write!(
                    f,
                    "runner invalid ingest path (refusing to send the scoped credential): {what}"
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

/// The §13.2 `ingest_path` comes from the fabric's acquire response and is
/// concatenated onto the configured base to form the URL the SCOPED ingest
/// credential is sent to. It MUST be a safe ABSOLUTE path so a buggy/compromised
/// producer cannot redirect that credential to an attacker host.
///
/// The attack this closes: `base = "https://runner.example"`, a hostile
/// `ingest_path = "@evil.com/x"` → `"https://runner.example@evil.com/x"`, whose
/// AUTHORITY is `evil.com` (`runner.example` is parsed as userinfo) — the scoped
/// bearer would be sent to `evil.com`. Likewise `".evil.com/x"` →
/// `"https://runner.example.evil.com/x"`, or `":9/x"` → a port swap. Requiring a
/// single leading `/` (and no `//`) TERMINATES the authority at the base's host,
/// so everything after is an in-authority path. We additionally reject
/// whitespace, control bytes, and backslash (never in the frozen
/// `/v1/leases/{id}/envelope/ingest` shape) as defense in depth.
fn validate_ingest_path(ingest_path: &str) -> Result<(), RunnerError> {
    let ok = ingest_path.starts_with('/')
        && !ingest_path.starts_with("//")
        && ingest_path.len() <= 512
        && !ingest_path
            .bytes()
            .any(|b| b == b'\\' || b.is_ascii_whitespace() || b.is_ascii_control());
    if ok {
        Ok(())
    } else {
        Err(RunnerError::InvalidIngestPath(format!(
            "ingest_path must be a single-leading-slash absolute path with no \
             `//`/whitespace/control/backslash (got {} chars)",
            ingest_path.len()
        )))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DTOs — minimal serde request/response shapes.
// ─────────────────────────────────────────────────────────────────────────────

/// Request body for `POST /v1/leases` — acquire a runner lease.
///
/// Transcribed from the AUTHORITATIVE frozen fabric DTO
/// (`corelink-fabric-api::dto::AcquireRequest`), which is `deny_unknown_fields`:
/// the caller supplies ONLY the fields the fabric cannot derive
/// (`principal_chain`/`path_set` are resolved server-side from the authenticated
/// tenant + claim, NOT sent). The prior shape carried `principal_chain` +
/// `path_set` + a `ttl_ms` field — an OLD/never-live contract the fake-transport,
/// PAT-gated client never exercised, so a real acquire `422`d
/// `unknown field principal_chain` against the live fabric (the same drift class
/// as the acquire-RESPONSE wrapper fix #204 and the close fix #205).
///
/// hugit is LIBERAL on its OWN side (no `deny_unknown_fields`); the load-bearing
/// invariant is that it SERIALIZES to exactly the four keys the fabric accepts.
/// The fabric's `runner` (direct-CI runner mode) + `toolchain_digest` (check-host
/// B) fields are `#[serde(default)]` there and hugit's classic OFF-BOX path never
/// sets them, so OMITTING them here is byte-identical to `runner: None,
/// toolchain_digest: None` — they are deliberately NOT transcribed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcquireLeaseRequest {
    /// Pinned image reference (the fabric only FORMAT-checks for `sha256:`, NOT
    /// existence). For hugit's off-box A-mode this is recorded metadata — no box
    /// is ever spawned (hugit submits §13 off-box, never execs).
    pub image_digest: String,
    /// Network policy name / ref governing the runner's outbound access.
    pub net_policy: String,
    /// Temporary root directory requested for this runner (mirrors
    /// `RunnerLease.tmp_root`).
    pub tmp_root: String,
    /// Requested lease TTL, in milliseconds; the fabric converts it into the
    /// absolute `RunnerLease.expiry`. (The fabric's field name for the same u64
    /// the prior `ttl_ms` carried.)
    pub expiry_ms: u64,
}

/// §13.2 OFF-BOX ingest credential surfaced on the acquire response — the
/// cost-killer path "A". Transcribed from the authoritative fabric DTO
/// (`corelink-fabric-api::dto::EnvelopeIngest`, frozen). Present for
/// non-runner/check leases when §13 is wired; ABSENT (skipped on the wire) for
/// runner leases. hugit is deliberately LIBERAL in what it accepts here (no
/// `deny_unknown_fields`) — the producer's frozen shape is the authority.
///
/// `credential` is the scoped, write-only, lease-folded ingest token — held like
/// the PAT: `Debug` is hand-written below so it can NEVER leak through `{:?}`,
/// and it never appears in any `Display`/error (the transport receives it as a
/// per-call bearer, never stored).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeIngest {
    /// Lease-scoped §13.2 ingest path — `POST` trajectory events here. Relative
    /// to the runner base (e.g. `/v1/leases/{lease_id}/envelope/ingest`).
    pub ingest_path: String,
    /// The scoped, WRITE-ONLY, lease-folded ingest credential (the Bearer for
    /// `ingest_path`). NOT a tenant PAT. SECRET — see the type docs.
    pub credential: String,
}

// `Debug` is hand-written so the scoped ingest credential can NEVER leak through
// `{:?}` — the same discipline the PAT gets in `RunnerConfig`.
impl std::fmt::Debug for EnvelopeIngest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EnvelopeIngest")
            .field("ingest_path", &self.ingest_path)
            .field("credential", &"<redacted>")
            .finish()
    }
}

/// Response body for `POST /v1/leases` — the granted lease WRAPPER. Transcribed
/// from the authoritative fabric DTO (`corelink-fabric-api::dto::AcquireResponse`).
///
/// The real fabric returns this WRAPPER (`{lease, exec_endpoint, envelope_ingest?}`),
/// NOT a flat [`RunnerLease`]. hugit is LIBERAL in what it accepts: only `lease`
/// is required; `exec_endpoint` and `envelope_ingest` default (an absent
/// `envelope_ingest` — a runner lease — is fine, not an error). `RunnerLease`
/// stays the inner type (its `conformance/RunnerLease.json` vector is the oracle).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AcquireResponse {
    /// The granted lease, wire-conformant to the frozen [`RunnerLease`] type.
    pub lease: RunnerLease,
    /// The exec endpoint for this lease (the fabric always sends it; defaulted so
    /// hugit stays liberal if a future/minimal producer omits it).
    #[serde(default)]
    pub exec_endpoint: String,
    /// §13.2 off-box ingest credential — present for non-runner/check leases when
    /// §13 is wired; `None` for runner leases (or when §13 is off). `#[serde(default)]`
    /// so an absent field deserializes to `None` (runner-lease responses parse
    /// byte-identically to before this field existed).
    #[serde(default)]
    pub envelope_ingest: Option<EnvelopeIngest>,
}

/// §13.2 per-turn usage — the four token classes (NO `total`; the fabric derives
/// totals at finalize, never supplied). Transcribed from the fabric ingest
/// handler's `IngestUsage`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestUsage {
    /// Input (non-cached) tokens.
    pub input: u64,
    /// Output tokens.
    pub output: u64,
    /// Tokens read from prompt cache.
    pub cache_read: u64,
    /// Tokens written to prompt cache.
    pub cache_write: u64,
}

/// One §13.2 trajectory event the OFF-BOX agent loop POSTs to `ingest_path`
/// (cost-killer path "A"). Transcribed from the fabric ingest handler's
/// `IngestEvent`. The fabric accepts ONE event, a JSON array, OR NDJSON; hugit
/// submits the JSON-array form. `bytes_b64` is the raw transcript bytes
/// (standard base64), forwarded VERBATIM (the runner never scrubs/persists —
/// §13.3); the forge owns redaction on its write path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestEvent {
    /// Event discriminator: `model_turn` | `tool_call` | `tool_result` | `prompt`.
    pub kind: String,
    /// Raw transcript bytes, standard-alphabet base64.
    pub bytes_b64: String,
    /// Tool name — REQUIRED by the fabric for `tool_call`/`tool_result`; omitted
    /// on the wire when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// Per-turn usage — only meaningful for `model_turn`; omitted on the wire
    /// when absent (the fabric accumulates nothing, never a fabricated zero).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<IngestUsage>,
    /// Model/tool busy span in ms (`model_turn`/`tool_call`); defaults to 0.
    #[serde(default)]
    pub busy_ms: u64,
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

// ── The agent-exec seam (cost-killer path B — exec-server drive) ──────────────
//
// The FROZEN cross-repo DTOs for the `mode: agent` lease's exec-drive, byte-frozen
// in `conformance/{AgentExecRequest,AgentExecAck,AgentExecResult}.json` (Runners TL
// #290, sha256 in `conformance/manifest.sha256`, matched against their published
// hashes). hugit's OFF-BOX §13 agent loop drives an egress-enabled, NON-memoized
// agent box via these: `POST /v1/leases/{id}/agent-exec` (→ [`AgentExecAck`]) then
// `GET /v1/leases/{id}/agent-exec/{step_id}` (→ [`AgentExecResult`]), possibly many
// times, then `close` with the provider-`/usage` `cost_usd_micros` + the §13 envelope
// (both UNCHANGED — already frozen + proven). The §13 agent LOOP itself is the P2
// deferral; these DTOs freeze the seam it will drive.

/// An agent-exec dispatch body: `POST /v1/leases/{lease_id}/agent-exec`. Runs an
/// ARBITRARY command (NOT a [`CheckDef`] — no `toolchain_ref`, never memoized) in the
/// egress-enabled agent box. `argv` avoids a shell-quoting seam; `env` is an ORDERED
/// map (byte-stability) and MUST NEVER carry a tenant PAT (the §13.2 ingest credential
/// is the separate, lease-scoped token). Empty `workdir` ⇒ the lease `tmp_root`
/// server-side; a `timeout_ms` kill ⇒ conventional exit `124`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentExecRequest {
    /// The command as an argv vector (no shell parsing on hugit's side).
    pub argv: Vec<String>,
    /// The scoped run env — ORDERED for byte-stable serialization; NEVER a tenant PAT.
    pub env: std::collections::BTreeMap<String, String>,
    /// Working directory; empty ⇒ the lease `tmp_root` (server-side default).
    pub workdir: String,
    /// Per-exec wall-clock bound in ms; a timeout kill surfaces as exit `124`.
    pub timeout_ms: u64,
}

/// Acknowledgement of an [`AgentExecRequest`]: the box accepted the step (200 ran |
/// 202 async). A REFUSAL is an HTTP error, NEVER `accepted: false`. Poll the result
/// via `GET /v1/leases/{lease_id}/agent-exec/{step_id}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentExecAck {
    /// The lease the step ran under.
    pub lease_id: String,
    /// The step handle to poll the [`AgentExecResult`] with.
    pub step_id: String,
    /// Whether the box accepted the step (always `true` on a 2xx; a refusal is an error).
    pub accepted: bool,
}

/// The captured result of an agent-exec step: `GET /v1/leases/{lease_id}/agent-exec/{step_id}`.
/// `exit_code` is verbatim (a `timeout_ms` kill ⇒ `124`; a signal ⇒ `128 + n`);
/// `truncated` flags captured stdio that hit the fabric's capture cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentExecResult {
    /// The step this result belongs to.
    pub step_id: String,
    /// The process exit code, verbatim (`124` = timeout kill, `128+n` = signal).
    pub exit_code: i32,
    /// Captured stdout (possibly `truncated`).
    pub stdout: String,
    /// Captured stderr (possibly `truncated`).
    pub stderr: String,
    /// Wall-clock duration of the step in ms.
    pub duration_ms: u64,
    /// True iff captured stdio hit the fabric's capture cap.
    pub truncated: bool,
}

/// The `agent`-mode marker on an acquire request (peer to `runner`/check-host). An
/// empty `{}` today — egress-enabled + memoization-OFF ARE the mode semantics
/// (server-side), so the marker's mere PRESENCE selects agent mode; the struct is
/// extensible without a wire break. An `agent` + `runner` both-present acquire is a
/// fabric `400`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct AgentSpec {}

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

/// Terminal job status a caller may claim on close — the `status` body the
/// fabric's `POST /v1/leases/{lease_id}/close` REQUIRES (transcribed from
/// `corelink-fabric-api::dto::CloseRequest`). The fabric accepts EXACTLY
/// `"succeeded"` | `"failed"` (lowercase); `killed` is the fabric's own
/// abnormal-path verdict and is NEVER caller-supplied. Anything else is a 400
/// `invalid` server-side, so we never emit it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseStatus {
    /// The job finished cleanly.
    Succeeded,
    /// The job ran but its verdict was a failure.
    Failed,
}

impl CloseStatus {
    /// The exact lowercase wire token the fabric's `status` field accepts.
    fn as_wire(self) -> &'static str {
        match self {
            CloseStatus::Succeeded => "succeeded",
            CloseStatus::Failed => "failed",
        }
    }
}

/// Request body for `POST /v1/leases/{lease_id}/close`. Mirrors the fabric DTO
/// `corelink-fabric-api::dto::CloseRequest` — the REQUIRED terminal `status`
/// (the prior empty-body close was a latent wire bug: the fabric `400`s a
/// status-less close). The fabric's `check_result: Option<CheckResult>` field
/// is OMITTED here: hugit's A-path delivers no `CheckResult` on close (the
/// off-box result is the fabric's signed envelope), and serde treats a missing
/// `Option` field as `None`, so `{"status":"..."}` is accepted by the fabric's
/// `deny_unknown_fields` body verbatim.
// `Deserialize` is derived only so the conformance test can round-trip the canonical
// fabric vector (hugit SENDS this DTO, never receives it); otherwise unused.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CloseRequest {
    /// `"succeeded"` | `"failed"` — the only two the caller may claim.
    pub status: String,

    /// The PROVIDER-billed total cost of the lease's work, in USD micro-dollars
    /// (`u64`; `1_000_000` == $1) — the owner's 2026-06-27 re-decision (#64). The
    /// fabric (`CloseRequest.cost_usd_micros`, #226) records this figure VERBATIM
    /// into `CloseResponse.metrics.cost_usd_micros`; it never recomputes or
    /// price-cards it. `Some(n)` is a REAL provider-billed figure (read from the
    /// provider's `/usage` by the off-box caller); `None` keeps the honest-zero
    /// derived floor. The honesty law is load-bearing: this is ONLY ever a figure
    /// measured for THIS lease's work — never a derived/misattributed stand-in.
    ///
    /// `skip_serializing_if` is REQUIRED, not cosmetic: the fabric body is
    /// `deny_unknown_fields`, so when `None` the field MUST NOT appear on the wire
    /// — the body stays byte-identical to today's `{"status":"..."}`, accepted by
    /// BOTH the not-yet-redeployed fabric (no such field) and the new one (which
    /// `#[serde(default)]`s it). When `Some(n)` it serializes as
    /// `"cost_usd_micros": n`, the exact key the frozen fabric DTO expects.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_usd_micros: Option<u64>,
}

/// Response body for `POST /v1/leases/{lease_id}/close` — the §13.1 finalized
/// per-job metrics delivered ATOMICALLY with the close. Transcribed from the
/// fabric DTO `corelink-fabric-api::dto::CloseResponse`.
///
/// hugit is LIBERAL in what it accepts (NO `deny_unknown_fields`): the fabric
/// also carries `attestation`, `result_binding_sig`, `result_binding_sig_v2`,
/// `fabric_key_id`, and an echoed `check_result` — none of which the A-mode cost
/// path consumes here, so they are tolerated-and-ignored rather than modelled
/// (keeping hugit forward-compatible with additive fabric fields, the §13.4
/// drift posture). `metrics` is the ONE load-bearing field: it is the fabric's
/// signed, finalized §13.1 figure — the attested per-job cost source of truth.
/// It is intentionally NOT defaulted (a metrics-less close is a contract
/// violation — §13.1 "never optional when the job succeeded" — so an absent
/// `metrics` is a decode error, never a fabricated zero).
// `Serialize` is derived only so the conformance test can round-trip the canonical
// fabric vector (hugit RECEIVES this DTO, never sends it); otherwise unused.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CloseResponse {
    /// The closed lease id (echoed).
    pub lease_id: String,
    /// Whether the lease reached `Released` as a result of this call.
    #[serde(default)]
    pub released: bool,
    /// `true` iff transcript capture was lossy/unconfirmed — honest, never
    /// silent. Carried through so the forge can record capture honesty.
    #[serde(default)]
    pub capture_incomplete: bool,
    /// The finalized §13.1 per-job metrics (the fabric's signed figure). Maps to
    /// the frozen `IntentMetrics` via [`RunnerJobMetrics::into_intent_metrics`],
    /// preserving the FULL token cache-split (never flattened to `total`).
    pub metrics: RunnerJobMetrics,
}

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

    /// Borrow the underlying transport. Test-only inspection hook so the
    /// orchestration tests in the sibling [`crate::runner::dispatch`] module can
    /// assert which calls were made (this module's own tests reach the private
    /// field directly).
    #[cfg(test)]
    pub(crate) fn transport(&self) -> &T {
        &self.transport
    }

    /// Acquire a runner lease: `POST {host}/v1/leases`.
    ///
    /// Parses the fabric's [`AcquireResponse`] WRAPPER (`{lease, exec_endpoint,
    /// envelope_ingest?}`), NOT a flat [`RunnerLease`] — the wrapper is the real
    /// wire shape (the prior flat parse was a latent bug masked by hermetic fakes
    /// that fed flat JSON; the live PAT-gated path was never run). Callers that
    /// only need the lease extract `.lease`.
    pub fn acquire(&self, req: &AcquireLeaseRequest) -> Result<AcquireResponse, RunnerError> {
        let url = format!("{}/v1/leases", self.config.base());
        let body = serde_json::to_vec(req).map_err(|e| RunnerError::Decode(e.to_string()))?;
        let (status, resp) = self.transport.post(&url, &self.config.bearer(), &body)?;
        match status {
            // 200 (returned existing) | 201 (fresh) both carry the wrapper body.
            200 | 201 => {
                serde_json::from_slice(&resp).map_err(|e| RunnerError::Decode(e.to_string()))
            }
            429 | 503 => Err(RunnerError::Busy {
                detail: format!("acquire returned HTTP {status}"),
            }),
            other => Err(RunnerError::Status(other)),
        }
    }

    /// Submit a §13.2 trajectory-event batch to the lease's OFF-BOX ingest
    /// endpoint (cost-killer path "A"): `POST {host}{ingest_path}` with
    /// `Authorization: Bearer {credential}`.
    ///
    /// `credential` is the SCOPED, write-only ingest token from
    /// [`AcquireResponse::envelope_ingest`] — NEVER the tenant PAT (two
    /// credentials by trust boundary: the scoped cred for ingest, the PAT for
    /// acquire/poll/close). It is taken as a per-call bearer and NEVER stored,
    /// logged, or rendered in any error (errors carry only the HTTP status).
    ///
    /// `ingest_path` is the server-supplied path from the acquire response. It is
    /// concatenated onto the configured base, so it is VALIDATED first
    /// ([`validate_ingest_path`]) — a safe single-leading-slash absolute path —
    /// so the credential can only ever be sent to the configured host: a
    /// buggy/compromised producer cannot redirect it elsewhere (e.g. an
    /// `@evil.com`/`.evil.com` authority-injection is rejected before any send).
    pub fn submit_envelope(
        &self,
        ingest_path: &str,
        credential: &str,
        events: &[IngestEvent],
    ) -> Result<(), RunnerError> {
        // Fail-closed BEFORE building the URL or attaching the credential: never
        // send the scoped ingest bearer to a host a hostile ingest_path chose.
        validate_ingest_path(ingest_path)?;
        let url = format!("{}{}", self.config.base(), ingest_path);
        let bearer = format!("Bearer {credential}");
        let body = serde_json::to_vec(events).map_err(|e| RunnerError::Decode(e.to_string()))?;
        let (status, _resp) = self.transport.post(&url, &bearer, &body)?;
        match status {
            // 200 | 201 | 202 | 204 all ack the ingest (the fabric returns 200).
            200 | 201 | 202 | 204 => Ok(()),
            429 | 503 => Err(RunnerError::Busy {
                detail: format!("submit_envelope returned HTTP {status}"),
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
    ///
    /// POSTs the REQUIRED [`CloseRequest`] `status` body (`succeeded`/`failed`)
    /// and PARSES the fabric's [`CloseResponse`], returning the finalized §13.1
    /// metrics. The prior version POSTed an EMPTY body and returned `()` — a
    /// latent wire bug (the fake transport never enforced the body, the live
    /// PAT-gated path was never run): the real fabric `400`s a status-less close
    /// AND delivers the attested metrics on this same atomic call.
    ///
    /// Fail-closed: ONLY a `200` with a parseable [`CloseResponse`] succeeds. A
    /// `204`/empty body cannot carry the §13.1-required `metrics`, so it is NOT
    /// accepted as success (it would mean a metrics-less close — a contract
    /// violation; surfaced as [`RunnerError::Status`]). The metrics are never
    /// fabricated: a close error surfaces, it is never papered over with a zero.
    /// `cost_usd_micros` is the PROVIDER-billed total cost for this lease's work
    /// (read from the provider's `/usage` by the off-box caller), recorded VERBATIM
    /// by the fabric (#226) into `CloseResponse.metrics.cost_usd_micros`. `None` is
    /// the honest default (the fabric keeps its derived honest-zero floor) and the
    /// body stays byte-identical to today (`{"status":"..."}`); `Some(n)` adds the
    /// `cost_usd_micros` key. NEVER pass a derived/misattributed figure — only a
    /// real cost measured for THIS lease's work (the per-PR honesty law).
    pub fn close(
        &self,
        lease_id: &str,
        status: CloseStatus,
        cost_usd_micros: Option<u64>,
    ) -> Result<CloseResponse, RunnerError> {
        validate_lease_id(lease_id)?;
        let url = format!("{}/v1/leases/{}/close", self.config.base(), lease_id);
        let req = CloseRequest {
            status: status.as_wire().to_string(),
            cost_usd_micros,
        };
        let body = serde_json::to_vec(&req).map_err(|e| RunnerError::Decode(e.to_string()))?;
        let (code, resp) = self.transport.post(&url, &self.config.bearer(), &body)?;
        match code {
            // The fabric returns 200 + the atomic CloseResponse body (metrics +
            // the honest capture_incomplete flag).
            200 => serde_json::from_slice(&resp).map_err(|e| RunnerError::Decode(e.to_string())),
            429 | 503 => Err(RunnerError::Busy {
                detail: format!("close returned HTTP {code}"),
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
        // The real fabric returns the AcquireResponse WRAPPER (not a flat lease).
        let acquire_resp = AcquireResponse {
            lease: lease.clone(),
            exec_endpoint: format!("/v1/leases/{lease_id}/exec"),
            envelope_ingest: Some(EnvelopeIngest {
                ingest_path: format!("/v1/leases/{lease_id}/envelope/ingest"),
                credential: "scoped-ingest-token-xyz".to_string(),
            }),
        };
        let ack = ExecAck {
            lease_id: lease_id.to_string(),
            accepted: true,
        };
        let transport = FakeTransport::with_responses(vec![
            (201, serde_json::to_vec(&acquire_resp).unwrap()),
            (202, serde_json::to_vec(&ack).unwrap()),
            (200, br#"{"status":"running","cost_usd_micros":0}"#.to_vec()),
            (200, b"event: started\nevent: done\n".to_vec()),
            (200, sample_close_response_body()),
        ]);
        let config = RunnerConfig::new("https://runner.example/", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);

        // acquire — the wrapper parses; the inner lease is byte-identical.
        let req = AcquireLeaseRequest {
            image_digest: "alpine@sha256:d9e853af2c8e".to_string(),
            net_policy: "deny-all".to_string(),
            tmp_root: "/work/tmp".to_string(),
            expiry_ms: 60_000,
        };
        let got = client.acquire(&req).unwrap();
        assert_eq!(got.lease, lease);
        assert_eq!(
            got.envelope_ingest.as_ref().unwrap().ingest_path,
            format!("/v1/leases/{lease_id}/envelope/ingest")
        );

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

        // close — POSTs the required status body and parses CloseResponse.
        let closed = client
            .close(lease_id, CloseStatus::Succeeded, None)
            .unwrap();
        assert_eq!(
            closed.metrics.cost_usd_micros, 1834290,
            "close returns the finalized §13.1 metrics"
        );

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

        // The acquire body is the serialized request; the close body carries the
        // required `status` field (never empty — the fixed wire contract).
        let sent_req: AcquireLeaseRequest = serde_json::from_slice(&calls[0].body).unwrap();
        assert_eq!(sent_req, req);
        let close_req: serde_json::Value = serde_json::from_slice(&calls[4].body).unwrap();
        assert_eq!(close_req["status"], "succeeded");
    }

    /// The acquire REQUEST body serializes to EXACTLY the four keys the frozen
    /// fabric `AcquireRequest` (`deny_unknown_fields`) accepts —
    /// `{image_digest, net_policy, tmp_root, expiry_ms}` — and NONE of the
    /// old/never-live fields (`principal_chain` / `path_set` / `ttl_ms`) that made
    /// a real acquire `422 unknown field principal_chain`. This guards the wire
    /// drift so it cannot recur.
    #[test]
    fn acquire_request_serializes_to_exactly_the_four_fabric_keys() {
        let req = AcquireLeaseRequest {
            image_digest: "alpine@sha256:d9e853af2c8e".to_string(),
            net_policy: "isolated".to_string(),
            tmp_root: "/work/tmp".to_string(),
            expiry_ms: 60_000,
        };
        let value = serde_json::to_value(&req).unwrap();
        let obj = value
            .as_object()
            .expect("the acquire request serializes to a JSON object");

        // Exactly the four fabric keys — no more, no fewer.
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec!["expiry_ms", "image_digest", "net_policy", "tmp_root"],
            "the acquire body must carry ONLY the four keys the fabric's \
             deny_unknown_fields AcquireRequest accepts"
        );

        // The retired fields must be ABSENT (their presence is the 422 root cause).
        for retired in ["principal_chain", "path_set", "ttl_ms"] {
            assert!(
                !obj.contains_key(retired),
                "the retired field `{retired}` must never be serialized (it 422s the fabric)"
            );
        }

        // The values are carried verbatim under the fabric's field names.
        assert_eq!(obj["image_digest"], "alpine@sha256:d9e853af2c8e");
        assert_eq!(obj["net_policy"], "isolated");
        assert_eq!(obj["tmp_root"], "/work/tmp");
        assert_eq!(obj["expiry_ms"], 60_000);
    }

    /// A representative fabric `CloseResponse` body — the §13.1 metrics PLUS the
    /// fabric extras (`attestation`, the result-binding sigs, `fabric_key_id`,
    /// echoed `check_result`) hugit deliberately tolerates-and-ignores (liberal,
    /// no `deny_unknown_fields`). The metrics mirror the pinned `IntentMetrics`
    /// vector so the cost asserts read a known value.
    fn sample_close_response_body() -> Vec<u8> {
        br#"{
            "lease_id": "lease-abc123",
            "released": true,
            "capture_incomplete": false,
            "metrics": {
                "tokens": {"input": 48211, "output": 9143, "cache_read": 120557, "cache_write": 3361, "total": 181272},
                "wall_ms": 754000,
                "active_ms": 612450,
                "tool_calls": 41,
                "tool_breakdown": [{"tool": "Bash", "count": 17}],
                "model_turns": 58,
                "cost_usd_micros": 1834290
            },
            "check_result": null,
            "attestation": {"tree": "", "def": "", "runner": "", "model": "", "principal": ["tenant:t-1"], "sig": "ZmFrZXNpZw=="},
            "result_binding_sig": "ZmFrZQ==",
            "result_binding_sig_v2": "ZmFrZTI=",
            "fabric_key_id": "0011223344556677"
        }"#
        .to_vec()
    }

    #[test]
    fn busy_status_is_typed_retryable() {
        let transport = FakeTransport::with_responses(vec![(503, Vec::new())]);
        let config = RunnerConfig::new("https://runner.example", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);
        let req = AcquireLeaseRequest {
            image_digest: "alpine@sha256:d9e853af2c8e".to_string(),
            net_policy: "deny-all".to_string(),
            tmp_root: "/work/tmp".to_string(),
            expiry_ms: 1,
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

    /// The fabric returns the `AcquireResponse` WRAPPER for a check lease — the
    /// `envelope_ingest` field is PRESENT. Proves the wrapper parses (the
    /// flat-parse bug is fixed) and the scoped ingest credential is surfaced.
    #[test]
    fn acquire_parses_wrapper_with_envelope_ingest() {
        let lease = sample_lease("lease-check-1");
        let wrapper = format!(
            r#"{{"lease":{lease},"exec_endpoint":"/v1/leases/lease-check-1/exec","envelope_ingest":{{"ingest_path":"/v1/leases/lease-check-1/envelope/ingest","credential":"scoped-tok"}}}}"#,
            lease = serde_json::to_string(&lease).unwrap()
        );
        let transport = FakeTransport::with_responses(vec![(200, wrapper.into_bytes())]);
        let config = RunnerConfig::new("https://runner.example", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);
        let req = AcquireLeaseRequest {
            image_digest: "alpine@sha256:d9e853af2c8e".to_string(),
            net_policy: "deny-all".to_string(),
            tmp_root: "/work/tmp".to_string(),
            expiry_ms: 1,
        };
        let got = client.acquire(&req).unwrap();
        assert_eq!(got.lease, lease);
        assert_eq!(got.exec_endpoint, "/v1/leases/lease-check-1/exec");
        let ingest = got
            .envelope_ingest
            .expect("check lease carries envelope_ingest");
        assert_eq!(
            ingest.ingest_path,
            "/v1/leases/lease-check-1/envelope/ingest"
        );
        assert_eq!(ingest.credential, "scoped-tok");
    }

    /// A RUNNER lease acquire response omits `envelope_ingest` (byte-identical to
    /// before the field existed). It must STILL parse — `envelope_ingest` defaults
    /// to `None`, never a decode error (hugit is liberal in what it accepts).
    #[test]
    fn acquire_runner_lease_absent_envelope_ingest_parses_none() {
        let lease = sample_lease("lease-runner-1");
        let wrapper = format!(
            r#"{{"lease":{lease},"exec_endpoint":"/v1/leases/lease-runner-1/exec"}}"#,
            lease = serde_json::to_string(&lease).unwrap()
        );
        let transport = FakeTransport::with_responses(vec![(201, wrapper.into_bytes())]);
        let config = RunnerConfig::new("https://runner.example", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);
        let req = AcquireLeaseRequest {
            image_digest: "alpine@sha256:d9e853af2c8e".to_string(),
            net_policy: "deny-all".to_string(),
            tmp_root: "/work/tmp".to_string(),
            expiry_ms: 1,
        };
        let got = client.acquire(&req).unwrap();
        assert_eq!(got.lease, lease);
        assert!(
            got.envelope_ingest.is_none(),
            "a runner lease has no §13 ingest credential"
        );
    }

    /// `submit_envelope` POSTs to `{base}{ingest_path}` with the SCOPED credential
    /// as the Bearer — NEVER the tenant PAT. A sentinel PAT proves the PAT never
    /// reaches the ingest endpoint.
    #[test]
    fn submit_envelope_uses_scoped_credential_not_pat() {
        const SCOPED: &str = "scoped-write-only-ingest-tok-44ab";
        let transport = FakeTransport::with_responses(vec![(200, Vec::new())]);
        let config = RunnerConfig::new("https://runner.example/", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);

        let events = vec![IngestEvent {
            kind: "model_turn".to_string(),
            bytes_b64: String::new(),
            tool: None,
            usage: Some(IngestUsage {
                input: 10,
                output: 20,
                cache_read: 30,
                cache_write: 40,
            }),
            busy_ms: 99,
        }];
        client
            .submit_envelope("/v1/leases/lease-z/envelope/ingest", SCOPED, &events)
            .unwrap();

        let calls = client.transport.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "POST");
        assert_eq!(
            calls[0].url,
            "https://runner.example/v1/leases/lease-z/envelope/ingest"
        );
        // The Bearer is the SCOPED credential, NOT the tenant PAT.
        assert_eq!(calls[0].bearer, format!("Bearer {SCOPED}"));
        assert_ne!(calls[0].bearer, format!("Bearer {SENTINEL_PAT}"));
        assert!(
            !calls[0].bearer.contains(SENTINEL_PAT),
            "the tenant PAT must never reach the ingest endpoint"
        );
        // The body is the serialized event array.
        let sent: Vec<IngestEvent> = serde_json::from_slice(&calls[0].body).unwrap();
        assert_eq!(sent, events);
    }

    /// `submit_envelope` maps a retryable status to `Busy` and a terminal status
    /// to `Status`, and the scoped credential never leaks in either error.
    #[test]
    fn submit_envelope_status_mapping_and_no_credential_leak() {
        const SCOPED: &str = "scoped-do-not-leak-77cd";
        let transport = FakeTransport::with_responses(vec![(503, Vec::new()), (401, Vec::new())]);
        let config = RunnerConfig::new("https://runner.example", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);
        let events = [IngestEvent {
            kind: "prompt".to_string(),
            bytes_b64: String::new(),
            tool: None,
            usage: None,
            busy_ms: 0,
        }];
        let busy = client
            .submit_envelope("/v1/leases/l/envelope/ingest", SCOPED, &events)
            .unwrap_err();
        assert!(matches!(busy, RunnerError::Busy { .. }));
        let term = client
            .submit_envelope("/v1/leases/l/envelope/ingest", SCOPED, &events)
            .unwrap_err();
        assert_eq!(term, RunnerError::Status(401));
        assert!(!format!("{busy:?}").contains(SCOPED));
        assert!(!format!("{term}").contains(SCOPED));
    }

    /// The scoped ingest credential is REDACTED in `Debug` (held like the PAT) —
    /// it must never leak through `{:?}` of `EnvelopeIngest` or `AcquireResponse`.
    #[test]
    fn envelope_ingest_credential_redacted_in_debug() {
        const SCOPED: &str = "scoped-SECRET-do-not-debug-9911";
        let ingest = EnvelopeIngest {
            ingest_path: "/v1/leases/l/envelope/ingest".to_string(),
            credential: SCOPED.to_string(),
        };
        let dbg = format!("{ingest:?}");
        assert!(!dbg.contains(SCOPED), "EnvelopeIngest Debug leaked: {dbg}");
        assert!(dbg.contains("<redacted>"));
        assert!(dbg.contains("/v1/leases/l/envelope/ingest"));

        let resp = AcquireResponse {
            lease: sample_lease("lease-dbg"),
            exec_endpoint: "/v1/leases/lease-dbg/exec".to_string(),
            envelope_ingest: Some(ingest),
        };
        let rdbg = format!("{resp:?}");
        assert!(
            !rdbg.contains(SCOPED),
            "AcquireResponse Debug leaked the credential: {rdbg}"
        );
    }

    /// `close` POSTs the REQUIRED `status` body and PARSES the fabric's
    /// `CloseResponse`, returning the finalized §13.1 metrics — the fixed wire
    /// contract (the prior empty-body, `()`-returning close was the latent bug).
    #[test]
    fn close_posts_status_body_and_parses_close_response() {
        let transport = FakeTransport::with_responses(vec![(200, sample_close_response_body())]);
        let config = RunnerConfig::new("https://runner.example/", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);

        let resp = client
            .close("lease-abc123", CloseStatus::Succeeded, Some(4_200_000))
            .unwrap();
        // The response carries the finalized metrics (full cache-split preserved).
        assert_eq!(resp.lease_id, "lease-abc123");
        assert!(resp.released);
        assert!(!resp.capture_incomplete);
        assert_eq!(resp.metrics.cost_usd_micros, 1834290);
        assert_eq!(resp.metrics.tokens.cache_read, 120557);
        assert_eq!(resp.metrics.tokens.cache_write, 3361);

        // The request carried the REQUIRED `status` field (never an empty body)
        // AND the provider-billed `cost_usd_micros` (the #64 submit side) with the
        // EXACT key the frozen fabric DTO expects.
        let calls = client.transport.calls();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].method, "POST");
        assert_eq!(
            calls[0].url,
            "https://runner.example/v1/leases/lease-abc123/close"
        );
        let body: serde_json::Value = serde_json::from_slice(&calls[0].body).unwrap();
        assert_eq!(body["status"], "succeeded");
        assert_eq!(body["cost_usd_micros"], 4_200_000);
        // A `failed` close maps to the other wire token.
        assert_eq!(CloseStatus::Failed.as_wire(), "failed");
    }

    /// `close` with `cost_usd_micros: None` sends a body BYTE-IDENTICAL to today's
    /// (`{"status":"..."}`) — the `skip_serializing_if` omits the field entirely,
    /// so the body stays accepted by the `deny_unknown_fields` fabric (BOTH the
    /// not-yet-redeployed one with no such field and the new one). The honest
    /// default never poisons the wire with a `null`.
    #[test]
    fn close_with_none_cost_omits_the_field_byte_identically() {
        let transport = FakeTransport::with_responses(vec![(200, sample_close_response_body())]);
        let config = RunnerConfig::new("https://runner.example", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);
        client
            .close("lease-abc123", CloseStatus::Succeeded, None)
            .unwrap();
        let calls = client.transport.calls();
        // EXACTLY `{"status":"succeeded"}` — no `cost_usd_micros` key at all.
        assert_eq!(
            String::from_utf8(calls[0].body.clone()).unwrap(),
            r#"{"status":"succeeded"}"#,
            "None must omit the field, keeping the body byte-identical to today"
        );
        let body: serde_json::Value = serde_json::from_slice(&calls[0].body).unwrap();
        assert!(
            body.as_object().unwrap().get("cost_usd_micros").is_none(),
            "the cost field must NOT appear when None"
        );
    }

    /// `close` authenticates with the tenant PAT (NEVER a scoped ingest
    /// credential — that is only for the §13.2 ingest submit). A sentinel PAT
    /// proves the PAT is the Bearer on the close call.
    #[test]
    fn close_uses_pat_not_scoped_credential() {
        let transport = FakeTransport::with_responses(vec![(200, sample_close_response_body())]);
        let config = RunnerConfig::new("https://runner.example", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);
        client
            .close("lease-abc123", CloseStatus::Succeeded, None)
            .unwrap();
        let calls = client.transport.calls();
        assert_eq!(calls[0].bearer, format!("Bearer {SENTINEL_PAT}"));
    }

    #[test]
    fn validate_ingest_path_accepts_the_frozen_shape_rejects_authority_injection() {
        // The frozen fabric shape is accepted.
        assert!(validate_ingest_path("/v1/leases/lease-abc123/envelope/ingest").is_ok());
        assert!(validate_ingest_path("/x").is_ok());

        // Authority-injection vectors — each would move the URL's host off the
        // configured base if concatenated, exfiltrating the scoped credential.
        for hostile in [
            "@evil.com/x",  // userinfo trick: base becomes userinfo, evil.com the host
            ".evil.com/x",  // suffix: base.evil.com becomes the host
            ":9999/x",      // port swap on the base host
            "//evil.com/x", // protocol-relative-style double slash
            "evil.com/x",   // no leading slash → joins the authority
            "",             // empty
            "v1/ingest",    // relative
            "/a\\b",        // backslash (some parsers treat as /)
            "/a b",         // whitespace
            "/a\nb",        // control char (header/URL smuggling)
        ] {
            assert!(
                validate_ingest_path(hostile).is_err(),
                "must reject hostile ingest_path {hostile:?}"
            );
        }
    }

    #[test]
    fn submit_envelope_rejects_hostile_ingest_path_before_sending_the_credential() {
        const SCOPED: &str = "scoped-ingest-cred-XYZ";
        // A response is queued; if the guard works, it is NEVER consumed because
        // the transport is never called.
        let transport = FakeTransport::with_responses(vec![(200, Vec::new())]);
        let config = RunnerConfig::new("https://runner.example", SENTINEL_PAT).unwrap();
        let client = LeaseClient::with_transport(config, transport);

        let err = client
            .submit_envelope("@evil.com/steal", SCOPED, &[])
            .expect_err("a hostile ingest_path must be refused, never sent");
        assert!(
            matches!(err, RunnerError::InvalidIngestPath(_)),
            "must be InvalidIngestPath, got {err:?}"
        );
        // The credential NEVER reached the wire — no transport call was made.
        assert!(
            client.transport.calls().is_empty(),
            "no request may be sent for a hostile ingest_path"
        );
        // And the scoped credential never appears in the error rendering.
        assert!(!format!("{err}").contains(SCOPED));
        assert!(!format!("{err:?}").contains(SCOPED));
    }
}
