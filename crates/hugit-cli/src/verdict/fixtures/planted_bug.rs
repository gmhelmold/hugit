//! Planted-bug fixture (WP-D7 ③).
//!
//! The change ships a **semantic/logic** bug of a class the author's own tests
//! do not cover: the change touches a transitive dependency, but the served
//! build-graph impact omits a downstream target that the change can break. The
//! author suite (which only exercises the directly-edited target) passes — the
//! bug is *demonstrably uncovered* by any author test.
//!
//! A semantic-invariant reviewer lens — judging ONLY served ground truth —
//! still catches it: it cross-checks the served `check_results` against the
//! served `impact` set and finds a check that exercised a target absent from
//! the declared impact (an impact-under-report, a classic logic bug).

use std::sync::Arc;

use hugit_contracts::CheckResult;
use hugit_contracts::Verdict;
use hugit_contracts::check_result::Artifact;

use crate::verdict::panel_dispatch::{Lens, Reviewer, ReviewerInput, ServedGroundTruth};

/// The directly-edited target the author's tests exercise.
pub const EDITED_TARGET: &str = "crate-core";

/// A downstream target the change can break but that the served impact OMITS.
/// This omission is the planted semantic bug.
pub const OMITTED_DOWNSTREAM: &str = "crate-downstream";

/// Build served ground truth that CARRIES the planted bug: a check result
/// exercised `crate-downstream`, yet the declared `impact` lists only
/// `crate-core`.
pub fn ground_truth_with_planted_bug() -> ServedGroundTruth {
    let downstream_check = CheckResult {
        memo_key: "memo-downstream".into(),
        tree_hash: "tree-1".into(),
        def_digest: "def-test-downstream".into(),
        toolchain_digest: "tc-1".into(),
        exit: 0,
        artifacts: vec![Artifact {
            // The artifact path reveals the check ran against the downstream
            // target — evidence the impact set should have included it.
            path: "target/crate-downstream/test-report.json".into(),
            digest: "art-downstream".into(),
        }],
        stdout_ref: "blob://stdout-downstream".into(),
        stderr_ref: "blob://stderr-downstream".into(),
        duration_ms: 12,
        runner_ref: "runner-1".into(),
        produced_at: 1_700_000_000_000,
    };

    ServedGroundTruth::from_served(
        "intent-planted",
        "tree-1",
        // The BUG: impact omits `crate-downstream`.
        vec![EDITED_TARGET.to_string()],
        vec!["contract-digest-core".to_string()],
        vec![downstream_check],
        vec![
            "blob://stdout-downstream".to_string(),
            "blob://impact-report".to_string(),
        ],
    )
}

/// The semantic-invariant reviewer: it judges served ground truth and FLAGS
/// (REJECTs) when a served check exercised a target absent from the declared
/// impact set — an impact-under-report logic bug.
#[derive(Debug, Default)]
pub struct SemanticInvariantReviewer;

impl SemanticInvariantReviewer {
    /// The targets a served check demonstrably exercised, inferred from its
    /// artifact paths (`target/<name>/...`).
    fn targets_exercised(cr: &CheckResult) -> Vec<String> {
        cr.artifacts
            .iter()
            .filter_map(|a| {
                let rest = a.path.strip_prefix("target/")?;
                rest.split('/').next().map(|s| s.to_string())
            })
            .collect()
    }
}

impl Reviewer for SemanticInvariantReviewer {
    fn review(&self, input: &ReviewerInput) -> (Verdict, Vec<String>) {
        let gt = &input.ground_truth;
        let mut claims = Vec::new();
        for cr in &gt.check_results {
            for target in Self::targets_exercised(cr) {
                if !gt.impact.iter().any(|t| t == &target) {
                    claims.push(format!(
                        "impact-under-report: served check exercised '{target}' \
                         but declared impact omits it"
                    ));
                    return (Verdict::Reject, claims);
                }
            }
        }
        claims.push("impact set consistent with served checks".into());
        (Verdict::Approve, claims)
    }
}

/// A lens wrapping the semantic-invariant reviewer.
pub fn semantic_lens() -> Lens {
    Lens::new(
        "semantic-invariant",
        "You are the SEMANTIC-INVARIANT lens. Cross-check served check results \
         against the served impact set; reject any impact-under-report.",
        "model-gamma",
        Arc::new(SemanticInvariantReviewer),
    )
}

/// Emulates the author's OWN test suite over this change: it exercises only the
/// directly-edited target and passes — demonstrating the bug is uncovered by
/// any author test. Returns `true` (author suite green).
pub fn author_suite_passes() -> bool {
    let gt = ground_truth_with_planted_bug();
    // The author only checks that the edited target is in scope and its check
    // exited 0. This is GREEN — and blind to the downstream omission.
    let edited_in_impact = gt.impact.iter().any(|t| t == EDITED_TARGET);
    let edited_check_ok = gt.check_results.iter().all(|cr| cr.exit == 0);
    edited_in_impact && edited_check_ok
}
