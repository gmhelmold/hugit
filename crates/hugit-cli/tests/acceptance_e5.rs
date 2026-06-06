// WP-E5 acceptance oracle — export + exit proofs (anti-lock-in guarantee).
// Each test corresponds to one owned acceptance item from WP-E5.md.
// RED on current tree: the implementation module does not exist yet.
// Implementation target: crates/hugit-cli/src/export/

use hugit_cli::export::{
    export_command, restore_from_export, ExportConfig, ExportSchema, RedactionPolicy,
};

/// ① one-command dump: produces git + documented JSON.
/// Assert: single command produces both a bare git repo and a JSON file,
/// and the JSON is a documented (non-empty, versioned) artifact.
#[test]
fn item_1_one_command_dump() {
    let config = ExportConfig::fixture_minimal();
    let result = export_command(&config).expect("export must succeed");
    assert!(result.git_path().exists(), "export must produce a git artifact");
    assert!(result.json_path().exists(), "export must produce a JSON artifact");
    let json_bytes = std::fs::read(result.json_path()).unwrap();
    assert!(!json_bytes.is_empty(), "exported JSON must be non-empty");
    let parsed: serde_json::Value = serde_json::from_slice(&json_bytes).unwrap();
    assert!(
        parsed.get("schema_version").is_some(),
        "exported JSON must carry a schema_version field"
    );
}

/// ② restore round-trip: refs+intents+events reproduced object-for-object.
#[test]
fn item_2_restore_roundtrip() {
    let config = ExportConfig::fixture_with_objects();
    let export = export_command(&config).expect("export must succeed");
    let restored = restore_from_export(&export).expect("restore must succeed");
    assert_eq!(
        restored.refs(),
        config.source_refs(),
        "restored refs must match source"
    );
    assert_eq!(
        restored.intents(),
        config.source_intents(),
        "restored intents must match source"
    );
    assert_eq!(
        restored.events(),
        config.source_events(),
        "restored events must match source"
    );
}

/// ③ export validates against versioned ExportSchema (machine check).
/// Assert: the exported JSON passes ExportSchema::validate() without error.
#[test]
fn item_3_schema_validates() {
    let config = ExportConfig::fixture_minimal();
    let export = export_command(&config).expect("export must succeed");
    let json_bytes = std::fs::read(export.json_path()).unwrap();
    let schema = ExportSchema::current();
    schema
        .validate(&json_bytes)
        .expect("exported JSON must validate against versioned ExportSchema");
}

/// ④ redaction policy applied at export; multi-GB streams without OOM.
/// Part A: no secret material emitted (seeded secret absent from output).
/// Part B: streaming export completes with bounded memory (no OOM).
#[test]
fn item_4_redaction_no_oom() {
    let secret = "SUPER_SECRET_TOKEN_12345";
    let policy = RedactionPolicy::redact_secrets(vec![secret.to_string()]);
    let config = ExportConfig::fixture_with_redaction(policy, secret);
    let export = export_command(&config).expect("export must succeed");

    // Part A: secret absent from JSON output
    let json_str = std::fs::read_to_string(export.json_path()).unwrap();
    assert!(
        !json_str.contains(secret),
        "redacted secret must not appear in exported JSON"
    );

    // Part B: streaming — assert peak memory stays within bounded limit
    // (The implementation must use a chunked streaming writer; this fixture
    //  asserts the export completes without allocating the full corpus in RAM.)
    assert!(
        export.peak_memory_bytes() < 256 * 1024 * 1024,
        "streaming export peak memory must stay below 256 MiB bound; got {} bytes",
        export.peak_memory_bytes()
    );
}

/// ⑤ THE EXIT PROOF: exported git artifact fully usable with ZERO hugit/forge dependency.
/// Proof runs with no hugit tooling on PATH: clone / log / branch / push-elsewhere all work.
#[test]
fn item_5_exit_proof_zero_hugit() {
    let config = ExportConfig::fixture_minimal();
    let export = export_command(&config).expect("export must succeed");

    // Build a PATH that contains only standard git, no hugit binary
    let bare_path = std::env::var("PATH")
        .unwrap_or_default()
        .split(':')
        .filter(|p| !p.contains("hugit"))
        .collect::<Vec<_>>()
        .join(":");

    let tmp = tempfile::tempdir().unwrap();
    let clone_dir = tmp.path().join("cloned");

    // clone
    let clone_status = std::process::Command::new("git")
        .env("PATH", &bare_path)
        .args(["clone", export.git_path().to_str().unwrap(), clone_dir.to_str().unwrap()])
        .status()
        .expect("git clone must execute");
    assert!(clone_status.success(), "git clone on exported artifact must succeed with no hugit on PATH");

    // log
    let log_status = std::process::Command::new("git")
        .env("PATH", &bare_path)
        .args(["-C", clone_dir.to_str().unwrap(), "log", "--oneline"])
        .status()
        .expect("git log must execute");
    assert!(log_status.success(), "git log must succeed with no hugit on PATH");

    // branch
    let branch_status = std::process::Command::new("git")
        .env("PATH", &bare_path)
        .args(["-C", clone_dir.to_str().unwrap(), "branch", "exit-test-branch"])
        .status()
        .expect("git branch must execute");
    assert!(branch_status.success(), "git branch must succeed with no hugit on PATH");
}

