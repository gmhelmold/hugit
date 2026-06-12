//! View-boundary redaction filter (④) — the real detector set.
//!
//! A string is redacted (replaced wholesale with the sentinel
//! [`REDACTED`]) when ANY of five detectors fires:
//!
//! 1. **The planted marker** `SECRET:` — the canonical test/marker path,
//!    kept for backward compatibility (existing fixtures plant it).
//! 2. **Known-prefix credential patterns** — a hand-rolled, gitleaks-class
//!    set of shapes that are secrets by construction: GitHub tokens
//!    (`ghp_`/`gho_`/`ghs_`/`github_pat_`), AWS access keys (`AKIA…`), Slack tokens
//!    (`xoxb-`/`xoxp-`/`xoxo-`/`xoxa-`/`xoxs-`), OpenAI-style keys
//!    (`sk-` followed by ≥20 base64/hex chars — short identifiers are NOT
//!    matched), CoreLink PATs (`clp_`), PEM private-key blocks
//!    (`-----BEGIN … PRIVATE KEY`), `Bearer ` tokens, and `eyJ…` JWTs.
//! 3. **Connection-string detector** — runs BEFORE tokenisation; a hand-scan
//!    (no regex, std only) that finds `://`, then checks whether the authority
//!    segment contains `:…@` (user:pass@host). A password segment is present
//!    when there is a `:` AFTER the `://` authority opening AND before the
//!    first `@` — the password segment is considered the secret. Catches
//!    `postgres://user:secret@host`, `mysql://…`, `redis://…`, `mongodb://…`,
//!    `amqp://…`, and any `https://user:pass@host` credential URL regardless
//!    of scheme length.
//! 4. **Keyword-context detector** — keyword-gated value redaction: when the
//!    string contains a recognised key name (`password`, `passwd`, `secret`,
//!    `token`, `api_key`, `pwd`) followed immediately by `=` or `:` and a
//!    non-empty, non-whitespace value, the whole string is treated as
//!    containing a credential. Case-insensitive on the keyword. Complements
//!    the entropy floor so that deliberately low-entropy passwords
//!    (`hunter2`, `abc123`) are still caught.
//! 5. **A high-entropy scan** — any base64/hex run of ≥ 20 chars whose
//!    Shannon entropy clears a threshold is treated as a credential. This
//!    catches unprefixed secrets while leaving prose, code, and structured
//!    hashes alone.
//!
//! **False-positive guard (context-scoped — WF-1).** Content-address refs are
//! load-bearing and must survive, but ONLY in a content-address CONTEXT — never
//! as a blanket bare-hex pass. A 40/64-char hex run is exempt from the entropy
//! scan ONLY when it is *prefixed* by a content-address algorithm tag
//! (`cas:` / `sha256:` / `sha1:` / any `<algo>:`). A **bare** 40/64-hex run in
//! free text (a charter / acceptance / owner / campaign / reason) is NOT a
//! content-address ref — it could be an HMAC, a Django `SECRET_KEY`, or a hex
//! API key of exactly that shape — so it is subject to the entropy scan and
//! redacts. We err toward redaction in free text.
//!
//! Structurally-typed hash fields (`files_read[].hash`, `commit`, `tree_hash`)
//! are NOT author free text: their callers store them verbatim WITHOUT routing
//! through [`apply`], so a bare digest in those positions never reaches this
//! filter and survives by construction. The only bare-hex tokens that ever hit
//! [`apply`] live inside free-text fields — exactly where a secret-shaped hex
//! run must redact.
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

/// Minimum run of base64/hex chars that must follow `sk-` for the prefix to
/// be treated as a real API key rather than a short identifier or prose word.
const SK_MIN_SUFFIX_LEN: usize = 20;

/// Keyword prefixes (lowercase) that gate the keyword-context detector.
/// Each keyword is followed by `=` or `:` and a value in the field being
/// scanned — the value is considered a credential regardless of entropy.
const KEYWORD_PREFIXES: &[&str] = &["password", "passwd", "secret", "token", "api_key", "pwd"];

