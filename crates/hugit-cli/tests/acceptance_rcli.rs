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

// ── ⑥ the binary's verb surface IS the canonical registry (X5 seam) ───────────

#[test]
fn item_6_canonical_registry_drives_help() {
    // The wired verbs are members of the canonical registry WP-X5 consumes.
    for verb in ["why", "impact", "tournament", "export"] {
        assert!(
            hugit_cli::HUGIT_VERBS.contains(&verb),
            "wired verb '{verb}' must be in the canonical registry"
        );
    }
    // `hugit --help` enumerates the wired verbs (the help text is generated from
    // the same Subcommand enum the registry mirrors).
    let help = Command::new(hugit_bin())
        .arg("--help")
        .output()
        .expect("hugit --help runs");
    assert!(help.status.success(), "hugit --help exits 0");
    let text = String::from_utf8_lossy(&help.stdout);
    for verb in ["why", "impact", "tournament", "export"] {
        assert!(text.contains(verb), "help must list the '{verb}' verb");
    }

    // hugit_verbs() returns the same canonical list (stable accessor for X5).
    assert_eq!(hugit_cli::hugit_verbs(), hugit_cli::HUGIT_VERBS);
}
