//! N-4 parity acceptance test — policy gate detection must match the engine's
//! `is_structural_secret` across the full specimen matrix.
//!
//! These are the specimens the old hand-maintained prefix list MISSED:
//! - `clp_` (CoreLink PAT)
//! - `github_pat_` (fine-grained GitHub PAT)
//! - `xoxo-` / `xoxa-` / `xoxs-` (Slack token variants)
//! - High-entropy blob (caught via keyword-context secret)
//! - `postgres://u:pass@h/db` connection-string with embedded password
//!
//! Plus a legit-non-secret matrix to confirm no false positives.

use hugit_ledger::secret_shape::is_structural_secret;
use hugit_policy::{EvalContext, GateOutcome, gates::secrets};

fn ctx_with_file(path: &str, content: &str) -> EvalContext {
    let mut ctx = EvalContext::new();
    ctx.changed_files.push(path.to_string());
    ctx.file_contents
        .insert(path.to_string(), content.to_string());
    ctx
}

/// Feed a specimen to both the policy gate and `is_structural_secret`; assert
/// they agree, and assert the specimen is caught (not a legit value).
fn assert_caught(label: &str, content: &str) {
    let engine_verdict = is_structural_secret(content);
    let gate_verdict = secrets::eval(&ctx_with_file("probe.txt", content));

    assert!(
        engine_verdict,
        "N-4 parity: engine should flag '{label}' as a structural secret"
    );
    assert!(
        matches!(gate_verdict, GateOutcome::Fail { .. }),
        "N-4 parity: policy gate should flag '{label}', got {:?}",
        gate_verdict
    );
}

/// Assert a legit value passes BOTH the engine and the gate.
fn assert_clean(label: &str, content: &str) {
    let engine_verdict = is_structural_secret(content);
    let gate_verdict = secrets::eval(&ctx_with_file("probe.txt", content));

    assert!(
        !engine_verdict,
        "N-4 parity: engine must NOT flag clean value '{label}'"
    );
    assert_eq!(
        gate_verdict,
        GateOutcome::Pass,
        "N-4 parity: policy gate must NOT flag clean value '{label}', got {:?}",
        gate_verdict
    );
}

// ── Specimens the old hand-maintained list MISSED ────────────────────────────

#[test]
fn clp_corelink_pat_is_caught() {
    assert_caught(
        "clp_ CoreLink PAT",
        "let pat = \"clp_live_9f8e7d6c5b4a3210fedcba9876543210\";",
    );
}

#[test]
fn github_pat_fine_grained_is_caught() {
    assert_caught(
        "github_pat_ fine-grained PAT",
        "token: github_pat_11AABBCCDD0011223344556677889900aabbccdd",
    );
}

#[test]
fn slack_xoxo_is_caught() {
    assert_caught(
        "xoxo- Slack token",
        "SLACK_TOKEN=xoxo-222222222-333333333-444444444-abcdefghijklmno",
    );
}

#[test]
fn slack_xoxa_is_caught() {
    assert_caught(
        "xoxa- Slack token",
        "SLACK_TOKEN=xoxa-2-222222222-333333333-abcdefghijklmno",
    );
}

#[test]
fn slack_xoxs_is_caught() {
    assert_caught(
        "xoxs- Slack token",
        "SLACK_TOKEN=xoxs-2-222222222-333333333-444444444-abcdefghijklmno",
    );
}

#[test]
fn connection_string_with_password_is_caught() {
    assert_caught(
        "postgres connection string with embedded password",
        "DATABASE_URL=postgres://dbuser:S3cr3tP4ssw0rdVeryLongToken99@db.host.example.com:5432/mydb",
    );
}

#[test]
fn keyword_context_secret_is_caught() {
    // keyword-context: `secret=` followed by a non-whitespace value
    assert_caught(
        "keyword-context secret= assignment",
        "app_secret=veryLongHighEntropyValueThatShouldBeRedacted",
    );
}

// ── Specimens the old list DID catch — must still be caught ──────────────────

#[test]
fn aws_akia_still_caught() {
    assert_caught(
        "AWS AKIA access key",
        "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE",
    );
}

#[test]
fn ghp_token_still_caught() {
    assert_caught(
        "GitHub ghp_ token",
        "token = \"ghp_16C7e42F292c6912E7710c838347Ae178B4a\"",
    );
}

#[test]
fn slack_xoxb_still_caught() {
    assert_caught(
        "xoxb- Slack bot token",
        "SLACK_BOT_TOKEN=xoxb-1234567890-abcdefghijklmnopqrstuvwxyz",
    );
}

#[test]
fn pem_private_key_still_caught() {
    assert_caught(
        "PEM RSA private key header",
        "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEAxxxxx",
    );
}

// ── Legit non-secrets must pass ──────────────────────────────────────────────

#[test]
fn clean_rust_code_passes() {
    assert_clean(
        "clean Rust source",
        "fn main() { println!(\"hello, world!\"); }",
    );
}

#[test]
fn sha256_digest_passes() {
    assert_clean(
        "sha-256 hex digest",
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
    );
}

#[test]
fn slug_and_branch_pass() {
    assert_clean("git branch ref", "refs/heads/feature/auth-hardening");
}

#[test]
fn plain_integer_passes() {
    assert_clean("plain integer", "run_id=42");
}

#[test]
fn short_sk_prefix_passes() {
    // sk-256 is a key-length marker, not an API key — must NOT fire
    assert_clean("short sk- non-key", "algorithm=sk-256");
}
