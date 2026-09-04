//! Outbound one-way sync (hugit → GitHub mirror) — WP-E1a items ① ③.
//!
//! This module is the **one-way mirror writer**: it replicates every landed ref
//! to the GitHub mirror, content-hash verified to byte-identity, fed by the
//! durable [`crate::queue`] in landing order. It is one-way ONLY — it never
//! reads GitHub state as a source of truth (no reverse-sync path lives here;
//! E1b⑦ proves reverse writes are divergence).
//!
//! Sub-modules:
//! - [`auth`] — GitHub App installation-token auth (not PAT/OAuth); secret
//!   material read via `std::fs`, never printed.
//! - [`live`] — [`LiveGitHubTarget`], the real-`git` push lane.
//! - [`writer`] — the push driver, per-push verify integration, SLA + soak.
//!
//! The live GitHub lane (item ①) is attempted against `HUGIT_GH_TEST_REPO`
//! using the App credentials; if the installation does not cover the repo (or
//! creds/network are unavailable), the live attempt reports **PARTIAL** — never
//! a faked success. The local hash-verify and ordering proofs stand on their
//! own as fixture proofs.

pub mod auth;
pub mod live;
pub mod writer;

pub use auth::{AppAuth, AppAuthError, InstallationToken};
pub use live::LiveGitHubTarget;
pub use writer::{
    FixtureMirror, MirrorPushTarget, OutboundWriter, PushError, PushReport, SLA_BOUND_MS,
    SoakSummary,
};

use std::path::Path;
use std::process::Command;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::verify::ContentHash;

/// The outcome of a *live* GitHub landing attempt against `HUGIT_GH_TEST_REPO`.
///
/// The live round-trip is infrastructure-gated. When the App installation does
/// not cover the test repo, or credentials/network are unavailable, the outcome
/// is [`LiveLandingOutcome::Partial`] — the WP is honestly PARTIAL on the live
/// lane, never faked GREEN. A real, verified live landing yields
/// [`LiveLandingOutcome::Verified`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveLandingOutcome {
    /// A real ref landed on GitHub and content-hash verified within SLA.
    Verified {
        /// The repo it landed to.
        repo: String,
        /// Measured land→verified latency in milliseconds.
        latency_ms: u64,
    },
    /// The live lane is unavailable — PARTIAL, never fake. Carries the reason.
    Partial {
        /// Why the live lane could not run to a verified landing.
        reason: String,
    },
}

impl LiveLandingOutcome {
    /// Whether this is a real verified live landing.
    pub fn is_verified(&self) -> bool {
        matches!(self, LiveLandingOutcome::Verified { .. })
    }

    /// Whether this is the honest PARTIAL path.
    pub fn is_partial(&self) -> bool {
        matches!(self, LiveLandingOutcome::Partial { .. })
    }
}

/// Attempt a live landing against `HUGIT_GH_TEST_REPO` using App auth.
///
/// Behaviour (item ①, never-fake rule):
/// - If `HUGIT_GH_TEST_REPO` is unset → PARTIAL (no live target configured).
/// - If App credentials are absent on disk → PARTIAL (installation
///   unavailable). Credentials are read via `std::fs`; secret bytes are never
///   returned or logged.
/// - If credentials are present, a REAL round-trip is attempted: a probe ref is
///   pushed to the authenticated GitHub remote with the real `git` binary and
///   re-read for byte-identity within the SLA. A verified match → [`LiveLandingOutcome::Verified`];
///   any failure (transport, coverage, mismatch, latency) → PARTIAL. The gate
///   lane has no creds, so it short-circuits before any network.
///
/// `auth` is the App auth client; passing the default dev dir wires the real
/// `~/.hugit/secrets/github-app-dev` location.
pub fn live_landing_attempt(auth: &AppAuth) -> LiveLandingOutcome {
    let repo = match std::env::var("HUGIT_GH_TEST_REPO") {
        Ok(r) if !r.is_empty() => r,
        _ => {
            return LiveLandingOutcome::Partial {
                reason: "HUGIT_GH_TEST_REPO not set; no live landing target configured".to_string(),
            };
        }
    };

    // Mint the App installation token from on-disk credentials (std::fs).
    // Absence/error → PARTIAL; never a faked landing.
    match auth.mint_installation_token() {
        Err(AppAuthError::CredentialsUnavailable) => LiveLandingOutcome::Partial {
            reason: format!(
                "GitHub App credentials unavailable; live landing to {repo} not attempted \
                 (shim/local hash-verify proofs stand on their own)"
            ),
        },
        Err(AppAuthError::RepoNotCovered { repo: r }) => LiveLandingOutcome::Partial {
            reason: format!("App installation does not cover {r}; live landing PARTIAL, not faked"),
        },
        // A JWT-signing or live-exchange failure (transport / non-2xx / decode) is
        // the honest fail-closed PARTIAL — never a fabricated Verified. The error
        // carries no secret (only a generic reason / HTTP status).
        Err(e) => LiveLandingOutcome::Partial {
            reason: format!(
                "App installation-token mint failed for {repo}: {e} (PARTIAL, not faked)"
            ),
        },
        Ok(token) => {
            // Token material present — attempt the REAL live round-trip: push a
            // probe ref from a scratch repo to the authenticated GitHub remote
            // and re-read it for byte-identity within the SLA. Any failure is
            // the honest PARTIAL — never a fabricated Verified. (The offline
            // gate lane never reaches here: creds absent → CredentialsUnavailable
            // above, unchanged.)
            debug_assert!(token.is_present());
            live_landing_with_token(&token, repo)
        }
    }
}

