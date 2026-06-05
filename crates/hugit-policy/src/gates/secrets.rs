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
///
/// Only inspects files listed in `ctx.changed_files` whose content is
/// present in `ctx.file_contents`.
pub fn eval(ctx: &EvalContext) -> GateOutcome {
    let mut hits: Vec<String> = Vec::new();

    for path in &ctx.changed_files {
        if let Some(content) = ctx.file_contents.get(path) {
            for (label, pattern) in SECRET_PATTERNS {
                if content.contains(pattern) {
                    hits.push(format!("{path}: matched pattern '{label}'"));
                    break; // one hit per file is enough
                }
            }
        }
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
}
