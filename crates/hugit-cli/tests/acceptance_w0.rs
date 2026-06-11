//! WP-W0 acceptance — the wedge-EXECUTE scaffold, against the REAL `hugit`
//! binary (PS-1, the wedge wave).
//!
//! W0 graduated `check` + `verdict` from RESERVED to LIVE dispatched verbs and
//! froze their flag seam (`--def --log [--store]` / `--intent --log [--store]`)
//! before the parallel builders filled the bodies. **W-INT landed those bodies
//! (the W-CHECK executor + the W-VERDICT recorder) behind the frozen seam**, so
//! this oracle now pins the DISPATCH + registry contract that survives the fill:
//!
//!   ① both verbs DISPATCH (they parse + route through main.rs, not an
//!      "unknown subcommand" clap error) — proven by reaching their own
//!      structured error path on a missing `--log`, NOT a clap usage error;
//!   ② the registry oracle holds: `check`/`verdict` are in `HUGIT_VERBS`, NOT in
//!      `HUGIT_RESERVED_VERBS`, and the two lists stay disjoint — the no-drift
//!      guarantee that must hold for the life of the verbs.
//!
//! The full EXECUTE behavior (cold→warm hit-rate, the null→real verdict proof)
//! is asserted in `acceptance_wcheck.rs` / `acceptance_wverdict.rs`; this file
//! pins only that the seam dispatches and the registry stays honest.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

/// Parse stdout as the canonical `{"error":{kind,…,fix}}` envelope (nested,
/// never flat; `fix` not `suggested_fix`) and return the structured `kind`.
/// Post-W-INT the verbs dispatch into their real bodies, so the envelope is the
/// verb's own structured fault (e.g. `log_not_found` / `missing_lens`), never
/// the W0 `not_implemented` stub.
fn assert_structured_error(stdout: &str) -> String {
    let v: Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|e| panic!("error must be JSON on stdout, got {stdout:?}: {e}"));
    assert!(v.get("error").is_some(), "error must be nested: {stdout}");
    assert!(v.get("kind").is_none(), "error must NOT be flat: {stdout}");
    assert!(v["error"]["fix"].is_string(), "fix is THE key: {stdout}");
    assert!(
        v["error"].get("suggested_fix").is_none(),
        "never suggested_fix: {stdout}"
    );
    v["error"]["kind"]
        .as_str()
        .unwrap_or_else(|| panic!("error.kind is a string: {stdout}"))
        .to_string()
}

fn assert_exit_two(out: &std::process::Output) {
    assert_eq!(
        out.status.code(),
        Some(2),
        "a structured-error path MUST exit 2; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

// ── ① `check` DISPATCHES into the real W-CHECK body (not a clap usage error) ──

#[test]
fn check_dispatches_into_its_real_body() {
    // A missing --log reaches the verb's OWN structured fault (log_not_found via
    // the --store recorder seam), proving dispatch — never the W0 stub, never a
    // clap "unknown subcommand". `fmt` is a built-in so no --cmd is needed.
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
    let kind = assert_structured_error(&String::from_utf8_lossy(&out.stdout));
    assert_ne!(kind, "not_implemented", "the W-CHECK body is live now");
    assert_eq!(
        kind, "log_not_found",
        "a missing --log under --store is the recorder's structured fault"
    );
}

#[test]
fn check_with_store_dispatches_into_its_real_body() {
    // The `--store` flag parses (the frozen seam) AND drives the real recorder —
    // a missing --log is log_not_found, never a fake "recorded" success.
    let out = Command::new(hugit_bin())
        .args([
            "check",
            "--def",
            "not-a-builtin",
            "--log",
            "/tmp/w0-no-such.json",
            "--store",
        ])
        .output()
        .expect("hugit runs");
    assert_exit_two(&out);
    let kind = assert_structured_error(&String::from_utf8_lossy(&out.stdout));
    // An unknown def with no --cmd is the resolver's own fault — proves dispatch
    // reached resolve_def, not the stub.
    assert_ne!(kind, "not_implemented", "the W-CHECK body is live now");
    assert_eq!(kind, "unknown_def", "dispatch reached the def resolver");
}

// ── ① `verdict` DISPATCHES into the real W-VERDICT body ──────────────────────

#[test]
fn verdict_dispatches_into_its_real_body() {
    // No lens/result pairs reaches the recorder's OWN validation fault
    // (missing_lens), proving dispatch — never the W0 stub.
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
    let kind = assert_structured_error(&String::from_utf8_lossy(&out.stdout));
    assert_ne!(kind, "not_implemented", "the W-VERDICT body is live now");
    assert_eq!(
        kind, "missing_lens",
        "dispatch reached the verdict recorder's own validation"
    );
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
