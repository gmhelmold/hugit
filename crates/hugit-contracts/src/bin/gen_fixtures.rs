//! Generator binary: write all 16 JSON Schemas + golden fixtures (15 WP-00
//! types + `ContextEnvelope` per ADR-0001 / WP-F1; the envelope ships FOUR
//! goldens — one per altitude + the nullable-refs sub-`full` capture case).
//! Run from crate root: `cargo run -p hugit-contracts --bin gen_fixtures`

use hugit_contracts::{
    AppWebhooks, AttentionRank, AttestationChain, CheckDef, CheckResult, ContextEnvelope,
    DiagnosisObject, EventRecord, ExportSchema, FenceManifest, IntentSidecar, QueueApi, RegenGate,
    RunnerLease, ShadowPolicy, VerdictObject,
    app_webhooks::{AckReceipt, ChecksWriteRequest, ChecksWriteResponse, SignedEventEnvelope},
    check_result::Artifact,
    context_envelope::{
        Altitude, Authorship, CONTEXT_ENVELOPE_SCHEMA_VERSION, FileRead, IntentMetrics, Snapshot,
        Spawn, TokenCounts, ToolCount, Trajectory,
    },
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

    // ── ContextEnvelope (ADR-0001 / WP-F1) ──────────────────────────────────
    // One schema, FOUR goldens: one per altitude (intent · pr · campaign)
    // plus the nullable-refs case (sub-`full` capture level).
    write_schema::<ContextEnvelope>("ContextEnvelope");
    write_golden("ContextEnvelope", &envelope_intent());
    write_golden("ContextEnvelopePr", &envelope_pr());
    write_golden("ContextEnvelopeCampaign", &envelope_campaign());
    write_golden("ContextEnvelopeNullRefs", &envelope_null_refs());

    println!("\nAll 16 schemas + golden fixtures written.");
}

// ── ContextEnvelope fixtures (ADR-0001 §2.2) ──────────────────────────────────

/// Intent altitude, capture level `full` — every ref present (the ADR §2.2
/// worked example).
fn envelope_intent() -> ContextEnvelope {
    ContextEnvelope {
        schema_version: CONTEXT_ENVELOPE_SCHEMA_VERSION.into(),
        altitude: Altitude::Intent,
        intent_id: "a31".into(),
        commit: "a31f9ca31f9ca31f9ca31f9ca31f9ca31f9ca31f".into(),
        tree_hash: "cafebabecafebabecafebabecafebabecafebabecafebabecafebabecafebabe".into(),
        authorship: Authorship {
            model: "claude-opus-4-8".into(),
            model_digest: "d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0".into(),
            agent_type: "implementer".into(),
            spawn: Spawn {
                run_id: "run-impl-0007".into(),
                parent_run_id: Some("orq-014".into()),
                born_at: 1_717_000_000_000,
                died_at: 1_717_000_840_000,
            },
            operator: "gustavo@humangr.com".into(),
        },
        charter: "fix: refresh token reused the old iat, shortening the window".into(),
        campaign: Some("auth-hardening".into()),
        constraints: vec!["no new dependencies".into()],
        acceptance: vec!["token lasts the full TTL".into()],
        parent_intents: vec![],
        trajectory: Trajectory {
            raw_transcript_ref: Some(
                "cas:7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a7e1a".into(),
            ),
            task_transcript_ref: Some(
                "cas:9b2c9b2c9b2c9b2c9b2c9b2c9b2c9b2c9b2c9b2c9b2c9b2c9b2c9b2c9b2c9b2c".into(),
            ),
            summary: Some(
                "Re-derived iat from now() to fix the refresh window; read 2 files; \
                 14 tool calls; 3/3 verdicts green."
                    .into(),
            ),
            journal_ref: Some(
                "cas:a902a902a902a902a902a902a902a902a902a902a902a902a902a902a902a902".into(),
            ),
            redaction_policy: "default-v1".into(),
        },
        snapshot: Snapshot {
            files_read: vec![FileRead {
                path: "auth/token.rs".into(),
                hash: "sha256:9c2f9c2f9c2f9c2f9c2f9c2f9c2f9c2f9c2f9c2f9c2f9c2f9c2f9c2f9c2f9c2f"
                    .into(),
            }],
            prompt_ref: Some(
                "cas:3b4f3b4f3b4f3b4f3b4f3b4f3b4f3b4f3b4f3b4f3b4f3b4f3b4f3b4f3b4f3b4f".into(),
            ),
            env_manifest: "rustc 1.96.0".into(),
        },
        metrics: IntentMetrics {
            tokens: TokenCounts {
                input: 48_211,
                output: 6_035,
                cache_read: 39_800,
                cache_write: 2_100,
                total: 96_146,
            },
            wall_ms: 840_000,
            active_ms: 612_000,
            tool_calls: 14,
            tool_breakdown: vec![
                ToolCount {
                    tool: "Edit".into(),
                    count: 6,
                },
                ToolCount {
                    tool: "Bash".into(),
                    count: 5,
                },
                ToolCount {
                    tool: "Read".into(),
                    count: 3,
                },
            ],
            model_turns: 9,
            cost_usd: 0.04,
        },
        verdicts_ref: Some(
            "cas:4d5e4d5e4d5e4d5e4d5e4d5e4d5e4d5e4d5e4d5e4d5e4d5e4d5e4d5e4d5e4d5e".into(),
        ),
    }
}

