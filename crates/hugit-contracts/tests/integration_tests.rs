// Integration tests for hugit-contracts.
//
// golden_<type>: read tests/golden/<TypeName>.json → deserialize → serialize
//   back → assert byte-identical to the committed fixture.
//
// schema_drift: regenerate all 15 schemas in-memory and assert byte-identity
//   with the committed schemas/<TypeName>.json.
//   Set UPDATE_SCHEMAS=1 to re-write the committed schema files.

use hugit_contracts::{
    AppWebhooks, AttentionRank, AttestationChain, CheckDef, CheckResult, DiagnosisObject,
    EventRecord, ExportSchema, FenceManifest, IntentSidecar, QueueApi, RegenGate, RunnerLease,
    ShadowPolicy, VerdictObject,
};
use hugit_contracts::{
    app_webhooks::{AckReceipt, ChecksWriteRequest, ChecksWriteResponse, SignedEventEnvelope},
    check_result::Artifact,
    fence_manifest::MaterializedEntry,
    queue_api::{BatchSeal, LandableEntry, MinimalFailingPair, UnionResult},
    runner_lease::RunnerState,
    verdict_object::Verdict,
};
use schemars::schema_for;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    // Integration tests run with cwd = crate root, but CARGO_MANIFEST_DIR
    // is more reliable.
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

// ── helpers ──────────────────────────────────────────────────────────────────

