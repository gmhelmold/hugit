//! Generator binary: write all 15 JSON Schemas + golden fixtures.
//! Run from crate root: `cargo run -p hugit-contracts --bin gen_fixtures`

use hugit_contracts::{
    AppWebhooks, AttentionRank, AttestationChain, CheckDef, CheckResult, DiagnosisObject,
    EventRecord, ExportSchema, FenceManifest, IntentSidecar, QueueApi, RegenGate, RunnerLease,
    ShadowPolicy, VerdictObject,
    app_webhooks::{AckReceipt, ChecksWriteRequest, ChecksWriteResponse, SignedEventEnvelope},
    check_result::Artifact,
    fence_manifest::MaterializedEntry,
    queue_api::{BatchSeal, LandableEntry, UnionResult},
    runner_lease::RunnerState,
    verdict_object::Verdict,
};
use schemars::schema_for;
use std::path::PathBuf;

fn manifest_dir() -> PathBuf {
    // When run with `cargo run`, the binary cwd is the workspace root.
    // CARGO_MANIFEST_DIR points to the crate root.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn write_schema<T: schemars::JsonSchema>(name: &str) {
    let schema = schema_for!(T);
    let json = serde_json::to_string_pretty(&schema).unwrap() + "\n";
    let path = manifest_dir().join("schemas").join(format!("{name}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &json).unwrap();
    println!("schema  → {path:?}");
}

fn write_golden<T: serde::Serialize>(name: &str, value: &T) {
    let json = serde_json::to_string_pretty(value).unwrap() + "\n";
    let path = manifest_dir()
        .join("tests")
        .join("golden")
        .join(format!("{name}.json"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, &json).unwrap();
    println!("golden  → {path:?}");
}

fn main() {
    // ── CheckDef ────────────────────────────────────────────────────────────
    write_schema::<CheckDef>("CheckDef");
    write_golden(
        "CheckDef",
        &CheckDef {
            def_digest: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".into(),
            command: "cargo test --workspace".into(),
            inputs: vec!["src/**/*.rs".into(), "Cargo.toml".into()],
            toolchain_ref: "rust-1.96.0-x86_64-unknown-linux-gnu".into(),
            env_manifest: "badc0ffee0badc0ffee0badc0ffee0badc0ffee0badc0ffee0badc0ffee0badc".into(),
            glob_set: vec!["src/**".into(), "tests/**".into()],
        },
    );

    // ── CheckResult ─────────────────────────────────────────────────────────
    write_schema::<CheckResult>("CheckResult");
    write_golden(
        "CheckResult",
        &CheckResult {
            memo_key: "deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef".into(),
            tree_hash: "cafebabecafebabecafebabecafebabecafebabecafebabecafebabecafebabe".into(),
            def_digest: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".into(),
            toolchain_digest: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                .into(),
            exit: 0,
            artifacts: vec![Artifact {
                path: "target/debug/hugit".into(),
                digest: "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210".into(),
            }],
            stdout_ref: "0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20".into(),
            stderr_ref: "2122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f40".into(),
            duration_ms: 4200,
            runner_ref: "runner-01-abc123".into(),
            produced_at: 1_717_000_000_000,
        },
    );

    // ── DiagnosisObject ─────────────────────────────────────────────────────
    write_schema::<DiagnosisObject>("DiagnosisObject");
    write_golden(
        "DiagnosisObject",
        &DiagnosisObject {
            culprit_ref: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            diff_vs_green_ref: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
                .into(),
            suspect_targets: vec![
                "//src/merge:merge_lib".into(),
                "//src/queue:queue_lib".into(),
            ],
            bisect_path: vec![
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc".into(),
                "dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd".into(),
            ],
            size_bytes: 8192,
        },
    );

    // ── IntentSidecar ───────────────────────────────────────────────────────
    write_schema::<IntentSidecar>("IntentSidecar");
    write_golden(
        "IntentSidecar",
        &IntentSidecar {
            intent_id: "01900000-0000-7000-8000-000000000001".into(),
            charter: "Add fence manifest enforcement to the runner sandbox".into(),
            acceptance: vec![
                "Runner cannot read paths outside its path_set".into(),
                "FenceManifest deny_default=true is enforced".into(),
            ],
            context_ref: "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee".into(),
            authoritative: false,
        },
    );

    // ── RunnerLease ─────────────────────────────────────────────────────────
    write_schema::<RunnerLease>("RunnerLease");
    write_golden(
        "RunnerLease",
        &RunnerLease {
            lease_id: "lease-0001-held".into(),
            principal_chain: vec!["agent:runner-dispatch-01".into(), "user:gustavo".into()],
            path_set: vec![
                "/tmp/hugit/run/abc123".into(),
                "/cache/hugit/toolchains".into(),
            ],
            expiry: 1_717_003_600_000,
            net_policy: "egress-deny-all".into(),
            tmp_root: "/tmp/hugit/run/abc123".into(),
            state: RunnerState::Held,
        },
    );

    // ── FenceManifest ───────────────────────────────────────────────────────
    write_schema::<FenceManifest>("FenceManifest");
    write_golden(
        "FenceManifest",
        &FenceManifest {
            path_set: vec!["src/".into(), "Cargo.toml".into()],
            deny_default: true,
            materialized: vec![
                MaterializedEntry {
                    path: "src/lib.rs".into(),
                    digest: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff"
                        .into(),
                },
                MaterializedEntry {
                    path: "Cargo.toml".into(),
                    digest: "0000000000000000000000000000000000000000000000000000000000000001"
                        .into(),
                },
            ],
        },
    );

    // ── EventRecord ─────────────────────────────────────────────────────────
    write_schema::<EventRecord>("EventRecord");
    write_golden(
        "EventRecord",
        &EventRecord {
            seq: 1,
            prev_hash: "0000000000000000000000000000000000000000000000000000000000000000".into(),
            this_hash: "1111111111111111111111111111111111111111111111111111111111111111".into(),
            kind: "check.completed".into(),
            principal_chain: vec!["agent:runner-01".into()],
            payload: r#"{"memo_key":"deadbeef","exit":0}"#.into(),
            recorded_at: 1_717_000_001_000,
        },
    );

    // ── VerdictObject ───────────────────────────────────────────────────────
    write_schema::<VerdictObject>("VerdictObject");
    write_golden(
        "VerdictObject",
        &VerdictObject {
            intent: "01900000-0000-7000-8000-000000000001".into(),
            tree_hash: "cafebabecafebabecafebabecafebabecafebabecafebabecafebabecafebabe".into(),
            lens: "security-review-v1".into(),
            model: "claude-sonnet-4-6".into(),
            prompt_digest: "2222222222222222222222222222222222222222222222222222222222222222"
                .into(),
            verdict: Verdict::Approve,
            claims_checked: vec!["no-credential-leak".into(), "no-rce-vector".into()],
            evidence_refs: vec![
                "3333333333333333333333333333333333333333333333333333333333333333".into(),
            ],
        },
    );

    // ── QueueApi ────────────────────────────────────────────────────────────
    write_schema::<QueueApi>("QueueApi");
    write_golden(
        "QueueApi",
        &QueueApi {
            landable: vec![LandableEntry {
                item_id: "item-001".into(),
                intent_id: "01900000-0000-7000-8000-000000000001".into(),
                tree_hash: "cafebabecafebabecafebabecafebabecafebabecafebabecafebabecafebabe"
                    .into(),
                order_index: 0,
            }],
            batch_id: "batch-20240601-001".into(),
            union_result: UnionResult {
                batch_id: "batch-20240601-001".into(),
                union_tree: "4444444444444444444444444444444444444444444444444444444444444444"
                    .into(),
                conflict_free: true,
            },
            seal: BatchSeal {
                batch_id: "batch-20240601-001".into(),
                union_tree: "4444444444444444444444444444444444444444444444444444444444444444"
                    .into(),
                order_index: 0,
                state: "sealed".into(),
                minimal_failing_pair: None,
            },
        },
    );

    // ── AppWebhooks ─────────────────────────────────────────────────────────
    write_schema::<AppWebhooks>("AppWebhooks");
    write_golden(
        "AppWebhooks",
        &AppWebhooks {
            inbound: SignedEventEnvelope {
                delivery_id: "delivery-0001".into(),
                event_type: "check_suite".into(),
                signature:
                    "sha256=5555555555555555555555555555555555555555555555555555555555555555".into(),
                payload: r#"{"action":"requested"}"#.into(),
                received_at: 1_717_000_002_000,
            },
            ack: AckReceipt {
                delivery_id: "delivery-0001".into(),
                processing_id: "proc-0001".into(),
                acked_at: 1_717_000_002_050,
            },
            write_request: ChecksWriteRequest {
                repo: "humangr/hugit".into(),
                head_sha: "6666666666666666666666666666666666666666".into(),
                check_name: "hugit/contracts".into(),
                status: "completed".into(),
                conclusion: Some("success".into()),
                summary: "All 15 contract types verified".into(),
                output_ref: "7777777777777777777777777777777777777777777777777777777777777777"
                    .into(),
            },
            write_response: ChecksWriteResponse {
                check_run_id: 987654321,
                html_url: "https://github.com/humangr/hugit/runs/987654321".into(),
            },
        },
    );

    // ── ShadowPolicy ────────────────────────────────────────────────────────
    write_schema::<ShadowPolicy>("ShadowPolicy");
    write_golden(
        "ShadowPolicy",
        &ShadowPolicy {
            cadence: "every_push".into(),
            budget: 300_000,
            optin: "humangr/hugit".into(),
        },
    );

    // ── AttentionRank ───────────────────────────────────────────────────────
    write_schema::<AttentionRank>("AttentionRank");
    write_golden(
        "AttentionRank",
        &AttentionRank {
            policy: "blast-radius-v1".into(),
            blast_radius: 42,
            confidence: 8750,
        },
    );

    // ── ExportSchema ────────────────────────────────────────────────────────
    write_schema::<ExportSchema>("ExportSchema");
    write_golden(
        "ExportSchema",
        &ExportSchema {
            version: "1.0.0".into(),
            object_classes: vec![
                "CheckDef".into(),
                "CheckResult".into(),
                "EventRecord".into(),
            ],
            redaction_manifest: "8888888888888888888888888888888888888888888888888888888888888888"
                .into(),
        },
    );

    // ── AttestationChain ────────────────────────────────────────────────────
    write_schema::<AttestationChain>("AttestationChain");
    write_golden(
        "AttestationChain",
        &AttestationChain {
            tree: "cafebabecafebabecafebabecafebabecafebabecafebabecafebabecafebabe".into(),
            def: "a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2c3d4e5f6a1b2".into(),
            runner: "runner-01-abc123".into(),
            model: "claude-sonnet-4-6".into(),
            principal: vec!["agent:runner-01".into(), "user:gustavo".into()],
            sig: "MEUCIQDexample+base64+signature==".into(),
        },
    );

    // ── RegenGate ───────────────────────────────────────────────────────────
    write_schema::<RegenGate>("RegenGate");
    write_golden(
        "RegenGate",
        &RegenGate {
            optin_scope: "humangr/hugit".into(),
            repass: true,
            indep_verdict: "9999999999999999999999999999999999999999999999999999999999999999"
                .into(),
        },
    );

    println!("\nAll 15 schemas + golden fixtures written.");
}
