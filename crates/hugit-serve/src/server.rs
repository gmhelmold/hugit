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
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tiny_http::{Header, Method, Request, Response, Server};

use crate::error::EngineErr;
use crate::handlers;
use crate::metrics::{Metrics, RouteClass, ShedGate};
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
///
/// W-SHED-METRICS adds two AVAILABILITY-safe things to this single-threaded serial
/// loop: (A) a fail-safe graceful load-shed ([`ShedGate`] — sheds a fast 503 only
/// when the loop has been CONTINUOUSLY saturated past a window; any idle gap resets
/// it, so normal/serviceable load never false-503s) and (B) cheap aggregate
/// [`Metrics`] + a per-request structured log line. Neither adds meaningful
/// per-request latency (atomics + one uncontended lock on the single thread).
pub fn serve_on(state: AppState, server: Server) -> std::io::Result<()> {
    let metrics = Metrics::new();
    let mut shed_gate = ShedGate::from_env();
    // The single wall-clock deadline that bounds any ONE potentially-socket-blocking
    // op (reading a POST body, writing a large response) so a slow/dribbling client
    // can never wedge this single-threaded accept loop INDEFINITELY. Read once here
    // (not per-request) — env-tunable, clamped. See [`io_deadline`].
    let io_budget = io_deadline();
    let loop_start = Instant::now();
    eprintln!(
        "hugit-serve: accept loop up (load-shed={}, io_deadline={}s)",
        if shed_gate.enabled() { "on" } else { "off" },
        io_budget.as_secs()
    );

    let mut incoming = server.incoming_requests();
    loop {
        // Time the BLOCK waiting for the next request: a long wait means the queue
        // was empty (the engine is keeping up) — the load-shed idle signal.
        let wait_start = Instant::now();
        let request = match incoming.next() {
            Some(r) => r,
            None => break, // server closed → end the loop (matches the prior for-loop)
        };
        let waited_ms = wait_start.elapsed().as_millis() as u64;
        let now_ms = loop_start.elapsed().as_millis() as u64;

        let req_id = metrics.next_request_id();
        metrics.set_in_flight(1);
        let started = Instant::now();
        let method = request.method().clone();
        let url = request.url().to_string();
        let class = classify_route(&method, &url);

        // ---- Part A: graceful load-shed (fail-safe) --------------------------
        // `should_shed` is pure saturating arithmetic (CANNOT panic); we STILL wrap
        // it in `catch_unwind` so a hypothetical bug degrades to `false` (SERVE) —
        // never a false shed, never a crash. Liveness (`/readyz`) + observability
        // (`/metrics`) are exempt so they stay answerable DURING an overload.
        let saturated = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            shed_gate.should_shed(waited_ms, now_ms)
        }))
        .unwrap_or(false);
        if saturated && !class.shed_exempt() {
            metrics.record_shed();
            respond_shed_503(request);
            let dur = started.elapsed().as_millis() as u64;
            metrics.record(class, dur);
            log_request(req_id, class, 503, dur);
            metrics.set_in_flight(0);
            continue;
        }

        let headers = request.headers().to_vec();
        // Read the body ONLY for mutating methods (reads ignore it). Bounded read:
        // at most MAX_BODY_BYTES+1 so the door's size cap rejects an oversize body
        // without us buffering it all.
        //
        // AVAILABILITY: this read touches the socket on THIS single thread, BEFORE
        // auth and BEFORE the `catch_unwind` handler-isolation below. A client that
        // dribbles the body (1 byte / minute) or a stalled TCP window would block the
        // read INDEFINITELY → starve `/readyz` + every other request → outage. tiny_http
        // 0.12 exposes no socket read timeout (neither `ServerConfig` nor `Request`
        // reaches the `TcpStream`), so we bound the read with a wall-clock deadline on a
        // worker thread instead: on timeout we DROP this connection and return to accept —
        // never an indefinite wedge. GET/HEAD carry no body, so reads keep the
        // zero-overhead inline path.
        let (request, body) = if method == Method::Post {
            match read_body_bounded(request, io_budget) {
                Some(pair) => pair,
                None => {
                    // Slow-loris body dribble (or thread exhaustion under a flood): the
                    // connection is abandoned, the accept loop is FREED. Record + move on.
                    eprintln!(
                        "hugit-serve: POST body read exceeded the {}s I/O deadline \
                         (or a worker could not be spawned) — connection dropped, \
                         accept loop freed",
                        io_budget.as_secs()
                    );
                    let dur = started.elapsed().as_millis() as u64;
                    metrics.record(class, dur);
                    log_request(req_id, class, 0, dur);
                    metrics.set_in_flight(0);
                    continue;
                }
            }
        } else {
            (request, Vec::new())
        };

        // GET /metrics — UNAUTHENTICATED aggregate counters (Part B). Handled here
        // (like SSE/git below) because it renders from the loop-owned `Metrics`. It
        // exposes NO tenant data: closed-vocabulary route-class labels + integers
        // only (never a repo slug / principal / id / path).
        if method == Method::Get && is_metrics_path(&url) {
            respond_metrics(request, metrics.render_json());
            let dur = started.elapsed().as_millis() as u64;
            metrics.record(class, dur);
            log_request(req_id, class, 200, dur);
            metrics.set_in_flight(0);
            continue;
        }

        // SSE replay: GET /v1/repos/{repo}/events?since=<seq>. Handled BEFORE the
        // standard (status, String) path because it needs a different Content-Type
        // and a Vec<u8> body. `respond_sse` consumes `request` in every branch.
        // PANIC ISOLATION: wrapped like `route_with_body` below — a panic here must
        // drop only THIS request, never unwind the single-threaded accept loop (one
        // panic would otherwise crash the whole engine; on `max_instances:1` that is
        // a full outage / crash-loop). On panic `request` is already consumed, so the
        // client just gets no response; the server survives. The sub-handler owns its
        // own status, so the log records status=0 ("handled internally").
        if method == Method::Get && is_events_path(&url) {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                respond_sse(&state, &url, &headers, request, io_budget);
            }))
            .is_err()
            {
                eprintln!("hugit-serve: SSE handler panicked — request dropped, server survives");
            }
            let dur = started.elapsed().as_millis() as u64;
            metrics.record(class, dur);
            log_request(req_id, class, 0, dur);
            metrics.set_in_flight(0);
            continue;
        }
        // Git smart-HTTP (clone/fetch AND push/receive-pack). Handled BEFORE the
        // standard (status,String) path because a packfile is a BINARY Vec<u8> body
        // with a git-specific Content-Type — it cannot ride `route_with_body`. The
        // POST body was already read above (capped). `respond_git` consumes `request`
        // in every branch. Same panic isolation as above (the receive-pack path
        // processes attacker-controlled pack bytes — a panic must not crash the loop).
        if crate::git::is_git_path(&url) {
            if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                crate::git::respond_git(&state, &method, &url, &body, request);
            }))
            .is_err()
            {
                eprintln!("hugit-serve: git handler panicked — request dropped, server survives");
            }
            let dur = started.elapsed().as_millis() as u64;
            metrics.record(class, dur);
            log_request(req_id, class, 0, dur);
            metrics.set_in_flight(0);
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
        let body_len = body.len();
        let response = Response::from_string(body)
            .with_status_code(status)
            .with_header(json_content_type());
        // Mark authenticated / private responses uncacheable (a public route —
        // `/readyz`, `/v1/me/login` — stays cacheable). SSE is handled separately in
        // `respond_sse` (always private).
        let response = if response_is_private(&method, &url) {
            response.with_header(cache_control_private())
        } else {
            response
        };
        // Write the response under the same wall-clock bound as the body read: a small
        // reply is sent INLINE (byte-identical to before — it fits the kernel send buffer
        // and cannot block), a LARGE one is offloaded to a worker so a zero-window /
        // slow-drain client cannot wedge the loop indefinitely. A broken pipe (client
        // hung up) is expected + silent; other faults are logged inside `respond_inline`.
        respond_bounded(request, response, body_len, io_budget, "response");
        let dur = started.elapsed().as_millis() as u64;
        metrics.record(class, dur);
        log_request(req_id, class, status, dur);
        metrics.set_in_flight(0);
    }
    Ok(())
}