/// Fixed content of the live-landing probe commit (idempotent across attempts).
const PROBE_CONTENT: &str = "hugit live landing probe\n";

/// Step a `git` command on the live lane with a fully hermetic environment: no
/// ambient global/system config (a pathological host config must never break or
/// prompt on the probe) and no terminal prompts. Returns trimmed stdout on
/// success, else a SECRET-FREE reason.
fn git_live(cwd: &Path, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .current_dir(cwd)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_SYSTEM", "/dev/null")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .map_err(|e| format!("git could not be started: {e}"))?;
    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(format!("git {args:?} failed: {}", stderr.trim()));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// A real live landing round-trip once an installation token is minted
/// (item ①, never-fake rule). Returns [`LiveLandingOutcome::Verified`] ONLY on
/// a byte-identical observed oid within the SLA; every other path is the honest
/// [`LiveLandingOutcome::Partial`], with a secret-free reason (never the token,
/// never key bytes).
fn live_landing_with_token(token: &InstallationToken, repo: String) -> LiveLandingOutcome {
    let started = Instant::now();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id();
    let probe_ref = format!("refs/heads/hugit/mirror-probe-{pid}-{nanos}");
    let scratch = std::env::temp_dir().join(format!("hugit-live-{pid}-{nanos}"));

    // The attempt runs to one `Result`; the scratch dir is removed BEFORE any
    // return, so cleanup never waits on the outcome.
    let attempt = (|| -> Result<String, String> {
        std::fs::create_dir_all(&scratch)
            .map_err(|e| format!("cannot create scratch repo: {e}"))?;
        git_live(&scratch, &["init", "-q", "-b", "main"])?;
        git_live(&scratch, &["config", "user.name", "hugit"])?;
        git_live(&scratch, &["config", "user.email", "bot@hugit.dev"])?;
        std::fs::write(scratch.join("probe.txt"), PROBE_CONTENT)
            .map_err(|e| format!("cannot write probe.txt: {e}"))?;
        git_live(&scratch, &["add", "probe.txt"])?;
        git_live(&scratch, &["commit", "-q", "-m", "hugit-live-probe"])?;
        let oid = git_live(&scratch, &["rev-parse", "HEAD"])?;

        // The authenticated remote — the token rides the `x-access-token`
        // username (GitHub's standard). The URL is never surfaced; the push
        // target scrubs the token from any failure detail.
        let auth_remote = format!(
            "https://x-access-token:{}@github.com/{repo}.git",
            token.expose()
        );
        let observed = LiveGitHubTarget::new(scratch.as_path(), auth_remote)
            .push_ref(&probe_ref, &ContentHash::new(&oid))
            .map_err(|e| format!("live push to {repo} failed (probe {probe_ref}): {e}"))?;
        if observed != ContentHash::new(&oid) {
            return Err(format!(
                "mirror observed {observed} for {probe_ref}, expected {oid} — NOT byte-identical \
                 (fail-closed, no verified landing)"
            ));
        }
        Ok(oid)
    })();

    // Unconditional cleanup, preceding every return.
    let _ = std::fs::remove_dir_all(&scratch);

    match attempt {
        Ok(oid) => {
            let elapsed_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            if elapsed_ms <= SLA_BOUND_MS {
                LiveLandingOutcome::Verified {
                    repo,
                    latency_ms: elapsed_ms,
                }
            } else {
                // The ref landed but missed the SLA bound — not a verified
                // landing (fail-closed on the bound, like the writer's SLA).
                LiveLandingOutcome::Partial {
                    reason: format!(
                        "probe ref {probe_ref} -> {oid} landed after {elapsed_ms}ms, exceeding \
                         the {SLA_BOUND_MS}ms SLA (PARTIAL, not verified)"
                    ),
                }
            }
        }
        Err(reason) => LiveLandingOutcome::Partial {
            reason: format!("{reason} (PARTIAL, not faked)"),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_attempt_without_creds_is_partial() {
        // Default dev dir may or may not have creds in CI; either way, a
        // non-covering / offline environment must be PARTIAL, never Verified.
        let auth = AppAuth::new("/nonexistent/github-app-dev");
        // Ensure a repo is set so we exercise the auth branch.
        // SAFETY: single-threaded test; no concurrent access to this env var.
        unsafe {
            std::env::set_var("HUGIT_GH_TEST_REPO", "example-org/example-repo");
        }
        let outcome = live_landing_attempt(&auth);
        assert!(outcome.is_partial());
        assert!(!outcome.is_verified());
    }
}
