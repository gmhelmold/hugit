//! WP-E-CLI acceptance — Wave E porcelain fixes, all over the REAL `hugit`
//! binary (`CARGO_BIN_EXE_hugit`).
//!
//! 1. **Redaction parity (P-REDACT-SURFACE).** A real `ghp_…` PAT (and a
//!    connection string) supplied in a charter / acceptance / campaign / owner /
//!    reason MUST NOT reach `.hugit/intents.json`, the canonical `--log`, OR any
//!    read-surface echo (`intent show`/`list`, `campaign list`/`open`) verbatim.
//!    The write path scrubs through the hardened engine before persisting (the
//!    log is hash-chained + forever — redact-before-append is the only fix); the
//!    read surfaces scrub again as defence-in-depth.
//! 2. **pr error-law + verify_chain (P-PR-LAW).** `pr show/list/land/abandon` on
//!    a missing / corrupt / TAMPERED `--log` emit the canonical
//!    `{"error":{kind,…}}` JSON on stdout + exit 2 (no plaintext-stderr/exit-1).
//! 3. **campaign missing-log (P-CAMPAIGN-EMPTY).** `campaign show`/`list` on a
//!    MISSING `--log` emit `log_not_found` exit-2 (never a silent empty world);
//!    `campaign open`'s bootstrap-on-absent is preserved.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-ecli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Run `hugit <args>`; return the raw process output (so we can grep stdout AND
/// inspect exit codes byte-exact).
fn run(args: &[&str]) -> Output {
    Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs")
}

fn stdout_of(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn json_of(out: &Output) -> Value {
    serde_json::from_str(stdout_of(out).trim()).unwrap_or(Value::Null)
}

/// A realistically-shaped GitHub classic PAT (the adversary's specimen). Never a
/// real credential — but the exact shape the `ghp_` detector keys on.
const REAL_SHAPE_PAT: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4aZ9";
/// A connection string with embedded password (the engine catches it).
const CONN_STRING: &str =
    "postgres://admin:S3cr3tP4ssw0rdVeryLongRandomToken9999@db.internal:5432/app";
const REDACTED: &str = "[REDACTED]";

// ─────────────────────────────────────────────────────────────────────────────
// 1. REDACTION PARITY — write + read surfaces (P-REDACT-SURFACE)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn intent_new_redacts_pat_on_every_write_and_read_surface() {
    let dir = scratch("redact-intent");
    let store = dir.join("intents.json");
    let log = dir.join("log.json");
    let charter = format!("Wire deploy with token {REAL_SHAPE_PAT} immediately");

    let out = run(&[
        "intent",
        "new",
        "--store",
        store.to_str().unwrap(),
        "--log",
        log.to_str().unwrap(),
        "--campaign",
        "ecli-redact",
        "--charter",
        &charter,
        "--acceptance",
        &format!("connect via {CONN_STRING}"),
    ]);
    assert!(
        out.status.success(),
        "intent new exits 0: {}",
        stdout_of(&out)
    );
    let created = json_of(&out);
    let id = created["intent_id"].as_str().unwrap().to_string();

    // ── WRITE surface 1: the store file ──────────────────────────────────────
    let store_bytes = std::fs::read_to_string(&store).unwrap();
    assert!(
        !store_bytes.contains(REAL_SHAPE_PAT),
        "the PAT must NOT persist verbatim in .hugit/intents.json:\n{store_bytes}"
    );
    assert!(
        !store_bytes.contains("S3cr3tP4ssw0rdVeryLongRandomToken9999"),
        "the connection-string password must NOT persist verbatim in the store"
    );
    assert!(
        store_bytes.contains(REDACTED),
        "the store carries the redaction sentinel instead of the secret"
    );

    // ── WRITE surface 2: the canonical hash-chained log ──────────────────────
    let log_bytes = std::fs::read_to_string(&log).unwrap();
    assert!(
        !log_bytes.contains(REAL_SHAPE_PAT),
        "the PAT must NOT persist verbatim in the canonical --log payload (forever):\n{log_bytes}"
    );

    // ── READ surface 1: intent show ──────────────────────────────────────────
    let show = run(&[
        "intent",
        "show",
        "--store",
        store.to_str().unwrap(),
        "--intent",
        &id,
    ]);
    let show_s = stdout_of(&show);
    assert!(show.status.success(), "intent show exits 0: {show_s}");
    assert!(
        !show_s.contains(REAL_SHAPE_PAT)
            && !show_s.contains("S3cr3tP4ssw0rdVeryLongRandomToken9999"),
        "intent show must NOT echo the secret verbatim:\n{show_s}"
    );

    // ── READ surface 2: intent list ──────────────────────────────────────────
    let list = run(&[
        "intent",
        "list",
        "--store",
        store.to_str().unwrap(),
        "--log",
        log.to_str().unwrap(),
    ]);
    let list_s = stdout_of(&list);
    assert!(list.status.success(), "intent list exits 0: {list_s}");
    assert!(
        !list_s.contains(REAL_SHAPE_PAT),
        "intent list must NOT echo the secret verbatim:\n{list_s}"
    );
}

