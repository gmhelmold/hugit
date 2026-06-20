//! Secrets gate — reject changes that introduce obvious secret patterns.
//!
//! Mirrors the house's CI secrets enforcement (WP-D6, item ①).
//! Checks file contents for well-known secret signatures by routing every
//! detection decision through the full redaction engine:
//! [`hugit_ledger::redact::apply`].
//!
//! N-4: previously this gate kept its own hand-maintained prefix list that
//! had diverged from the engine detector (`hugit_ledger::secret_shape`),
//! missing `clp_`, `github_pat_`, `xoxo-`/`xoxa-`/`xoxs-`, entropy scan,
//! connection-string-password detection, and the `cas:` value-gate.  The
//! third scrub copy is deleted; detection now comes from the single source
//! of truth (Wave M / M-2).
//!
//! N-5: upgraded from `is_structural_secret` (which missed high-entropy
//! unprefixed secrets) to `redact::apply` — a secret is flagged when
//! `apply(content) != content`, i.e. the full 5-detector engine (structural
//! classes + entropy scan + bare-hex credential shapes) detects something.

use crate::{EvalContext, GateOutcome};
use hugit_ledger::redact::apply as redact_apply;

/// Evaluate the secrets gate.
///
/// **Pass**: no file content contains a secret as detected by the full
/// 5-detector redaction engine ([`hugit_ledger::redact::apply`]).
/// **Fail**: at least one file introduces a secret pattern (structural classes,
/// entropy scan, or bare-hex credential shapes).
/// **Blocked**: a changed file has no content entry in the context — fail-closed;
///   we cannot vouch for content we have not seen.
///
/// Every file listed in `ctx.changed_files` must have a corresponding entry in
/// `ctx.file_contents`.  A missing entry means the caller did not supply the
/// content; blocking is the only safe action.
pub fn eval(ctx: &EvalContext) -> GateOutcome {
    let mut hits: Vec<String> = Vec::new();
    let mut missing_content: Vec<String> = Vec::new();

    for path in &ctx.changed_files {
        match ctx.file_contents.get(path) {
            Some(content) => {
                // Route through the full 5-detector redaction engine. A secret
                // is present when apply() returns the REDACTED sentinel rather
                // than the original string. This covers: structural prefixes
                // (ghp_, gho_, ghs_, github_pat_, AKIA, xoxb-/xoxp-/xoxo-/
                // xoxa-/xoxs-, clp_, dop_v1_, Bearer <ws>, eyJ), PEM private
                // key headers, connection-string passwords, keyword-context
                // secrets (password=, access_token=, private_key=, …), AND
                // high-entropy unprefixed secrets + bare-hex credential shapes
                // that is_structural_secret alone would miss.
                if redact_apply(content) != content.as_str() {
                    hits.push(format!("{path}: matched secret pattern"));
                }
            }
            None => {
                missing_content.push(path.clone());
            }
        }
    }

    // Fail-closed: a changed file with no content cannot be scanned.
    if !missing_content.is_empty() {
        return GateOutcome::Blocked {
            reason: format!(
                "secrets: {} changed file(s) have no content in eval context \
                 (cannot scan — fail-closed): {}",
                missing_content.len(),
                missing_content.join(", ")
            ),
        };
    }

    if hits.is_empty() {
        GateOutcome::Pass
    } else {
        GateOutcome::Fail {
            reason: format!(
                "secrets: {} file(s) contain suspected secret patterns: {}",
                hits.len(),
                hits.join("; ")
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EvalContext;

    fn ctx_with_file(path: &str, content: &str) -> EvalContext {
        let mut ctx = EvalContext::new();
        ctx.changed_files.push(path.to_string());
        ctx.file_contents
            .insert(path.to_string(), content.to_string());
        ctx
    }

    #[test]
    fn clean_file_passes() {
        let ctx = ctx_with_file("src/main.rs", "fn main() { println!(\"hello\"); }");
        assert_eq!(eval(&ctx), GateOutcome::Pass);
    }

    #[test]
    fn aws_key_fails() {
        let ctx = ctx_with_file("config.env", "AWS_KEY=AKIAIOSFODNN7EXAMPLE");
        assert!(matches!(eval(&ctx), GateOutcome::Fail { .. }));
    }

    #[test]
    fn private_key_fails() {
        let ctx = ctx_with_file(
            "key.pem",
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAK...\n-----END RSA PRIVATE KEY-----",
        );
        assert!(matches!(eval(&ctx), GateOutcome::Fail { .. }));
    }

    #[test]
    fn github_token_fails() {
        let ctx = ctx_with_file(
            "src/client.rs",
            "let token = \"ghp_abcdefghijklmnopqrstuvwxyz\";",
        );
        assert!(matches!(eval(&ctx), GateOutcome::Fail { .. }));
    }

    #[test]
    fn no_changed_files_passes() {
        let ctx = EvalContext::new();
        assert_eq!(eval(&ctx), GateOutcome::Pass);
    }

    // ── Oracle: changed file with no content entry → Blocked (fail-closed) ─────
    // Previously the gate silently skipped files with no content, allowing a
    // changed file to bypass secret scanning. Fail-closed: missing content → Blocked.
    #[test]
    fn changed_file_without_content_is_blocked() {
        let mut ctx = EvalContext::new();
        ctx.changed_files.push("src/secret.rs".to_string());
        // Intentionally NOT inserting into file_contents
        let outcome = eval(&ctx);
        assert!(
            matches!(outcome, GateOutcome::Blocked { .. }),
            "a changed file with no content entry must produce Blocked, got {:?}",
            outcome
        );
    }
    // ── Boundary tests ──────────────────────────────────────────────────────────

    /// A URL containing `password=` in a query string triggers the matcher.
    ///
    /// `password=` anywhere in the line is flagged, including in URLs such as
    /// `https://db.example.com/connect?password=secret`. This IS a true positive
    /// for the typical case (a literal password in a URL is a secret leak) but
    /// could be a false positive for a URL with a `password=` parameter that
    /// holds only a placeholder. The fail-closed behavior is correct here.
    #[test]
    fn url_with_password_param_is_flagged() {
        let ctx = ctx_with_file(
            "config.toml",
            r#"db_url = "https://db.example.com/connect?password=hunter2""#,
        );
        // The keyword-context scanner fires on `password=` — intentionally fail-closed.
        assert!(
            matches!(eval(&ctx), GateOutcome::Fail { .. }),
            "a URL containing `password=` must be flagged (fail-closed)"
        );
    }

    // ── Fix 5: high-entropy unprefixed secrets caught via redact::apply ──────────

    /// An AWS secret access key (40-char base64 — NOT an AKIA prefix) must be
    /// flagged by the policy gate via the entropy scan. Previously the gate only
    /// called `is_structural_secret` which misses this class.
    #[test]
    fn aws_secret_key_high_entropy_fails() {
        // wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY — the canonical AWS example
        // secret access key. It has no known prefix but high entropy (base64 mix).
        let ctx = ctx_with_file(
            "config.toml",
            "AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY",
        );
        assert!(
            matches!(eval(&ctx), GateOutcome::Fail { .. }),
            "an AWS secret access key must be flagged by the policy gate"
        );
    }

    /// A high-entropy unprefixed 32-char base64 secret must be caught.
    #[test]
    fn high_entropy_unprefixed_secret_fails() {
        // A 32-char dense random base64 blob with no known prefix.
        let ctx = ctx_with_file(
            "src/config.rs",
            r#"let secret = "8Kp2mZ9qLx4vTn7wRj3sYb6cFd1gHe0";"#,
        );
        assert!(
            matches!(eval(&ctx), GateOutcome::Fail { .. }),
            "a high-entropy unprefixed credential must be caught by the policy gate"
        );
    }

    /// Clean file with no secrets passes the gate.
    #[test]
    fn clean_code_passes_gate() {
        let ctx = ctx_with_file(
            "src/lib.rs",
            "pub fn greet(name: &str) -> String { format!(\"Hello, {}!\", name) }",
        );
        assert_eq!(eval(&ctx), GateOutcome::Pass);
    }
}
