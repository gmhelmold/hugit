//! WP-W0 acceptance — the wedge-EXECUTE scaffold, against the REAL `hugit`
//! binary (PS-1, the wedge wave).
//!
//! W0 graduates `check` + `verdict` from RESERVED to LIVE dispatched verbs and
//! freezes their flag seam (`--def --log [--store]` / `--intent --log [--store]`)
//! before the parallel builders fill the bodies (W-CHECK / W-VERDICT). This
//! oracle pins the SCAFFOLD contract — and ONLY the scaffold:
//!
//!   ① both new verbs DISPATCH (they parse + route through main.rs, not an
//!      "unknown subcommand" clap error);
//!   ② each emits the canonical NOT-IMPLEMENTED envelope on stdout naming its
//!      owning WP (`W-CHECK` / `W-VERDICT`), exit `2` (the WB0 one-error/one-exit
//!      law) — an honest stub, never a fake success;
//!   ③ the registry oracle holds: `check`/`verdict` are in `HUGIT_VERBS`, NOT in
//!      `HUGIT_RESERVED_VERBS`, and the two lists stay disjoint.
//!
//! The EXECUTE behavior (real memoized run / panel dispatch / the `--store`
//! recorder onto the canonical log) is W-CHECK / W-VERDICT's, not W0's; this
//! file must NOT grow assertions about it.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

/// Assert stdout is the canonical NOT-IMPLEMENTED envelope naming `wp`, nested
/// under `error`, with `fix` (never `suggested_fix`) as THE remediation key.
fn assert_not_implemented(stdout: &str, wp: &str) {
    let v: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("stub must be JSON on stdout, got {stdout:?}: {e}"));
    assert!(v.get("error").is_some(), "error must be nested: {stdout}");
    assert!(v.get("kind").is_none(), "error must NOT be flat: {stdout}");
    assert_eq!(v["error"]["kind"], "not_implemented", "kind: {stdout}");
    assert_eq!(
        v["error"]["wp"], wp,
        "the stub names its owning WP: {stdout}"
    );
    assert!(v["error"]["fix"].is_string(), "fix is THE key: {stdout}");
    assert!(
        v["error"].get("suggested_fix").is_none(),
        "never suggested_fix: {stdout}"
    );
}

fn assert_exit_two(out: &std::process::Output) {
    assert_eq!(
        out.status.code(),
        Some(2),
        "the NOT-IMPLEMENTED stub MUST exit 2; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── ① + ② `check` dispatches + emits the W-CHECK stub, exit 2 ────────────────

#[test]
fn check_dispatches_and_emits_the_w_check_stub() {
    let out = Command::new(hugit_bin())
        .args(["check", "--def", "fmt", "--log", "/tmp/w0-no-such.json"])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    assert_not_implemented(&String::from_utf8_lossy(&out.stdout), "W-CHECK");
}

#[test]
fn check_with_store_still_emits_the_w_check_stub() {
    // The `--store` flag parses (the seam is frozen) but the recorder body is
    // W-CHECK's — the stub still fires, never a fake "recorded" success.
    let out = Command::new(hugit_bin())
        .args([
            "check",
            "--def",
            "fmt",
            "--log",
            "/tmp/w0-no-such.json",
            "--store",
        ])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    assert_not_implemented(&String::from_utf8_lossy(&out.stdout), "W-CHECK");
}

// ── ① + ② `verdict` dispatches + emits the W-VERDICT stub, exit 2 ────────────

#[test]
fn verdict_dispatches_and_emits_the_w_verdict_stub() {
    let out = Command::new(hugit_bin())
        .args([
            "verdict",
            "--intent",
            "intent-1",
            "--log",
            "/tmp/w0-no-such.json",
        ])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    assert_not_implemented(&String::from_utf8_lossy(&out.stdout), "W-VERDICT");
}

// ── ③ the registry oracle: check/verdict are LIVE, not RESERVED, disjoint ────

#[test]
fn check_and_verdict_are_in_the_live_registry_not_reserved() {
    for verb in ["check", "verdict"] {
        assert!(
            hugit_cli::HUGIT_VERBS.contains(&verb),
            "'{verb}' must be a LIVE dispatched verb (in HUGIT_VERBS) at W0"
        );
        assert!(
            !hugit_cli::HUGIT_RESERVED_VERBS.contains(&verb),
            "'{verb}' graduated out of HUGIT_RESERVED_VERBS at W0"
        );
    }
}

#[test]
fn live_and_reserved_verb_lists_stay_disjoint() {
    for v in hugit_cli::HUGIT_VERBS {
        assert!(
            !hugit_cli::HUGIT_RESERVED_VERBS.contains(v),
            "verb '{v}' is in BOTH HUGIT_VERBS and HUGIT_RESERVED_VERBS — a live \
             verb can never also be reserved"
        );
    }
}