/// PR altitude — the orchestrator-session envelope (owner 2026-06-10):
/// `intent_id` carries the PR id; the author session is top-level
/// (`parent_run_id: null`, `agent_type: "main"`).
fn envelope_pr() -> ContextEnvelope {
    ContextEnvelope {
        schema_version: CONTEXT_ENVELOPE_SCHEMA_VERSION.into(),
        altitude: Altitude::Pr,
        intent_id: "128".into(),
        commit: "f00df00df00df00df00df00df00df00df00df00d".into(),
        tree_hash: "beadbeadbeadbeadbeadbeadbeadbeadbeadbeadbeadbeadbeadbeadbeadbead".into(),
        authorship: Authorship {
            model: "claude-opus-4-8".into(),
            model_digest: "d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0".into(),
            agent_type: "main".into(),
            spawn: Spawn {
                run_id: "orq-014".into(),
                parent_run_id: None,
                born_at: 1_717_000_000_000,
                died_at: 1_717_004_200_000,
            },
            operator: "gustavo@humangr.com".into(),
        },
        charter: "land auth-hardening wave 1: refresh-window fixes".into(),
        campaign: Some("auth-hardening".into()),
        constraints: vec![],
        acceptance: vec!["all intents land conflict-free".into()],
        parent_intents: vec![],
        trajectory: Trajectory {
            raw_transcript_ref: Some(
                "cas:c0dec0dec0dec0dec0dec0dec0dec0dec0dec0dec0dec0dec0dec0dec0dec0de".into(),
            ),
            task_transcript_ref: Some(
                "cas:deaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddeaddead".into(),
            ),
            summary: Some(
                "Planned 3 intents, dispatched 3 subagents, cold-verified, landed the bundle."
                    .into(),
            ),
            journal_ref: None,
            redaction_policy: "default-v1".into(),
        },
        snapshot: Snapshot {
            files_read: vec![FileRead {
                path: "docs/plan/wave-1.md".into(),
                hash: "sha256:77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa77aa"
                    .into(),
            }],
            prompt_ref: Some(
                "cas:beefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeefbeef".into(),
            ),
            env_manifest: "rustc 1.96.0".into(),
        },
        metrics: IntentMetrics {
            tokens: TokenCounts {
                input: 122_000,
                output: 18_400,
                cache_read: 96_000,
                cache_write: 8_000,
                total: 244_400,
            },
            wall_ms: 4_200_000,
            active_ms: 1_900_000,
            tool_calls: 41,
            tool_breakdown: vec![
                ToolCount {
                    tool: "Bash".into(),
                    count: 22,
                },
                ToolCount {
                    tool: "Read".into(),
                    count: 13,
                },
                ToolCount {
                    tool: "Task".into(),
                    count: 6,
                },
            ],
            model_turns: 28,
            cost_usd: 0.62,
        },
        verdicts_ref: Some(
            "cas:5eed5eed5eed5eed5eed5eed5eed5eed5eed5eed5eed5eed5eed5eed5eed5eed".into(),
        ),
    }
}

