//! `result_binding_sig_v2` verifier (contract §7.1 amendment v1.4.0).
//!
//! The v1 `result_binding_sig` covers `memo_key‖stdout_ref‖stderr_ref` but NOT the
//! pass/fail verdict (`CheckResult.exit`) or the output `artifacts` — so a malicious
//! runner or MITM could flip `exit: 1 → 0` and rewrite `artifacts` with the
//! v1-covered fields intact, and a v1-only verifier still accepts it (the
//! verdict-forgery window). v2 extends the signed pre-image to cover `exit` +
//! `artifacts`; this is hugit's verifier for it.
//!
//! Path 1 (transcribe in hugit's own Rust, no SDK dep) — decided with the
//! corelink-runners TL. The pre-image is built by the single-sourced
//! [`hugit_refstore::result_binding_preimage_v2`] (never re-transcribed here); this
//! module only decodes the key/sig and runs the ed25519 check. Proven byte-exact
//! against the shared `conformance/result_binding_v2.json` vector (see tests).
//!
//! Live wiring into the attestation-verify path is the P2 AC seam; the verifier
//! itself is complete + conformance-green now, so P2 is plumbing.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as B64;
use ed25519_dalek::{Signature, VerifyingKey};

/// Verify a `result_binding_sig_v2` over a `CheckResult`'s bound fields.
///
/// Returns `true` IFF the detached ed25519 signature `sig_b64` is valid for
/// `fabric_pubkey_b64` over the v2 pre-image of the given fields. Any malformed
/// input (bad base64, wrong key/sig length, a signature that does not verify)
/// returns `false` — fail-closed, never a panic, never a partial "maybe".
///
/// `artifacts` are `(path, digest)` pairs in `CheckResult.artifacts` Vec order —
/// the order IS part of the binding (reordering changes the pre-image).
#[must_use]
pub fn verify_result_binding_v2(
    fabric_pubkey_b64: &str,
    memo_key: &str,
    stdout_ref: &str,
    stderr_ref: &str,
    exit: i32,
    artifacts: &[(String, String)],
    sig_b64: &str,
) -> bool {
    let Some(verifying_key) = decode_verifying_key(fabric_pubkey_b64) else {
        return false;
    };
    let Some(signature) = decode_signature(sig_b64) else {
        return false;
    };
    let preimage = hugit_refstore::result_binding_preimage_v2(
        memo_key, stdout_ref, stderr_ref, exit, artifacts,
    );
    // verify_strict rejects the ed25519 malleability / small-order edge cases.
    verifying_key.verify_strict(&preimage, &signature).is_ok()
}

/// Decode a base64 ed25519 public key into a [`VerifyingKey`] (32 bytes). `None` on
/// any decode/length/point error.
fn decode_verifying_key(b64: &str) -> Option<VerifyingKey> {
    let bytes = B64.decode(b64).ok()?;
    let arr: [u8; 32] = bytes.try_into().ok()?;
    VerifyingKey::from_bytes(&arr).ok()
}

