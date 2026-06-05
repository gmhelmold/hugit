//! Changelog gate — feat/fix commits require a non-empty `## [Unreleased]`
//! section in CHANGELOG.md.
//!
//! Mirrors `.github/scripts/changelog-gate.sh` exactly (WP-D6, item ①).

use crate::{EvalContext, GateOutcome};

/// Subject pattern for feat/fix commits: starts with `feat`, `fix` followed
/// by `(`, `:`, or `!`.
fn is_feat_or_fix(subject: &str) -> bool {
    let s = subject.trim();
    (s.starts_with("feat") || s.starts_with("fix"))
        && s.chars()
            .nth(4)
            .map(|c| matches!(c, '(' | ':' | '!'))
            .unwrap_or(false)
}

/// Evaluate the changelog gate.
///
/// **No-op** (Pass) when no feat/fix commits are present.
/// **Fail** when:
/// - feat/fix commits are present AND `CHANGELOG.md` is not in `changed_files`
/// - feat/fix commits are present AND the `## [Unreleased]` section is empty
///
/// **Pass** when feat/fix commits are present AND CHANGELOG.md is changed AND
/// `## [Unreleased]` is non-empty.
pub fn eval(ctx: &EvalContext) -> GateOutcome {
    let has_feat_fix = ctx.commit_messages.iter().any(|msg| {
        // First line is the subject
        let subject = msg.lines().next().unwrap_or("").trim();
        is_feat_or_fix(subject)
    });

    if !has_feat_fix {
        return GateOutcome::Pass;
    }

    // (a) CHANGELOG.md must be among changed files
    let changelog_changed = ctx.changed_files.iter().any(|f| f == "CHANGELOG.md");

    if !changelog_changed {
        return GateOutcome::Fail {
            reason: "changelog: feat/fix commit(s) present but CHANGELOG.md was not updated; add an entry under ## [Unreleased]".into(),
        };
    }

    // (b) The ## [Unreleased] section must be non-empty
    let content = match ctx.file_contents.get("CHANGELOG.md") {
        Some(c) => c.clone(),
        None => {
            return GateOutcome::Fail {
                reason: "changelog: CHANGELOG.md content not provided in eval context".into(),
            };
        }
    };

    let unreleased_nonempty = has_nonempty_unreleased(&content);
    if unreleased_nonempty {
        GateOutcome::Pass
    } else {
        GateOutcome::Fail {
            reason: "changelog: ## [Unreleased] section is empty; add at least one entry".into(),
        }
    }
}

/// Returns `true` if the `## [Unreleased]` section contains at least one
/// non-blank line of content.
fn has_nonempty_unreleased(content: &str) -> bool {
    let mut in_section = false;
    for line in content.lines() {
        if line.starts_with("## [Unreleased]") {
            in_section = true;
            continue;
        }
        if in_section {
            if line.starts_with("## ") {
                break;
            }
            if !line.trim().is_empty() {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EvalContext;
    use std::collections::HashMap;

    fn ctx(msgs: &[&str], changed: &[&str], contents: &[(&str, &str)]) -> EvalContext {
        let mut ctx = EvalContext::new();
        ctx.commit_messages = msgs.iter().map(|s| s.to_string()).collect();
        ctx.changed_files = changed.iter().map(|s| s.to_string()).collect();
        ctx.file_contents = contents
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect::<HashMap<_, _>>();
        ctx
    }

    const CHANGELOG_GOOD: &str =
        "# Changelog\n\n## [Unreleased]\n\n- feat: something cool\n\n## [0.1.0]\n";
    const CHANGELOG_EMPTY: &str = "# Changelog\n\n## [Unreleased]\n\n## [0.1.0]\n";

    #[test]
    fn no_feat_fix_passes() {
        let ctx = ctx(
            &["docs: update readme\n\nSigned-off-by: A <a@b.com>"],
            &[],
            &[],
        );
        assert_eq!(eval(&ctx), GateOutcome::Pass);
    }

    #[test]
    fn feat_with_good_changelog_passes() {
        let ctx = ctx(
            &["feat: add widget\n\nSigned-off-by: A <a@b.com>"],
            &["CHANGELOG.md"],
            &[("CHANGELOG.md", CHANGELOG_GOOD)],
        );
        assert_eq!(eval(&ctx), GateOutcome::Pass);
    }

    #[test]
    fn feat_missing_changelog_file_fails() {
        let ctx = ctx(
            &["feat: add widget\n\nSigned-off-by: A <a@b.com>"],
            &[],
            &[],
        );
        assert!(matches!(eval(&ctx), GateOutcome::Fail { .. }));
    }

    #[test]
    fn feat_empty_unreleased_fails() {
        let ctx = ctx(
            &["feat: add widget\n\nSigned-off-by: A <a@b.com>"],
            &["CHANGELOG.md"],
            &[("CHANGELOG.md", CHANGELOG_EMPTY)],
        );
        assert!(matches!(eval(&ctx), GateOutcome::Fail { .. }));
    }
}
