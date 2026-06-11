//! View-boundary redaction filter (④) — the real detector set.
//!
//! A string is redacted (replaced wholesale with the sentinel
//! [`REDACTED`]) when ANY of three detectors fires:
//!
//! 1. **The planted marker** `SECRET:` — the canonical test/marker path,
//!    kept for backward compatibility (existing fixtures plant it).
//! 2. **Known-prefix credential patterns** — a hand-rolled, gitleaks-class
//!    set of shapes that are secrets by construction: GitHub tokens
//!    (`ghp_`/`gho_`/`github_pat_`), AWS access keys (`AKIA…`), Slack tokens
//!    (`xoxb-`/`xoxp-`/`xoxo-`/`xoxa-`/`xoxs-`), OpenAI-style keys (`sk-`),
//!    CoreLink PATs (`clp_`), PEM private-key blocks
//!    (`-----BEGIN … PRIVATE KEY`), `Bearer ` tokens, and `eyJ…` JWTs.
//! 3. **A high-entropy scan** — any base64/hex run of ≥ 20 chars whose
//!    Shannon entropy clears a threshold is treated as a credential. This
//!    catches unprefixed secrets while leaving prose, code, and structured
//!    hashes alone.
//!
//! **False-positive guard.** Content-address refs are load-bearing and must
//! survive: a `cas:` ref and any bare 40/64-char hex digest (sha-1 / sha-256
//! shapes) are EXEMPT from the entropy scan. They are honest references to
//! deduped content, not secrets, and the integrity spine depends on them
//! appearing verbatim in ledger/verdict output.
//!
//! The replacement is single-sourced from the canonical
//! [`hugit_contracts::REDACTED_MARKER`] so it cannot drift from the CLI
//! export redaction.

/// The planted-secret prefix that triggers redaction (the marker path).
pub const SECRET_MARKER: &str = "SECRET:";

/// The redaction sentinel rendered in ledger/verdict views. Single-sourced from
/// the canonical [`hugit_contracts::REDACTED_MARKER`] so it cannot drift from the
/// CLI export redaction.
pub const REDACTED: &str = hugit_contracts::REDACTED_MARKER;

/// Minimum length of a base64/hex token run before the entropy scan considers
/// it (short runs are noise — variable names, hex colours, etc.).
const ENTROPY_MIN_LEN: usize = 20;

/// Shannon-entropy threshold (bits/char) above which a long token run is
/// treated as a credential. Random base64 approaches ~6.0 bits/char; a
/// 64-hex digest sits near ~4.0; English prose runs well under 3.0. 4.0 keeps
/// hex digests below the line (and they are exempted regardless) while
/// catching the dense alphabet of real keys.
const ENTROPY_THRESHOLD: f64 = 4.0;

/// Known credential prefixes that are secrets by construction. A field
/// containing any of these (case-sensitive, as the issuers mint them) is
/// redacted wholesale.
const KNOWN_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "github_pat_",
    "AKIA",
    "xoxb-",
    "xoxp-",
    "xoxo-",
    "xoxa-",
    "xoxs-",
    "sk-",
    "clp_",
    "Bearer ",
    "eyJ", // JWT header (base64 of `{"`)
];

/// Apply view-boundary redaction to a string field.
///
/// Returns the [`REDACTED`] sentinel if any detector fires; otherwise returns
/// the input unchanged (as an owned `String`).
pub fn apply(s: &str) -> String {
    if is_secret(s) {
        REDACTED.to_string()
    } else {
        s.to_string()
    }
}

/// True iff any detector classifies `s` as secret-bearing.
fn is_secret(s: &str) -> bool {
    // (1) marker path
    if s.contains(SECRET_MARKER) {
        return true;
    }
    // (2) known prefixes — substring match (a secret embedded mid-line counts)
    if KNOWN_PREFIXES.iter().any(|p| s.contains(p)) {
        return true;
    }
    // PEM private-key blocks: `-----BEGIN … PRIVATE KEY` (the `BEGIN ` prefix
    // is not in the table because we gate on the PRIVATE-KEY phrasing).
    if s.contains("-----BEGIN") && s.contains("PRIVATE KEY") {
        return true;
    }
    // (3) high-entropy scan over base64/hex runs, with the cas-ref exemption.
    high_entropy_token(s)
}

/// True iff `s` contains a long base64/hex run whose Shannon entropy clears the
/// threshold — EXCLUDING content-address refs (`cas:` and bare 40/64-hex
/// digests), which are load-bearing references, not secrets.
fn high_entropy_token(s: &str) -> bool {
    for token in s.split(|c: char| !is_token_char(c)) {
        if token.len() < ENTROPY_MIN_LEN {
            continue;
        }
        if is_content_address_ref(token) {
            continue;
        }
        if shannon_entropy(token) >= ENTROPY_THRESHOLD {
            return true;
        }
    }
    false
}

/// A character that can appear inside a base64/hex token run.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '-' || c == '_'
}

