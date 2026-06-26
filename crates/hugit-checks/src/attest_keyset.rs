//! Attestation key-set SELECTOR (WP-#3 solo half).
//!
//! Transcribed — not redesigned — from
//! `corelink-runners/crates/corelink-fabric-api/src/dto.rs` lines 361-457.
//! Both repos pin the same `conformance/attestation_keyset_selection.json` so
//! the selection decisions are byte-identical on the producer (fabric) and the
//! consumer (hugit verifier). The crypto verify step itself is handled by the
//! existing [`crate::attest_v2::verify_result_binding_v2`] path; this module
//! adds ONLY the thin selection layer above it.

use serde::{Deserialize, Serialize};

/// One entry in the `GET /v1/attestation/key` key-set response.
///
/// Transcribed from `corelink_fabric_api::KeyEntry` (dto.rs:361-377).
/// `deny_unknown_fields` pins the wire shape: any unrecognised field on the
/// consumer side is a hard error rather than silent drift.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeyEntry {
    /// Deterministic routing id — `lower_hex(SHA-256(pubkey_bytes))[..16]`.
    pub key_id: String,

    /// 32-byte ed25519 public key, standard-base64 encoded (RFC 4648 §4,
    /// padded) — the wire form [`crate::attest_v2::verify_result_binding_v2`]
    /// accepts.
    pub pubkey_b64: String,

    /// Optional expiry (Unix epoch milliseconds). `None` = active with no
    /// announced expiry. Reserved for the key-rotation machinery (M1+).
    #[serde(default)]
    pub expires_ms: Option<u64>,
}

/// Why a key-set selection rejected.
///
/// Transcribed from `corelink_fabric_api::KeySelectError` (dto.rs:401-424).
/// The variant vocabulary is frozen by
/// `conformance/attestation_keyset_selection.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeySelectError {
    /// No key in the set matches the attestation's `key_id`. Fail-closed.
    UnknownKeyId,
    /// The matched key has `expires_ms <= now_ms` — it has been retired.
    /// Fail-closed (the `<=` makes the exact cutover instant already-expired).
    Expired,
}

impl core::fmt::Display for KeySelectError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            KeySelectError::UnknownKeyId => {
                f.write_str("attestation key_id not in the published key set")
            }
            KeySelectError::Expired => {
                f.write_str("attestation key has expired (expires_ms <= now)")
            }
        }
    }
}

impl std::error::Error for KeySelectError {}

/// Select the active key from `keys` for the given `key_id` at `now_ms`.
///
/// Transcribed byte-for-byte from `corelink_fabric_api::select_attestation_key`
/// (dto.rs:442-457). Decision order:
/// 1. No entry with `key_id` → [`KeySelectError::UnknownKeyId`].
/// 2. Matched entry with `expires_ms = Some(t)` where `t <= now_ms`
///    → [`KeySelectError::Expired`].
/// 3. Otherwise → `Ok(&entry)`.
pub fn select_attestation_key<'a>(
    keys: &'a [KeyEntry],
    key_id: &str,
    now_ms: u64,
) -> Result<&'a KeyEntry, KeySelectError> {
    let entry = keys
        .iter()
        .find(|k| k.key_id == key_id)
        .ok_or(KeySelectError::UnknownKeyId)?;
    if let Some(expires_ms) = entry.expires_ms
        && expires_ms <= now_ms
    {
        return Err(KeySelectError::Expired);
    }
    Ok(entry)
}

