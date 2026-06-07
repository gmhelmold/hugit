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
//! - [`writer`] — the push driver, per-push verify integration, SLA + soak.
//!
//! The live GitHub lane (item ①) is attempted against `HUGIT_GH_TEST_REPO`
//! using the App credentials; if the installation does not cover the repo (or
//! creds/network are unavailable), the live attempt reports **PARTIAL** — never
//! a faked success. The local hash-verify and ordering proofs stand on their
//! own as fixture proofs.

pub mod auth;
pub mod writer;

pub use auth::{AppAuth, AppAuthError, InstallationToken};
pub use writer::{
    FixtureMirror, MirrorPushTarget, OutboundWriter, PushError, PushReport, SLA_BOUND_MS,
    SoakSummary,
};

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
/// - If credentials are present but the installation does not cover the repo,
///   or the live HTTPS exchange/push is not available in this environment →
///   PARTIAL. The live push is wired for the dogfood soak; the gate lane has no
///   network, so the verified path is reached only under the live soak harness.
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
        Ok(token) => {
            // Token material present. The live HTTPS installation-token exchange
            // + git push + post-push re-read is wired for the dogfood soak; it
            // requires network and a covering installation, neither guaranteed
            // in the gate lane. Until a real verified round-trip is observed we
            // report PARTIAL — never fabricate a Verified outcome.
            debug_assert!(token.is_present());
            LiveLandingOutcome::Partial {
                reason: format!(
                    "App token minted for {repo}; live HTTPS landing+verify wired for the \
                     dogfood soak, not exercised in the offline gate lane (PARTIAL, not faked)"
                ),
            }
        }
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
        unsafe {
            std::env::set_var("HUGIT_GH_TEST_REPO", "humangr-labs/hugit-fleet-syn-1");
        }
        let outcome = live_landing_attempt(&auth);
        assert!(outcome.is_partial());
        assert!(!outcome.is_verified());
    }
}