/// True for content-address refs that must survive verbatim: a `cas:`-prefixed
/// ref, or a bare 40/64-char hex digest (sha-1 / sha-256 shapes). These are
/// honest references to deduped content; the integrity spine surfaces them.
fn is_content_address_ref(token: &str) -> bool {
    if token.starts_with("cas:") {
        return true;
    }
    // Strip a leading algorithm prefix like `sha256:` so `sha256:<hex>` is also
    // recognised as a digest ref.
    let hex = token.rsplit(':').next().unwrap_or(token);
    matches!(hex.len(), 40 | 64) && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Shannon entropy of a token in bits per character.
fn shannon_entropy(token: &str) -> f64 {
    let bytes = token.as_bytes();
    if bytes.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for &b in bytes {
        counts[b as usize] += 1;
    }
    let len = bytes.len() as f64;
    let mut entropy = 0.0;
    for &count in counts.iter() {
        if count == 0 {
            continue;
        }
        let p = count as f64 / len;
        entropy -= p * p.log2();
    }
    entropy
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── marker path (existing tests rely on it) ──────────────────────────────

    #[test]
    fn non_secret_passthrough() {
        assert_eq!(apply("normal text"), "normal text");
    }

    #[test]
    fn secret_marker_becomes_redacted() {
        assert_eq!(apply("SECRET:my-api-key-12345"), REDACTED);
    }

    #[test]
    fn embedded_marker_becomes_redacted() {
        assert_eq!(apply("prefix SECRET: suffix"), REDACTED);
    }

    // ── (a) known-prefix detectors — one realistic specimen each ─────────────

    #[test]
    fn github_pat_classic() {
        assert_eq!(
            apply("token=ghp_16C7e42F292c6912E7710c838347Ae178B4a"),
            REDACTED
        );
    }

    #[test]
    fn github_oauth() {
        assert_eq!(apply("gho_16C7e42F292c6912E7710c838347Ae178B4a"), REDACTED);
    }

    #[test]
    fn github_fine_grained_pat() {
        assert_eq!(
            apply("github_pat_11ABCDEFG0aBcDeFgHiJ_kLmNoPqRsTuVwXyZ012345"),
            REDACTED
        );
    }

    #[test]
    fn aws_access_key() {
        assert_eq!(apply("AKIAIOSFODNN7EXAMPLE"), REDACTED);
    }

    #[test]
    fn slack_bot_token() {
        assert_eq!(
            apply("xoxb-2222222222-3333333333-abcdefghijklmnop"),
            REDACTED
        );
    }

    #[test]
    fn slack_user_token() {
        assert_eq!(apply("xoxp-1111-2222-aaaaaaaaaaaa"), REDACTED);
    }

    #[test]
    fn openai_style_key() {
        assert_eq!(apply("sk-proj-aBcDeF0123456789ghIjKlMnOpQrStUv"), REDACTED);
    }

    #[test]
    fn corelink_pat() {
        assert_eq!(apply("clp_live_9f8e7d6c5b4a3210fedcba9876543210"), REDACTED);
    }

    #[test]
    fn pem_private_key() {
        assert_eq!(
            apply("-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQ..."),
            REDACTED
        );
    }

    #[test]
    fn bearer_token() {
        assert_eq!(apply("Authorization: Bearer abc123def456ghi789"), REDACTED);
    }

    #[test]
    fn jwt() {
        assert_eq!(
            apply("eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.abc123def"),
            REDACTED
        );
    }

    // ── (b) high-entropy scan — unprefixed secret ────────────────────────────

    #[test]
    fn high_entropy_base64_secret() {
        // A dense 32-char base64 blob with no known prefix.
        assert_eq!(apply("creds: 8Kp2mZ9qLx4vTn7wRj3sYb6cFd1gHe0"), REDACTED);
    }

    // ── benign content must NOT false-positive ───────────────────────────────

    #[test]
    fn benign_prose_survives() {
        let s = "Refactored the authentication module to drop the legacy path.";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn benign_code_survives() {
        let s = "let result = compute_checksum(&buffer).unwrap_or_default();";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn sha256_digest_survives() {
        // A bare 64-hex sha-256 digest is load-bearing — must NOT be redacted.
        let s = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn sha1_digest_survives() {
        let s = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn cas_ref_survives() {
        let s = "cas:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn prefixed_digest_ref_survives() {
        let s = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn commit_message_with_hash_survives() {
        // A realistic provenance line carrying a content-address ref.
        let s = "landed intent a31f9c at tree 7777777777777777777777777777777777777777";
        assert_eq!(apply(s), s);
    }

    // ── direct entropy-fn sanity ─────────────────────────────────────────────

    #[test]
    fn entropy_orders_random_above_repeated() {
        let random = shannon_entropy("8Kp2mZ9qLx4vTn7wRj3sYb6cFd1gHe0");
        let repeated = shannon_entropy("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(random > repeated);
        assert!(random >= ENTROPY_THRESHOLD);
        assert!(repeated < ENTROPY_THRESHOLD);
    }

    #[test]
    fn content_address_ref_recognised() {
        assert!(is_content_address_ref(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        assert!(is_content_address_ref(
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        ));
        assert!(is_content_address_ref("cas:anything-here"));
        assert!(!is_content_address_ref("8Kp2mZ9qLx4vTn7wRj3sYb6cFd1gHe0"));
    }
}
