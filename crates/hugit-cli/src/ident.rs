//! Identifier validation — WH-IDENT (adversarial Round-4, Cluster A).
//!
//! Identifier fields (`--campaign`, `--owner`, `--pr`, `--id`, `--run-id`) are
//! ADDRESSES, not free text.  WH-SCRUB exempts them from the scrub engine so
//! addressing stays consistent — but that exemption is only safe when every
//! identifier is validated at INPUT so an exempt field can never carry a secret
//! or be empty.
//!
//! ## Two rules
//!
//! 1. **Non-empty**: an all-whitespace identifier is rejected with
//!    `invalid_argument` / exit-2.  An agent that passes `--campaign ''` must
//!    not create a log entry from nothing (the same ghost-record principle as
//!    the WF-CLI2 `close`/`abandon` ghost-record fix).
//!
//! 2. **No known-secret prefix shape**: identifiers that match a known
//!    credential-prefix pattern (`ghp_` / `gho_` / `ghs_` / `github_pat_` /
//!    `sk-` / `AKIA` / `eyJ` JWT-start / `-----BEGIN` PEM / connection-string
//!    `://…@`) are rejected with `secret_in_identifier` / exit-2 and a hint
//!    explaining that identifiers are stored UNREDACTED.
//!
//!    ⚠ Bare 40-hex or 64-hex strings are NOT rejected — a 40-hex campaign key
//!    is a legitimate content-address and MUST be allowed.  Only recognisable
//!    credential SHAPES are rejected; high-entropy hex is an address.
//!
//! ## One shared function
//!
//! [`validate_identifier`] is the single implementation.  Each verb calls it
//! and maps [`IdentError`] to its own domain error type.

/// The canonical fix hint for a secret-shaped identifier — printed once here so
/// every verb surfaces exactly the same text.
pub const SECRET_HINT: &str = "identifiers are addresses, not secrets — they are stored unredacted; \
     do not put a credential in --campaign/--pr/--id/--run-id/--owner; \
     use a non-secret short name or content-hash instead";

/// The error returned by [`validate_identifier`].
///
/// Each verb maps this to its own domain error type (`CampaignError`,
/// `PorcelainError`, `PrError`) via the idiomatic `.map_err(IdentError::into_…)`.
/// The `kind` is a `&'static str` so it can be passed directly to error
/// constructors that require a static kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentError {
    /// Stable machine kind — `"invalid_argument"` or `"secret_in_identifier"`.
    pub kind: &'static str,
    /// Human/agent-readable message naming the offending field.
    pub message: String,
    /// The fix hint the caller can act on.
    pub fix: &'static str,
}

