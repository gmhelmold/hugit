//! Acceptance — WP N-6 (LLM-SURFACE ergonomics): machine-surface SOTA for the
//! no-args / missing-subcommand case + unified `invalid_argument` kind.
//!
//! Findings addressed:
//!   F-5 (P1) — `hugit` with no subcommand previously exited 0 with plain-text
//!              help.  An orchestrating agent cannot distinguish that from success.
//!              Fix: the missing-subcommand case is a usage error → structured
//!              `{"error":{"kind":"invalid_argument",…}}` on STDOUT + exit 2.
//!              `--help`/`-h` explicitly requested stays exit 0.
//!   F-1 (P2) — `kind` was split: `"invalid_argument"` (singular, domain) vs
//!              `"invalid_arguments"` (plural, clap parse). Unified to the
//!              singular `"invalid_argument"` so an agent can match ONE stable kind.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

/// Run `hugit <args>` → (exit code, stdout string, parsed stdout JSON, stderr string).
fn run(args: &[&str]) -> (i32, String, Value, String) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary is reachable");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), stdout, v, stderr)
}

// ── F-5: no-args is a structured exit-2 error ────────────────────────────────

/// `hugit` with NO subcommand → structured envelope on STDOUT + exit 2.
/// An orchestrating agent that omits the subcommand gets a machine-parseable
/// error signal, not a fake success.
#[test]
fn no_args_exits_2_with_structured_envelope() {
    let (code, _stdout, v, _stderr) = run(&[]);
    assert_eq!(
        code, 2,
        "no subcommand is a usage error → exit 2 (the one exit-code law)"
    );
    assert_eq!(
        v.pointer("/error/kind").and_then(Value::as_str),
        Some("invalid_argument"),
        "envelope kind is `invalid_argument` (unified singular): {v}"
    );
    assert!(
        v.pointer("/error/message")
            .and_then(Value::as_str)
            .is_some(),
        "envelope carries a message: {v}"
    );
    assert!(
        v.pointer("/error/fix").and_then(Value::as_str).is_some(),
        "envelope carries a fix hint: {v}"
    );
}

/// The envelope from no-args is the canonical nested shape (never flat).
#[test]
fn no_args_envelope_is_nested_not_flat() {
    let (_, _stdout, v, _) = run(&[]);
    assert!(
        v.get("error").is_some(),
        "top-level `error` key must be present: {v}"
    );
    assert!(
        v.get("kind").is_none(),
        "kind must be nested under `error`, never at the top level: {v}"
    );
}

/// `hugit --help` (explicitly requested) stays exit 0 — that is a success, not
/// a botched dispatch.
#[test]
fn explicit_help_flag_is_exit_0() {
    let (code, stdout, _v, _stderr) = run(&["--help"]);
    assert_eq!(code, 0, "`hugit --help` must exit 0 (success): {stdout}");
    assert!(
        stdout.contains("Usage") || stdout.contains("hugit"),
        "help text lands on stdout: {stdout}"
    );
}

/// `hugit -h` (short form) is also exit 0.
#[test]
fn explicit_short_help_flag_is_exit_0() {
    let (code, stdout, _v, _stderr) = run(&["-h"]);
    assert_eq!(code, 0, "`hugit -h` must exit 0 (success): {stdout}");
    assert!(
        stdout.contains("Usage") || stdout.contains("hugit"),
        "short-help text lands on stdout: {stdout}"
    );
}

// ── F-5 + F-1: bad subcommand also uses the unified kind ─────────────────────

/// An unrecognised subcommand → the SAME envelope + exit 2, same kind.
#[test]
fn bad_subcommand_exits_2_with_unified_kind() {
    let (code, _stdout, v, _stderr) = run(&["frobnicate"]);
    assert_eq!(code, 2, "bad subcommand → exit 2");
    assert_eq!(
        v.pointer("/error/kind").and_then(Value::as_str),
        Some("invalid_argument"),
        "bad subcommand uses the unified `invalid_argument` kind: {v}"
    );
}

// ── F-1: kind consistency — domain validation also uses the same spelling ────

/// The `tournament` verb's domain-validation path uses `"invalid_argument"`
/// (singular).  This confirms the unified spelling is the same as the clap-parse
/// path above (no split between clap errors and domain-validation errors).
#[test]
fn tournament_domain_error_uses_same_singular_kind() {
    // `-n 0` is caught by `run_tournament`'s domain validation (not by clap).
    let (code, _stdout, v, _stderr) = run(&["tournament", "--n", "0", "--intent", "i-1"]);
    // clap may reject --n/n=0 before the domain check, that still yields the
    // same unified kind.
    assert_eq!(code, 2, "tournament domain error → exit 2");
    let kind = v
        .pointer("/error/kind")
        .and_then(Value::as_str)
        .unwrap_or("");
    assert_eq!(
        kind, "invalid_argument",
        "tournament domain validation uses the unified `invalid_argument` kind: {v}"
    );
}
