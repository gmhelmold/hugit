//! The P2 seam: **detecting** a GitHub-side change (webhook / poll).
//!
//! All of the *arbitration / convergence / single-writer* logic in
//! [`super::engine`] is proven hermetically in-process. The one piece that
//! genuinely needs live infrastructure is *noticing* that the GitHub side
//! changed: a webhook delivery or a poll loop against a real repo. That live
//! detection is the documented P2 seam — it needs a live GitHub repo named by
//! `HUGIT_GH_TEST_REPO` plus an App installation.
//!
//! Per the project's PARTIAL-over-fake law, this seam is **gated behind env,
//! run-not-skip when set, and asserted "not wired" in the bare gate** so it can
//! never silently rot to a fake green:
//!
//! - `HUGIT_GH_TEST_REPO` unset → [`GitHubDetectOutcome::NotConfigured`]
//!   (no live target; the bare gate's positive expectation).
//! - `HUGIT_GH_TEST_REPO` set → the live webhook/poll transport is **not wired**
//!   in this build, so the honest outcome is [`GitHubDetectOutcome::NotWired`]
//!   carrying the repo — never a fabricated "detected" result.
//!
//! When the live transport lands (P2), it replaces the `NotWired` arm with a
//! real `Detected { ref_name, new_tip }` feeding [`super::engine::BidirSync`];
//! the engine logic above does not change.

/// The outcome of attempting to detect a GitHub-side change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitHubDetectOutcome {
    /// No live target configured (`HUGIT_GH_TEST_REPO` unset). The bare,
    /// offline gate's expected outcome.
    NotConfigured,
    /// A live target IS configured but the live webhook/poll transport is not
    /// wired in this build — honestly PARTIAL, never a fabricated detection.
    /// Carries the configured repo so the gap is diagnosable.
    NotWired {
        /// The repo named by `HUGIT_GH_TEST_REPO`.
        repo: String,
    },
}

impl GitHubDetectOutcome {
    /// Whether a live target was configured at all.
    pub fn is_configured(&self) -> bool {
        matches!(self, GitHubDetectOutcome::NotWired { .. })
    }

    /// Whether the live detection transport is wired (always `false` until P2).
    pub fn is_wired(&self) -> bool {
        false
    }
}

/// Attempt to detect a GitHub-side change for the configured test repo.
///
/// Reads `HUGIT_GH_TEST_REPO`. Unset → [`GitHubDetectOutcome::NotConfigured`].
/// Set → [`GitHubDetectOutcome::NotWired`] (the live transport is the P2 seam;
/// never a faked detection). This function performs no network I/O.
pub fn detect_github_change() -> GitHubDetectOutcome {
    match std::env::var("HUGIT_GH_TEST_REPO") {
        Ok(repo) if !repo.is_empty() => GitHubDetectOutcome::NotWired { repo },
        _ => GitHubDetectOutcome::NotConfigured,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unconfigured_is_not_configured() {
        // We cannot safely mutate process env in a multi-threaded test harness
        // here; assert the structural property of the not-wired outcome instead.
        let configured = GitHubDetectOutcome::NotWired {
            repo: "example-org/example-repo".into(),
        };
        assert!(configured.is_configured());
        assert!(!configured.is_wired());

        let unconfigured = GitHubDetectOutcome::NotConfigured;
        assert!(!unconfigured.is_configured());
        assert!(!unconfigured.is_wired());
    }
}