/// Known credential prefixes that are secrets by construction (except `sk-`,
/// which is handled separately with a length gate). A field containing any of
/// these (case-sensitive, as the issuers mint them) is redacted wholesale.
const KNOWN_PREFIXES: &[&str] = &[
    "ghp_",
    "gho_",
    "ghs_", // GitHub Actions / server-to-server token
    "github_pat_",
    "AKIA",
    "xoxb-",
    "xoxp-",
    "xoxo-",
    "xoxa-",
    "xoxs-",
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
    // (2b) sk- with length gate: only fire if followed by ≥SK_MIN_SUFFIX_LEN
    //      base64/hex chars. This prevents `sk-256` or `src/sk-learn/` from
    //      triggering.
    if has_sk_key(s) {
        return true;
    }
    // PEM private-key blocks: `-----BEGIN … PRIVATE KEY` (the `BEGIN ` prefix
    // is not in the table because we gate on the PRIVATE-KEY phrasing).
    if s.contains("-----BEGIN") && s.contains("PRIVATE KEY") {
        return true;
    }
    // (3) connection-string detector — runs before tokenisation
    if has_connection_string_password(s) {
        return true;
    }
    // (4) keyword-context detector
    if has_keyword_context_secret(s) {
        return true;
    }
    // (5) high-entropy scan over base64/hex runs, with the cas-ref exemption.
    high_entropy_token(s)
}

// ── Detector (2b): sk- with length gate ──────────────────────────────────────

/// True iff `s` contains an `sk-` API key. The suffix run after `sk-` is the
/// maximal run of [`is_token_char`] chars — which INCLUDES `-` and `_`, so the
/// modern hyphenated `sk-proj-<id>` format is one continuous run (the old scan
/// already counts `-`/`_`; this keeps that behaviour explicit). The key fires
/// when EITHER:
///
/// - the suffix run is ≥ [`SK_MIN_SUFFIX_LEN`] token chars (a dense classic
///   `sk-<base64>` key), OR
/// - the suffix is the OpenAI **project-key** format `proj-<id>` with a
///   non-empty `<id>` — a strong structural marker (like `ghp_`), so a SHORT
///   `sk-proj-leaklens99999` redacts even though its 18-char run sits just under
///   the generic length gate (WJ-INT, Round-6 residual). We err toward redaction
///   in free text (the WF-1 policy).
///
/// Short non-key `sk-` words (`sk-256`, `sk-learn`, bare `sk-`) do NOT match:
/// their run is below the gate AND they carry no `proj-` marker.
fn has_sk_key(s: &str) -> bool {
    let needle = "sk-";
    let mut search = s;
    while let Some(pos) = search.find(needle) {
        let after = &search[pos + needle.len()..];
        // The suffix run includes `-`/`_` (is_token_char), so `proj-…` is one run.
        let run: String = after.chars().take_while(|&c| is_token_char(c)).collect();
        if run.chars().count() >= SK_MIN_SUFFIX_LEN {
            return true;
        }
        // OpenAI project-key marker: `sk-proj-<non-empty id>` is a key regardless
        // of the generic length gate (the `proj-` marker is the structural signal).
        if let Some(id) = run.strip_prefix("proj-")
            && !id.is_empty()
        {
            return true;
        }
        // Advance past this occurrence to find any further ones.
        let advance = pos + needle.len();
        if advance >= search.len() {
            break;
        }
        search = &search[advance..];
    }
    false
}

// ── Detector (3): connection-string password ──────────────────────────────────

