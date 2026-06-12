//! Identifier validation — WH-IDENT (adversarial Round-4, Cluster A);
//! detector-unified at WJ-INT (adversarial Round-6 residual).
//!
//! Identifier fields (`--campaign`, `--owner`, `--pr`, `--id`, `--run-id`) are
//! ADDRESSES, not free text.  The scrub boundary in `porcelain.rs` exempts them
//! from the bare-hex + entropy scan so addressing stays consistent — but that
//! exemption is only safe when the STRUCTURAL-secret scrub there still redacts a
//! prefixed/conn-string/JWT/PEM secret in an identifier field.
//!
//! ⚠ This validator is the door (an early, clear rejection of obvious credential
//! shapes — a clean exit-2 at input instead of a `[REDACTED]` at rest).  The
//! SECURITY boundary is the structural-secret scrub in `porcelain.rs`; do not
//! weaken that on the assumption this validator is sufficient.  This file only
//! rejects identifiers that carry a recognisable credential shape; bare hex,
//! ULIDs, slugs, and URLs without embedded credentials all pass through to the
//! central scrub boundary unchanged.
//!
//! ## Two rules
//!
//! 1. **Non-empty**: an all-whitespace identifier is rejected with
//!    `invalid_argument` / exit-2.  An agent that passes `--campaign ''` must
//!    not create a log entry from nothing (the same ghost-record principle as
//!    the WF-CLI2 `close`/`abandon` ghost-record fix).
//!
//! 2. **No structural-secret shape — by REUSING the engine's detector**: rule 2
//!    no longer carries a hand-maintained prefix list (which drifted WEAKER than
//!    the redaction engine — it omitted `xoxb-`/`xoxp-`/other Slack, `clp_`
//!    CoreLink PATs, `Bearer `, used `starts_with` not substring, and never
//!    trimmed whitespace, so `intent new --id xoxb-…` PASSED the door and was
//!    stored VERBATIM in `.hugit/intents.json`).  Instead it asks the SAME
//!    structural-secret detector the scrub boundary uses:
//!    [`crate::porcelain::structural_secret_scrub`].  The value (trimmed) is
//!    rejected with `secret_in_identifier` / exit-2 **iff** scrubbing it changes
//!    it — i.e. iff a structural detector (known-prefix credential, `sk-…` key,
//!    PEM block, connection-string password, keyword-context) fired.  The door
//!    and the engine can therefore never drift apart again.
//!
//!    ⚠ Bare 40-hex / 64-hex / ULID / slug ADDRESSES are NOT rejected — the
//!    structural scrub deliberately exempts the bare-hex + entropy scan, so a
//!    high-entropy content-address survives the door exactly as it survives the
//!    scrub boundary.  Only the structural credential SHAPES the engine knows
//!    are rejected.
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
/// Returns `Ok(())` when the value is non-empty and trips NO structural-secret
/// detector (so a bare-hex / ULID / slug address passes through).
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

    // Rule 2 — reject structural-secret shapes by REUSING the engine's detector.
    //
    // The door and the scrub boundary share ONE detector
    // ([`crate::porcelain::structural_secret_scrub`]) so they can never drift:
    // an identifier whose trimmed value would be SCRUBBED at the boundary (a
    // known-prefix credential — ghp_/gho_/ghs_/github_pat_/AKIA/xoxb-/xoxp-/…/
    // clp_/Bearer/eyJ, an `sk-…` key, a PEM private-key block, a
    // `://user:pass@` connection string, or a `keyword=value` context secret)
    // is rejected here with a clear exit-2 at the door — BEFORE it is stored
    // verbatim in `.hugit/intents.json` or the forever-log.
    //
    // The structural scrub deliberately EXEMPTS the bare-hex + entropy scan, so
    // a 40/64-hex content address, a ULID, or a high-entropy slug id is NOT a
    // structural secret and passes through unchanged (it MUST — it is an
    // address). We trim first so leading/trailing whitespace cannot smuggle a
    // prefix past a naive `starts_with`.
    let trimmed = value.trim();
    if crate::porcelain::structural_secret_scrub(trimmed) != trimmed {
        return Err(IdentError {
            kind: "secret_in_identifier",
            message: format!(
                "{field_name} looks like a credential (it trips the redaction \
                 engine's structural-secret detector): identifiers are stored \
                 unredacted in the forever-log"
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
    fn sk_real_key_is_secret() {
        // A real `sk-` key (≥20 token chars after the prefix) trips the engine's
        // length-gated `sk-` detector. A SHORT `sk-…` id (e.g. `sk-256`,
        // `sk-learn`) is NOT a secret and survives — see `sk_short_id_is_valid`.
        let e = validate_identifier(
            "sk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF",
            "--campaign",
        )
        .unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn slack_bot_token_is_secret() {
        // WJ-INT: `xoxb-` was OMITTED by the old hand-list — it now rejects via
        // the unified engine detector (was accepted + stored verbatim).
        let e = validate_identifier("xoxb-2222-3333-abcdefghij", "--id").unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn slack_user_token_is_secret() {
        let e = validate_identifier("xoxp-1111-2222-aaaaaaaaaaaa", "--campaign").unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn corelink_pat_is_secret() {
        // `clp_` was OMITTED by the old hand-list — now rejected via the engine.
        let e = validate_identifier("clp_live_9f8e7d6c5b4a3210fedcba9876543210", "--campaign")
            .unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn bearer_token_is_secret() {
        // `Bearer ` was OMITTED by the old hand-list — now rejected.
        let e = validate_identifier("Bearer abc123def456ghi789jkl", "--pr").unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn leading_whitespace_cannot_smuggle_a_prefix() {
        // The old `starts_with` (no trim) let `" ghp_…"` slip past the door; the
        // unified validator trims first, so a padded prefix is still rejected.
        let e = validate_identifier("   ghp_16C7e42F292c6912E7710c838347Ae178B4a", "--campaign")
            .unwrap_err();
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
    fn ulid_address_is_valid() {
        // A ULID is a legitimate high-entropy address — the structural scrub
        // exempts the entropy scan, so it MUST pass the door.
        validate_identifier("01HQXW8ZK4M9P2N7R3T5V6Y8BC", "--id").expect("ULID must be valid");
    }

    #[test]
    fn short_sk_id_is_valid() {
        // `sk-256` / `sk-learn` are short `sk-` words below the engine's 20-char
        // gate — NOT secrets, so they survive the door (no over-trigger).
        validate_identifier("sk-256", "--id").expect("short sk- id must be valid");
        validate_identifier("sk-learn", "--id").expect("sk-learn must be valid");
    }

    #[test]
    fn slash_slug_branch_name_is_valid() {
        // A git-style ref name with a `/` is an address, not a credential.
        validate_identifier("feature/login", "--id").expect("branch slug must be valid");
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
