//! Real-socket integration tests for the per-principal engine rate limit (G10).
//!
//! These drive the ACTUAL `serve_on_with` accept loop over a `TcpStream` with an
//! INJECTED [`RateLimiter`] carrying tight limits (env-race-free — no process-global
//! `set_var`). They prove two loop-level properties the pure unit tests in
//! `hugit_serve::ratelimit` cannot reach:
//!
//!   * an over-budget principal is rejected `429` INLINE in the accept loop, and
//!   * that rejection happens BEFORE the POST body is read — an over-budget request
//!     that DECLARES a huge `Content-Length` but never ships it still gets an instant
//!     `429`, instead of the loop blocking on the body read up to the (≥5 s) I/O
//!     deadline. That is the core G10 guarantee: a flood cannot even make the
//!     single-threaded loop buffer bodies.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use hugit_serve::ratelimit::RateLimiter;
use hugit_serve::server::serve_on_with;
use hugit_serve::state::AppState;
use tiny_http::Server;

const TOKEN: &str = "dev-token-ratelimit";

fn scratch_dir() -> PathBuf {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "hugit-serve-rl-{}-{}-{}",
        std::process::id(),
        nanos,
        seq
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Spawn the real accept loop on an ephemeral port with the given rate limiter;
/// return the bound `127.0.0.1:<port>` address.
fn spawn(rl: RateLimiter) -> String {
    let dir = scratch_dir();
    std::fs::write(dir.join("hugit.json"), "[]").unwrap();
    let state = AppState::new(dir, TOKEN.to_string());
    let server = Server::http("127.0.0.1:0").expect("bind ephemeral port");
    let addr = server
        .server_addr()
        .to_ip()
        .expect("ip listen addr")
        .to_string();
    std::thread::spawn(move || {
        let _ = serve_on_with(state, server, rl);
    });
    addr
}

/// One raw HTTP/1.1 GET (no Bearer → the anonymous bucket) → the status line's code.
fn anon_get_status(addr: &str, path: &str) -> u16 {
    let mut stream = TcpStream::connect(addr).expect("connect");
    let req = format!("GET {path} HTTP/1.1\r\nHost: t\r\nConnection: close\r\n\r\n");
    stream.write_all(req.as_bytes()).unwrap();
    let mut resp = String::new();
    stream.read_to_string(&mut resp).unwrap();
    parse_status(&resp)
}

fn parse_status(resp: &str) -> u16 {
    resp.split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .unwrap_or(0)
}

/// A tight anon limiter: burst 2, 1/s refill. Tenant/push limits are generous (they
/// are not exercised here).
fn tight_anon() -> RateLimiter {
    RateLimiter::for_test(60, 120, 1, 2, 5, 10, 1024)
}

/// The over-budget anonymous request is rejected `429` inline in the accept loop —
/// proving the gate fires over a real socket, not just in unit arithmetic.
#[test]
fn anon_flood_gets_429_over_the_socket() {
    let addr = spawn(tight_anon());
    // Fire a rapid burst of anonymous GETs from localhost (one shared edge bucket).
    // burst=2 with a 1/s refill → most of a rapid burst is over budget.
    let mut got_429 = false;
    let mut got_non_429 = false;
    for _ in 0..10 {
        match anon_get_status(&addr, "/v1/repos/hugit/home") {
            429 => got_429 = true,
            _ => got_non_429 = true,
        }
    }
    assert!(got_429, "an anonymous flood must be throttled with a 429");
    assert!(
        got_non_429,
        "the first within-budget requests must NOT be throttled (the gate is a rate limit, not a block)"
    );
}

/// THE loop-safety guarantee: the gate fires BEFORE the POST body is read. An
/// over-budget request that DECLARES a large `Content-Length` but never sends the
/// body still gets an INSTANT 429 — the loop does not buffer the (never-arriving)
/// body. If the gate ran AFTER `read_body_bounded`, the server would block on the
/// body read up to the I/O deadline (≥ 5 s) and send NO response in that window.
#[test]
fn gate_fires_before_body_read() {
    let addr = spawn(tight_anon());

    // First, drain the anon burst with cheap GETs so the next request is over budget.
    for _ in 0..6 {
        let _ = anon_get_status(&addr, "/v1/repos/hugit/home");
    }

    // Now an over-budget POST that PROMISES 10 MB of body but ships only a few bytes.
    let mut stream = TcpStream::connect(&addr).expect("connect");
    // Read timeout well UNDER the 5 s I/O-deadline floor: if the server blocked on the
    // body read (gate misordered), we would see no 429 before this fires.
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let head = "POST /v1/repos/hugit/land HTTP/1.1\r\nHost: t\r\n\
                Content-Type: application/json\r\nContent-Length: 10000000\r\n\
                Connection: close\r\n\r\n";
    stream.write_all(head.as_bytes()).unwrap();
    stream.write_all(b"{\"partial\":").unwrap(); // a few bytes — the rest never comes
    stream.flush().unwrap();

    let start = Instant::now();
    let mut resp = String::new();
    // Read whatever the server sends. A prompt 429 proves the gate short-circuited
    // BEFORE the body read; a read timeout with no 429 means it blocked on the body.
    let _ = stream.read_to_string(&mut resp);
    let elapsed = start.elapsed();

    assert_eq!(
        parse_status(&resp),
        429,
        "an over-budget POST must be rejected 429 BEFORE the body is read (got: {resp:?})"
    );
    assert!(
        elapsed < Duration::from_secs(5),
        "the 429 must arrive before the I/O deadline — the loop must not buffer the body \
         (elapsed {elapsed:?})"
    );
}