/// True iff `s` contains a URL with an embedded password in the authority.
///
/// Hand-scan (no regex, std only):
/// 1. Find `://` — marks the start of the authority.
/// 2. Within the authority (up to the next `/`, `?`, `#`, or end-of-string),
///    look for `:` followed by `@` — the segment between `:` and `@` is the
///    password. An empty password segment (`user:@host`) is not treated as a
///    secret (no value = no credential).
fn has_connection_string_password(s: &str) -> bool {
    let mut search = s;
    while let Some(scheme_end) = search.find("://") {
        // Authority starts after `://`
        let authority_start = scheme_end + 3;
        if authority_start >= search.len() {
            break;
        }
        let authority_str = &search[authority_start..];
        // Authority ends at the first `/`, `?`, `#`, or end-of-string.
        let authority_len = authority_str
            .find(['/', '?', '#'])
            .unwrap_or(authority_str.len());
        let authority = &authority_str[..authority_len];

        // Check for `@` in the authority — only then is there a userinfo block.
        if let Some(at_pos) = authority.find('@') {
            let userinfo = &authority[..at_pos];
            // A `:` in userinfo separates user from password.
            if let Some(colon_pos) = userinfo.find(':') {
                let password = &userinfo[colon_pos + 1..];
                // Non-empty password segment = credential present.
                if !password.is_empty() {
                    return true;
                }
            }
        }

        // Advance past this `://` to search for further occurrences.
        let advance = authority_start;
        if advance >= search.len() {
            break;
        }
        search = &search[advance..];
    }
    false
}

// ── Detector (4): keyword-context secret ─────────────────────────────────────

/// True iff `s` contains a keyword from [`KEYWORD_PREFIXES`] (case-insensitive)
/// immediately followed by `=` or `:` and at least one non-whitespace character.
///
/// Scans the lowercased form for each keyword, then validates the separator and
/// value in the original string to avoid allocating the whole lowercased copy
/// for each check.
fn has_keyword_context_secret(s: &str) -> bool {
    // Work on a lowercase copy once to avoid repeated allocations.
    let lower = s.to_ascii_lowercase();
    for kw in KEYWORD_PREFIXES {
        let mut pos = 0usize;
        while pos < lower.len() {
            let Some(kw_pos) = lower[pos..].find(kw) else {
                break;
            };
            let abs_kw_start = pos + kw_pos;
            let after_kw = abs_kw_start + kw.len();
            // Check that the character BEFORE the keyword (if any) is a word
            // boundary — we don't want `notapassword=x` to fire on `password`.
            // Underscore IS allowed as a separator (env-var convention:
            // `DB_PASSWORD`, `APP_SECRET`), so only block alphanumeric chars
            // immediately preceding the keyword.
            let before_ok = abs_kw_start == 0 || {
                let prev = lower.as_bytes()[abs_kw_start - 1];
                !prev.is_ascii_alphanumeric()
            };
            if before_ok {
                // Allow optional whitespace BEFORE the separator so
                // `password = x` and `token : x` fire, not just `password=x`.
                let bytes = lower.as_bytes();
                let mut sep_idx = after_kw;
                while bytes
                    .get(sep_idx)
                    .is_some_and(|b| *b == b' ' || *b == b'\t')
                {
                    sep_idx += 1;
                }
                // Now check the separator and value in the original string.
                if let Some(sep_byte) = bytes.get(sep_idx)
                    && (*sep_byte == b'=' || *sep_byte == b':')
                {
                    let value_start = sep_idx + 1;
                    // Skip optional leading whitespace after the separator too.
                    let value = s.get(value_start..).unwrap_or("").trim_start();
                    if !value.is_empty() {
                        return true;
                    }
                }
            }
            // Advance past this occurrence.
            let advance = abs_kw_start + kw.len();
            if advance <= pos {
                break; // safety: ensure progress
            }
            pos = advance;
        }
    }
    false
}

// ── Detector (5): high-entropy + bare-digest-shape token scan ─────────────────