fn read_golden(type_name: &str) -> String {
    let path = repo_root()
        .join("tests")
        .join("golden")
        .join(format!("{type_name}.json"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read golden fixture {path:?}: {e}"))
}

fn roundtrip<T>(type_name: &str)
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let committed = read_golden(type_name);
    let value: T = serde_json::from_str(&committed)
        .unwrap_or_else(|e| panic!("deserialize {type_name} golden: {e}"));
    let re_serialized = serde_json::to_string_pretty(&value).unwrap() + "\n";
    assert_eq!(
        committed, re_serialized,
        "golden round-trip failed for {type_name}: serialized bytes differ from committed fixture",
    );
}

// ── golden round-trip tests (one per type) ───────────────────────────────────

#[test]
fn golden_check_def() {
    roundtrip::<CheckDef>("CheckDef");
}

#[test]
fn golden_check_result() {
    roundtrip::<CheckResult>("CheckResult");
}

#[test]
fn golden_diagnosis_object() {
    roundtrip::<DiagnosisObject>("DiagnosisObject");
}

#[test]
fn golden_intent_sidecar() {
    roundtrip::<IntentSidecar>("IntentSidecar");
}

#[test]
fn golden_runner_lease() {
    roundtrip::<RunnerLease>("RunnerLease");
}

#[test]
fn golden_fence_manifest() {
    roundtrip::<FenceManifest>("FenceManifest");
}

#[test]
fn golden_event_record() {
    roundtrip::<EventRecord>("EventRecord");
}

#[test]
fn golden_verdict_object() {
    roundtrip::<VerdictObject>("VerdictObject");
}

#[test]
fn golden_queue_api() {
    roundtrip::<QueueApi>("QueueApi");
}

#[test]
fn golden_app_webhooks() {
    roundtrip::<AppWebhooks>("AppWebhooks");
}

#[test]
fn golden_shadow_policy() {
    roundtrip::<ShadowPolicy>("ShadowPolicy");
}

#[test]
fn golden_attention_rank() {
    roundtrip::<AttentionRank>("AttentionRank");
}

#[test]
fn golden_export_schema() {
    roundtrip::<ExportSchema>("ExportSchema");
}

#[test]
fn golden_attestation_chain() {
    roundtrip::<AttestationChain>("AttestationChain");
}

#[test]
fn golden_regen_gate() {
    roundtrip::<RegenGate>("RegenGate");
}

// ── independent hash-pin (R0: lock the frozen formula byte-exactly) ────────────
//
// These pins are computed by an INDEPENDENT implementation (a hand-written
// Python reference, see the R0 SEAL) over a FIXED fixture, then hardcoded here.
// They guard the frozen byte-format itself: if anyone changes the LP/VEC framing,
// the field order, the seq width, or the digest, this test goes RED — and so does
// the cross-consistency test in hugit-refstore that pins `compute_this_hash` /
// `compute_memo_key` to these SAME literals. Re-serialization cannot launder a
// formula change past a hardcoded digest.
//
// Fixed `this_hash` fixture (genesis event):
//   prev_hash       = 64 ASCII '0'
//   kind            = "ref.update"
//   principal_chain = ["agent:runner-01", "user:gustavo"]
//   payload         = {"ref":"refs/heads/main","target":"abc123"}  (canonical)
//   seq             = 0
const PIN_THIS_HASH_FIXTURE_PREV: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";
const PIN_THIS_HASH_FIXTURE_KIND: &str = "ref.update";
const PIN_THIS_HASH_FIXTURE_PAYLOAD: &str = r#"{"ref":"refs/heads/main","target":"abc123"}"#;
const PIN_THIS_HASH_FIXTURE_SEQ: u64 = 0;
const PIN_THIS_HASH_EXPECTED: &str =
    "b53e6bd85641955c36a04eecc060691eb7f888b60f250358418b569ec6735416";

// Fixed `memo_key` fixture:
//   tree_hash        = "cafebabe" * 8
//   def_digest       = "a1b2c3d4" * 8
//   toolchain_digest = "12345678" * 8
const PIN_MEMO_TREE: &str = "cafebabecafebabecafebabecafebabecafebabecafebabecafebabecafebabe";
const PIN_MEMO_DEF: &str = "a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4";
const PIN_MEMO_TOOLCHAIN: &str = "1234567812345678123456781234567812345678123456781234567812345678";
const PIN_MEMO_KEY_EXPECTED: &str =
    "de0d40a5e86f64a15ebe5d6227d9fc30ecf1bd7c95edbb8b98f9e92bffebda84";

fn principal_chain_fixture() -> Vec<String> {
    vec!["agent:runner-01".to_string(), "user:gustavo".to_string()]
}

/// SHA-256 over the spec'd pre-image, recomputed here with a SECOND independent
/// implementation (not refstore's) so the pin is double-anchored: hardcoded
/// literal == this in-test recomputation == refstore::compute_this_hash.
fn lp(buf: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    buf.extend_from_slice(&(b.len() as u32).to_be_bytes());
    buf.extend_from_slice(b);
}

#[test]
fn pin_this_hash_matches_independent_recompute() {
    use sha2::{Digest, Sha256};
    let mut buf = Vec::new();
    lp(&mut buf, PIN_THIS_HASH_FIXTURE_PREV);
    lp(&mut buf, PIN_THIS_HASH_FIXTURE_KIND);
    let pc = principal_chain_fixture();
    buf.extend_from_slice(&(pc.len() as u32).to_be_bytes());
    for e in &pc {
        lp(&mut buf, e);
    }
    lp(&mut buf, PIN_THIS_HASH_FIXTURE_PAYLOAD);
    buf.extend_from_slice(&PIN_THIS_HASH_FIXTURE_SEQ.to_be_bytes());
    let got = hex::encode(Sha256::digest(&buf));
    assert_eq!(
        got, PIN_THIS_HASH_EXPECTED,
        "this_hash byte-format drifted from the hardcoded R0 pin"
    );
}

#[test]
fn pin_memo_key_matches_independent_recompute() {
    use sha2::{Digest, Sha256};
    let mut buf = Vec::new();
    lp(&mut buf, PIN_MEMO_TREE);
    lp(&mut buf, PIN_MEMO_DEF);
    lp(&mut buf, PIN_MEMO_TOOLCHAIN);
    let got = hex::encode(Sha256::digest(&buf));
    assert_eq!(
        got, PIN_MEMO_KEY_EXPECTED,
        "memo_key byte-format drifted from the hardcoded R0 pin"
    );
}

// ── schema drift test ─────────────────────────────────────────────────────────

macro_rules! check_schema {
    ($name:expr, $T:ty, $update:expr, $root:expr) => {{
        let schema = schema_for!($T);
        let generated = serde_json::to_string_pretty(&schema).unwrap() + "\n";
        let schema_path = $root.join("schemas").join(format!("{}.json", $name));
        if $update {
            std::fs::write(&schema_path, &generated)
                .unwrap_or_else(|e| panic!("write schema {schema_path:?}: {e}"));
        } else {
            let committed = std::fs::read_to_string(&schema_path)
                .unwrap_or_else(|e| panic!("read schema {schema_path:?}: {e}"));
            assert_eq!(
                committed, generated,
                "schema drift detected for {}: committed schema differs from generated schema\n\
                 Run with UPDATE_SCHEMAS=1 to regenerate.",
                $name
            );
        }
    }};
}

#[test]
fn schema_drift() {
    let update = std::env::var("UPDATE_SCHEMAS")
        .map(|v| v == "1")
        .unwrap_or(false);
    let root = repo_root();

    check_schema!("CheckDef", CheckDef, update, root);
    check_schema!("CheckResult", CheckResult, update, root);
    check_schema!("DiagnosisObject", DiagnosisObject, update, root);
    check_schema!("IntentSidecar", IntentSidecar, update, root);
    check_schema!("RunnerLease", RunnerLease, update, root);
    check_schema!("FenceManifest", FenceManifest, update, root);
    check_schema!("EventRecord", EventRecord, update, root);
    check_schema!("VerdictObject", VerdictObject, update, root);
    check_schema!("QueueApi", QueueApi, update, root);
    check_schema!("AppWebhooks", AppWebhooks, update, root);
    check_schema!("ShadowPolicy", ShadowPolicy, update, root);
    check_schema!("AttentionRank", AttentionRank, update, root);
    check_schema!("ExportSchema", ExportSchema, update, root);
    check_schema!("AttestationChain", AttestationChain, update, root);
    check_schema!("RegenGate", RegenGate, update, root);
}

// ── verify all sub-type imports compile and values are constructable ──────────
#[test]
fn all_subtypes_importable() {
    // Exercises every re-exported sub-type so the compiler proves they are
    // importable and structurally intact.  Previously a dead-code helper;
    // promoted to a real test so it actually runs in `cargo test`.
    let _: RunnerState = RunnerState::Held;
    let _: Verdict = Verdict::Approve;
    let _a = Artifact {
        path: "".into(),
        digest: "".into(),
    };
    let _m = MaterializedEntry {
        path: "".into(),
        digest: "".into(),
    };
    let _l = LandableEntry {
        item_id: "".into(),
        intent_id: "".into(),
        tree_hash: "".into(),
        order_index: 0,
    };
    let _u = UnionResult {
        batch_id: "".into(),
        union_tree: "".into(),
        conflict_free: true,
    };
    let _mfp = MinimalFailingPair {
        item_a: "".into(),
        item_b: "".into(),
    };
    let _bs = BatchSeal {
        batch_id: "".into(),
        union_tree: "".into(),
        order_index: 0,
        state: "".into(),
        minimal_failing_pair: None,
    };
    let _se = SignedEventEnvelope {
        delivery_id: "".into(),
        event_type: "".into(),
        signature: "".into(),
        payload: "".into(),
        received_at: 0,
    };
    let _ar = AckReceipt {
        delivery_id: "".into(),
        processing_id: "".into(),
        acked_at: 0,
    };
    let _cwr = ChecksWriteRequest {
        repo: "".into(),
        head_sha: "".into(),
        check_name: "".into(),
        status: "".into(),
        conclusion: None,
        summary: "".into(),
        output_ref: "".into(),
    };
    let _cwresp = ChecksWriteResponse {
        check_run_id: 0,
        html_url: "".into(),
    };
}
