//! PR/issue fetch and projection into proposed, non-authoritative intents.
//!
//! This module projects [`ImportedPrIssue`] fixtures into [`ProposedIntent`]s
//! and enforces the fidelity contract:
//!
//! **Preserved with per-element provenance**: body · comment/review threads ·
//! state · labels · cross-refs.
//!
//! **Not imported (explicitly enumerated)**: see [`NON_IMPORTED`].
//!
//! **Idempotency**: the `intent_id` is derived from the source URL, so
//! importing the same PR/issue twice produces the same id. The caller is
//! responsible for deduplication (unchanged → no-op; changed → incremental
//! re-sync). See [`import_prissue`].
//!
//! **Boundary law (E2a⑤)**: this module only mints intents from PR/issue
//! *metadata*. Bare commits MUST NOT produce intents; that law is E2a's.
//! Nothing in this module touches the git history or commits.

use crate::import::prissue::model::{ImportedPrIssue, ProposedIntent};

/// Import error kinds for PR/issue import.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrIssueImportError {
    /// The source URL is empty — there is no stable identity to derive an
    /// `intent_id` from.
    EmptySourceUrl,
}

impl std::fmt::Display for PrIssueImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PrIssueImportError::EmptySourceUrl => {
                write!(f, "source_url is empty — cannot derive a stable intent_id")
            }
        }
    }
}

impl std::error::Error for PrIssueImportError {}

/// Import a single PR/issue into a proposed, non-authoritative intent.
///
/// The resulting [`ProposedIntent`] is **always** flagged non-authoritative
/// (`sidecar.authoritative == false`); it never gates or blocks any landing.
///
/// The `intent_id` is derived from `source_url` — re-importing the same
/// PR/issue with unchanged metadata produces the same sidecar (idempotent).
/// If metadata has changed the caller should detect the delta and re-call;
/// the returned intent replaces the previous one with the same `intent_id`
/// (no duplication).
///
/// Fails on an empty `source_url` (no stable identity).
///
/// # Boundary
///
/// This function only handles PR/issue **metadata**. Bare commits MUST NOT
/// be passed through this path — the E2a⑤ boundary law is the caller's
/// responsibility and is enforced at the call site (not here).
pub fn import_prissue(pr: &ImportedPrIssue) -> Result<ProposedIntent, PrIssueImportError> {
    if pr.source_url.is_empty() {
        return Err(PrIssueImportError::EmptySourceUrl);
    }
    Ok(pr.to_proposed_intent())
}

/// Import a batch of PR/issue fixtures, skipping any with empty source URLs and
/// **deduplicating by `intent_id`**.
///
/// Returns only the successfully projected intents, with at most one intent per
/// distinct `intent_id`. Because `intent_id` is derived from `source_url`, two
/// fixtures for the same PR/issue collapse to a single intent — the importer
/// itself never emits a duplicate `intent_id` (re-sync of *changed* metadata is
/// still the caller's job via the same id, but a single batch never persists the
/// same intent twice). The **first** occurrence wins; later duplicates drop.
///
/// Each returned intent is non-authoritative and carries per-element provenance
/// for every preserved fidelity element.
pub fn import_prissue_batch(prs: &[ImportedPrIssue]) -> Vec<ProposedIntent> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<ProposedIntent> = Vec::new();
    for pr in prs {
        if let Ok(intent) = import_prissue(pr) {
            // Dedup by intent_id: never emit the same intent twice.
            if seen.insert(intent.sidecar.intent_id.clone()) {
                out.push(intent);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::prissue::model::{ElementProvenance, PrIssueState, SourceKind};

    fn fixture(url: &str) -> ImportedPrIssue {
        ImportedPrIssue {
            kind: SourceKind::PullRequest,
            source_id: 1,
            source_url: url.to_string(),
            body: "b".to_string(),
            body_provenance: ElementProvenance::new(url, "body"),
            state: PrIssueState::Open,
            state_provenance: ElementProvenance::new(url, "state"),
            labels: vec![],
            comments: vec![],
            cross_refs: vec![],
        }
    }

    #[test]
    fn batch_dedups_identical_fixtures_to_one_intent() {
        let url = "https://github.com/o/r/pull/1";
        let batch = import_prissue_batch(&[fixture(url), fixture(url)]);
        assert_eq!(
            batch.len(),
            1,
            "two identical PR fixtures must yield ONE intent"
        );
    }

    #[test]
    fn batch_keeps_distinct_intents() {
        let a = fixture("https://github.com/o/r/pull/1");
        let b = fixture("https://github.com/o/r/pull/2");
        let batch = import_prissue_batch(&[a, b]);
        assert_eq!(batch.len(), 2, "distinct PRs must yield distinct intents");
    }

    #[test]
    fn batch_skips_empty_source_url() {
        let batch = import_prissue_batch(&[fixture("")]);
        assert!(batch.is_empty());
    }
}
