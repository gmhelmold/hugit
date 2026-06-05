//! Exactly-one-edited PR comment — item ② of WP-B7.
//!
//! The App maintains a SINGLE PR comment and edits it in place (upsert by a
//! stable marker), never posting a second. Comment count == 1 per PR.

use hugit_contracts::CheckResult;
use serde::{Deserialize, Serialize};

/// Stable HTML marker embedded in every hugit status comment.
///
/// GitHub renders HTML comments as invisible; this marker lets the App
/// identify its own comment for upsert, guaranteeing exactly-one comment/PR.
pub const COMMENT_MARKER: &str = "<!-- hugit-status-comment-v1 -->";

/// A rendered PR status comment body (item ②).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RenderedComment {
    /// The full comment body, starting with `COMMENT_MARKER`.
    pub body: String,
    /// GitHub PR number this comment targets.
    pub pr_number: u64,
}

/// Renders and upserts the single hugit status comment per PR.
pub struct CommentRenderer;

impl CommentRenderer {
    /// Render the status comment body for a PR.
    ///
    /// The body always starts with [`COMMENT_MARKER`] so the App can locate
    /// and edit (never duplicate) its comment. Saved-minutes figures link to
    /// their `CheckResult` audit refs (item ③).
    pub fn render(
        pr_number: u64,
        saved_minutes: u64,
        audit_ref: &str,
        check_results: &[CheckResult],
        dollar_saved: &str,
    ) -> RenderedComment {
        let result_list = check_results
            .iter()
            .map(|r| format!("  - `{}` (duration: {}ms)", r.memo_key, r.duration_ms))
            .collect::<Vec<_>>()
            .join("\n");

        let body = format!(
            "{marker}\n\
             ## hugit CI Status — PR #{pr_number}\n\n\
             **Saved:** {saved_minutes} min ({dollar_saved}) · [audit]({audit_ref})\n\n\
             <details><summary>CheckResult set ({n} entries)</summary>\n\n\
             {result_list}\n\
             </details>\n",
            marker = COMMENT_MARKER,
            pr_number = pr_number,
            saved_minutes = saved_minutes,
            dollar_saved = dollar_saved,
            audit_ref = audit_ref,
            n = check_results.len(),
            result_list = result_list,
        );

        RenderedComment { body, pr_number }
    }

    /// Returns the stable upsert marker used to identify the comment.
    ///
    /// Callers: search PR comments for this string; update if found, post if absent.
    /// This ensures comment count == 1 per PR at all times.
    pub fn stable_marker() -> &'static str {
        COMMENT_MARKER
    }

    /// Assert that a comment body carries the stable marker (upsert guard).
    pub fn has_stable_marker(body: &str) -> bool {
        body.contains(COMMENT_MARKER)
    }
}
