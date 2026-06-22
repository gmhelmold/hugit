//! Acceptance — Round-8 C6 (Wave L, WP L-C) + N-6 ergonomics unification: the
//! clap argument-error path obeys the ONE error law (the typed `ac_busy`
//! taxonomy is unit-tested in `checks::run`'s module, where `map_exec_error`
//! lives).
//!
//! ROOT (audit class-6-errorlaw F-2): `Cli::parse()` let clap own the failure
//! path — it printed a bare English `error:` to STDERR and exited BEFORE `main`'s
//! envelope choke-point, so an orchestrating agent parsing STDOUT for
//! `{"error":{"kind",…}}` got empty stdout + an unparseable string. The L-C fix
//! routes `Cli::try_parse()` through the envelope: a bad invocation emits
//! `{"error":{"kind":"invalid_argument",…}}` on STDOUT with exit 2.
//!
//! N-6 F-1: the kind is unified to `"invalid_argument"` (singular, matching the
//! domain-validation spelling used everywhere else — no split between clap-parse
//! and domain kinds for the same error class).

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

/// Run `hugit <args>` → (exit, stdout_string, parsed_stdout_json, stderr_string).
fn run(args: &[&str]) -> (i32, String, Value, String) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit runs");
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let v: Value = serde_json::from_str(stdout.trim()).unwrap_or(Value::Null);
    (out.status.code().unwrap_or(-1), stdout, v, stderr)
}

/// An unknown flag on a real verb → the structured envelope on STDOUT, exit 2.
#[test]
fn unknown_flag_emits_invalid_arguments_envelope_on_stdout_exit_2() {
    let (code, _stdout, v, _stderr) = run(&["verdict", "record", "--bogusflag"]);
    assert_eq!(
        code, 2,
        "clap arg error exits 2 (the user/domain error law)"
    );
    assert_eq!(
        v.pointer("/error/kind").and_then(Value::as_str),
        Some("invalid_argument"),
        "the bad-args path renders the canonical envelope on STDOUT: {v}"
    );
    assert!(
        v.pointer("/error/message")
            .and_then(Value::as_str)
            .is_some(),
        "the envelope carries a message: {v}"
    );
    assert!(
        v.pointer("/error/fix").and_then(Value::as_str).is_some(),
        "the envelope carries a fix: {v}"
    );
}

/// An unrecognized subcommand → the same envelope, exit 2 (whole sub-class).
#[test]
fn unknown_subcommand_emits_invalid_arguments_envelope() {
    let (code, _stdout, v, _stderr) = run(&["frobnicate"]);
    assert_eq!(code, 2);
    assert_eq!(
        v.pointer("/error/kind").and_then(Value::as_str),
        Some("invalid_argument"),
        "an unknown subcommand obeys the error law on STDOUT: {v}"
    );
}

/// Missing required args → the envelope on STDOUT, not a bare stderr string.
#[test]
fn missing_required_args_emits_envelope_on_stdout() {
    // `why` requires --log and --path; omitting them is a clap arg error.
    let (code, _stdout, v, _stderr) = run(&["why"]);
    assert_eq!(code, 2);
    assert_eq!(
        v.pointer("/error/kind").and_then(Value::as_str),
        Some("invalid_argument"),
        "missing-required-args yields a machine-parseable envelope on STDOUT: {v}"
    );
}

/// `--help` is NOT an error: clap renders help and exits 0 (stdout carries it,
/// the envelope path is bypassed for the help/version success kinds).
#[test]
fn help_is_success_exit_0() {
    let (code, stdout, _v, _stderr) = run(&["--help"]);
    assert_eq!(code, 0, "--help is the success path, exit 0");
    assert!(
        stdout.contains("Usage") || stdout.contains("hugit"),
        "help text lands on stdout: {stdout}"
    );
}