/// Campaign altitude — the campaign's own envelope (owner 2026-06-10):
/// `intent_id` carries the campaign key.
fn envelope_campaign() -> ContextEnvelope {
    ContextEnvelope {
        schema_version: CONTEXT_ENVELOPE_SCHEMA_VERSION.into(),
        altitude: Altitude::Campaign,
        intent_id: "auth-hardening".into(),
        commit: "feedfeedfeedfeedfeedfeedfeedfeedfeedfeed".into(),
        tree_hash: "f1cef1cef1cef1cef1cef1cef1cef1cef1cef1cef1cef1cef1cef1cef1cef1ce".into(),
        authorship: Authorship {
            model: "claude-opus-4-8".into(),
            model_digest: "d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0".into(),
            agent_type: "main".into(),
            spawn: Spawn {
                run_id: "campaign-auth-hardening-01".into(),
                parent_run_id: None,
                born_at: 1_716_990_000_000,
                died_at: 1_717_010_000_000,
            },
            operator: "gustavo@humangr.com".into(),
        },
        charter: "harden the authentication edge".into(),
        campaign: Some("auth-hardening".into()),
        constraints: vec![],
        acceptance: vec!["all auth PRs landed and proven".into()],
        parent_intents: vec![],
        trajectory: Trajectory {
            raw_transcript_ref: Some(
                "cas:c4a9c4a9c4a9c4a9c4a9c4a9c4a9c4a9c4a9c4a9c4a9c4a9c4a9c4a9c4a9c4a9".into(),
            ),
            task_transcript_ref: Some(
                "cas:ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12ab12".into(),
            ),
            summary: Some(
                "Campaign session: scoped 2 PRs, tracked landing, closed the campaign.".into(),
            ),
            journal_ref: Some(
                "cas:09f109f109f109f109f109f109f109f109f109f109f109f109f109f109f109f1".into(),
            ),
            redaction_policy: "default-v1".into(),
        },
        snapshot: Snapshot {
            files_read: vec![],
            prompt_ref: Some(
                "cas:77fe77fe77fe77fe77fe77fe77fe77fe77fe77fe77fe77fe77fe77fe77fe77fe".into(),
            ),
            env_manifest: "rustc 1.96.0".into(),
        },
        metrics: IntentMetrics {
            tokens: TokenCounts {
                input: 310_000,
                output: 42_000,
                cache_read: 250_000,
                cache_write: 21_000,
                total: 623_000,
            },
            wall_ms: 20_000_000,
            active_ms: 5_400_000,
            tool_calls: 88,
            tool_breakdown: vec![
                ToolCount {
                    tool: "Bash".into(),
                    count: 40,
                },
                ToolCount {
                    tool: "Read".into(),
                    count: 30,
                },
                ToolCount {
                    tool: "Task".into(),
                    count: 18,
                },
            ],
            model_turns: 64,
            cost_usd: 1.85,
        },
        verdicts_ref: None,
    }
}

/// Intent altitude at capture level `metrics` (a per-repo privacy opt-DOWN,
/// ADR-0001 §3): every ref is `null` and must round-trip — consumers MUST
/// tolerate nulls.
fn envelope_null_refs() -> ContextEnvelope {
    ContextEnvelope {
        schema_version: CONTEXT_ENVELOPE_SCHEMA_VERSION.into(),
        altitude: Altitude::Intent,
        intent_id: "a2f".into(),
        commit: "a2f0a2f0a2f0a2f0a2f0a2f0a2f0a2f0a2f0a2f0".into(),
        tree_hash: "0b570b570b570b570b570b570b570b570b570b570b570b570b570b570b570b57".into(),
        authorship: Authorship {
            model: "claude-sonnet-4-6".into(),
            model_digest: "d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0d1f0".into(),
            agent_type: "implementer".into(),
            spawn: Spawn {
                run_id: "run-impl-0008".into(),
                parent_run_id: None,
                born_at: 1_717_000_100_000,
                died_at: 1_717_000_195_000,
            },
            operator: "gustavo@humangr.com".into(),
        },
        charter: "chore: bump toolchain pin".into(),
        campaign: None,
        constraints: vec![],
        acceptance: vec![],
        parent_intents: vec![],
        trajectory: Trajectory {
            raw_transcript_ref: None,
            task_transcript_ref: None,
            summary: None,
            journal_ref: None,
            redaction_policy: "default-v1".into(),
        },
        snapshot: Snapshot {
            files_read: vec![],
            prompt_ref: None,
            env_manifest: "rustc 1.96.0".into(),
        },
        metrics: IntentMetrics {
            tokens: TokenCounts {
                input: 9_000,
                output: 1_200,
                cache_read: 6_000,
                cache_write: 500,
                total: 16_700,
            },
            wall_ms: 95_000,
            active_ms: 80_000,
            tool_calls: 4,
            tool_breakdown: vec![
                ToolCount {
                    tool: "Bash".into(),
                    count: 3,
                },
                ToolCount {
                    tool: "Edit".into(),
                    count: 1,
                },
            ],
            model_turns: 3,
            cost_usd: 0.01,
        },
        verdicts_ref: None,
    }
}
