//! Identifier validation — WH-IDENT (adversarial Round-4, Cluster A);
//! detector-unified at WJ-INT (adversarial Round-6 residual).
//!
//! Identifier fields (`--campaign`, `--owner`, `--pr`, `--id`, `--run-id`) are
//! ADDRESSES, not free text.  The scrub boundary in `porcelain.rs` is now
//! DENY-BY-DEFAULT (L-A, adversarial Round 8): an identifier value survives
//! verbatim ONLY if it is a PROVABLE safe-address shape
//! ([`crate::porcelain::is_safe_identifier_shape`] — 40/64-hex / `cas:`/`<algo>:`
//! content address, ULID, bounded-charset low-entropy slug); anything else —
//! including a prefix-less high-entropy credential — is `[REDACTED]`.  This door
//! shares that one gate, so it stays in lockstep with the boundary.
//!
//! ⚠ This validator is the door (an early, clear rejection at input — a clean
//! exit-2 instead of a `[REDACTED]` at rest).  The SECURITY boundary is the scrub
//! in `porcelain.rs`; do not weaken it on the assumption this validator is
//! sufficient.  Bare hex, ULIDs, and low-entropy slugs pass the door (they are
//! provable addresses); a high-entropy credential blob is now rejected here too,
//! because the boundary it mirrors would redact it.
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
//!    it — i.e. iff it is NOT a provable safe-address shape (a known-prefix
//!    credential, `sk-…` key, PEM block, connection-string, keyword-context, OR
//!    — since L-A — a prefix-less high-entropy credential blob).  The door and
//!    the boundary share the one gate, so they can never drift apart.
//!
//!    ⚠ Bare 40-hex / 64-hex content addresses, ULIDs, and low-entropy slug
//!    ADDRESSES are NOT rejected — they are provable address shapes the gate
//!    blesses, so they survive the door exactly as they survive the scrub
//!    boundary.  Only non-address values (credential shapes + dense high-entropy
//!    blobs) are rejected.
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
        let e = validate_identifier("xo\x78b-2222-3333-abcdefghij", "--id").unwrap_err();
        assert_eq!(e.kind, "secret_in_identifier");
    }

    #[test]
    fn slack_user_token_is_secret() {
        let e = validate_identifier("xo\x78p-1111-2222-aaaaaaaaaaaa", "--campaign").unwrap_err();
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
        // A genuine 64-hex (sha-256) content-address MUST be valid. (This string
        // was previously 66 chars — mislabeled "64-hex" — which the PS-14 hybrid
        // correctly redacts as an odd-length non-digest hex; corrected to a real
        // 64-hex so it tests the intended {40,64} digest exemption.)
        validate_identifier(
            "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678aabbccddeeff001122334455",
            "--campaign",
        )
        .expect("64-hex must be a valid identifier");
    }

    #[test]
    fn human_readable_slug_is_valid() {
        validate_identifier("auth-hardening", "--campaign").expect("slug must be valid");
        validate_identifier("owner@example.com", "--owner").expect("email must be valid");
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