/// Select the active key then verify a `result_binding_sig_v2`.
///
/// Convenience wrapper: selects via [`select_attestation_key`], then forwards
/// to [`crate::attest_v2::verify_result_binding_v2`] using the entry's
/// `pubkey_b64`. Returns `Err` if selection rejects; the inner `bool` mirrors
/// the existing verifier's fail-closed semantics.
#[allow(clippy::too_many_arguments)]
pub fn verify_with_keyset(
    keys: &[KeyEntry],
    key_id: &str,
    now_ms: u64,
    memo_key: &str,
    stdout_ref: &str,
    stderr_ref: &str,
    exit: i32,
    artifacts: &[(String, String)],
    sig_b64: &str,
) -> Result<bool, KeySelectError> {
    let entry = select_attestation_key(keys, key_id, now_ms)?;
    Ok(crate::attest_v2::verify_result_binding_v2(
        &entry.pubkey_b64,
        memo_key,
        stdout_ref,
        stderr_ref,
        exit,
        artifacts,
        sig_b64,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// Workspace-root `conformance/` dir (crates/hugit-checks → two levels up).
    fn conformance_path(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root")
            .join("conformance")
            .join(name)
    }

    #[derive(Debug, serde::Deserialize)]
    struct SelectionVector {
        #[allow(dead_code)]
        description: String,
        keys: Vec<KeyEntry>,
        cases: Vec<Case>,
    }

    #[derive(Debug, serde::Deserialize)]
    struct Case {
        name: String,
        attestation_key_id: String,
        now_ms: u64,
        expect: String,
        expected_key_id: Option<String>,
        expected_reason: Option<String>,
    }

    /// Map a [`KeySelectError`] to the vector's stable `expected_reason` string.
    fn reason_str(e: KeySelectError) -> &'static str {
        match e {
            KeySelectError::UnknownKeyId => "unknown_key_id",
            KeySelectError::Expired => "expired",
        }
    }

    /// Every case in `conformance/attestation_keyset_selection.json` must match
    /// the reference selector — mirroring the corelink-runners golden test.
    #[test]
    fn attestation_keyset_selection_vector_matches_reference_selector() {
        let raw = std::fs::read_to_string(conformance_path("attestation_keyset_selection.json"))
            .expect("attestation_keyset_selection.json must be present");
        let vector: SelectionVector =
            serde_json::from_str(&raw).expect("attestation_keyset_selection.json must deserialize");

        assert!(
            !vector.cases.is_empty(),
            "the vector must carry decision cases"
        );

        for case in &vector.cases {
            let got = select_attestation_key(&vector.keys, &case.attestation_key_id, case.now_ms);
            match case.expect.as_str() {
                "accept" => {
                    let entry = got.unwrap_or_else(|e| {
                        panic!("case {:?}: expected ACCEPT, got reject {e:?}", case.name)
                    });
                    let want = case
                        .expected_key_id
                        .as_deref()
                        .expect("an accept case must pin expected_key_id");
                    assert_eq!(
                        entry.key_id, want,
                        "case {:?}: selected the wrong key",
                        case.name
                    );
                    assert!(
                        case.expected_reason.is_none(),
                        "case {:?}: an accept case must not pin expected_reason",
                        case.name
                    );
                }
                "reject" => {
                    let err = got.expect_err(&format!(
                        "case {:?}: expected REJECT, got accept",
                        case.name
                    ));
                    let want = case
                        .expected_reason
                        .as_deref()
                        .expect("a reject case must pin expected_reason");
                    assert_eq!(
                        reason_str(err),
                        want,
                        "case {:?}: wrong reject reason",
                        case.name
                    );
                    assert!(
                        case.expected_key_id.is_none(),
                        "case {:?}: a reject case must not pin expected_key_id",
                        case.name
                    );
                }
                other => panic!("case {:?}: unknown expect {other:?}", case.name),
            }
        }
    }

    /// SECURITY REGRESSION (CoreLink Runners TL hardening, 2026-06-26): an absent /
    /// empty / malformed `result_binding_sig_v2` MUST verify as FALSE — NEVER a silent
    /// fallback to v1 (v1 does not bind `exit`/`artifacts`, so a v1-only result is
    /// forgeable on the pass/fail verdict). `verify_with_keyset` is pure-v2 (no v1
    /// branch exists by construction); this PINS it: a SELECTED, in-window key with an
    /// empty/garbage sig is a hard reject — `Ok(false)`, never `Ok(true)`, never a
    /// panic. The downgrade-by-stripping-v2 vector therefore cannot succeed in the
    /// keyset enforce path. Pinned against the dev key from
    /// `conformance/attestation_keyset_selection.json` (the prod key flips on at enforce).
    #[test]
    fn empty_or_malformed_v2_sig_is_a_hard_reject_never_v1_fallback() {
        // A real, in-set key so SELECTION succeeds — the rejection is then the SIGNATURE
        // check (the enforce path), not a key-select miss.
        let keys = [KeyEntry {
            key_id: "2d16e9ef2102df2a".into(),
            pubkey_b64: "+X0vGNFOSY5t9jo7OTlJNZsoLZOxE172jw/QURNEYw4=".into(),
            expires_ms: None,
        }];
        let artifacts: &[(String, String)] = &[];
        for stripped in ["", "AAAA", "not-valid-base64-$$$"] {
            assert_eq!(
                verify_with_keyset(
                    &keys,
                    "2d16e9ef2102df2a",
                    1_000,
                    "memo",
                    "blob:o",
                    "blob:e",
                    0,
                    artifacts,
                    stripped,
                ),
                Ok(false),
                "an absent/empty/malformed v2 sig must be a hard reject (sig={stripped:?})"
            );
        }
    }
}