/// True iff `s` contains a secret-shaped token run. A run fires when either:
///
/// - it clears the Shannon-entropy threshold (dense random keys), OR
/// - it is a **bare** 40/64-hex run of content-address SHAPE that is NOT in a
///   content-address context (WF-1) — a real HMAC / `SECRET_KEY` / hex API key
///   sits well below the entropy floor (~3.7 bits/char), so the entropy scan
///   alone would miss it. In free text such a run is secret-shaped and redacts;
///   the same run with a `cas:`/`sha256:`/`sha1:`/`<algo>:` prefix is exempt.
///
/// The token splitter treats `:` as a separator, so a `sha256:<hex>` ref splits
/// into `sha256` + `<hex>`. The prefix is therefore recovered from the byte
/// immediately preceding the hex run in the ORIGINAL string: a digest run whose
/// preceding context names a hash algorithm is exempt; a bare one is not.
fn high_entropy_token(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Skip separators.
        if !is_token_char(bytes[i] as char) {
            i += 1;
            continue;
        }
        // Collect one maximal token run [start, end).
        let start = i;
        while i < bytes.len() && is_token_char(bytes[i] as char) {
            i += 1;
        }
        let token = &s[start..i];
        if token.len() < ENTROPY_MIN_LEN {
            continue;
        }
        // `cas:`-prefixed single token (e.g. `cas:abc…`) survives.
        if is_content_address_ref(token) {
            continue;
        }
        // Bare 40/64-hex of digest shape: exempt ONLY when the byte sequence
        // immediately preceding the run is `<algo>:` (`sha256:`/`sha1:`/…) — the
        // `:` that the token scanner treats as a separator. A bare run in free
        // text is secret-shaped and redacts (WF-1).
        if is_bare_hex_digest_shape(token) {
            if preceding_context_is_digest_algo(bytes, start) {
                continue;
            }
            return true;
        }
        if shannon_entropy(token) >= ENTROPY_THRESHOLD {
            return true;
        }
    }
    false
}

/// True iff `token` is a bare 40- or 64-char run of hex digits (the sha-1 /
/// sha-256 shapes). No prefix logic here — purely shape.
fn is_bare_hex_digest_shape(token: &str) -> bool {
    matches!(token.len(), 40 | 64) && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// True iff the bytes immediately before `run_start` form `<algo>:` where
/// `<algo>` is a recognised content-address tag — i.e. the hex run is the
/// payload of a `sha256:`/`sha1:`/`cas:`/… ref. This recovers the prefix that
/// the `:`-splitting token scanner discarded.
fn preceding_context_is_digest_algo(bytes: &[u8], run_start: usize) -> bool {
    // The char right before the run must be `:`.
    if run_start == 0 || bytes[run_start - 1] != b':' {
        return false;
    }
    // Walk back over the algorithm name (alphanumerics + `-`).
    let colon = run_start - 1; // index of the ':'
    let mut algo_start = colon;
    while algo_start > 0 {
        let c = bytes[algo_start - 1];
        if c.is_ascii_alphanumeric() || c == b'-' {
            algo_start -= 1;
        } else {
            break;
        }
    }
    // `colon` indexes the ':'; the algo is bytes[algo_start..colon].
    if algo_start == colon {
        return false; // empty algo (`:<hex>`)
    }
    let algo = std::str::from_utf8(&bytes[algo_start..colon]).unwrap_or("");
    is_digest_algo(algo)
}

/// A character that can appear inside a base64/hex token run.
fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '-' || c == '_'
}