/// Classify a request into a fixed [`RouteClass`] for per-route metrics. Uses ONLY
/// the method + coarse path SHAPE — never the concrete repo slug / id / query — so a
/// metric label can never carry tenant data.
fn classify_route(method: &Method, url: &str) -> RouteClass {
    // Git (upload-pack GET + receive-pack POST) is matched by path shape first.
    if crate::git::is_git_path(url) {
        return RouteClass::Git;
    }
    let path = url.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    if method == &Method::Post {
        return match segs.as_slice() {
            ["v1", "token"] => RouteClass::Token,
            _ => RouteClass::Write,
        };
    }
    match segs.as_slice() {
        ["readyz"] => RouteClass::Readyz,
        ["metrics"] => RouteClass::Metrics,
        ["v1", "me", "login"] => RouteClass::Login,
        ["v1", "repos", _, "events"] => RouteClass::Sse,
        ["v1", "repos", ..] => RouteClass::RepoRead,
        ["v1", "me", ..] | ["v1", "orgs", ..] => RouteClass::MeRead,
        _ => RouteClass::Other,
    }
}

/// Whether `url`'s path is exactly `/metrics` (query ignored).
fn is_metrics_path(url: &str) -> bool {
    let path = url.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    segs.as_slice() == ["metrics"]
}

/// A per-request structured log line: request-id + route class + status + duration.
/// `status == 0` means the sub-handler (SSE/git) owns + already sent its own status.
fn log_request(req_id: u64, class: RouteClass, status: u16, dur_ms: u64) {
    eprintln!(
        "hugit-serve req id={req_id} route={} status={status} duration_ms={dur_ms}",
        class.label()
    );
}

