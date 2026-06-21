//! The minimal synchronous `/v1` HTTP server (tiny_http) + routing.
//!
//! [`route`] is socket-free (method + url + headers → (status, body)) so it is
//! unit-testable without binding a port; [`serve`] is the thin tiny_http loop
//! that calls it. Transport law (backend-API-v1): GET-only reads; `/readyz` is
//! unauthenticated; every `/v1/*` read requires `Bearer`; a missing resource is
//! 404 (no existence leak); a tampered/unreadable log is 503 (fail-honest); the
//! error body is the `{code, reason}` envelope.

use std::io::Read;
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use tiny_http::{Header, Method, Request, Response, Server};

use crate::error::EngineErr;
use crate::handlers;
use crate::state::AppState;
use crate::writes::{self, LogSink, verbs, with_write};
use hugit_http_contracts::actions::Accepted;
use hugit_http_contracts::write_requests as wr;

/// Bind `addr` (e.g. `127.0.0.1:8787`) and run the server forever.
pub fn serve(state: AppState, addr: &str) -> std::io::Result<()> {
    let server = Server::http(addr).map_err(|e| std::io::Error::other(e.to_string()))?;
    eprintln!(
        "hugit-serve listening on {addr} (source={})",
        state.source_label()
    );
    serve_on(state, server)
}

/// Run the request loop over a PRE-BOUND server (the loop the real binary runs;
/// split out so a test can bind `:0`, learn the port, and exercise it end-to-end).
pub fn serve_on(state: AppState, server: Server) -> std::io::Result<()> {
    for mut request in server.incoming_requests() {
        let method = request.method().clone();
        let url = request.url().to_string();
        let headers = request.headers().to_vec();
        // Read the body ONLY for mutating methods (reads ignore it). Bounded read:
        // at most MAX_BODY_BYTES+1 so the door's size cap rejects an oversize body
        // without us buffering it all.
        let body = if method == Method::Post {
            read_body_capped(&mut request)
        } else {
            Vec::new()
        };
        // SSE replay: GET /v1/repos/{repo}/events?since=<seq>. Handled BEFORE the
        // standard (status, String) path because it needs a different Content-Type
        // and a Vec<u8> body. `respond_sse` consumes `request` in every branch.
        if method == Method::Get && is_events_path(&url) {
            respond_sse(&state, &url, &headers, request);
            continue;
        }
        // Git smart-HTTP (clone/fetch). Handled BEFORE the standard (status,String)
        // path because a packfile is a BINARY Vec<u8> body with a git-specific
        // Content-Type — it cannot ride `route_with_body`. The POST body was already
        // read above (capped); it is threaded in (the upload-pack want/have lines are
        // tiny, well under the cap). `respond_git` consumes `request` in every branch.
        // Push (git-receive-pack) is intentionally out of scope and 404s here.
        if crate::git::is_git_path(&url) {
            crate::git::respond_git(&state, &method, &url, &body, request);
            continue;
        }
        // PANIC ISOLATION: a panic inside a handler must degrade to a 503 for THAT
        // request, never take down the whole single-threaded server.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            route_with_body(&state, &method, &url, &headers, &body)
        }));
        let (status, body) = outcome.unwrap_or_else(|_| {
            eprintln!("hugit-serve: handler panicked — degraded to 503");
            (503, EngineErr::unavailable("internal error").to_body())
        });
        let response = Response::from_string(body)
            .with_status_code(status)
            .with_header(json_content_type());
        // A broken pipe (client hung up) is expected + silent; log other faults.
        if let Err(e) = request.respond(response)
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            eprintln!("hugit-serve: respond error: {e}");
        }
    }
    Ok(())
}

/// The `Content-Type: application/json` header — built once, cloned per response
/// (a static valid header; the `expect` is provably infallible).
fn json_content_type() -> Header {
    static CT: OnceLock<Header> = OnceLock::new();
    CT.get_or_init(|| {
        Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
            .expect("static content-type header is valid")
    })
    .clone()
}

/// The `Content-Type: text/event-stream` header (SSE) — built once, cloned per use.
fn sse_content_type() -> Header {
    static CT: OnceLock<Header> = OnceLock::new();
    CT.get_or_init(|| {
        Header::from_bytes(&b"Content-Type"[..], &b"text/event-stream"[..])
            .expect("static sse content-type header is valid")
    })
    .clone()
}

