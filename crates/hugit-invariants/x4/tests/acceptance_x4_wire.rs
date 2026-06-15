//! WP-X4 — the wire-level conformance assertion (hugit side).
//!
//! The supply-chain spawn-surface oracle (`pin.rs` + `acceptance_x4.rs`:
//! content-pinning, integrity-verify-before-spawn, fail-closed ordering)
//! TRANSFERRED to corelink-runners with the execution core it proves
//! (runner-transfer campaign WP-R4, R0 freeze item 4) — it is a
//! runner-product invariant now, relocated so it keeps driving the LIVE
//! spawn surface (`ContainerSpec::from_lease()` + `Engine::spawn()`) with
//! rigor preserved by relocation, never mocked.
//!
//! What hugit keeps — and this oracle owns — is the **wire**: the seam to
//! the transferred runner is the wire contract, types transcribed on each
//! side and proven equivalent by shared JSON conformance vectors committed
//! **byte-identical in both repos** (the AC/CAS pattern). This test pins
//! hugit's copy:
//!
//! 1. `conformance/manifest.sha256` lists exactly the frozen vector set
//!    (`RunnerLease.json`, `FenceManifest.json`, `IntentMetrics.json` —
//!    the third added by the contract v1.2.0 §13.4 amendment), and every
//!    committed vector
//!    hashes to its manifest digest **byte-exactly** — any drift on either
//!    repo's copy breaks this (or the twin golden in corelink-runners)
//!    immediately, since both repos commit the SAME `manifest.sha256`.
//! 2. Every vector parses through the FROZEN hugit-contracts type
//!    (`deny_unknown_fields`) and re-serializes byte-exactly — so the bytes
//!    both repos pin are exactly what hugit's frozen types speak, proving
//!    the transcribed corelink-runners types wire-equivalent without a git
//!    dependency in either direction (campaign iron rule).

use std::path::{Path, PathBuf};

use hugit_contracts::{FenceManifest, IntentMetrics, RunnerLease};
use sha2::{Digest, Sha256};

/// Workspace-root `conformance/` directory, resolved from this crate's
/// manifest dir (`crates/hugit-invariants` → two levels up).
fn conformance_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/
        .and_then(Path::parent) // workspace root
        .expect("workspace root not found")
        .join("conformance")
}

/// Read a committed conformance file, failing loudly if absent.
fn read(name: &str) -> Vec<u8> {
    let path = conformance_dir().join(name);
    std::fs::read(&path)
        .unwrap_or_else(|e| panic!("cannot read conformance vector {}: {e}", path.display()))
}

/// Lowercase-hex SHA-256 of `bytes`.
fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The frozen vector set, in manifest order: R0 (two), the §13.4 `IntentMetrics`
/// vector (contract v1.2.0), and the §7.1 `result_binding_v2` vector (contract
/// v1.4.0 — the full-outcome attestation signature; fabric-generated, mirrored
/// byte-identical here under the same drift tripwire).
const VECTORS: [&str; 4] = [
    "RunnerLease.json",
    "FenceManifest.json",
    "IntentMetrics.json",
    "result_binding_v2.json",
];

// ── ① the manifest pins every vector byte-exactly ────────────────────────────
#[test]
fn item_1_manifest_pins_every_vector_byte_exact() {
    let manifest = String::from_utf8(read("manifest.sha256")).expect("manifest is utf-8");

    // Parse `<64-hex>  <name>` lines (the shasum -a 256 format both repos
    // committed). Fail closed on anything malformed.
    let mut pinned: Vec<(String, String)> = Vec::new();
    for line in manifest.lines().filter(|l| !l.trim().is_empty()) {
        let (digest, name) = line
            .split_once("  ")
            .unwrap_or_else(|| panic!("malformed manifest line: {line:?}"));
        assert!(
            digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()),
            "manifest digest {digest:?} is not 64-hex"
        );
        pinned.push((digest.to_string(), name.trim().to_string()));
    }

    // Exactly the frozen vector set — nothing missing, nothing extra.
    let names: Vec<&str> = pinned.iter().map(|(_, n)| n.as_str()).collect();
    assert_eq!(
        names, VECTORS,
        "manifest must list exactly the frozen vector set"
    );

    // Every committed vector hashes to its pinned digest, byte-exactly.
    for (digest, name) in &pinned {
        let actual = sha256_hex(&read(name));
        assert_eq!(
            &actual, digest,
            "conformance vector {name} drifted from its pinned digest — \
             the wire contract is broken (both repos commit this manifest; \
             fix the drift, never the pin)"
        );
    }
}

