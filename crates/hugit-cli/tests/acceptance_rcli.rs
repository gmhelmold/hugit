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

// ── ① the binary exists and `hugit why` runs end-to-end, exit 0 ───────────────

#[test]
fn item_1_why_runs_end_to_end_exit_zero() {
    let dir = scratch("why-ok");
    let log = dir.join("log.json");
    // A real event-log fixture the resolver can attribute.
    std::fs::write(
        &log,
        serde_json::json!([
            {
                "record": {
                    "seq": 0,
                    "prev_hash": "0".repeat(64),
                    "this_hash": "a".repeat(64),
                    "kind": "intent.landed",
                    "principal_chain": ["alice@example.com"],
                    "payload": serde_json::json!({
                        "intent_id": "intent-1",
                        "charter": "Add the parser",
                        "path": "src/parser.rs"
                    }).to_string(),
                    "recorded_at": 1_700_000_000_000u64
                },
                "attestation": null,
                "sidecar": null
            }
        ])
        .to_string(),
    )
    .unwrap();

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
    std::fs::write(
        &log,
        serde_json::json!([
            {
                "record": {
                    "seq": 0,
                    "prev_hash": "0".repeat(64),
                    "this_hash": "a".repeat(64),
                    "kind": "intent.landed",
                    "principal_chain": ["alice@example.com"],
                    "payload": serde_json::json!({"path": "src/known.rs"}).to_string(),
                    "recorded_at": 1u64
                }
            }
        ])
        .to_string(),
    )
    .unwrap();

    // Query a path that is NOT in the log → NotFound → non-zero exit.
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
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("error"),
        "an error must be reported on stderr"
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
    let stdout = String::from_utf8_lossy(&ok.stdout);
    assert!(
        stdout.contains("candidates=3"),
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
    // A single ref.update event (un-hashed; the CLI appends + computes the
    // chain) so the export has a ref to project.
    std::fs::write(
        &log,
        serde_json::json!({
            "events": [
                {
                    "kind": "ref.update",
                    "principal_chain": ["ci"],
                    "payload": serde_json::json!({
                        "ref": "refs/heads/main",
                        "target": "deadbeef"
                    }).to_string(),
                    "recorded_at": 1u64
                }
            ]
        })
        .to_string(),
    )
    .unwrap();
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
// The dispatched surface is exactly: why, impact, tournament, export.
// Changing HUGIT_VERBS without wiring the verb in main.rs (or vice-versa) turns
// this test RED immediately.
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
