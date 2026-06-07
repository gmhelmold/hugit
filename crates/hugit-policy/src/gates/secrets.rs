//! Secrets gate — reject changes that introduce obvious secret patterns.
//!
//! Mirrors the house's CI secrets enforcement (WP-D6, item ①).
//! Checks file contents for well-known secret signatures:
//! AWS access keys, generic API keys, bearer tokens, private key headers,
//! and password assignments.

use crate::{EvalContext, GateOutcome};

/// Known secret patterns as (label, pattern) pairs.
/// Each pattern is a simple string prefix or substring check for maximum
/// portability (no regex dependency in this crate).
static SECRET_PATTERNS: &[(&str, &str)] = &[
    ("AWS access key", "AKIA"),
    ("AWS secret key prefix", "aws_secret_access_key"),
    ("generic API key", "api_key="),
    ("generic API key upper", "API_KEY="),
    ("bearer token", "Bearer "),
    ("private key header", "-----BEGIN RSA PRIVATE KEY-----"),
    ("private key header (EC)", "-----BEGIN EC PRIVATE KEY-----"),
    (
        "private key header (OPENSSH)",
        "-----BEGIN OPENSSH PRIVATE KEY-----",
    ),
    ("password assignment", "password="),
    ("password assignment upper", "PASSWORD="),
    ("token assignment", "token="),
    ("token assignment upper", "TOKEN="),
    ("github token", "ghp_"),
    ("github actions token", "ghs_"),
    ("slack token", "xoxb-"),
    ("slack token (user)", "xoxp-"),
];

/// Evaluate the secrets gate.
///
/// **Pass**: no file content contains a known secret pattern.
/// **Fail**: at least one file introduces a known secret pattern.
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
                for (label, pattern) in SECRET_PATTERNS {
                    if content.contains(pattern) {
                        hits.push(format!("{path}: matched pattern '{label}'"));
                        break; // one hit per file is enough
                    }
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
}