/// True for content-address refs that must survive verbatim — but ONLY in a
/// content-address CONTEXT (WF-1). A digest is a ref iff it carries an explicit
/// algorithm prefix: `cas:` (any payload) or `<algo>:<40|64-hex>` where `<algo>`
/// is a recognised content-address tag. A **bare** 40/64-hex run (no prefix) is
/// NOT treated as a ref here — in free text it could be an HMAC / SECRET_KEY /
/// hex API key of exactly that shape, so it falls through to the entropy scan
/// and redacts. Typed hash fields never reach [`apply`], so they are unaffected.
fn is_content_address_ref(token: &str) -> bool {
    if token.starts_with("cas:") {
        return true;
    }
    // Require an explicit `<algo>:` prefix and a hex payload of digest shape.
    // A bare hex run (no `:`) is deliberately NOT exempt.
    let Some((algo, hex)) = token.split_once(':') else {
        return false;
    };
    is_digest_algo(algo)
        && matches!(hex.len(), 40 | 64)
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Recognised content-address algorithm tags for the prefixed-digest exemption.
/// Case-insensitive (`sha256`/`SHA256` both count). A bare unknown prefix does
/// NOT exempt — the prefix must name a hash family.
fn is_digest_algo(algo: &str) -> bool {
    let a = algo.to_ascii_lowercase();
    matches!(
        a.as_str(),
        "sha1" | "sha256" | "sha-1" | "sha-256" | "sha512" | "sha-512" | "blake3" | "cas" | "oid"
    )
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
        // Real OpenAI key shape: sk- followed by long suffix (>20 chars)
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

    // ── (c) connection-string detector (new) ─────────────────────────────────

    #[test]
    fn conn_string_postgres_redacted() {
        // Classic postgres DSN with embedded password — password segment is
        // short (under 20 chars) and would miss the entropy floor entirely.
        assert_eq!(
            apply("postgres://user:hunter2@db.example.com/mydb"),
            REDACTED
        );
    }

    #[test]
    fn conn_string_mysql_redacted() {
        assert_eq!(
            apply("mysql://admin:S3cr3tP4ss@localhost:3306/app"),
            REDACTED
        );
    }

    #[test]
    fn conn_string_redis_redacted() {
        assert_eq!(apply("redis://:somepassword@redis.host:6379"), REDACTED);
    }

    #[test]
    fn conn_string_mongodb_redacted() {
        assert_eq!(
            apply("mongodb://dbuser:dbp4ssw0rd@mongo.host:27017/data"),
            REDACTED
        );
    }

    #[test]
    fn conn_string_amqp_redacted() {
        assert_eq!(
            apply("amqp://guest:guest_password@rabbitmq.host/vhost"),
            REDACTED
        );
    }

    #[test]
    fn conn_string_https_with_creds_redacted() {
        assert_eq!(
            apply("https://api_user:my_api_secret@api.example.com/endpoint"),
            REDACTED
        );
    }

    // Non-regression: URLs without a password survive.
    #[test]
    fn conn_string_no_password_survives() {
        // A URL with a user but no password should NOT be redacted by this
        // detector (no `:` before the `@`).
        let s = "https://username@github.com/org/repo.git";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn conn_string_no_userinfo_survives() {
        let s = "https://api.example.com/v1/endpoint";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn conn_string_empty_password_survives() {
        // `user:@host` — empty password segment is not a credential.
        let s = "redis://user:@redis.host:6379/0";
        // Note: this may still be redacted by keyword-context via "user:" —
        // we test the connection-string detector specifically via the helper.
        assert!(!has_connection_string_password(s));
    }

    // ── (d) keyword-context detector (new) ───────────────────────────────────

    #[test]
    fn keyword_password_eq_redacted() {
        // Deliberately low-entropy value — would escape entropy scan.
        assert_eq!(apply("password=hunter2"), REDACTED);
    }

    #[test]
    fn keyword_with_spaces_around_eq_redacted() {
        // WF-1: whitespace around the separator must still fire.
        assert_eq!(apply("password = hunter2"), REDACTED);
        assert_eq!(apply("token : some_value"), REDACTED);
        assert_eq!(apply("API_KEY\t=\tabc123"), REDACTED);
    }

    #[test]
    fn github_actions_token_ghs_redacted() {
        // WF-1: `ghs_` (GitHub Actions / server-to-server) now a known prefix.
        assert_eq!(
            apply("token=ghs_16C7e42F292c6912E7710c838347Ae178B4a"),
            REDACTED
        );
    }

    #[test]
    fn keyword_passwd_eq_redacted() {
        assert_eq!(apply("passwd=abc123"), REDACTED);
    }

    #[test]
    fn keyword_secret_eq_redacted() {
        assert_eq!(apply("secret=my_secret_value"), REDACTED);
    }

    #[test]
    fn keyword_token_colon_redacted() {
        assert_eq!(apply("token: some_value_here"), REDACTED);
    }

    #[test]
    fn keyword_api_key_eq_redacted() {
        assert_eq!(apply("api_key=1234567890abcdef"), REDACTED);
    }

    #[test]
    fn keyword_pwd_eq_redacted() {
        assert_eq!(apply("pwd=short"), REDACTED);
    }

    #[test]
    fn keyword_uppercase_redacted() {
        // Keywords are matched case-insensitively.
        assert_eq!(apply("PASSWORD=SomePa55word"), REDACTED);
    }

    #[test]
    fn keyword_embedded_in_line_redacted() {
        // Realistic env-file line.
        assert_eq!(apply("DB_PASSWORD=very_secret"), REDACTED);
    }

    // Non-regression: prose containing keyword words without assignment survives.
    #[test]
    fn keyword_prose_without_assignment_survives() {
        // "token" as a word in prose — no `=` or `:` separator immediately after.
        let s = "The token was invalidated by the server.";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn keyword_word_boundary_guard() {
        // `notapassword=x` must NOT fire because `password` is not at a word
        // boundary (the char before `password` is `a`, an alphanumeric).
        let s = "notapassword=x";
        assert_eq!(apply(s), s);
    }

    // ── (e) sk- tightening (fix + non-regression) ────────────────────────────

    #[test]
    fn sk_real_key_redacted() {
        // A genuine OpenAI-style key: sk- followed by ≥20 base64/hex chars.
        assert_eq!(
            apply("sk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"),
            REDACTED
        );
    }

    #[test]
    fn sk_short_identifier_survives() {
        // `sk-256` — only 3 chars after `sk-`, far below the 20-char gate.
        let s = "sk-256";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn sk_path_component_survives() {
        // A source-tree path that happens to contain `sk-` — must not fire.
        let s = "src/sk-learn/model.rs";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn sk_proj_short_hyphenated_key_redacts() {
        // WJ-INT (Round-6 residual): the modern `sk-proj-<id>` format leaked when
        // SHORT — the run after `sk-` (`proj-leaklens99999`, 18 chars) sat just
        // under the generic 20-char gate. The `proj-` marker now forces redaction.
        assert_eq!(apply("sk-proj-leaklens99999"), REDACTED);
        // Even embedded in a free-text field (e.g. a verdict lens name).
        assert_eq!(apply("lens sk-proj-leaklens99999 review"), REDACTED);
    }

    #[test]
    fn sk_proj_long_real_key_redacts() {
        // A genuine long project key: `sk-proj-` + ≥48 chars.
        assert_eq!(
            apply("sk-proj-aBcDeF0123456789ghIjKlMnOpQrStUvWxYz0123456789AbCd"),
            REDACTED
        );
    }

    #[test]
    fn sk_learn_word_survives_alongside_proj_rule() {
        // The `proj-` rule must NOT over-trigger on short non-key `sk-` words:
        // `sk-learn`'s suffix is `learn` (no `proj-` marker, below the gate).
        assert_eq!(apply("use sk-learn here"), "use sk-learn here");
    }

    #[test]
    fn sk_proj_bare_marker_without_id_survives() {
        // `sk-proj-` with NOTHING after the marker is not a key (empty id) and is
        // below the length gate — it survives (no over-trigger on the bare marker).
        let s = "sk-proj-";
        assert!(!has_sk_key(s));
        assert_eq!(apply(s), s);
    }

    #[test]
    fn sk_medium_identifier_survives() {
        // 19 repetitive chars after `sk-` — just below the `has_sk_key`
        // threshold AND below the entropy floor (low entropy, short run).
        // Uses a repeated-char suffix to ensure the entropy scan also passes.
        let s = "sk-aaaaaaaaaaaaaaaaaaa"; // 19 'a' chars after `sk-`
        // Verify `has_sk_key` does NOT fire (19 < 20).
        assert!(!has_sk_key(s));
        // Verify apply passes through (entropy on repeated chars is low).
        assert_eq!(apply(s), s);
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

    // ── WF-1: a PREFIXED digest survives; a BARE hex secret in free text now
    //          REDACTS (the exemption is content-address CONTEXT, not shape).

    #[test]
    fn prefixed_sha256_digest_survives() {
        // `sha256:<64-hex>` is a content-address ref — must NOT be redacted.
        let s = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn prefixed_sha1_digest_survives() {
        let s = "sha1:da39a3ee5e6b4b0d3255bfef95601890afd80709";
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
    fn bare_sha256_hex_secret_now_redacts() {
        // WF-1 (was `sha256_digest_survives`, which BLESSED the leak): a BARE
        // 64-hex run with no content-address prefix — could be an HMAC / Django
        // SECRET_KEY / hex API key. In free text it clears the entropy floor
        // and MUST redact. Err toward redaction.
        let s = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(apply(s), REDACTED);
    }

    #[test]
    fn bare_sha1_hex_secret_now_redacts() {
        // WF-1 (was `sha1_digest_survives`): a BARE 40-hex high-entropy run is
        // no longer blessed; in free text it redacts.
        let s = "3f786850e387550fdab836ed7e6dc881de23001b"; // sha1("a\n"), high entropy
        assert_eq!(apply(s), REDACTED);
    }

    #[test]
    fn charter_with_bare_hex_secret_redacts() {
        // WF-1 proof: a real secret of exactly content-address SHAPE planted in
        // a charter (free text) leaks no more. Both 40-hex and 64-hex variants.
        let charter_64 = "Deploy with SECRET_KEY 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";
        assert_eq!(apply(charter_64), REDACTED);
        let charter_40 = "rotate HMAC 3f786850e387550fdab836ed7e6dc881de23001b before merge";
        assert_eq!(apply(charter_40), REDACTED);
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
        // WF-1: only PREFIXED forms are content-address refs now. A BARE hex
        // digest is NOT (it falls to the digest-shape detector in free text).
        assert!(is_content_address_ref(
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        assert!(is_content_address_ref(
            "sha1:da39a3ee5e6b4b0d3255bfef95601890afd80709"
        ));
        assert!(is_content_address_ref("cas:anything-here"));
        // Bare hex digests are NO LONGER refs by themselves.
        assert!(!is_content_address_ref(
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        assert!(!is_content_address_ref(
            "da39a3ee5e6b4b0d3255bfef95601890afd80709"
        ));
        // Unknown prefix is not a content-address tag.
        assert!(!is_content_address_ref(
            "deadbeef:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        assert!(!is_content_address_ref("8Kp2mZ9qLx4vTn7wRj3sYb6cFd1gHe0"));
    }

    #[test]
    fn preceding_context_recovers_prefix() {
        // The `:`-splitting scanner relies on this to keep `sha256:<hex>` exempt.
        let s = "sha256:da39a3ee5e6b4b0d3255bfef95601890afd80709";
        // run starts at index 7 (after `sha256:`).
        assert!(preceding_context_is_digest_algo(s.as_bytes(), 7));
        let bare = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        assert!(!preceding_context_is_digest_algo(bare.as_bytes(), 0));
        let unknown = "xyz:da39a3ee5e6b4b0d3255bfef95601890afd80709";
        assert!(!preceding_context_is_digest_algo(unknown.as_bytes(), 4));
    }

    // ── has_sk_key helper sanity ─────────────────────────────────────────────

    #[test]
    fn has_sk_key_long_fires() {
        assert!(has_sk_key("sk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"));
    }

    #[test]
    fn has_sk_key_short_no_fire() {
        assert!(!has_sk_key("sk-256"));
        assert!(!has_sk_key("sk-learn"));
        assert!(!has_sk_key("sk-abc")); // only 3 chars
    }
}
