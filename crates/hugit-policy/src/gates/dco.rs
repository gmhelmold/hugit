//! DCO gate — every non-merge commit must carry a `Signed-off-by:` trailer.
//!
//! Mirrors `.github/workflows/dco.yml` exactly (WP-D6, item ①).

use crate::{EvalContext, GateOutcome};

/// Evaluate the DCO gate.
///
/// **Pass**: all commit messages contain at least one `Signed-off-by:` line.
/// **Fail**: one or more commits are missing the trailer.
///
/// Merge commits (messages starting with `Merge `) are exempt, mirroring the
/// `--no-merges` flag in the GitHub workflow.
pub fn eval(ctx: &EvalContext) -> GateOutcome {
    let mut missing: Vec<usize> = Vec::new();

    for (i, msg) in ctx.commit_messages.iter().enumerate() {
        // Exempt merge commits (GitHub workflow uses --no-merges).
        if msg.trim_start().starts_with("Merge ") {
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
        let ctx = ctx_with_msgs(&["Merge branch 'main' into feature"]);
        assert_eq!(eval(&ctx), GateOutcome::Pass);
    }

    #[test]
    fn empty_commit_list_passes() {
        let ctx = ctx_with_msgs(&[]);
        assert_eq!(eval(&ctx), GateOutcome::Pass);
    }
}