#[test]
fn campaign_open_redacts_secrets_in_charter_owner_on_write_and_read() {
    let dir = scratch("redact-campaign");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();

    let out = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        "ecli-camp",
        "--charter",
        &format!("ship it; key {REAL_SHAPE_PAT}"),
        "--owner",
        "owner@example.com",
    ]);
    assert!(
        out.status.success(),
        "campaign open exits 0: {}",
        stdout_of(&out)
    );
    // open's own success echo is redacted.
    assert!(
        !stdout_of(&out).contains(REAL_SHAPE_PAT),
        "campaign open echo must NOT carry the secret: {}",
        stdout_of(&out)
    );

    // WRITE: the hash-chained log payload carries no verbatim secret.
    let log_bytes = std::fs::read_to_string(&log).unwrap();
    assert!(
        !log_bytes.contains(REAL_SHAPE_PAT),
        "the PAT must NOT persist verbatim in the campaign.opened payload:\n{log_bytes}"
    );

    // READ: campaign list echoes redacted charter.
    let list = run(&["campaign", "list", "--log", log_s]);
    let list_s = stdout_of(&list);
    assert!(list.status.success(), "campaign list exits 0: {list_s}");
    assert!(
        !list_s.contains(REAL_SHAPE_PAT),
        "campaign list must NOT echo the secret verbatim:\n{list_s}"
    );

    // READ: idempotent re-open echoes the redacted projected charter.
    let reopen = run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        "ecli-camp",
        "--charter",
        "ignored on re-open",
        "--owner",
        "owner@example.com",
    ]);
    let reopen_s = stdout_of(&reopen);
    assert!(reopen.status.success());
    assert!(
        !reopen_s.contains(REAL_SHAPE_PAT),
        "re-open echo must NOT carry the secret from the projected record:\n{reopen_s}"
    );
}