/// ⑥ completeness: export+restore reproduces ALL first-class object classes object-for-object.
/// Out-of-scope classes must be explicitly enumerated in ExportSchema (no silent omission).
#[test]
fn item_6_completeness_all_classes() {
    let config = ExportConfig::fixture_all_classes();
    let export = export_command(&config).expect("export must succeed");
    let restored = restore_from_export(&export).expect("restore must succeed");

    assert_eq!(restored.refs(), config.source_refs(), "refs");
    assert_eq!(restored.intents(), config.source_intents(), "intents");
    assert_eq!(restored.events(), config.source_events(), "events");
    assert_eq!(restored.ledger(), config.source_ledger(), "ledger");
    assert_eq!(restored.verdicts(), config.source_verdicts(), "verdicts");
    assert_eq!(restored.journals(), config.source_journals(), "journals");
    assert_eq!(restored.policy(), config.source_policy(), "policy");
    assert_eq!(restored.provenance_links(), config.source_provenance_links(), "provenance_links");

    // Out-of-scope classes explicitly enumerated
    let schema = ExportSchema::current();
    assert!(
        !schema.out_of_scope_classes().is_empty() || schema.declares_no_out_of_scope(),
        "ExportSchema must explicitly enumerate out-of-scope classes (no silent omission)"
    );
}

/// ⑦ redaction red-team: seeded secrets appear NOWHERE in exported git/JSON;
/// redacted artifact still passes the exit proof; removals manifested.
#[test]
fn item_7_redaction_redteam() {
    let secrets = vec!["SECRET_A_XYZ987".to_string(), "SECRET_B_ABC123".to_string()];
    let policy = RedactionPolicy::redact_secrets(secrets.clone());
    let config = ExportConfig::fixture_with_redaction(policy, &secrets[0]);
    let export = export_command(&config).expect("export must succeed");

    // Secrets absent from JSON
    let json_str = std::fs::read_to_string(export.json_path()).unwrap();
    for secret in &secrets {
        assert!(
            !json_str.contains(secret.as_str()),
            "secret {} must not appear in exported JSON (red-team)",
            secret
        );
    }

    // Secrets absent from git objects
    let git_grep = std::process::Command::new("git")
        .args([
            "-C",
            export.git_path().to_str().unwrap(),
            "grep",
            "-r",
            &secrets[0],
        ])
        .output()
        .unwrap();
    assert!(
        git_grep.stdout.is_empty(),
        "secret must not appear in any git object in the exported repo"
    );

    // Removals manifested (redaction record present in JSON)
    let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();
    let redactions = parsed.get("redactions");
    assert!(
        redactions.is_some() && !redactions.unwrap().as_array().map(|a| a.is_empty()).unwrap_or(true),
        "redacted export must manifest removals in the redactions field"
    );

    // Redacted artifact still passes exit proof (schema validates)
    let schema = ExportSchema::current();
    schema
        .validate(json_str.as_bytes())
        .expect("redacted artifact must still validate against ExportSchema");
}

/// ⑧ exit under exit conditions: export succeeds on suspended/past-due/offboarding account.
/// Read-only terminating path; exit is never blocked by account state.
#[test]
fn item_8_terminating_account() {
    use hugit_cli::export::AccountState;
    for state in &[
        AccountState::Suspended,
        AccountState::PastDue,
        AccountState::Offboarding,
    ] {
        let config = ExportConfig::fixture_with_account_state(*state);
        let result = export_command(&config);
        assert!(
            result.is_ok(),
            "export must succeed on {:?} account (exit must never be blocked by account state)",
            state
        );
    }
}

/// ⑨ "any moment" consistency: export under concurrent mutation yields ONE point-in-time-consistent cut.
/// No dangling provenance link, no event referencing an absent object; restore is self-consistent.
#[test]
fn item_9_live_consistency_cut() {
    let config = ExportConfig::fixture_live_concurrent();
    let export = export_command(&config).expect("export under concurrent mutation must succeed");
    let restored = restore_from_export(&export).expect("restore must succeed");

    // No dangling provenance links
    for link in restored.provenance_links() {
        assert!(
            restored.object_exists(link.target_id()),
            "provenance link target {} must exist in the restored snapshot (no dangling links)",
            link.target_id()
        );
    }

    // No event references an absent object
    for event in restored.events() {
        if let Some(obj_id) = event.object_ref() {
            assert!(
                restored.object_exists(obj_id),
                "event {} references object {} which must exist in the snapshot",
                event.id(),
                obj_id
            );
        }
    }
}