/// Whether `url`'s path is `/v1/repos/{repo}/events` (query ignored).
fn is_events_path(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    matches!(segs.as_slice(), ["v1", "repos", _, "events"])
}

/// Parse `?since=<u64>`; absent/unparseable → 0 (replay from the start).
fn parse_since(query: &str) -> u64 {
    query_param(query, "since").parse::<u64>().unwrap_or(0)
}

/// Serve a replay-then-close SSE response. Consumes `request` in EVERY branch.
/// Auth + slug + load are validated first, identical to the standard read path;
/// error cases respond with the JSON `{code, reason}` envelope.
fn respond_sse(state: &AppState, url: &str, headers: &[Header], request: Request) {
    // A small helper to send a JSON error envelope and return.
    fn send_json(request: Request, status: u16, body: String) {
        let resp = Response::from_string(body)
            .with_status_code(status)
            .with_header(json_content_type());
        if let Err(e) = request.respond(resp)
            && e.kind() != std::io::ErrorKind::BrokenPipe
        {
            eprintln!("hugit-serve: sse error-respond: {e}");
        }
    }

    // Auth BEFORE any resource work (the SAME two-tier gate as every other read:
    // a Clerk-minted engine token, else the dev-token fallback).
    let principal = match two_tier_auth(state, headers) {
        Ok((p, _)) => p,
        Err(e) => {
            let (status, body) = err(e);
            return send_json(request, status, body);
        }
    };
    let path = url.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let repo = match segs.as_slice() {
        ["v1", "repos", repo, "events"] => *repo,
        _ => {
            let (status, body) = err(EngineErr::not_found());
            return send_json(request, status, body);
        }
    };
    if !crate::state::is_safe_repo_slug(repo) {
        let (status, body) = err(EngineErr::not_found());
        return send_json(request, status, body);
    }
    let log = match state.load_verified(repo) {
        Ok(log) => log,
        Err(e) => {
            // Non-operator → 404 (no existence/integrity oracle on the SSE path).
            let (status, body) = err(hide_load_err(&principal, e));
            return send_json(request, status, body);
        }
    };
    // Per-tenant read gate — the SAME fail-closed decision as the standard read
    // path (a denied private repo's event stream is 404, no existence leak).
    let meta = crate::authz::project_repo_meta(&log);
    if !crate::authz::authorize_read(&principal, &meta) {
        let (status, body) = err(EngineErr::not_found());
        return send_json(request, status, body);
    }
    let since = parse_since(url.split('?').nth(1).unwrap_or(""));
    let bytes = handlers::build_events(&log, repo, since);
    let resp = Response::from_data(bytes)
        .with_status_code(200)
        .with_header(sse_content_type());
    if let Err(e) = request.respond(resp)
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        eprintln!("hugit-serve: sse respond: {e}");
    }
}

/// Map a load/verify failure to the status the CALLER may see. The operator gets
/// the honest error (a 503 carrying integrity/transport detail); every other caller
/// gets a uniform 404 — so a 503 on a private repo they don't own can never be an
/// existence/integrity oracle (audit 2026-06-16; upholds "deny→404, no existence
/// leak" on the load-failure path, matching the gate-deny path which is already 404).
fn hide_load_err(principal: &[String], e: EngineErr) -> EngineErr {
    if crate::authz::is_operator(principal) {
        e
    } else {
        EngineErr::not_found()
    }
}

