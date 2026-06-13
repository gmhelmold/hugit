//! Bearer auth — the Wave-1 STUB.
//!
//! Validates `Authorization: Bearer <token>` against a single configured dev
//! token (`HUGIT_ENGINE_DEV_TOKEN`). This is the deliberate Wave-1 stand-in for
//! the real **ADR-0002 / RFC-8693 Clerk-JWKS** validation, which is the **P2
//! identity seam** (needs the CoreLink tenant). Fail-closed: no configured token
//! ⇒ the server does not start (see `main`); a missing/wrong header ⇒ 401.
//! A constant-time compare avoids a token-guessing timing oracle.

use crate::error::EngineErr;

/// Verify the request's `Authorization` header carries `Bearer <expected>`.
/// Returns `Ok(())` on match, else `EngineErr::token_invalid()` (401).
pub fn check_bearer(headers: &[tiny_http::Header], expected: &str) -> Result<(), EngineErr> {
    let presented = headers
        .iter()
        .find(|h| h.field.equiv("Authorization"))
        .and_then(|h| {
            let v = h.value.as_str();
            v.strip_prefix("Bearer ").map(str::to_string)
        });
    match presented {
        Some(tok) if constant_time_eq(tok.as_bytes(), expected.as_bytes()) => Ok(()),
        _ => Err(EngineErr::token_invalid()),
    }
}

/// Length-checked constant-time byte compare (no early-out on the first differing
/// byte) — denies a timing side-channel on the dev token.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
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