/// Validate a single identifier value at verb entry.
///
/// Returns `Ok(())` when the value is non-empty and does not match any
/// known-credential-prefix shape.
///
/// Returns `Err(IdentError)` (kind `invalid_argument` or
/// `secret_in_identifier`) when either rule is violated.
///
/// # Parameters
///
/// - `value`: the raw flag value supplied by the caller.
/// - `field_name`: the flag name for the error message (e.g. `"--campaign"`).
pub fn validate_identifier(value: &str, field_name: &str) -> Result<(), IdentError> {
    // Rule 1 — reject empty / whitespace-only.
    if value.trim().is_empty() {
        return Err(IdentError {
            kind: "invalid_argument",
            message: format!("{field_name} is empty"),
            fix: "pass a non-empty identifier value",
        });
    }

    // Rule 2 — reject known-credential-prefix shapes.
    //
    // Patterns chosen to match token SHAPES, not arbitrary entropy:
    //   • ghp_ / gho_ / ghs_  — GitHub PAT prefixes (classic / OAuth / server)
    //   • github_pat_           — fine-grained GitHub PATs
    //   • sk-                   — OpenAI-style secret-key prefix
    //   • AKIA                  — AWS access-key prefix
    //   • eyJ                   — JWT (base64url of `{"` always starts with this)
    //   • -----BEGIN            — PEM block (private key / cert)
    //   • ://…@                 — connection-string with embedded credentials
    //
    // We do NOT reject bare high-entropy hex (40-hex is a valid git sha / campaign
    // key — it MUST be allowed as an address).
    let prefixes: &[&str] = &[
        "ghp_",
        "gho_",
        "ghs_",
        "github_pat_",
        "sk-",
        "AKIA",
        "eyJ",
        "-----BEGIN",
    ];
    for &prefix in prefixes {
        if value.starts_with(prefix) {
            return Err(IdentError {
                kind: "secret_in_identifier",
                message: format!(
                    "{field_name} looks like a credential (starts with '{prefix}'): \
                     identifiers are stored unredacted in the forever-log"
                ),
                fix: SECRET_HINT,
            });
        }
    }
    // Connection-string pattern: scheme://…@… (embedded password)
    if value.contains("://") && value.contains('@') {
        return Err(IdentError {
            kind: "secret_in_identifier",
            message: format!(
                "{field_name} looks like a connection string with embedded credentials \
                 (contains '://' and '@'): identifiers are stored unredacted"
            ),
            fix: SECRET_HINT,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Rule 1: empty ──────────────────────────────────────────────────────────

    #[test]
    fn empty_string_is_invalid() {
        let e = validate_identifier("", "--campaign").unwrap_err();
        assert_eq!(e.kind, "invalid_argument");
    }

    #[test]
    fn whitespace_only_is_invalid() {
        let e = validate_identifier("   ", "--campaign").unwrap_err();
        assert_eq!(e.kind, "invalid_argument");
    }

    // ── Rule 2: known-secret prefixes ─────────────────────────────────────────

    #[test]
    fn ghp_prefix_is_secret_in_identifier() {
        let e = validate_identifier("ghp_16C7e42F292c6912E7710c838347Ae178B4a", "--campaign")
            .unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn gho_prefix_is_secret() {
        let e = validate_identifier("gho_abc", "--pr").unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn ghs_prefix_is_secret() {
        let e = validate_identifier("ghs_abc", "--pr").unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn github_pat_prefix_is_secret() {
        let e = validate_identifier("github_pat_abc", "--campaign").unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn sk_dash_prefix_is_secret() {
        let e = validate_identifier("sk-abc123", "--campaign").unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn akia_prefix_is_secret() {
        let e = validate_identifier("AKIAIOSFODNN7EXAMPLE", "--campaign").unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn jwt_eyj_prefix_is_secret() {
        let e = validate_identifier("eyJhbGciOiJSUzI1NiJ9.payload.sig", "--campaign").unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn pem_begin_prefix_is_secret() {
        let e = validate_identifier("-----BEGIN RSA PRIVATE KEY-----", "--campaign").unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn connection_string_is_secret() {
        let e = validate_identifier("postgres://admin:secret@db.internal:5432/app", "--campaign")
            .unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    // ── Legitimate addresses MUST pass ────────────────────────────────────────

    #[test]
    fn forty_hex_campaign_key_is_valid() {
        // A 40-hex key is a legitimate content-address; MUST NOT be rejected.
        validate_identifier("a1b2c3d4e5f60718293a4b5c6d7e8f9012345678", "--campaign")
            .expect("40-hex must be a valid identifier");
    }

    #[test]
    fn sixty_four_hex_key_is_valid() {
        validate_identifier(
            "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678aabbccddeeff00112233445566",
            "--campaign",
        )
        .expect("64-hex must be a valid identifier");
    }

    #[test]
    fn human_readable_slug_is_valid() {
        validate_identifier("auth-hardening", "--campaign").expect("slug must be valid");
        validate_identifier("gustavo@humangr.com", "--owner").expect("email must be valid");
        validate_identifier("PR-1", "--pr").expect("PR id must be valid");
        validate_identifier("intent-abc123def", "--id").expect("intent id must be valid");
    }

    #[test]
    fn url_without_credentials_is_valid() {
        // A URL with :// but no @ is not a credential-shaped connection string.
        validate_identifier("https://github.com/org/repo", "--campaign")
            .expect("URL without credentials must be valid");
    }

    #[test]
    fn at_sign_without_scheme_is_valid() {
        // An email has @ but no ://, so it is not a connection string.
        validate_identifier("user@example.com", "--owner")
            .expect("email must be valid (@ without :// is not a conn-string)");
    }
}
