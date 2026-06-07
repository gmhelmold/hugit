//! R0 cross-consistency pins for the SINGLE canonical hash/memo/attestation
//! byte-format.
//!
//! These assertions lock `hugit_refstore`'s canonical functions to the SAME
//! hardcoded digests that `hugit-contracts/tests/integration_tests.rs` pins via
//! an independent recomputation. If the canonical byte-format ever drifts, BOTH
//! suites go RED — re-serialization or a refactor cannot launder a formula
//! change past a hand-computed digest pinned in two crates.
//!
//! Pin provenance: computed by an independent hand-written Python reference
//! implementation over the fixed fixtures below (see the R0 SEAL).

use hugit_refstore::{
    EventLog, GENESIS_PREV_HASH, attestation_sig_preimage, canonical_json, compute_memo_key,
    compute_this_hash,
};

// ── the SAME literals pinned in hugit-contracts ───────────────────────────────

const PIN_KIND: &str = "ref.update";
const PIN_PAYLOAD: &str = r#"{"ref":"refs/heads/main","target":"abc123"}"#;
const PIN_SEQ: u64 = 0;
const PIN_THIS_HASH_EXPECTED: &str =
    "b53e6bd85641955c36a04eecc060691eb7f888b60f250358418b569ec6735416";

const PIN_MEMO_TREE: &str = "cafebabecafebabecafebabecafebabecafebabecafebabecafebabecafebabe";
const PIN_MEMO_DEF: &str = "a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4a1b2c3d4";
const PIN_MEMO_TOOLCHAIN: &str = "1234567812345678123456781234567812345678123456781234567812345678";
const PIN_MEMO_KEY_EXPECTED: &str =
    "de0d40a5e86f64a15ebe5d6227d9fc30ecf1bd7c95edbb8b98f9e92bffebda84";

fn principal_chain_fixture() -> Vec<String> {
    vec!["agent:runner-01".to_string(), "user:gustavo".to_string()]
}

/// Genesis fixture uses 64 ASCII '0' for prev_hash.
#[test]
fn genesis_is_64_ascii_zeros() {
    assert_eq!(GENESIS_PREV_HASH.len(), 64);
    assert!(GENESIS_PREV_HASH.bytes().all(|b| b == b'0'));
}

/// The canonical `compute_this_hash` must equal the cross-crate hardcoded pin.
#[test]
fn this_hash_matches_cross_crate_pin() {
    let got = compute_this_hash(
        GENESIS_PREV_HASH,
        PIN_KIND,
        &principal_chain_fixture(),
        PIN_PAYLOAD,
        PIN_SEQ,
    );
    assert_eq!(
        got, PIN_THIS_HASH_EXPECTED,
        "refstore::compute_this_hash drifted from the R0 pin shared with hugit-contracts"
    );
}

/// The append path must produce the same pinned hash for the same inputs (the
/// producer and the bare formula can never drift).
#[test]
fn append_path_matches_pin() {
    let mut log = EventLog::new();
    let rec = log.append(
        PIN_KIND,
        principal_chain_fixture(),
        PIN_PAYLOAD,
        1_717_000_000_000, // recorded_at is excluded from the hash
    );
    assert_eq!(rec.this_hash, PIN_THIS_HASH_EXPECTED);
    // recorded_at must NOT affect the hash.
    let mut log2 = EventLog::new();
    let rec2 = log2.append(PIN_KIND, principal_chain_fixture(), PIN_PAYLOAD, 999);
    assert_eq!(
        rec.this_hash, rec2.this_hash,
        "recorded_at must not be hashed"
    );
}

/// The canonical `compute_memo_key` must equal the cross-crate hardcoded pin.
#[test]
fn memo_key_matches_cross_crate_pin() {
    let got = compute_memo_key(PIN_MEMO_TREE, PIN_MEMO_DEF, PIN_MEMO_TOOLCHAIN);
    assert_eq!(
        got, PIN_MEMO_KEY_EXPECTED,
        "refstore::compute_memo_key drifted from the R0 pin shared with hugit-contracts"
    );
}

/// `canonical_json` sorts keys and strips insignificant whitespace, so logically
/// equal JSON maps to identical bytes — and equal canonical payloads chain to
/// equal hashes.
#[test]
fn canonical_json_is_order_and_whitespace_invariant() {
    let a = canonical_json(r#"{ "b": 2, "a": 1 }"#).unwrap();
    let b = canonical_json(r#"{"a":1,"b":2}"#).unwrap();
    assert_eq!(a, b);
    assert_eq!(a, r#"{"a":1,"b":2}"#);
    assert!(canonical_json("not json").is_none());

    let h1 = compute_this_hash(GENESIS_PREV_HASH, "k", &[], &a, 0);
    let h2 = compute_this_hash(GENESIS_PREV_HASH, "k", &[], &b, 0);
    assert_eq!(h1, h2);
}

/// The attestation pre-image is byte-exact, length-prefixed + vector-framed, and
/// distinguishes inputs that share a naive concatenation (framing is unambiguous).
#[test]
fn attestation_preimage_is_unambiguously_framed() {
    let p = attestation_sig_preimage("tree", "def", "runner", "model", &["alice".into()]);

    // Hand-built expected bytes: LP(tree) LP(def) LP(runner) LP(model) VEC([alice]).
    let mut expected = Vec::new();
    for f in ["tree", "def", "runner", "model"] {
        expected.extend_from_slice(&(f.len() as u32).to_be_bytes());
        expected.extend_from_slice(f.as_bytes());
    }
    expected.extend_from_slice(&1u32.to_be_bytes()); // principal count
    expected.extend_from_slice(&("alice".len() as u32).to_be_bytes());
    expected.extend_from_slice(b"alice");
    assert_eq!(p, expected);

    // Framing makes field-boundary collisions impossible.
    let shifted = attestation_sig_preimage("treedef", "", "runner", "model", &["alice".into()]);
    assert_ne!(
        p, shifted,
        "LP framing must keep field boundaries unambiguous"
    );
}