/// Route + dispatch one request to a `(status, body)` pair. Socket-free.
#[must_use]
pub fn route(state: &AppState, method: &Method, url: &str, headers: &[Header]) -> (u16, String) {
    // Strip the query string; split into non-empty path segments.
    let path = url.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

    // /readyz — unauthenticated liveness/readiness (the window's boot probe).
    if method == &Method::Get && segs == ["readyz"] {
        return (200, r#"{"ready":true}"#.to_string());
    }

    // /v1/me/login — PUBLIC (no Bearer): the auth entry-point, served before any
    // session exists. A pre-match guard (mirrors /readyz) so it bypasses the
    // method gate + two_tier_auth. Static presentational card, no EventLog.
    if method == &Method::Get && segs == ["v1", "me", "login"] {
        return ok(&handlers::build_login());
    }

    // All other routes are GET-only reads (writes are Wave-2).
    if method != &Method::Get {
        return err(EngineErr::not_found());
    }

    // /v1/repos/{repo}/... — every read is Bearer-authenticated (auth BEFORE any
    // repo/resource work, so 401 never depends on whether the repo exists).
    match segs.as_slice() {
        ["v1", "repos", repo, tail @ ..] => {
            let (principal, _) = match two_tier_auth(state, headers) {
                Ok(p) => p,
                Err(e) => return err(e),
            };
            // Load + verify the repo log ONCE (404 absent/unsafe-slug, 503
            // tampered) BEFORE gating — and reuse it for the handler (no
            // double-verify).
            let log = match state.load_verified(repo) {
                Ok(l) => l,
                // Non-operator → 404 (a load failure on a private repo a tenant does
                // not own must not reveal it exists / is tampered — same no-leak law
                // as the gate-deny path below, which is already 404).
                Err(e) => return err(hide_load_err(&principal, e)),
            };
            // PER-TENANT READ GATE (fail-closed, re-decided server-side on every
            // repo read — githugr TL request 2026-06-15 / ADR-0007 §3): a denied
            // PRIVATE repo is a 404, identical to a non-existent one (no existence
            // oracle). Operator (dev/orchestrator) bypass keeps single-tenant dev +
            // the launch repo working until owner_tenant is assigned.
            let meta = crate::authz::project_repo_meta(&log);
            if !crate::authz::authorize_read(&principal, &meta) {
                return err(EngineErr::not_found());
            }
            // The query (stripped above) is re-passed for the param-driven reads.
            // The git content seam (`blob`/`edit`) is threaded from state; `None`
            // when no `HUGIT_SERVE_GIT_DIR` is wired → those reads 404 honestly.
            dispatch_repo(
                repo,
                tail,
                url.split('?').nth(1).unwrap_or(""),
                &log,
                &principal,
                state.git_source.as_ref(),
                state.git_root_tree.as_ref(),
            )
        }
        // Identity-scoped reads (/v1/me/*): no {repo} path param — they bind the
        // launch repo (`ME_DEFAULT_REPO`) until the P2 per-principal multi-repo
        // `me` aggregation lands. They STILL run the per-tenant gate against that
        // repo (audit 2026-06-15): without it, any authenticated tenant could read
        // the launch repo's operational data here — a cross-tenant leak. Operator
        // bypass keeps the dev/launch view working; a non-owner tenant → 404.
        ["v1", "me", "dashboard"] => {
            let (principal, _) = match two_tier_auth(state, headers) {
                Ok(p) => p,
                Err(e) => return err(e),
            };
            let repo = ME_DEFAULT_REPO;
            with_log(
                || {
                    state
                        .load_verified(repo)
                        .map_err(|e| hide_load_err(&principal, e))
                },
                |log| {
                    if !crate::authz::authorize_read(
                        &principal,
                        &crate::authz::project_repo_meta(log),
                    ) {
                        return err(EngineErr::not_found());
                    }
                    ok(&handlers::build_dashboard(log, repo))
                },
            )
        }
        ["v1", "me", "attention"] => {
            let (principal, _) = match two_tier_auth(state, headers) {
                Ok(p) => p,
                Err(e) => return err(e),
            };
            let repo = ME_DEFAULT_REPO;
            with_log(
                || {
                    state
                        .load_verified(repo)
                        .map_err(|e| hide_load_err(&principal, e))
                },
                |log| {
                    if !crate::authz::authorize_read(
                        &principal,
                        &crate::authz::project_repo_meta(log),
                    ) {
                        return err(EngineErr::not_found());
                    }
                    ok(&handlers::build_attention(log, repo))
                },
            )
        }
        // Admin control-plane: active engine-token sessions. Reads the in-process
        // token store (not the log), so it does not route through `dispatch_repo`.
        ["v1", "admin", "tokens"] => {
            let (principal, _) = match two_tier_auth(state, headers) {
                Ok(p) => p,
                Err(e) => return err(e),
            };
            // Scope to the caller's tenant (audit 2026-06-15 — no cross-tenant
            // session enumeration): the platform operator (dev/`orchestrator:`
            // principal) sees ALL sessions; a Clerk principal (`clerk:{org}:{user}`)
            // sees ONLY its own org; any unrecognized principal is fail-closed to
            // no org (empty list).
            let caller = principal.first().map(String::as_str).unwrap_or("");
            let scope: Option<&str> = if caller.starts_with("orchestrator:") {
                None
            } else if let Some(rest) = caller.strip_prefix("clerk:") {
                Some(rest.split(':').next().unwrap_or(""))
            } else {
                Some("")
            };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            ok(&handlers::build_admin_tokens(
                &state.token_store.list_for_org(scope),
                now,
            ))
        }
        _ => err(EngineErr::not_found()),
    }
}

/// The default repo context for identity-scoped (`/v1/me/*`) reads until the P2
/// Clerk identity seam resolves a per-principal repo set: the launch repo.
const ME_DEFAULT_REPO: &str = "hugit";

/// Extract a percent-decoded query-param value (`+` → space) from a raw query
/// string (the part after `?`). Returns `""` when absent — the read handlers
/// treat an empty query as an honest no-op.
fn query_param(query: &str, key: &str) -> String {
    let prefix = format!("{key}=");
    let Some(raw) = query.split('&').find_map(|kv| kv.strip_prefix(&prefix)) else {
        return String::new();
    };
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                let hi = (bytes[i + 1] as char).to_digit(16);
                let lo = (bytes[i + 2] as char).to_digit(16);
                match (hi, lo) {
                    (Some(h), Some(l)) => {
                        out.push((h * 16 + l) as u8);
                        i += 3;
                    }
                    _ => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The full entry (reads + writes). GET delegates to [`route`]; POST is a mutating
/// verb routed through the write-door. Socket-free (body passed in).
#[must_use]
pub fn route_with_body(
    state: &AppState,
    method: &Method,
    url: &str,
    headers: &[Header],
    body: &[u8],
) -> (u16, String) {
    if method == &Method::Post {
        return route_write(state, url, headers, body);
    }
    route(state, method, url, headers)
}

/// Read a request body, capped at the door's `MAX_BODY_BYTES` (+1 byte so the
/// door detects + rejects an over-cap body without buffering the whole thing).
fn read_body_capped(request: &mut Request) -> Vec<u8> {
    let mut buf = Vec::new();
    let _ = request
        .as_reader()
        .take(writes::MAX_BODY_BYTES as u64 + 1)
        .read_to_end(&mut buf);
    buf
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Case-insensitive header lookup.
fn header_val(headers: &[Header], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
}

/// The v1 acting principal. Real authenticated identity (Clerk/RFC-8693) is the
/// disclosed P2 seam; the dev-token path acts as the configured orchestrator.
fn dev_principal() -> Vec<String> {
    vec!["orchestrator:hugit".to_string()]
}

/// `Accepted` → a 200 JSON body with the spec §3 top-level `"accepted": true`
/// alongside the typed fields (the client treats the 2xx as truth; the key is
/// additive). A serialize fault is an internal 503.
fn ok_accepted(a: &Accepted) -> (u16, String) {
    match serde_json::to_value(a) {
        Ok(mut v) => {
            if let Some(obj) = v.as_object_mut() {
                obj.insert("accepted".to_string(), serde_json::Value::Bool(true));
            }
            (200, v.to_string())
        }
        Err(e) => err(EngineErr::unavailable(format!("serialize: {e}"))),
    }
}

/// POST routing: Bearer auth (BEFORE any work), then the mutating-verb dispatch.
fn route_write(state: &AppState, url: &str, headers: &[Header], body: &[u8]) -> (u16, String) {
    let path = url.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segs.as_slice() {
        // POST /v1/token — the ONLY no-Bearer route (it IS the auth-issuance
        // endpoint; the handler forwards the Clerk JWT to CoreLink's
        // `/v1/session/exchange`). Without a configured exchange client the
        // endpoint does not exist → 404 (we do not disclose its presence in
        // dev-only mode).
        ["v1", "token"] => match &state.exchange {
            Some(c) => crate::token::handle_token_exchange(c, &state.token_store, body),
            None => err(EngineErr::not_found()),
        },
        ["v1", "repos", repo, tail @ ..] => {
            // Two-tier Bearer auth: a Clerk-minted engine token, else the dev token.
            let (principal, fresh_auth) = match two_tier_auth(state, headers) {
                Ok(pair) => pair,
                Err(e) => return err(e),
            };
            dispatch_repo_write(state, repo, tail, headers, body, principal, fresh_auth)
        }
        _ => err(EngineErr::not_found()),
    }
}

/// Two-tier Bearer auth (Wave-5b token seam):
///   Tier 1 — `token_store.lookup(raw)`: a real Clerk-minted engine token →
///            `(clerk principal, fresh_auth from the minted record)`.
///   Tier 2 — the dev-token fallback (constant-time, mirrors `check_bearer`) →
///            `(dev principal, fresh_auth=false)`.
/// An in-store-but-EXPIRED engine token is `TOKEN_EXPIRED` (client renews + retries);
/// anything else is `TOKEN_INVALID`. Returns `(principal_chain, fresh_auth)`.
fn two_tier_auth(state: &AppState, headers: &[Header]) -> Result<(Vec<String>, bool), EngineErr> {
    let raw = header_val(headers, "Authorization")
        .and_then(|v| v.strip_prefix("Bearer ").map(str::to_string));
    let raw = match raw {
        Some(r) => r,
        None => return Err(EngineErr::token_invalid()),
    };

    // Tier 1: the engine-token store (a Clerk exchange minted this).
    match state.token_store.lookup(&raw) {
        crate::token::LookupResult::Ok(rec) => {
            return Ok((
                vec![format!("clerk:{}:{}", rec.org, rec.user)],
                rec.fresh_auth,
            ));
        }
        crate::token::LookupResult::Expired => return Err(EngineErr::token_expired()),
        crate::token::LookupResult::Invalid => {} // fall through to the dev token
    }

    // Tier 2: dev-token fallback (same constant-time SHA-256+XOR as check_bearer).
    if crate::auth::tokens_match(raw.as_bytes(), state.dev_token.as_bytes()) {
        return Ok((dev_principal(), false));
    }
    Err(EngineErr::token_invalid())
}

/// Dispatch an authenticated `/v1/repos/{repo}/<tail...>` POST through the
/// write-door (`with_write`): idempotency + step-up + persist + atomic ledger.
fn dispatch_repo_write(
    state: &AppState,
    repo: &str,
    tail: &[&str],
    headers: &[Header],
    body: &[u8],
    principal: Vec<String>,
    fresh_auth: bool,
) -> (u16, String) {
    // Idempotency-Key cap (DoS / R2 log-bloat guard): reject keys that exceed the
    // max before anything is persisted. 256 bytes is large enough for any UUID or
    // structured key a well-behaved client sends; over-cap is a 400.
    const MAX_IDEM_KEY_BYTES: usize = 256;
    let idem_raw = header_val(headers, "Idempotency-Key").unwrap_or_default();
    if idem_raw.len() > MAX_IDEM_KEY_BYTES {
        return err(EngineErr::invalid_request(
            "Idempotency-Key excede o limite de 256 bytes",
        ));
    }
    let idem = idem_raw;
    // Step-up is satisfied by a fresh Clerk session (Tier-1 `fresh_auth`, derived
    // from the signed `auth_time`) OR — for the DEV path ONLY — the explicit
    // `X-Step-Up` header. The header is honored solely for the single trusted dev
    // orchestrator credential; a Clerk principal can NEVER self-assert step-up via
    // a header (its step-up MUST come from a freshly re-authenticated session), so
    // the header gate is future-proofed against the P2 Clerk seam (audit hardening).
    let is_dev_principal = principal.first().map(String::as_str) == Some("orchestrator:hugit");
    let step_up_header = is_dev_principal
        && header_val(headers, "X-Step-Up")
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);
    let step_up = fresh_auth || step_up_header;
    let p = principal;
    let at = now_ms();
    let sink: &dyn LogSink = state;
    // The URL tail (e.g. `prs/1/land`) is the idempotency RESOURCE — keyed in the
    // ledger so a key reused across resources never replays the wrong outcome (audit P0).
    let resource = tail.join("/");

    // Parse the body as `$T` or short-circuit to 400 INVALID_REQUEST.
    macro_rules! parse {
        ($T:ty) => {
            match serde_json::from_slice::<$T>(body) {
                Ok(v) => v,
                Err(e) => return err(EngineErr::invalid_request(format!("corpo inválido: {e}"))),
            }
        };
    }

    let result: Result<Accepted, EngineErr> = match tail {
        ["prs", n, "land"] => match n.parse::<u32>() {
            Ok(pr) => {
                let req = parse!(wr::LandReq);
                with_write(
                    sink,
                    repo,
                    "land",
                    &resource,
                    &idem,
                    body,
                    step_up,
                    p,
                    at,
                    |log, p, at| verbs::write_land::write_land(log, repo, pr, &req, p, at),
                )
            }
            Err(_) => Err(EngineErr::not_found()),
        },
        ["prs", n, "verdict"] => match n.parse::<u32>() {
            Ok(pr) => {
                let req = parse!(wr::VerdictReq);
                with_write(
                    sink,
                    repo,
                    "verdict",
                    &resource,
                    &idem,
                    body,
                    step_up,
                    p,
                    at,
                    |log, p, at| verbs::write_verdict::write_verdict(log, repo, pr, &req, p, at),
                )
            }
            Err(_) => Err(EngineErr::not_found()),
        },
        ["prs", n, "comments"] => match n.parse::<u32>() {
            Ok(pr) => {
                let req = parse!(wr::CommentReq);
                with_write(
                    sink,
                    repo,
                    "comment",
                    &resource,
                    &idem,
                    body,
                    step_up,
                    p,
                    at,
                    |log, p, at| verbs::write_comment::write_comment(log, repo, pr, &req, p, at),
                )
            }
            Err(_) => Err(EngineErr::not_found()),
        },
        ["dispatch"] => {
            let req = parse!(wr::DispatchReq);
            with_write(
                sink,
                repo,
                "dispatch",
                &resource,
                &idem,
                body,
                step_up,
                p,
                at,
                |log, p, at| verbs::write_dispatch::write_dispatch(log, repo, &req, p, at),
            )
        }
        ["issues", n, "transition"] => match n.parse::<u32>() {
            Ok(num) => {
                let req = parse!(wr::IssueTransitionReq);
                with_write(
                    sink,
                    repo,
                    "issue_transition",
                    &resource,
                    &idem,
                    body,
                    step_up,
                    p,
                    at,
                    |log, p, at| {
                        verbs::write_issue_transition::write_issue_transition(
                            log, repo, num, &req, p, at,
                        )
                    },
                )
            }
            Err(_) => Err(EngineErr::not_found()),
        },
        ["policy"] => {
            let req = parse!(wr::PolicyReq);
            with_write(
                sink,
                repo,
                "policy",
                &resource,
                &idem,
                body,
                step_up,
                p,
                at,
                |log, p, at| verbs::write_policy::write_policy(log, repo, &req, p, at),
            )
        }
        ["erasure", id, "decide"] => {
            let id = (*id).to_string();
            let req = parse!(wr::ErasureDecideReq);
            with_write(
                sink,
                repo,
                "erasure",
                &resource,
                &idem,
                body,
                step_up,
                p,
                at,
                |log, p, at| {
                    verbs::write_erasure_decide::write_erasure_decide(log, repo, &id, &req, p, at)
                },
            )
        }
        ["edit", mid @ .., "propose"] if !mid.is_empty() => {
            let path = mid.join("/");
            let req = parse!(wr::EditProposeReq);
            with_write(
                sink,
                repo,
                "edit_propose",
                &resource,
                &idem,
                body,
                step_up,
                p,
                at,
                |log, p, at| {
                    verbs::write_edit_propose::write_edit_propose(log, repo, &path, &req, p, at)
                },
            )
        }
        ["undo"] => {
            let req = parse!(wr::UndoReq);
            with_write(
                sink,
                repo,
                "undo",
                &resource,
                &idem,
                body,
                step_up,
                p,
                at,
                |log, p, at| verbs::write_undo::write_undo(log, repo, &req, p, at),
            )
        }
        _ => Err(EngineErr::not_found()),
    };
    match result {
        Ok(a) => ok_accepted(&a),
        Err(e) => err(e),
    }
}

/// Dispatch an authenticated `/v1/repos/{repo}/<tail...>` read. `query` is the
/// raw query string (after `?`), threaded for the param-driven reads (search).
/// Dispatch a repo read over an ALREADY loaded + verified + tenant-gated `log`
/// (the caller owns load → 404/503 → authz gate). Every arm projects over `log`;
/// by-id reads return 404 on an absent resource (no existence leak).
fn dispatch_repo(
    repo: &str,
    tail: &[&str],
    query: &str,
    log: &hugit_refstore::EventLog,
    principal: &[String],
    git_source: Option<&std::sync::Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    root_tree: Option<&gix_hash::ObjectId>,
) -> (u16, String) {
    match tail {
        ["home"] => ok(&handlers::build_home(log, repo)),
        ["new-pr"] => ok(&handlers::build_new_pr(log, repo)),
        ["knowledge"] => ok(&handlers::build_knowledge(log, repo)),
        ["compare", base, head] => ok(&handlers::build_compare(log, repo, base, head)),
        ["landing"] => ok(&handlers::build_landing(log, repo)),
        ["checks"] => ok(&handlers::build_checks(log, repo)),
        ["commits"] => ok(&handlers::build_commits(log, repo)),
        // Phase-2 collection reads (real engine backbone).
        ["chrome"] => ok(&handlers::build_repo_chrome(log, repo)),
        ["branches"] => ok(&handlers::build_branches(log, repo)),
        ["insights"] => ok(&handlers::build_insights(log, repo)),
        // Wave-3 collection reads (write-backed: issues←issue.transition,
        // security←policy.set/erasure.decided).
        ["issues"] => ok(&handlers::build_issues(log, repo)),
        ["security"] => ok(&handlers::build_security(log, repo)),
        // Wave-4 collection reads (real log-backed): settings←policy.set,
        // releases←pr.landed, search←q over records, viewer-can←D14 authz matrix.
        ["settings"] => ok(&handlers::build_repo_settings(log, repo)),
        ["releases"] => ok(&handlers::build_releases(log, repo)),
        ["search"] => {
            let mut q = query_param(query, "q");
            // SECURITY: cap the search query before it reaches the log scan.
            // An unbounded `q` that is lowercased per record is a CPU/memory DoS.
            // 1 024 bytes is ample for any real search term.
            q.truncate(1024);
            ok(&handlers::build_search(log, repo, &q))
        }
        // viewer-can mirrors the REAL per-caller write gate (`authorize_write`,
        // ownership) for THIS repo (audit 2026-06-16) — using the real caller +
        // the repo's meta, not a hardcoded operator stub + the (non-enforcing)
        // class matrix. The log was already load-verified + read-gated above.
        ["viewer-can"] => {
            let meta = crate::authz::project_repo_meta(log);
            ok(&handlers::build_viewer_can(principal, &meta))
        }
        ["prs", n] => match n.parse::<u32>() {
            Ok(num) => match handlers::build_pr_detail(log, repo, num) {
                Some(vm) => ok(&vm),
                None => err(EngineErr::not_found()), // get_opt: absent PR → 404, no leak
            },
            // A non-numeric PR id is not a resource that exists → 404 (no leak).
            Err(_) => err(EngineErr::not_found()),
        },
        // Wave-3 by-PR read: the review panel (write-backed: verdict.recorded + pr.comment).
        ["prs", n, "review"] => match n.parse::<u32>() {
            Ok(num) => match handlers::build_review(log, repo, num) {
                Some(vm) => ok(&vm),
                None => err(EngineErr::not_found()),
            },
            Err(_) => err(EngineErr::not_found()),
        },
        // Phase-2 by-id reads — absent resource → 404, no existence leak.
        ["intents", id] => match handlers::build_intent_detail(log, repo, id) {
            Some(vm) => ok(&vm),
            None => err(EngineErr::not_found()),
        },
        ["commit", sha] => match handlers::build_commit_detail(log, repo, sha) {
            Some(vm) => ok(&vm),
            None => err(EngineErr::not_found()),
        },
        ["campaigns", name] => match handlers::build_campaign(log, repo, name) {
            Some(vm) => ok(&vm),
            None => err(EngineErr::not_found()),
        },
        // Admin control-plane reads (operator area) — pure projections over the
        // verified log, no P2 infra. Audit timeline, erasure governance history,
        // the one-call overview.
        //
        // SECURITY (audit 2026-06-20): these are gated on OPERATOR status, NOT on
        // read-visibility. `authorize_read` (run by the caller above) opens NORMAL
        // repo screens to anonymous/any-tenant callers when the repo is `public`
        // (the documented anonymous-`git clone` gate) — but the control plane
        // (authz/denial timeline, principal chains, record hashes, erasure
        // governance, admin overview) must NEVER ride that gate. A non-operator
        // gets the uniform 404 (same as a denied read — no existence oracle)
        // REGARDLESS of the repo's visibility.
        ["audit" | "erasure", ..] | ["admin", ..] if !crate::authz::is_operator(principal) => {
            err(EngineErr::not_found())
        }
        ["audit"] => {
            let since = query_param(query, "since").parse::<u64>().unwrap_or(0);
            let limit = query_param(query, "limit").parse::<usize>().unwrap_or(0);
            let kind = query_param(query, "kind");
            let principal = query_param(query, "principal");
            let kind_filter = (!kind.is_empty()).then_some(kind);
            let principal_filter = (!principal.is_empty()).then_some(principal);
            ok(&handlers::build_audit(
                log,
                repo,
                since,
                limit,
                kind_filter.as_deref(),
                principal_filter.as_deref(),
            ))
        }
        ["erasure"] => ok(&handlers::build_erasure(log, repo)),
        ["admin", "overview"] => ok(&handlers::build_admin_overview(log, repo)),
        // File-content reads (PS-18 reversal): walk the git tree to the blob and
        // serve its scrubbed content. `{*path}` is the multi-segment tail (joined
        // with '/'); an EMPTY tail is not a file → fall through to 404. A path that
        // does not resolve (or no git content seam wired) → 404, no content oracle.
        // NOTE: distinct from the POST `edit/.../propose` WRITE route, handled by
        // `dispatch_repo_write` — this is the GET read of the file to edit.
        ["blob", rest @ ..] if !rest.is_empty() => {
            let path = rest.join("/");
            match handlers::build_blob(log, repo, &path, git_source, root_tree) {
                Some(vm) => ok(&vm),
                None => err(EngineErr::not_found()),
            }
        }
        ["edit", rest @ ..] if !rest.is_empty() => {
            let path = rest.join("/");
            match handlers::build_edit(log, repo, &path, git_source, root_tree) {
                Some(vm) => ok(&vm),
                None => err(EngineErr::not_found()),
            }
        }
        _ => err(EngineErr::not_found()),
    }
}

/// Run `f` over a freshly verified log, mapping the load error to its envelope.
/// Still used by the identity-scoped `/v1/me/*` reads (fixed launch repo, the
/// caller's own context — NOT an arbitrary-slug repo read, so the per-tenant
/// repo-gate does not apply there).
fn with_log(
    load: impl FnOnce() -> Result<hugit_refstore::EventLog, EngineErr>,
    f: impl FnOnce(&hugit_refstore::EventLog) -> (u16, String),
) -> (u16, String) {
    match load() {
        Ok(log) => f(&log),
        Err(e) => err(e),
    }
}

/// Serialize a view-model to a 200 JSON body (a serialize fault is an internal
/// 503 — never a partial/garbage body).
fn ok<T: serde::Serialize>(vm: &T) -> (u16, String) {
    match serde_json::to_string(vm) {
        Ok(body) => (200, body),
        Err(e) => err(EngineErr::unavailable(format!("serialize: {e}"))),
    }
}

/// An `EngineErr` → its `(status, {code,reason})` pair.
fn err(e: EngineErr) -> (u16, String) {
    (e.status, e.to_body())
}

#[cfg(test)]
mod search_q_tests {
    use super::*;

    /// query_param basic decode.
    #[test]
    fn query_param_basic() {
        assert_eq!(query_param("q=hello+world&limit=10", "q"), "hello world");
        assert_eq!(query_param("q=a%20b", "q"), "a b");
        assert_eq!(query_param("limit=5", "q"), "");
    }

    /// A `q` value longer than 1 024 chars is silently truncated to 1 024 chars
    /// (the cap applied in the route before passing to `build_search`). The
    /// truncation happens in the route handler, not in `query_param` — so this
    /// test replicates the route logic directly.
    #[test]
    fn search_q_over_1024_bytes_is_truncated() {
        // Simulate the route logic: decode then truncate.
        let long_q = "a".repeat(2048);
        let query = format!("q={long_q}");
        let mut q = query_param(&query, "q");
        q.truncate(1024);
        assert_eq!(q.len(), 1024, "truncated q must be exactly 1024 bytes");
        // All chars are ASCII 'a', so truncating bytes == truncating chars here.
        assert!(q.chars().all(|c| c == 'a'));
    }
}