/// Decode a base64 ed25519 signature into a [`Signature`] (64 bytes). `None` on a
/// decode/length error.
fn decode_signature(b64: &str) -> Option<Signature> {
    let bytes = B64.decode(b64).ok()?;
    let arr: [u8; 64] = bytes.try_into().ok()?;
    Some(Signature::from_bytes(&arr))
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

    struct Vector {
        fabric_pubkey_b64: String,
        memo_key: String,
        stdout_ref: String,
        stderr_ref: String,
        exit: i32,
        artifacts: Vec<(String, String)>,
        preimage_hex: String,
        sig_b64: String,
    }

    fn load_vector() -> Vector {
        let raw = std::fs::read_to_string(conformance_path("result_binding_v2.json"))
            .expect("read result_binding_v2.json");
        let v: serde_json::Value = serde_json::from_str(&raw).expect("parse vector");
        let input = &v["input"];
        let artifacts = input["artifacts"]
            .as_array()
            .expect("artifacts array")
            .iter()
            .map(|a| {
                (
                    a["path"].as_str().unwrap().to_string(),
                    a["digest"].as_str().unwrap().to_string(),
                )
            })
            .collect();
        Vector {
            fabric_pubkey_b64: v["fabric_pubkey_b64"].as_str().unwrap().to_string(),
            memo_key: input["memo_key"].as_str().unwrap().to_string(),
            stdout_ref: input["stdout_ref"].as_str().unwrap().to_string(),
            stderr_ref: input["stderr_ref"].as_str().unwrap().to_string(),
            exit: i32::try_from(input["exit"].as_i64().unwrap()).unwrap(),
            artifacts,
            preimage_hex: v["preimage_hex"].as_str().unwrap().to_string(),
            sig_b64: v["result_binding_sig_v2"].as_str().unwrap().to_string(),
        }
    }

    /// The single-sourced pre-image builder reproduces the frozen vector bytes
    /// EXACTLY — the drift tripwire for the v2 formula.
    #[test]
    fn preimage_matches_conformance_vector_byte_exact() {
        let v = load_vector();
        let preimage = hugit_refstore::result_binding_preimage_v2(
            &v.memo_key,
            &v.stdout_ref,
            &v.stderr_ref,
            v.exit,
            &v.artifacts,
        );
        assert_eq!(
            hex::encode(&preimage),
            v.preimage_hex,
            "v2 pre-image bytes must match conformance/result_binding_v2.json exactly"
        );
    }

    /// The fabric's published signature verifies under the fabric pubkey — hugit
    /// accepts a genuine v2 attestation.
    #[test]
    fn genuine_signature_verifies() {
        let v = load_vector();
        assert!(
            verify_result_binding_v2(
                &v.fabric_pubkey_b64,
                &v.memo_key,
                &v.stdout_ref,
                &v.stderr_ref,
                v.exit,
                &v.artifacts,
                &v.sig_b64,
            ),
            "the genuine fabric signature must verify"
        );
    }

    /// THE point of v2: flipping the verdict (`exit: 1 → 0`) with every v1-covered
    /// field intact must FAIL — the forgery the v1 binding could not catch.
    #[test]
    fn flipped_exit_is_rejected() {
        let v = load_vector();
        assert!(
            !verify_result_binding_v2(
                &v.fabric_pubkey_b64,
                &v.memo_key,
                &v.stdout_ref,
                &v.stderr_ref,
                0, // forged pass
                &v.artifacts,
                &v.sig_b64,
            ),
            "a flipped exit must NOT verify (this is the verdict-forgery v2 closes)"
        );
    }

    /// Rewriting an artifact digest must FAIL — artifacts are bound.
    #[test]
    fn tampered_artifact_is_rejected() {
        let v = load_vector();
        let mut artifacts = v.artifacts.clone();
        artifacts[0].1 = "0".repeat(64); // swap the first artifact's digest
        assert!(
            !verify_result_binding_v2(
                &v.fabric_pubkey_b64,
                &v.memo_key,
                &v.stdout_ref,
                &v.stderr_ref,
                v.exit,
                &artifacts,
                &v.sig_b64,
            ),
            "a rewritten artifact digest must NOT verify"
        );
    }

    /// Malformed inputs fail closed (no panic, returns false).
    #[test]
    fn malformed_inputs_fail_closed() {
        let v = load_vector();
        assert!(!verify_result_binding_v2(
            "not-base64-!!!",
            &v.memo_key,
            &v.stdout_ref,
            &v.stderr_ref,
            v.exit,
            &v.artifacts,
            &v.sig_b64
        ));
        assert!(!verify_result_binding_v2(
            &v.fabric_pubkey_b64,
            &v.memo_key,
            &v.stdout_ref,
            &v.stderr_ref,
            v.exit,
            &v.artifacts,
            "AAAA"
        ));
    }
}
