//! Shared HTTP helpers for the engine-touching tools (`cost-attest`,
//! `liveness-probe`).
//!
//! Two invariants enforced here, both go-live-grade:
//!
//! 1. **A git/browser User-Agent.** The prod engine sits behind Cloudflare
//!    bot-protection: a non-git/non-browser UA gets `403 error 1010`. We send a
//!    `git/`-prefixed UA so an honest probe is never mistaken for a bot (the
//!    op-note in CLAUDE.md). The disambiguator still REPORTS a 403 distinctly if
//!    one slips through.
//!
//! 2. **Bounded timeouts.** The single-threaded lazy-CAS engine serves every
//!    request on one accept loop; a slow call blocks `/readyz` for everyone. We
//!    set tight connect/read timeouts so a probe can never hang the caller, and
//!    the heavy-read REFUSAL lives in the probe tool itself.

use std::time::Duration;

/// A git-shaped User-Agent so Cloudflare bot-protection treats the probe as a
/// legitimate client (the engine returns `403 error 1010` to unknown UAs).
pub const GIT_UA: &str = "git/2.43.0 hugit-mcp";

/// Connect timeout — fail fast if the engine is unreachable.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Read timeout for a LIGHT request (`/readyz`, an `insights` summary). A heavy
/// read is REFUSED upstream, never issued, so this never has to cover a
/// minutes-long lazy-CAS walk.
const READ_TIMEOUT: Duration = Duration::from_secs(15);

/// A ureq agent with bounded timeouts and the git UA baked into every request.
pub fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(CONNECT_TIMEOUT)
        .timeout_read(READ_TIMEOUT)
        .user_agent(GIT_UA)
        .build()
}

/// The classified outcome of a probe HTTP call — disambiguates the status
/// classes a hugit caller must tell apart (404-missing vs 404-denied is
/// resolved by the CALLER with auth context; here we expose the raw signal).
pub enum HttpClass {
    /// 2xx — the body is available.
    Ok { status: u16, body: String },
    /// 401 — the Worker auth-gate (a token is required / invalid). NOT
    /// route-absence (the "probe 401 ≠ route exists" lesson).
    Unauthorized,
    /// 403 — Cloudflare bot-protection (`error 1010`) or an authz denial.
    Forbidden { body: String },
    /// 404 — the route/repo is absent OR a no-oracle denial (the engine returns
    /// 404 for a private repo a principal may not see). The caller disambiguates
    /// using whether a valid token was supplied.
    NotFound,
    /// Any other status — surfaced with the code + a body snippet.
    Other { status: u16, body: String },
    /// The request never completed (DNS / connect / timeout). Transport-level.
    Transport { message: String },
}

/// Issue a GET, optionally with a Bearer token, and classify the response.
/// `body` is read for the non-transport branches (bounded by `READ_TIMEOUT`).
pub fn get_classified(agent: &ureq::Agent, url: &str, bearer: Option<&str>) -> HttpClass {
    let mut req = agent.get(url);
    if let Some(tok) = bearer {
        req = req.set("Authorization", &format!("Bearer {tok}"));
    }
    match req.call() {
        Ok(resp) => {
            let status = resp.status();
            let body = resp.into_string().unwrap_or_default();
            HttpClass::Ok { status, body }
        }
        Err(ureq::Error::Status(code, resp)) => {
            let body = resp.into_string().unwrap_or_default();
            match code {
                401 => HttpClass::Unauthorized,
                403 => HttpClass::Forbidden { body },
                404 => HttpClass::NotFound,
                other => HttpClass::Other {
                    status: other,
                    body,
                },
            }
        }
        Err(ureq::Error::Transport(t)) => HttpClass::Transport {
            message: t.to_string(),
        },
    }
}

/// Trim a body to a bounded snippet so a tool result never echoes an unbounded
/// (or secret-shaped) response wholesale.
pub fn snippet(body: &str) -> String {
    const MAX: usize = 280;
    let trimmed = body.trim();
    if trimmed.len() <= MAX {
        trimmed.to_string()
    } else {
        format!("{}…", &trimmed[..MAX])
    }
}