#[test]
fn campaign_abandon_redacts_secret_in_reason() {
    let dir = scratch("redact-abandon");
    let log = dir.join("log.json");
    let log_s = log.to_str().unwrap();

    run(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        "ecli-ab",
        "--charter",
        "c",
        "--owner",
        "o@h.com",
    ]);
    let out = run(&[
        "campaign",
        "abandon",
        "--log",
        log_s,
        "--campaign",
        "ecli-ab",
        "--reason",
        &format!("leaked {REAL_SHAPE_PAT}; rotating"),
    ]);
    assert!(out.status.success(), "abandon exits 0: {}", stdout_of(&out));
    assert!(
        !stdout_of(&out).contains(REAL_SHAPE_PAT),
        "abandon echo must NOT carry the secret reason: {}",
        stdout_of(&out)
    );
    let log_bytes = std::fs::read_to_string(&log).unwrap();
    assert!(
        !log_bytes.contains(REAL_SHAPE_PAT),
        "the secret reason must NOT persist verbatim in campaign.abandoned:\n{log_bytes}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. PR ERROR-LAW + verify_chain (P-PR-LAW)
// ─────────────────────────────────────────────────────────────────────────────

fn assert_pr_error_law(out: &Output, expect_kind: &str) {
    assert_eq!(
        out.status.code(),
        Some(2),
        "pr read fault MUST exit 2 (the one exit-code law); stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v = json_of(out);
    assert!(v.get("error").is_some(), "nested under error: {v}");
    assert!(v.get("kind").is_none(), "never flat: {v}");
    assert_eq!(v["error"]["kind"], expect_kind, "kind: {v}");
    assert!(
        v["error"]["fix"].is_string(),
        "fix is THE remediation key: {v}"
    );
}

#[test]
fn pr_show_list_land_abandon_missing_log_emit_log_not_found_exit_two() {
    let dir = scratch("pr-missing");
    let absent = dir.join("nope.json");
    let a = absent.to_str().unwrap();

    assert_pr_error_law(
        &run(&["pr", "show", "--log", a, "--pr", "1"]),
        "log_not_found",
    );
    assert_pr_error_law(&run(&["pr", "list", "--log", a]), "log_not_found");
    assert_pr_error_law(
        &run(&["pr", "land", "--log", a, "--pr", "1"]),
        "log_not_found",
    );
    assert_pr_error_law(
        &run(&["pr", "abandon", "--log", a, "--pr", "1", "--reason", "x"]),
        "log_not_found",
    );
}

#[test]
fn pr_corrupt_log_emits_parse_log_on_stdout_exit_two() {
    let dir = scratch("pr-corrupt");
    let log = dir.join("L.json");
    std::fs::write(&log, br#"{"events":[]}"#).unwrap(); // valid JSON, wrong shape
    let out = run(&["pr", "show", "--log", log.to_str().unwrap(), "--pr", "1"]);
    assert_pr_error_law(&out, "parse_log");
}

#[test]
fn pr_read_path_rejects_a_tampered_chain() {
    // Build a real 2-record log via the binary, then corrupt a `this_hash` so the
    // chain no longer verifies. The pr READ path must reject it (verify_chain) —
    // it previously skipped the check the siblings run.
    let dir = scratch("pr-tamper");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap();
    let open = |pr: &str| {
        run(&[
            "pr",
            "open",
            "--log",
            log_s,
            "--pr",
            pr,
            "--campaign",
            "c",
            "--author-kind",
            "orchestrator",
            "--run-id",
            "r",
            "--intent",
            "i1",
        ])
    };
    assert!(open("1").status.success());
    assert!(open("2").status.success());

    // Tamper: flip a hex digit in the LAST record's this_hash.
    let mut records: Vec<Value> = serde_json::from_slice(&std::fs::read(&log).unwrap()).unwrap();
    let last = records.last_mut().unwrap();
    let h = last["this_hash"].as_str().unwrap().to_string();
    let flipped: String = {
        let mut c: Vec<char> = h.chars().collect();
        c[0] = if c[0] == 'a' { 'b' } else { 'a' };
        c.into_iter().collect()
    };
    last["this_hash"] = Value::String(flipped);
    std::fs::write(&log, serde_json::to_vec(&records).unwrap()).unwrap();

    let out = run(&["pr", "show", "--log", log_s, "--pr", "1"]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "a tampered chain MUST be rejected on the pr read path; stdout: {}",
        stdout_of(&out)
    );
    let v = json_of(&out);
    // A broken chain surfaces as either a rehydrate (push_record) or chain_broken
    // (verify_chain) fault — both are structured, exit-2, fix-keyed.
    let kind = v["error"]["kind"].as_str().unwrap_or("");
    assert!(
        kind == "chain_broken" || kind == "rehydrate",
        "tampered chain is a structured integrity fault (chain_broken/rehydrate), got {v}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. CAMPAIGN MISSING-LOG (P-CAMPAIGN-EMPTY)
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn campaign_show_missing_log_is_log_not_found_not_empty_world() {
    let dir = scratch("camp-show-missing");
    let absent = dir.join("nope.json");
    let out = run(&[
        "campaign",
        "show",
        "--log",
        absent.to_str().unwrap(),
        "--campaign",
        "x",
    ]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "missing --log on a read-only query MUST exit 2, never a silent empty world: {}",
        stdout_of(&out)
    );
    let v = json_of(&out);
    assert_eq!(v["error"]["kind"], "log_not_found", "{v}");
    assert!(v["error"]["fix"].is_string());
}

#[test]
fn campaign_list_missing_log_is_log_not_found_not_empty_world() {
    let dir = scratch("camp-list-missing");
    let absent = dir.join("nope.json");
    let out = run(&["campaign", "list", "--log", absent.to_str().unwrap()]);
    assert_eq!(
        out.status.code(),
        Some(2),
        "missing --log on campaign list MUST exit 2: {}",
        stdout_of(&out)
    );
    assert_eq!(json_of(&out)["error"]["kind"], "log_not_found");
}

#[test]
fn campaign_open_still_bootstraps_on_an_absent_log() {
    // The read-only fix must NOT break open's create-on-absent behavior.
    let dir = scratch("camp-open-bootstrap");
    let log = dir.join("fresh.json");
    assert!(!log.exists(), "precondition: the log is absent");
    let out = run(&[
        "campaign",
        "open",
        "--log",
        log.to_str().unwrap(),
        "--campaign",
        "boot",
        "--charter",
        "c",
        "--owner",
        "o@h.com",
    ]);
    assert!(
        out.status.success(),
        "campaign open MUST still bootstrap an absent log (exit 0): {}",
        stdout_of(&out)
    );
    assert!(log.exists(), "open created the log");
    // And now show finds it (exit 0).
    let show = run(&[
        "campaign",
        "show",
        "--log",
        log.to_str().unwrap(),
        "--campaign",
        "boot",
    ]);
    assert!(
        show.status.success(),
        "show reads the now-existing log: {}",
        stdout_of(&show)
    );
}
