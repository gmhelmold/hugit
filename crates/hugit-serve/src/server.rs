//! The minimal synchronous `/v1` HTTP server (tiny_http) + routing.
//!
//! [`route`] is socket-free (method + url + headers → (status, body)) so it is
//! unit-testable without binding a port; [`serve`] is the thin tiny_http loop
//! that calls it. Transport law (backend-API-v1): GET-only reads; `/readyz` is
//! unauthenticated; every `/v1/*` read requires `Bearer`; a missing resource is
//! 404 (no existence leak); a tampered/unreadable log is 503 (fail-honest); the
//! error body is the `{code, reason}` envelope.

use std::sync::OnceLock;

use tiny_http::{Header, Method, Response, Server};

use crate::auth::check_bearer;
use crate::error::EngineErr;
use crate::handlers;
use crate::state::AppState;

/// Bind `addr` (e.g. `127.0.0.1:8787`) and run the server forever.
pub fn serve(state: AppState, addr: &str) -> std::io::Result<()> {
    let server = Server::http(addr).map_err(|e| std::io::Error::other(e.to_string()))?;
    eprintln!(
        "hugit-serve listening on {addr} (log_dir={})",
        state.log_dir.display()
    );
    serve_on(state, server)
}

/// Run the request loop over a PRE-BOUND server (the loop the real binary runs;
/// split out so a test can bind `:0`, learn the port, and exercise it end-to-end).
pub fn serve_on(state: AppState, server: Server) -> std::io::Result<()> {
    for request in server.incoming_requests() {
        // PANIC ISOLATION: a panic inside a handler must degrade to a 503 for THAT
        // request, never take down the whole single-threaded server.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            route(&state, request.method(), request.url(), request.headers())
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
            dispatch_repo(state, repo, tail)
        }
        _ => err(EngineErr::not_found()),
    }
}

/// Dispatch an authenticated `/v1/repos/{repo}/<tail...>` read.
fn dispatch_repo(state: &AppState, repo: &str, tail: &[&str]) -> (u16, String) {
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
        // Phase-2 by-id reads — absent resource → 404, no existence leak.
        ["intents", id] => with_log(load, |log| match handlers::build_intent_detail(log, repo, id) {
            Some(vm) => ok(&vm),
            None => err(EngineErr::not_found()),
        }),
        ["commit", sha] => with_log(load, |log| match handlers::build_commit_detail(log, repo, sha) {
            Some(vm) => ok(&vm),
            None => err(EngineErr::not_found()),
        }),
        ["campaigns", name] => with_log(load, |log| match handlers::build_campaign(log, repo, name) {
            Some(vm) => ok(&vm),
            None => err(EngineErr::not_found()),
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
