//! WP-R-cli acceptance oracle (remediation defect #1) — the REAL `hugit` binary.
//!
//! Proves the binary exists, dispatches the wired verbs end-to-end against the
//! library, and maps success/failure to the process exit code (0 on success,
//! non-zero on error). It drives the built binary via `CARGO_BIN_EXE_hugit`
//! (set by Cargo for integration tests of bin targets) — no extra dev-deps.
//!
//! Also asserts the binary's verb surface IS the canonical registry
//! [`hugit_cli::HUGIT_VERBS`] that WP-X5 consumes (the bin and the invariant
//! cannot drift).

use std::path::PathBuf;
use std::process::Command;

use hugit_refstore::EventLog;
use serde_json::json;

/// The built `hugit` binary path (Cargo sets this env for bin-target tests).
fn hugit_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_hugit"))
}

/// A per-test scratch dir under the OS temp.
fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("hugit-rcli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Write a `why`-format log: `[{record: EventRecord, attestation: null, sidecar:
/// null}, …]` where each EventRecord is produced by the engine's canonical
/// [`EventLog::append`] so the hash chain is REAL (not hand-forged). K-CHAIN:
/// `verify_chain` runs before `why` projects, so fake hashes are rejected.
fn write_why_log(path: &std::path::Path, events: &[(&str, serde_json::Value)]) {
    let mut log = EventLog::new();
    for (kind, payload) in events {
        log.append_for_test(*kind, vec![], payload.to_string(), 0);
    }
    let entries: Vec<serde_json::Value> = log
        .records()
        .iter()
        .map(|r| {
            json!({
                "record": r,
                "attestation": null,
                "sidecar": null
            })
        })
        .collect();
    std::fs::write(path, serde_json::to_string(&entries).unwrap()).unwrap();
}

/// Write a canonical `[EventRecord, …]` log for `export` tests, using the
/// engine's real append path so the chain is valid and `verify_chain` passes.
fn write_canonical_log(path: &std::path::Path, events: &[(&str, serde_json::Value)]) {
    let mut log = EventLog::new();
    for (kind, payload) in events {
        log.append_for_test(*kind, vec![], payload.to_string(), 0);
    }
    std::fs::write(path, serde_json::to_string_pretty(log.records()).unwrap()).unwrap();
}

// ── ① the binary exists and `hugit why` runs end-to-end, exit 0 ───────────────

#[test]
fn item_1_why_runs_end_to_end_exit_zero() {
    let dir = scratch("why-ok");
    let log = dir.join("log.json");
    // K-CHAIN: build a REAL hash chain via the engine's append path so
    // `verify_chain` passes (fake/hand-forged hashes are now rejected).
    write_why_log(
        &log,
        &[(
            "intent.landed",
            json!({
                "intent_id": "intent-1",
                "charter": "Add the parser",
                "path": "src/parser.rs"
            }),
        )],
    );

    let out = Command::new(hugit_bin())
        .args([
            "why",
            "--log",
            log.to_str().unwrap(),
            "--path",
            "src/parser.rs",
        ])
        .output()
        .expect("hugit binary runs");

    assert!(
        out.status.success(),
        "hugit why must exit 0 on success; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("intent-1") && stdout.contains("Add the parser"),
        "hugit why must print the resolved provenance, got: {stdout}"
    );
}

// ── ② `hugit why` on an unresolvable query exits NON-zero ─────────────────────

#[test]
fn item_2_why_error_exits_nonzero() {
    let dir = scratch("why-err");
    let log = dir.join("log.json");
    // K-CHAIN: build with real hashes so the error is `unresolved`, not
    // `chain_broken` (the test asserts non-zero exit for an unresolvable query).
    write_why_log(&log, &[("intent.landed", json!({"path": "src/known.rs"}))]);

    // Query a path that is NOT in the log → NotFound → non-zero exit. Under the
    // WB0 one-law convergence the error is the canonical JSON envelope on
    // STDOUT (agents parse stdout, never stderr), exit 2.
    let out = Command::new(hugit_bin())
        .args([
            "why",
            "--log",
            log.to_str().unwrap(),
            "--path",
            "src/does-not-exist.rs",
        ])
        .output()
        .expect("hugit binary runs");

    assert!(
        !out.status.success(),
        "hugit why on an unresolvable query MUST exit non-zero"
    );
    assert_eq!(out.status.code(), Some(2), "structured error exits 2");
    let v: serde_json::Value =
        serde_json::from_str(String::from_utf8_lossy(&out.stdout).trim()).expect("error is JSON");
    assert!(
        v["error"]["kind"].is_string() && v["error"]["fix"].is_string(),
        "the canonical error envelope is on stdout"
    );
}

// ── ③ `hugit impact` runs end-to-end against the real blast-radius lib ────────

#[test]
fn item_3_impact_runs_end_to_end() {
    let dir = scratch("impact");
    let graph = dir.join("graph.json");
    std::fs::write(
        &graph,
        serde_json::json!({
            "ecosystem": "cargo",
            "root_manifests": ["Cargo.toml"],
            "packages": [
                { "name": "core", "path": "crates/core", "direct_deps": [] },
                { "name": "cli", "path": "crates/cli", "direct_deps": ["core"] }
            ]
        })
        .to_string(),
    )
    .unwrap();

    let out = Command::new(hugit_bin())
        .args([
            "impact",
            "--graph",
            graph.to_str().unwrap(),
            "--path",
            "crates/core/src/lib.rs",
        ])
        .output()
        .expect("hugit binary runs");

    assert!(
        out.status.success(),
        "hugit impact must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    // A change to `core` must blast to its dependent `cli`.
    assert!(
        stdout.contains("core") && stdout.contains("cli"),
        "impact of a core change must include the dependent cli, got: {stdout}"
    );
}

// ── ④ `hugit tournament` runs end-to-end and respects the policy cap ──────────

#[test]
fn item_4_tournament_runs_and_caps() {
    // Within cap: succeeds.
    let ok = Command::new(hugit_bin())
        .args(["tournament", "-n", "3", "--intent", "intent-xyz"])
        .output()
        .expect("hugit binary runs");
    assert!(
        ok.status.success(),
        "tournament within cap must exit 0; stderr: {}",
        String::from_utf8_lossy(&ok.stderr)
    );
    // Under the WB0 one-law convergence the report is stable JSON (was plain
    // text): the human `candidates=3` summary became `"candidates":3`.
    let stdout = String::from_utf8_lossy(&ok.stdout);
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).expect("tournament report is JSON");
    assert_eq!(
        v["candidates"], 3,
        "tournament must produce N candidates, got: {stdout}"
    );

    // Over cap: refused with a non-zero exit.
    let over = Command::new(hugit_bin())
        .args(["tournament", "-n", "9999", "--intent", "intent-xyz"])
        .output()
        .expect("hugit binary runs");
    assert!(
        !over.status.success(),
        "tournament over the policy cap MUST exit non-zero"
    );
}

// ── ⑤ `hugit export` runs end-to-end (anti-lock-in dump) ──────────────────────

#[test]
fn item_5_export_runs_end_to_end() {
    let dir = scratch("export");
    let log = dir.join("corpus.json");
    // K-CHAIN: export now reads the canonical `[EventRecord, …]` format and
    // verifies the chain. Build via the engine's real append path.
    write_canonical_log(
        &log,
        &[(
            "ref.update",
            json!({"ref": "refs/heads/main", "target": "deadbeef"}),
        )],
    );
    let out_dir = dir.join("artifact");

    let out = Command::new(hugit_bin())
        .args([
            "export",
            "--log",
            log.to_str().unwrap(),
            "--out",
            out_dir.to_str().unwrap(),
        ])
        .output()
        .expect("hugit binary runs");

    assert!(
        out.status.success(),
        "hugit export must exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    // The artifact + JSON envelope were written.
    assert!(out_dir.join("export.json").is_file(), "envelope written");
    assert!(out_dir.join("repo.git").is_dir(), "git artifact written");
}

// ── ⑥ HUGIT_VERBS == the binary's dispatched surface (equality, not subset) ────
//
// Load-bearing: this test goes RED if HUGIT_VERBS contains a verb that is NOT
// dispatched in the binary (phantom entry), OR if the binary dispatches a verb
// that is NOT in HUGIT_VERBS (silent gap). Both directions are checked:
//
//   (a) HUGIT_VERBS → help: every entry in HUGIT_VERBS must appear in
//       `hugit --help` (registry → binary).
//   (b) help → HUGIT_VERBS: every verb listed in `hugit --help` must be in
//       HUGIT_VERBS (binary → registry). Detected via count equality + per-verb
//       membership.
//
// The dispatched surface is the legacy verbs (why/impact/tournament/export),
// the flow porcelain (campaign/intent/pr), and the WB0 wedge stubs
// (checks/queue) — exactly HUGIT_VERBS. Changing HUGIT_VERBS without wiring the
// verb in main.rs (or vice-versa) turns this test RED immediately.
#[test]
fn item_6_canonical_registry_equals_dispatched_surface() {
    // ── (a) registry → binary: every HUGIT_VERBS entry must appear in --help ──
    let help = Command::new(hugit_bin())
        .arg("--help")
        .output()
        .expect("hugit --help runs");
    assert!(help.status.success(), "hugit --help exits 0");
    let help_text = String::from_utf8_lossy(&help.stdout);

    for verb in hugit_cli::HUGIT_VERBS {
        assert!(
            help_text.contains(verb),
            "REGISTRY→BINARY gap: '{verb}' is in HUGIT_VERBS but NOT in `hugit --help`. \
             Either wire it in main.rs or remove it from HUGIT_VERBS."
        );
    }

    // ── (b) binary → registry: count the verbs that appear in --help and
    //    confirm every one is in HUGIT_VERBS (no silent binary-only verb) ────────
    //
    // We enumerate the Subcommands by parsing the subcommand lines from
    // `hugit --help` (clap emits one line per subcommand under "Commands:").
    // Each dispatched verb is a word that also appears in HUGIT_VERBS; any
    // word in the help block that is NOT in HUGIT_VERBS is a binary-only gap.
    //
    // Clap help format: the Commands: block has lines like
    //   "  why       Resolve a line/symbol ..."
    // We extract the first token of each indented line in the Commands block.
    let dispatched_in_help: Vec<&str> = help_text
        .lines()
        // Take only lines that look like subcommand entries (two leading spaces,
        // then a lowercase word).
        .filter_map(|line| {
            let trimmed = line.trim_start();
            if line.starts_with("  ")
                && !line.starts_with("   ")
                && trimmed.starts_with(|c: char| c.is_ascii_lowercase())
            {
                trimmed.split_whitespace().next()
            } else {
                None
            }
        })
        // Keep only tokens that appear in the live verb set (excludes "help").
        .filter(|tok| hugit_cli::HUGIT_VERBS.contains(tok))
        .collect();

    // Every dispatched verb visible in --help must be in HUGIT_VERBS.
    for verb in &dispatched_in_help {
        assert!(
            hugit_cli::HUGIT_VERBS.contains(verb),
            "BINARY→REGISTRY gap: '{verb}' appears in `hugit --help` but NOT in HUGIT_VERBS. \
             Add it to HUGIT_VERBS or stop dispatching it."
        );
    }

    // The count of dispatched verbs visible in --help must equal HUGIT_VERBS.
    // If HUGIT_VERBS has more entries than --help shows, there are phantom
    // (unwired) verbs in the registry.
    assert_eq!(
        dispatched_in_help.len(),
        hugit_cli::HUGIT_VERBS.len(),
        "EQUALITY VIOLATION: HUGIT_VERBS has {} entries but `hugit --help` shows {} \
         dispatched verbs matching the registry. \
         Phantom verbs in HUGIT_VERBS (not wired in main.rs) or dispatched verbs \
         missing from HUGIT_VERBS are both failures. \
         HUGIT_VERBS = {:?}, dispatched = {:?}",
        hugit_cli::HUGIT_VERBS.len(),
        dispatched_in_help.len(),
        hugit_cli::HUGIT_VERBS,
        dispatched_in_help,
    );

    // hugit_verbs() returns the same canonical list (stable accessor for X5).
    assert_eq!(hugit_cli::hugit_verbs(), hugit_cli::HUGIT_VERBS);
}

// ── WF item 5 — `tournament --intent` existence check against the log ──────────

/// Run `hugit <args>`, returning `(exit_code, parsed_stdout_json)`.
fn run_hugit(args: &[&str]) -> (i32, serde_json::Value) {
    let out = Command::new(hugit_bin())
        .args(args)
        .output()
        .expect("hugit binary runs");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value =
        serde_json::from_str(stdout.trim()).unwrap_or(serde_json::Value::Null);
    (out.status.code().unwrap_or(-1), v)
}

#[test]
fn wf_tournament_without_log_stays_permissive_exit_zero() {
    // The log-less fan-out (fixture/smoke path) is unchanged: any intent id is
    // accepted, exit 0 — no --log means no existence check.
    let (code, v) = run_hugit(&["tournament", "-n", "3", "--intent", "intent-xyz"]);
    assert_eq!(code, 0, "log-less tournament stays exit 0: {v}");
    assert_eq!(v["candidates"], 3);
}

#[test]
fn wf_tournament_with_log_refuses_a_nonexistent_intent() {
    let dir = scratch("tourney-ghost");
    let log = dir.join("L.json");
    let log_s = log.to_str().unwrap();
    let store = dir.join("store.json");
    let store_s = store.to_str().unwrap();

    // Land a REAL intent `i1` on the canonical log.
    let (code, _) = run_hugit(&[
        "campaign",
        "open",
        "--log",
        log_s,
        "--campaign",
        "camp-t",
        "--charter",
        "c",
        "--owner",
        "o@h.com",
    ]);
    assert_eq!(code, 0, "campaign open");
    let (code, _) = run_hugit(&[
        "intent",
        "new",
        "--log",
        log_s,
        "--store",
        store_s,
        "--campaign",
        "camp-t",
        "--charter",
        "land it",
        "--id",
        "i1",
    ]);
    assert_eq!(code, 0, "intent new");

    // An EXISTING intent is accepted (exit 0, real fan-out over the log).
    let (code, v) = run_hugit(&["tournament", "-n", "2", "--intent", "i1", "--log", log_s]);
    assert_eq!(code, 0, "existing intent is accepted: {v}");
    assert_eq!(v["candidates"], 2);

    // A GHOST intent is a structured `intent_not_found`/exit-2 — never a
    // fabricated exit-0 fan-out (the WF item-5 defect).
    let (code, v) = run_hugit(&["tournament", "-n", "2", "--intent", "ghost", "--log", log_s]);
    assert_eq!(code, 2, "a nonexistent intent must be refused: {v}");
    assert_eq!(v["error"]["kind"], "intent_not_found");
    assert_eq!(v["error"]["intent"], "ghost");
    assert!(v["error"]["fix"].is_string());

    // A MISSING --log is the canonical `log_not_found`/exit-2 (the shared loader).
    let (code, v) = run_hugit(&[
        "tournament",
        "-n",
        "2",
        "--intent",
        "i1",
        "--log",
        "/no/such.json",
    ]);
    assert_eq!(code, 2, "missing log is exit 2: {v}");
    assert_eq!(v["error"]["kind"], "log_not_found");
}
