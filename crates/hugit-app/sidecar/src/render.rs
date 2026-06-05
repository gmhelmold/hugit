//! Render a validated `IntentSidecar` as a PR comment and a check summary.
//!
//! ① parsed/validated/rendered — render charter + acceptance + context-ref
//! as a PR comment AND a check summary via AppWebhooks.

use hugit_contracts::IntentSidecar;

/// Output of rendering a validated `IntentSidecar`.
#[derive(Debug, Clone, PartialEq)]
pub struct RenderOutput {
    /// Rendered PR comment body (Markdown).
    pub pr_comment: String,
    /// Rendered check summary (shorter, for GitHub Checks API).
    pub check_summary: String,
}

/// Render a validated `IntentSidecar` into a PR comment and check summary.
///
/// The PR comment contains charter, acceptance criteria, and the context-ref.
/// The check summary is a condensed version for the GitHub Checks API output.
pub fn render_sidecar(sidecar: &IntentSidecar) -> RenderOutput {
    let pr_comment = render_pr_comment(sidecar);
    let check_summary = render_check_summary(sidecar);
    RenderOutput {
        pr_comment,
        check_summary,
    }
}

fn render_pr_comment(sidecar: &IntentSidecar) -> String {
    let mut lines = vec![
        "## hugit Intent Sidecar".to_string(),
        "".to_string(),
        format!("**Intent ID:** `{}`", sidecar.intent_id),
        "".to_string(),
        "### Charter".to_string(),
        "".to_string(),
        sidecar.charter.clone(),
        "".to_string(),
        "### Acceptance Criteria".to_string(),
        "".to_string(),
    ];

    for (i, criterion) in sidecar.acceptance.iter().enumerate() {
        lines.push(format!("{}. {}", i + 1, criterion));
    }

    lines.push("".to_string());
    lines.push("### Context Reference".to_string());
    lines.push("".to_string());
    lines.push(format!("`{}`", sidecar.context_ref));
    lines.push("".to_string());
    lines.push(
        "> This sidecar is **non-authoritative** — it does not gate or block landing.".to_string(),
    );

    lines.join("\n")
}

fn render_check_summary(sidecar: &IntentSidecar) -> String {
    let acceptance_count = sidecar.acceptance.len();
    format!(
        "Intent `{}` — {} acceptance item(s) | context: `{}` | non-authoritative",
        sidecar.intent_id, acceptance_count, sidecar.context_ref
    )
}