/// Respond with a fast `503 Service Unavailable` + `Retry-After` (the load-shed).
/// The body is the standard `{code, reason}` envelope; marked private/no-store so
/// no intermediary caches the transient shed.
fn respond_shed_503(request: Request) {
    let body = EngineErr::unavailable("overloaded — retry shortly").to_body();
    let resp = Response::from_string(body)
        .with_status_code(503)
        .with_header(json_content_type())
        .with_header(retry_after_header())
        .with_header(cache_control_private());
    if let Err(e) = request.respond(resp)
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        eprintln!("hugit-serve: shed respond error: {e}");
    }
}

/// Respond with the `/metrics` JSON body (200). Marked private/no-store so no
/// intermediary caches a point-in-time snapshot.
fn respond_metrics(request: Request, body: String) {
    let resp = Response::from_string(body)
        .with_status_code(200)
        .with_header(json_content_type())
        .with_header(cache_control_private());
    if let Err(e) = request.respond(resp)
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        eprintln!("hugit-serve: metrics respond error: {e}");
    }
}

/// The `Retry-After: 1` header (seconds) for a load-shed 503 — built once, cloned.
fn retry_after_header() -> Header {
    static RA: OnceLock<Header> = OnceLock::new();
    RA.get_or_init(|| {
        Header::from_bytes(&b"Retry-After"[..], &b"1"[..])
            .expect("static retry-after header is valid")
    })
    .clone()
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

/// The `Cache-Control: private, no-store` header — built once, cloned per use.
///
/// Attached to every AUTHENTICATED `/v1` response and UNCONDITIONALLY to every
/// private-repo / SSE response (githugr seam #18): a response gated on a caller's
/// identity must never be cached by ANY intermediary (browser, CDN, the www proxy).
/// `private` forbids shared-cache storage; `no-store` forbids storage entirely. A
/// PUBLIC anonymous response (`/readyz`, `/v1/me/login`) is intentionally NOT marked
/// — it carries identity-independent static content and may stay cacheable.
fn cache_control_private() -> Header {
    static CC: OnceLock<Header> = OnceLock::new();
    CC.get_or_init(|| {
        Header::from_bytes(&b"Cache-Control"[..], &b"private, no-store"[..])
            .expect("static cache-control header is valid")
    })
    .clone()
}

/// Whether a response must carry `Cache-Control: private, no-store`. TRUE for every
/// route that passes through [`two_tier_auth`] (authenticated) — i.e. EVERYTHING
/// except the two PUBLIC pre-auth routes (`GET /readyz`, `GET /v1/me/login`), which
/// serve identity-independent static content and may stay cacheable. Classifying by
/// route keeps the header decision in lock-step with the auth gate and is FAIL-CLOSED
/// by default: a response is left cacheable ONLY when it is explicitly one of the two
/// public routes; anything else (incl. auth failures, unknown paths, and the SSE
/// stream — which is marked directly in `respond_sse`) is treated as private.
fn response_is_private(method: &Method, url: &str) -> bool {
    let path = url.split('?').next().unwrap_or("");
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let is_public_route = method == &Method::Get
        && (segs.as_slice() == ["readyz"] || segs.as_slice() == ["v1", "me", "login"]);
    !is_public_route
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
fn respond_sse(
    state: &AppState,
    url: &str,
    headers: &[Header],
    request: Request,
    io_budget: Duration,
) {
    // A small helper to send a JSON error envelope and return. The events path is
    // ALWAYS authenticated (two_tier_auth runs first), so every response on it —
    // including these error envelopes — is private + uncacheable (githugr seam #18).
    fn send_json(request: Request, status: u16, body: String) {
        let resp = Response::from_string(body)
            .with_status_code(status)
            .with_header(json_content_type())
            .with_header(cache_control_private());
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
    let body_len = bytes.len();
    // UNCONDITIONALLY private: a private-repo event stream must never be cached by
    // any intermediary (githugr seam #18). The stream is authed + tenant-gated above.
    let resp = Response::from_data(bytes)
        .with_status_code(200)
        .with_header(sse_content_type())
        .with_header(cache_control_private());
    // A long replay stream can be large; bound the write so a stalled reader cannot
    // wedge the accept loop (small streams stay on the inline path).
    respond_bounded(request, resp, body_len, io_budget, "sse");
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
    // `git_serving` reflects whether `HUGIT_SERVE_GIT_DIR` (or the CAS source)
    // is wired at boot — a missing true here means clone/fetch are not live.
    // No repo content is leaked; it is a capability flag only.
    if method == &Method::Get && segs == ["readyz"] {
        // `git_repos` = how many repos have a git content seam loaded (the forge
        // serves many); `git_serving` stays for backward-compat (true iff ≥1). No
        // repo content/names are leaked — capability counts only.
        let git_repos = state.git_serving_count();
        let git_serving = git_repos > 0;
        // The deployed engine version (the deploy tag, set as `HUGIT_SERVE_VERSION`
        // in the container env). Lets a consumer self-confirm a cutover reached the
        // serving instance without pinging the operator — `/readyz` was otherwise
        // version-blind, which masked a stale-instance/decode mismatch during a
        // rollout. "dev" when unset (local/test). Sanitized to a JSON-safe charset
        // (a controlled tag, but never trust an env value into a hand-built JSON).
        let version: String = std::env::var("HUGIT_SERVE_VERSION")
            .unwrap_or_else(|_| "dev".to_string())
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | ':'))
            .take(128)
            .collect();
        let body = format!(
            r#"{{"ready":true,"git_serving":{git_serving},"git_repos":{git_repos},"version":"{version}"}}"#
        );
        return (200, body);
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
            // repo read): a denied
            // PRIVATE repo is a 404, identical to a non-existent one (no existence
            // oracle). Operator (dev/orchestrator) bypass keeps single-tenant dev +
            // the launch repo working until owner_tenant is assigned.
            let meta = crate::authz::project_repo_meta(&log);
            if !crate::authz::authorize_read(&principal, &meta) {
                return err(EngineErr::not_found());
            }
            // The query (stripped above) is re-passed for the param-driven reads.
            // The git content seam (`blob`/`edit`) is resolved PER-REPO from the
            // map; a repo with no git seam loaded → `None` → those reads 404
            // honestly (identical to a not-wired engine, no oracle).
            let repo_git = state.repo_state(repo);
            // The per-repo git content seam (object source + HEAD root-tree + HEAD
            // commit), bundled so the dispatcher takes one param not three. The HEAD
            // commit (default-branch tip) is the start of the blob "Histórico"
            // per-path history walk; `None` for a refless repo.
            let git = repo_git.map(|r| RepoGit {
                source: &r.git_source,
                root_tree: &r.git_root_tree,
                head_commit: r.head_commit(),
                refs: r.git_refs.snapshot(),
            });
            dispatch_repo(
                repo,
                tail,
                url.split('?').nth(1).unwrap_or(""),
                &log,
                &principal,
                git.as_ref(),
            )
        }
        // Identity-scoped reads (/v1/me/*): no {repo} path param — they resolve to
        // the CALLER's OWN repos (W-METENANT). `AppState::me_repo_logs` returns the
        // caller's authorized `(slug, verified-log)` set — the SAME read-authz
        // predicate the per-repo gate runs, so a foreign tenant's private repo is
        // simply ABSENT (no oracle). The builders aggregate ACROSS the caller's
        // repos; an operator sees all loaded repos; a caller who owns none → an
        // honest EMPTY view (never a default repo's data). This CLOSES the prior
        // cross-principal exposure (every caller used to get the launch repo).
        ["v1", "me", "dashboard"] => {
            let (principal, _) = match two_tier_auth(state, headers) {
                Ok(p) => p,
                Err(e) => return err(e),
            };
            ok(&handlers::build_me_dashboard(
                &state.me_repo_logs(&principal),
            ))
        }
        ["v1", "me", "attention"] => {
            let (principal, _) = match two_tier_auth(state, headers) {
                Ok(p) => p,
                Err(e) => return err(e),
            };
            ok(&handlers::build_me_attention(
                &state.me_repo_logs(&principal),
            ))
        }
        // `GET /v1/orgs/{name}` — thin real org view. `name` is the path param
        // (the display header); the `repos` list is the CALLER's OWN authorized
        // repos (W-METENANT — same per-tenant index as `/v1/me/*`), never a
        // hardcoded default. Cross-tenant repo enumeration stays the P2 seam.
        ["v1", "orgs", org_name] => {
            let (principal, _) = match two_tier_auth(state, headers) {
                Ok(p) => p,
                Err(e) => return err(e),
            };
            ok(&handlers::build_me_org(
                org_name,
                &state.me_repo_logs(&principal),
            ))
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

/// A response body at or below this size is written INLINE on the accept loop: it
/// fits inside the kernel TCP send buffer, so `Request::respond` copies it in and
/// returns even if the peer never reads — it CANNOT wedge the loop. Only a body
/// LARGER than this can block the write on a zero-window / slow-drain client, so
/// only those are offloaded to the bounded worker (see [`respond_bounded`]). The
/// threshold is deliberately conservative (below any realistic `SO_SNDBUF` floor)
/// so the inline fast path stays provably non-blocking; every small reply
/// (`/readyz`, `/metrics`, an ordinary JSON view) keeps its pre-change behavior.
const RESPOND_INLINE_MAX: usize = 16 * 1024;

/// The wall-clock deadline that bounds ONE potentially-socket-blocking operation
/// (reading a POST body, writing a large response). GENEROUS by design: it must
/// NEVER fire for a legitimate client — only convert an otherwise-INDEFINITE loop
/// wedge (a dribbling / stalled peer) into a bounded one. Overridable via
/// `HUGIT_SERVE_IO_DEADLINE_SECS`, clamped to `[5, 3600]` s so a mis-set env can
/// neither false-drop a real slow request (floor) nor un-bound the wedge (ceiling).
///
/// Default 60 s: the deployed engine sits behind an edge (Cloudflare) that buffers
/// the client's request body and drains the response over a fast backbone, so the
/// origin sees I/O at backbone speed — 60 s is orders of magnitude above a real
/// request's transfer time (the 8 MiB body cap clears 60 s at just ~140 KB/s), while
/// a 1-byte/60 s slow-loris is caught long before it could ever complete a body.
fn io_deadline() -> Duration {
    let raw = std::env::var("HUGIT_SERVE_IO_DEADLINE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok());
    Duration::from_secs(clamp_io_deadline_secs(raw))
}

/// The pure clamp behind [`io_deadline`] (env-free, hermetically testable): an unset
/// / unparseable value → the 60 s default; any value is clamped to `[5, 3600]` s.
fn clamp_io_deadline_secs(raw: Option<u64>) -> u64 {
    const DEFAULT_SECS: u64 = 60;
    const FLOOR_SECS: u64 = 5;
    const CEIL_SECS: u64 = 3600;
    raw.unwrap_or(DEFAULT_SECS).clamp(FLOOR_SECS, CEIL_SECS)
}

/// Run `f` — a potentially socket-blocking operation — on a detached worker thread,
/// waiting at most `budget`. Returns `Some(v)` if it finished in time (the worker
/// joins, nothing leaks); `None` on timeout, on a worker panic, OR when the worker
/// could not be spawned. In EVERY `None` case the caller (the accept loop) is FREED
/// and any in-flight work is ABANDONED: a parked worker lingers on its stuck socket
/// until the OS unblocks its blocking syscall, then drops the `Request` it owns,
/// closing the peer. This can therefore neither crash nor wedge the loop — a panic
/// in `f` disconnects the channel, so `recv_timeout` returns `Err` → `None`.
///
/// tiny_http 0.12 exposes no per-connection socket timeout (the `TcpStream` is never
/// reachable from `Server`/`Request`), so this worker-thread + wall-clock deadline is
/// the available bound. Residual (flagged): a timed-out worker parks one thread per
/// malicious connection until its syscall unblocks — a bounded cost, NOT an
/// accept-loop wedge (and a spawn failure under that flood degrades to a shed `None`,
/// never an inline block).
fn run_bounded<T, F>(budget: Duration, f: F) -> Option<T>
where
    T: Send + 'static,
    F: FnOnce() -> T + Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::sync_channel::<T>(1);
    if std::thread::Builder::new()
        .name("hugit-io".into())
        .spawn(move || {
            // The receiver may already be gone (the caller timed out); discard on Err.
            let _ = tx.send(f());
        })
        .is_err()
    {
        // Thread exhaustion (an overload / slow-loris flood) — shed this operation
        // rather than fall back to an unbounded inline block. Fail-safe under load.
        return None;
    }
    rx.recv_timeout(budget).ok()
}

/// Read a POST body (capped) under `budget` without letting a slow/dribbling client
/// wedge the single-threaded accept loop. The blocking read runs on a worker via
/// [`run_bounded`]: `Some((request, body))` when it completes in time (the `Request`
/// is moved back intact, ready to respond); `None` on timeout — the loop drops the
/// connection and returns to accept.
fn read_body_bounded(request: Request, budget: Duration) -> Option<(Request, Vec<u8>)> {
    run_bounded(budget, move || {
        let mut request = request;
        let body = read_body_capped(&mut request);
        (request, body)
    })
}

/// Send `response`, logging any non-`BrokenPipe` fault. INLINE (no worker) — used
/// directly for small bodies and as the worker body for large ones.
fn respond_inline<R>(request: Request, response: Response<R>, what: &str)
where
    R: Read,
{
    if let Err(e) = request.respond(response)
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        eprintln!("hugit-serve: {what} respond error: {e}");
    }
}

/// Write `response` without letting a stalled / zero-window client wedge the accept
/// loop. A body `<= RESPOND_INLINE_MAX` is written INLINE (it fits the kernel send
/// buffer and cannot block — the fast path, unchanged). A LARGER body is offloaded to
/// a worker bounded by `budget`; the loop waits at most `budget` (so handler execution
/// stays SERIAL — the next request is not accepted until this write finishes or the
/// deadline fires) then moves on, abandoning a stuck write to its worker. `what` names
/// the site for the error log.
fn respond_bounded<R>(
    request: Request,
    response: Response<R>,
    body_len: usize,
    budget: Duration,
    what: &'static str,
) where
    R: Read + Send + 'static,
{
    if body_len <= RESPOND_INLINE_MAX {
        respond_inline(request, response, what);
        return;
    }
    if run_bounded(budget, move || respond_inline(request, response, what)).is_none() {
        eprintln!(
            "hugit-serve: {what} write exceeded the {}s I/O deadline — connection \
             abandoned, accept loop freed",
            budget.as_secs()
        );
    }
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
        // POST /v1/repos — self-service repo CREATE (W-PROVISION, v0 create-empty).
        // A NEW repo has no head to load/gate on, so it does NOT ride the
        // `dispatch_repo_write` → `with_write` door (which 404s an absent log +
        // gates on the loaded meta); the verb seeds the genesis atomically itself.
        // Auth is the SAME two-tier gate; the verb then derives `owner_tenant` from
        // the principal and refuses operator/anon (401 — no god-create).
        ["v1", "repos"] => {
            let (principal, _fresh_auth) = match two_tier_auth(state, headers) {
                Ok(pair) => pair,
                Err(e) => return err(e),
            };
            verbs::write_provision::handle_provision(state, body, &principal, now_ms())
        }
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
pub(crate) fn two_tier_auth(
    state: &AppState,
    headers: &[Header],
) -> Result<(Vec<String>, bool), EngineErr> {
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

    // Tier 2: the dev-token → OPERATOR god-path — GATED behind the break-glass flag
    // (`HUGIT_ALLOW_DEV_OPERATOR=1`; [`AppState::allow_dev_operator`]). The PUBLIC
    // prod deploy OMITS the flag, so a Bearer that matches the dev-token confers NO
    // elevation: it degrades to an ANONYMOUS principal (empty chain) — it reads a
    // PUBLIC repo exactly like any anonymous visitor and NOTHING private/operator-
    // gated (fail-closed; the write-door re-checks `authorize_write`, which denies an
    // anonymous chain). When the flag is ON (documented ops/bootstrap break-glass)
    // the historical dev-token → operator behavior is preserved.
    //
    // GO-LIVE INVARIANT: a real Clerk-minted session token is resolved in Tier 1
    // above (→ `clerk:{org}:{user}`), so it can NEVER reach this branch and can NEVER
    // become the operator — flag or no flag. A real user is structurally incapable of
    // holding a god-token.
    if crate::auth::tokens_match(raw.as_bytes(), state.dev_token.as_bytes()) {
        if state.allow_dev_operator {
            return Ok((dev_principal(), false));
        }
        // Flag OFF: no god-path. The dev-token is worth exactly an anonymous visit —
        // never operator, never a partial/elevated identity.
        return Ok((Vec::new(), false));
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
        ["repo", "meta"] => {
            let req = parse!(wr::RepoMetaReq);
            with_write(
                sink,
                repo,
                "repo_meta",
                &resource,
                &idem,
                body,
                step_up,
                p,
                at,
                |log, p, at| verbs::write_repo_meta::write_repo_meta(log, repo, &req, p, at),
            )
        }
        _ => Err(EngineErr::not_found()),
    };
    match result {
        Ok(a) => ok_accepted(&a),
        Err(e) => err(e),
    }
}

/// The per-repo git content seam, borrowed for the duration of one read dispatch:
/// the object source (`blob`/`edit`/`search` content), HEAD's root tree, and the
/// HEAD commit (the default-branch tip — the start of the blob "Histórico" history
/// walk). Bundled into one struct so [`dispatch_repo`] takes a single git param
/// rather than three positional ones (clippy `too_many_arguments`). `None` for a
/// repo with no content seam loaded → the git-backed reads 404 honestly.
struct RepoGit<'a> {
    source: &'a std::sync::Arc<dyn hugit_proto::ObjectSource + Send + Sync>,
    root_tree: &'a gix_hash::ObjectId,
    head_commit: Option<gix_hash::ObjectId>,
    /// The LIVE ref snapshot (ref-name → oid hex) — the compare base/head resolver.
    /// Owned (a `snapshot()`), so a just-pushed tip is reflected without a reboot.
    refs: std::collections::BTreeMap<String, String>,
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
    git: Option<&RepoGit<'_>>,
) -> (u16, String) {
    // Unpack the bundled git seam (or `None` legs for a repo with no content seam).
    let git_source = git.map(|g| g.source);
    let root_tree = git.map(|g| g.root_tree);
    let head_commit = git.and_then(|g| g.head_commit);
    // The live ref snapshot for the compare base/head resolver (empty for a repo
    // with no content seam → an honest-empty compare diff).
    let git_refs = git.map(|g| g.refs.clone()).unwrap_or_default();
    match tail {
        ["home"] => ok(&handlers::build_home(log, repo)),
        ["new-pr"] => ok(&handlers::build_new_pr(log, repo)),
        ["knowledge"] => ok(&handlers::build_knowledge(log, repo)),
        ["compare", base, head] => ok(&handlers::build_compare(
            log, repo, base, head, git_source, &git_refs,
        )),
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
            ok(&handlers::build_search(
                log, repo, &q, git_source, root_tree,
            ))
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
            Ok(num) => match handlers::build_pr_detail(log, repo, num, git_source) {
                Some(vm) => ok(&vm),
                None => err(EngineErr::not_found()), // get_opt: absent PR → 404, no leak
            },
            // A non-numeric PR id is not a resource that exists → 404 (no leak).
            Err(_) => err(EngineErr::not_found()),
        },
        // Wave-3 by-PR read: the review panel (write-backed: verdict.recorded + pr.comment).
        ["prs", n, "review"] => match n.parse::<u32>() {
            Ok(num) => match handlers::build_review(log, repo, num, git_source) {
                Some(vm) => ok(&vm),
                None => err(EngineErr::not_found()),
            },
            Err(_) => err(EngineErr::not_found()),
        },
        // Grounded human-review Q&A over the broadened evidence corpus (checks +
        // verdicts + journal notes + intent charter/acceptance). 404 when the PR
        // is absent (no existence oracle); else an honest cited/refused answer.
        ["prs", n, "review", "qa"] => match n.parse::<u32>() {
            Ok(num) if handlers::build_review(log, repo, num, git_source).is_some() => {
                let mut q = query_param(query, "q");
                // SECURITY: cap the question before it reaches the log scan (an
                // unbounded `q` tokenized per record is a CPU/memory DoS).
                q.truncate(1024);
                ok(&handlers::build_review_qa(log, &q))
            }
            _ => err(EngineErr::not_found()),
        },
        // Phase-2 by-id reads — absent resource → 404, no existence leak.
        ["intents", id] => match handlers::build_intent_detail(log, repo, id, git_source) {
            Some(vm) => ok(&vm),
            None => err(EngineErr::not_found()),
        },
        ["commit", sha] => match handlers::build_commit_detail(log, repo, sha, git_source) {
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
            match handlers::build_blob(
                log,
                repo,
                &path,
                git_source,
                root_tree,
                head_commit.as_ref(),
            ) {
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

/// Part A — `Cache-Control: private, no-store` classification (W-CACHE-GODPATH).
#[cfg(test)]
mod cache_control_tests {
    use super::*;

    /// The ONLY cacheable responses are the two PUBLIC pre-auth GET routes.
    #[test]
    fn public_routes_stay_cacheable() {
        assert!(!response_is_private(&Method::Get, "/readyz"));
        assert!(!response_is_private(&Method::Get, "/v1/me/login"));
    }

    /// Every authenticated route (and any unknown / error path) is private —
    /// fail-closed: cacheable ONLY when explicitly one of the public routes.
    #[test]
    fn authed_and_unknown_routes_are_private() {
        assert!(response_is_private(&Method::Get, "/v1/repos/hugit/home"));
        assert!(response_is_private(
            &Method::Get,
            "/v1/repos/hugit/events?since=0"
        ));
        assert!(response_is_private(&Method::Get, "/v1/me/dashboard"));
        assert!(response_is_private(
            &Method::Post,
            "/v1/repos/hugit/prs/1/comments"
        ));
        // The token-issuance response carries a session token — never cacheable.
        assert!(response_is_private(&Method::Post, "/v1/token"));
        // A POST to the login path is NOT the public GET card → private (fail-closed).
        assert!(response_is_private(&Method::Post, "/v1/me/login"));
        // An unknown path → private (fail-closed default).
        assert!(response_is_private(&Method::Get, "/nope"));
    }

    /// The header value is exactly `private, no-store`.
    #[test]
    fn header_value_is_private_no_store() {
        let h = cache_control_private();
        assert!(
            h.field
                .as_str()
                .as_str()
                .eq_ignore_ascii_case("Cache-Control")
        );
        assert_eq!(h.value.as_str(), "private, no-store");
    }
}

/// Part B — the dev-token → operator god-path gate (W-CACHE-GODPATH). The
/// NON-NEGOTIABLE invariant: a real Clerk user can NEVER become the operator.
#[cfg(test)]
mod godpath_gate_tests {
    use super::*;
    use crate::state::AppState;
    use crate::token::ClerkPrincipal;
    use std::path::PathBuf;

    const DEV: &str = "dev-token-godpath";

    fn bearer(tok: &str) -> Vec<Header> {
        vec![Header::from_bytes(&b"Authorization"[..], format!("Bearer {tok}").as_bytes()).unwrap()]
    }

    fn state() -> AppState {
        AppState::new(PathBuf::from("/tmp/hugit-godpath-test"), DEV.to_string())
    }

    /// Break-glass ON (the `AppState::new` dev/test default): the dev-token derives
    /// the operator principal — the historical behavior, preserved.
    #[test]
    fn break_glass_on_dev_token_is_operator() {
        let s = state();
        assert!(s.allow_dev_operator, "new() defaults the break-glass ON");
        let (principal, fresh) = two_tier_auth(&s, &bearer(DEV)).expect("dev-token authenticates");
        assert_eq!(principal, vec!["orchestrator:hugit".to_string()]);
        assert!(crate::authz::is_operator(&principal));
        assert!(!fresh);
    }

    /// Break-glass OFF (the PUBLIC prod default): a dev-token Bearer degrades to an
    /// ANONYMOUS principal (empty chain) — NEVER the operator, never a partial
    /// elevated identity.
    #[test]
    fn flag_off_dev_token_degrades_to_anonymous_never_operator() {
        let mut s = state();
        s.allow_dev_operator = false;
        let (principal, fresh) =
            two_tier_auth(&s, &bearer(DEV)).expect("dev-token resolves as anonymous, not a 401");
        assert!(
            principal.is_empty(),
            "flag-off dev-token → anonymous (empty chain), got {principal:?}"
        );
        assert!(
            !crate::authz::is_operator(&principal),
            "a dev-token is NEVER operator with the flag off"
        );
        assert!(!fresh);
    }

    /// An unrecognized bearer is a hard 401 regardless of the flag (unchanged).
    #[test]
    fn garbage_bearer_is_401_regardless_of_flag() {
        for allow in [true, false] {
            let mut s = state();
            s.allow_dev_operator = allow;
            let err = two_tier_auth(&s, &bearer("not-the-dev-token")).unwrap_err();
            assert_eq!(err.status, 401, "unrecognized bearer → 401 (flag={allow})");
        }
    }

    /// THE go-live invariant: a real Clerk-minted session token can NEVER become the
    /// operator — with the break-glass ON or OFF. A real user never holds a god-token.
    #[test]
    fn clerk_token_is_never_operator_regardless_of_flag() {
        for allow in [true, false] {
            let mut s = state();
            s.allow_dev_operator = allow;
            let raw = s
                .token_store
                .mint(&ClerkPrincipal {
                    user: "user-1".to_string(),
                    org: "org-a".to_string(),
                    fresh_auth: true,
                })
                .expect("mint a clerk engine token");
            let (principal, fresh) =
                two_tier_auth(&s, &bearer(&raw)).expect("clerk token authenticates");
            assert_eq!(principal, vec!["clerk:org-a:user-1".to_string()]);
            assert!(
                !crate::authz::is_operator(&principal),
                "a real Clerk user is NEVER the operator (flag={allow})"
            );
            assert!(fresh, "fresh_auth propagates from the minted record");
        }
    }
}

/// The inbound socket-timeout bound (FIX-SOCKET-TIMEOUT): the wall-clock deadline
/// that stops a slow-loris body-dribble / slow-response-drain from wedging the
/// single-threaded accept loop. These prove the load-bearing primitive
/// ([`run_bounded`]) frees the caller at the deadline (never waits for a stuck op),
/// passes a completed value through untouched, and cannot be crashed by a worker
/// panic — plus the deadline clamp. The `read_body_bounded` / `respond_bounded`
/// call sites are thin, by-construction adapters over this tested primitive.
#[cfg(test)]
mod io_timeout_tests {
    use super::*;
    use std::time::Instant;

    /// A fast operation returns its value, unchanged, well within the budget.
    #[test]
    fn run_bounded_fast_returns_value() {
        let out = run_bounded(Duration::from_secs(5), || 40 + 2);
        assert_eq!(out, Some(42), "a fast op returns its value");
    }

    /// A moved (non-Copy) value round-trips through the worker intact — the shape
    /// `read_body_bounded` relies on (it moves the `Request` in and back out).
    #[test]
    fn run_bounded_moves_value_through() {
        let payload = vec![1u8, 2, 3, 4];
        let out = run_bounded(Duration::from_secs(5), move || (payload, "ok"));
        assert_eq!(out, Some((vec![1u8, 2, 3, 4], "ok")));
    }

    /// THE anti-wedge property: an operation that runs far longer than the budget
    /// does NOT block the caller for its full duration — `run_bounded` returns `None`
    /// at ~the deadline, freeing the loop. The op here sleeps 5 s with a 100 ms
    /// budget; we assert the call returns in well under the op's 5 s (a loose 2 s
    /// bound — 20x the budget — so it never flakes on a contended runner) AND that a
    /// slow op yields `None`. This is exactly the slow-loris body-dribble outcome.
    #[test]
    fn run_bounded_slow_op_times_out_without_waiting() {
        let start = Instant::now();
        let out = run_bounded(Duration::from_millis(100), || {
            std::thread::sleep(Duration::from_secs(5));
            99u32
        });
        let elapsed = start.elapsed();
        assert_eq!(
            out, None,
            "a slow op past the deadline yields None (dropped)"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "the caller must be freed near the deadline, not wait for the slow op \
             (elapsed = {elapsed:?}, op = 5s, budget = 100ms)"
        );
    }

    /// A panic inside the worker degrades to `None` — it can NEVER unwind into / crash
    /// the caller (the accept loop). The channel disconnects, `recv_timeout` errs.
    #[test]
    fn run_bounded_worker_panic_is_none_not_crash() {
        let out: Option<u32> = run_bounded(Duration::from_secs(5), || panic!("boom in worker"));
        assert_eq!(out, None, "a worker panic is a drop, never a caller crash");
    }

    /// The deadline clamp: unset/unparseable → 60 s default; clamped to `[5, 3600]`
    /// so a mis-set env can neither false-drop a legit slow request nor un-bound the
    /// wedge.
    #[test]
    fn io_deadline_clamp() {
        assert_eq!(clamp_io_deadline_secs(None), 60, "default when unset");
        assert_eq!(clamp_io_deadline_secs(Some(0)), 5, "floor");
        assert_eq!(clamp_io_deadline_secs(Some(1)), 5, "below floor → floor");
        assert_eq!(
            clamp_io_deadline_secs(Some(120)),
            120,
            "in-range passes through"
        );
        assert_eq!(
            clamp_io_deadline_secs(Some(9_999)),
            3600,
            "above ceiling → ceiling"
        );
        // The live default is always inside the safe window.
        let d = io_deadline().as_secs();
        assert!(
            (5..=3600).contains(&d),
            "io_deadline within [5,3600], got {d}"
        );
    }
}
