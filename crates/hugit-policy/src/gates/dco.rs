//! DCO gate — every non-merge commit must carry a `Signed-off-by:` trailer.
//!
//! Mirrors `.github/workflows/dco.yml` exactly (WP-D6, item ①).

use crate::{EvalContext, GateOutcome};

/// Evaluate the DCO gate.
///
/// **Pass**: all commit messages contain at least one `Signed-off-by:` line.
/// **Fail**: one or more commits are missing the trailer.
///
/// Merge commits are exempt, mirroring the `--no-merges` flag in the GitHub
/// workflow.  A commit is a merge commit when its parent count (from
/// `ctx.commit_parent_counts`) is ≥ 2.  When `commit_parent_counts` has fewer
/// entries than `commit_messages` the missing counts default to 1 (regular
/// commit) — fail-closed for unknown commits.
pub fn eval(ctx: &EvalContext) -> GateOutcome {
    let mut missing: Vec<usize> = Vec::new();

    for (i, msg) in ctx.commit_messages.iter().enumerate() {
        // Exempt real merge commits: parent_count >= 2.
        // Missing entry → default 1 (not a merge) — fail-closed.
        let parent_count = ctx.commit_parent_counts.get(i).copied().unwrap_or(1);
        if parent_count >= 2 {
            continue;
        }
        if !msg.lines().any(|l| l.starts_with("Signed-off-by:")) {
            missing.push(i);
        }
    }

    if missing.is_empty() {
        GateOutcome::Pass
    } else {
        GateOutcome::Fail {
            reason: format!(
                "DCO: {} commit(s) missing Signed-off-by: trailer (indices: {:?})",
                missing.len(),
                missing
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EvalContext;

    fn ctx_with_msgs(msgs: &[&str]) -> EvalContext {
        let mut ctx = EvalContext::new();
        ctx.commit_messages = msgs.iter().map(|s| s.to_string()).collect();
        ctx
    }

    #[test]
    fn pass_when_all_signed() {
        let ctx = ctx_with_msgs(&["feat: add thing\n\nSigned-off-by: Alice <a@b.com>"]);
        assert_eq!(eval(&ctx), GateOutcome::Pass);
    }

    #[test]
    fn fail_when_missing() {
        let ctx = ctx_with_msgs(&["feat: add thing"]);
        assert!(matches!(eval(&ctx), GateOutcome::Fail { .. }));
    }

    #[test]
    fn exempt_merge_commits() {
        let mut ctx = EvalContext::new();
        ctx.commit_messages = vec!["Merge branch 'main' into feature".to_string()];
        ctx.commit_parent_counts = vec![2];
        assert_eq!(eval(&ctx), GateOutcome::Pass);
    }

    #[test]
    fn empty_commit_list_passes() {
        let ctx = ctx_with_msgs(&[]);
        assert_eq!(eval(&ctx), GateOutcome::Pass);
    }

    // ── Oracle: parent-count drives merge exemption, not message prefix ─────────
    // A non-merge commit whose subject starts "Merge " must NOT be exempted:
    // it still requires Signed-off-by:.
    #[test]
    fn non_merge_commit_with_merge_prefix_requires_signoff() {
        // parent_count == 1 (default) + subject starts with "Merge " → must fail
        let ctx = ctx_with_msgs(&["Merge notes for this quarter"]);
        assert!(
            matches!(eval(&ctx), GateOutcome::Fail { .. }),
            "a non-merge commit (parent_count=1) whose subject starts 'Merge ' \
             must NOT be exempted — DCO still required"
        );
    }

    // A real merge commit (parent_count >= 2) with "Merge " prefix is exempt.
    #[test]
    fn real_merge_commit_is_exempt() {
        let mut ctx = EvalContext::new();
        ctx.commit_messages = vec!["Merge branch 'main' into feature".to_string()];
        ctx.commit_parent_counts = vec![2];
        assert_eq!(
            eval(&ctx),
            GateOutcome::Pass,
            "a real merge commit (parent_count=2) must be exempt from DCO"
        );
    }

    // A real merge commit without "Merge " prefix is also exempt.
    #[test]
    fn real_merge_commit_without_merge_prefix_is_exempt() {
        let mut ctx = EvalContext::new();
        ctx.commit_messages = vec!["squash all the things".to_string()];
        ctx.commit_parent_counts = vec![2];
        assert_eq!(
            eval(&ctx),
            GateOutcome::Pass,
            "a real merge commit (parent_count=2) is exempt regardless of subject"
        );
    }
}