// ── ② vectors round-trip byte-exactly through the FROZEN hugit types ─────────
#[test]
fn item_2_runner_lease_vector_round_trips_through_frozen_type() {
    let raw = String::from_utf8(read("RunnerLease.json")).expect("vector is utf-8");
    let lease: RunnerLease = serde_json::from_str(&raw)
        .expect("RunnerLease vector must parse through the frozen type (deny_unknown_fields)");
    // Re-serialize and append the trailing newline that the committed vector
    // carries (POSIX text-file convention: serde_json::to_string_pretty does
    // NOT add a trailing newline, but the committed vector does). The comparison
    // is byte-exact — trim_end() is intentionally absent so any trailing-byte
    // drift (added or removed whitespace) immediately breaks this assertion.
    let re = format!(
        "{}\n",
        serde_json::to_string_pretty(&lease).expect("frozen RunnerLease serializes")
    );
    assert_eq!(
        raw,
        re,
        "RunnerLease wire round-trip is not byte-exact: hugit's frozen type \
         and the committed vector disagree (byte difference: raw {} bytes, re {} bytes)",
        raw.len(),
        re.len(),
    );
}

#[test]
fn item_2_fence_manifest_vector_round_trips_through_frozen_type() {
    let raw = String::from_utf8(read("FenceManifest.json")).expect("vector is utf-8");
    let manifest: FenceManifest = serde_json::from_str(&raw)
        .expect("FenceManifest vector must parse through the frozen type (deny_unknown_fields)");
    // Re-serialize and append the trailing newline that the committed vector
    // carries (POSIX text-file convention: serde_json::to_string_pretty does
    // NOT add a trailing newline, but the committed vector does). The comparison
    // is byte-exact — trim_end() is intentionally absent so any trailing-byte
    // drift (added or removed whitespace) immediately breaks this assertion.
    let re = format!(
        "{}\n",
        serde_json::to_string_pretty(&manifest).expect("frozen FenceManifest serializes")
    );
    assert_eq!(
        raw,
        re,
        "FenceManifest wire round-trip is not byte-exact: hugit's frozen type \
         and the committed vector disagree (byte difference: raw {} bytes, re {} bytes)",
        raw.len(),
        re.len(),
    );
}

#[test]
fn item_2_intent_metrics_vector_round_trips_through_frozen_type() {
    let raw = String::from_utf8(read("IntentMetrics.json")).expect("vector is utf-8");
    let metrics: IntentMetrics = serde_json::from_str(&raw)
        .expect("IntentMetrics vector must parse through the frozen type (deny_unknown_fields)");
    // Re-serialize and append the trailing newline that the committed vector
    // carries (POSIX text-file convention: serde_json::to_string_pretty does
    // NOT add a trailing newline, but the committed vector does). The comparison
    // is byte-exact — trim_end() is intentionally absent so any trailing-byte
    // drift (added or removed whitespace) immediately breaks this assertion.
    let re = format!(
        "{}\n",
        serde_json::to_string_pretty(&metrics).expect("frozen IntentMetrics serializes")
    );
    assert_eq!(
        raw,
        re,
        "IntentMetrics wire round-trip is not byte-exact: hugit's frozen type \
         and the committed vector disagree (byte difference: raw {} bytes, re {} bytes)",
        raw.len(),
        re.len(),
    );
}
