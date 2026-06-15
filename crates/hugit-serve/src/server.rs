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

use crate::auth::check_bearer;
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

    // Auth BEFORE any resource work (identical law to the standard read path).
    if let Err(e) = check_bearer(headers, &state.dev_token) {
        let (status, body) = err(e);
        return send_json(request, status, body);
    }
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
            let (status, body) = err(e);
            return send_json(request, status, body);
        }
    };
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

    // All other routes are GET-only reads (writes are Wave-2).
    if method != &Method::Get {
        return err(EngineErr::not_found());
    }

    // /v1/repos/{repo}/... — every read is Bearer-authenticated (auth BEFORE any
    // repo/resource work, so 401 never depends on whether the repo exists).
    match segs.as_slice() {
        ["v1", "repos", repo, tail @ ..] => {
            if let Err(e) = check_bearer(headers, &state.dev_token) {
                return err(e);
            }
            // The query (stripped above) is re-passed for the param-driven reads.
            dispatch_repo(state, repo, tail, url.split('?').nth(1).unwrap_or(""))
        }
        // Identity-scoped reads (/v1/me/*): no {repo} path param. Auth BEFORE any
        // work, then bind the dev-principal's default context = the launch repo
        // (the per-principal multi-repo `me` aggregation is the P2 identity seam).
        ["v1", "me", "dashboard"] => {
            if let Err(e) = check_bearer(headers, &state.dev_token) {
                return err(e);
            }
            let repo = ME_DEFAULT_REPO;
            with_log(
                || state.load_verified(repo),
                |log| ok(&handlers::build_dashboard(log, repo)),
            )
        }
        ["v1", "me", "attention"] => {
            if let Err(e) = check_bearer(headers, &state.dev_token) {
                return err(e);
            }
            let repo = ME_DEFAULT_REPO;
            with_log(
                || state.load_verified(repo),
                |log| ok(&handlers::build_attention(log, repo)),
            )
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
        ["v1", "repos", repo, tail @ ..] => {
            if let Err(e) = check_bearer(headers, &state.dev_token) {
                return err(e);
            }
            dispatch_repo_write(state, repo, tail, headers, body)
        }
        _ => err(EngineErr::not_found()),
    }
}

/// Dispatch an authenticated `/v1/repos/{repo}/<tail...>` POST through the
/// write-door (`with_write`): idempotency + step-up + persist + atomic ledger.
fn dispatch_repo_write(
    state: &AppState,
    repo: &str,
    tail: &[&str],
    headers: &[Header],
    body: &[u8],
) -> (u16, String) {
    let idem = header_val(headers, "Idempotency-Key").unwrap_or_default();
    let step_up = header_val(headers, "X-Step-Up")
        .map(|v| v == "true" || v == "1")
        .unwrap_or(false);
    let p = dev_principal();
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
fn dispatch_repo(state: &AppState, repo: &str, tail: &[&str], query: &str) -> (u16, String) {
    // Resolve + verify the repo's log once (404 if absent/unsafe, 503 if tampered).
    let load = || state.load_verified(repo);
    match tail {
        ["home"] => with_log(load, |log| ok(&handlers::build_home(log, repo))),
        ["landing"] => with_log(load, |log| ok(&handlers::build_landing(log, repo))),
        ["checks"] => with_log(load, |log| ok(&handlers::build_checks(log, repo))),
        ["commits"] => with_log(load, |log| ok(&handlers::build_commits(log, repo))),
        // Phase-2 collection reads (real engine backbone).
        ["chrome"] => with_log(load, |log| ok(&handlers::build_repo_chrome(log, repo))),
        ["branches"] => with_log(load, |log| ok(&handlers::build_branches(log, repo))),
        ["insights"] => with_log(load, |log| ok(&handlers::build_insights(log, repo))),
        // Wave-3 collection reads (write-backed: issues←issue.transition,
        // security←policy.set/erasure.decided).
        ["issues"] => with_log(load, |log| ok(&handlers::build_issues(log, repo))),
        ["security"] => with_log(load, |log| ok(&handlers::build_security(log, repo))),
        // Wave-4 collection reads (real log-backed): settings←policy.set,
        // releases←pr.landed, search←q over records, viewer-can←D14 authz matrix.
        ["settings"] => with_log(load, |log| ok(&handlers::build_repo_settings(log, repo))),
        ["releases"] => with_log(load, |log| ok(&handlers::build_releases(log, repo))),
        ["search"] => {
            let q = query_param(query, "q");
            with_log(load, |log| ok(&handlers::build_search(log, repo, &q)))
        }
        // The capability matrix is per-principal-class (repo-agnostic); the log is
        // still load-verified for consistent 404/503 semantics with the other reads.
        ["viewer-can"] => with_log(load, |_log| {
            ok(&handlers::build_viewer_can(&dev_principal()))
        }),
        ["prs", n] => match n.parse::<u32>() {
            Ok(num) => with_log(load, |log| {
                match handlers::build_pr_detail(log, repo, num) {
                    Some(vm) => ok(&vm),
                    None => err(EngineErr::not_found()), // get_opt: absent PR → 404, no leak
                }
            }),
            // A non-numeric PR id is not a resource that exists → 404 (no leak).
            Err(_) => err(EngineErr::not_found()),
        },
        // Wave-3 by-PR read: the review panel (write-backed: verdict.recorded + pr.comment).
        ["prs", n, "review"] => match n.parse::<u32>() {
            Ok(num) => with_log(load, |log| match handlers::build_review(log, repo, num) {
                Some(vm) => ok(&vm),
                None => err(EngineErr::not_found()),
            }),
            Err(_) => err(EngineErr::not_found()),
        },
        // Phase-2 by-id reads — absent resource → 404, no existence leak.
        ["intents", id] => with_log(load, |log| {
            match handlers::build_intent_detail(log, repo, id) {
                Some(vm) => ok(&vm),
                None => err(EngineErr::not_found()),
            }
        }),
        ["commit", sha] => with_log(load, |log| {
            match handlers::build_commit_detail(log, repo, sha) {
                Some(vm) => ok(&vm),
                None => err(EngineErr::not_found()),
            }
        }),
        ["campaigns", name] => with_log(load, |log| {
            match handlers::build_campaign(log, repo, name) {
                Some(vm) => ok(&vm),
                None => err(EngineErr::not_found()),
            }
        }),
        _ => err(EngineErr::not_found()),
    }
}

/// Run `f` over a freshly verified log, mapping the load error to its envelope.
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
