//! Bearer auth — the Wave-1 STUB.
//!
//! Validates `Authorization: Bearer <token>` against a single configured dev
//! token (`HUGIT_ENGINE_DEV_TOKEN`). This is the deliberate Wave-1 stand-in for
//! the real **ADR-0002 / RFC-8693 Clerk-JWKS** validation, which is the **P2
//! identity seam** (needs the CoreLink tenant). Fail-closed: no configured token
//! ⇒ the server does not start (see `main`); a missing/wrong header ⇒ 401.
//!
//! Comparison is constant-time AND length-invariant: both sides are SHA-256'd
//! and the 32-byte digests are XOR-folded. This leaks neither which byte differs
//! NOR the token's length (a plain length-checked compare leaks the length via an
//! early return) — no timing oracle on the dev token.

use sha2::{Digest, Sha256};

use crate::error::EngineErr;

/// Verify the request's `Authorization` header carries `Bearer <expected>`.
/// Returns `Ok(())` on match, else `EngineErr::token_invalid()` (401).
pub fn check_bearer(headers: &[tiny_http::Header], expected: &str) -> Result<(), EngineErr> {
    let presented = headers
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .and_then(|h| h.value.as_str().strip_prefix("Bearer ").map(str::to_string));
    match presented {
        Some(tok) if tokens_match(tok.as_bytes(), expected.as_bytes()) => Ok(()),
        _ => Err(EngineErr::token_invalid()),
    }
}

/// Length-invariant constant-time token equality: compare `SHA-256(a)` vs
/// `SHA-256(b)` (fixed 32-byte digests) with a branch-free XOR fold. No early
/// return reveals the length or the first differing byte.
fn tokens_match(a: &[u8], b: &[u8]) -> bool {
    let da = Sha256::digest(a);
    let db = Sha256::digest(b);
    let mut diff: u8 = 0;
    for (x, y) in da.iter().zip(db.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdr(name: &str, value: &str) -> tiny_http::Header {
        tiny_http::Header::from_bytes(name.as_bytes(), value.as_bytes()).unwrap()
    }

    #[test]
    fn correct_bearer_passes() {
        let h = [hdr("Authorization", "Bearer s3cr3t-dev-token")];
        assert!(check_bearer(&h, "s3cr3t-dev-token").is_ok());
    }

    #[test]
    fn wrong_token_is_401() {
        let h = [hdr("Authorization", "Bearer nope")];
        assert_eq!(
            check_bearer(&h, "s3cr3t-dev-token").unwrap_err().status,
            401
        );
    }

    #[test]
    fn missing_header_is_401() {
        let h: [tiny_http::Header; 0] = [];
        assert_eq!(
            check_bearer(&h, "s3cr3t-dev-token").unwrap_err().status,
            401
        );
    }

    #[test]
    fn non_bearer_scheme_is_401() {
        let h = [hdr("Authorization", "Basic dXNlcjpwYXNz")];
        assert_eq!(
            check_bearer(&h, "s3cr3t-dev-token").unwrap_err().status,
            401
        );
    }
}
