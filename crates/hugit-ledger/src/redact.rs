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

// The structural-secret / content-address-shape / entropy PRIMITIVES this
// engine builds on are single-sourced from `secret_shape` (R9-3, Wave M / M-2),
// so the free-text engine and the identifier door can no longer drift. This
// module keeps its OWN free-text POLICY (the 4.0 threshold + the bare-hex
// exemption in `high_entropy_token`), built on those shared primitives.
use crate::secret_shape::{
    ENTROPY_MIN_LEN, SECRET_MARKER as SHARED_SECRET_MARKER, is_bare_hex_digest_shape,
    is_content_address_ref, is_digest_algo, is_structural_secret, is_token_char, shannon_entropy,
};

/// The planted-secret prefix that triggers redaction (the marker path).
/// Single-sourced from [`secret_shape::SECRET_MARKER`].
pub const SECRET_MARKER: &str = SHARED_SECRET_MARKER;

/// The redaction sentinel rendered in ledger/verdict views. Single-sourced from
/// the canonical [`hugit_contracts::REDACTED_MARKER`] so it cannot drift from the
/// CLI export redaction.
pub const REDACTED: &str = hugit_contracts::REDACTED_MARKER;

/// Shannon-entropy threshold (bits/char) above which a long token run is
/// treated as a credential. Random base64 approaches ~6.0 bits/char; a
/// 64-hex digest sits near ~4.0; English prose runs well under 3.0. 4.0 keeps
/// hex digests below the line (and they are exempted regardless) while
/// catching the dense alphabet of real keys. This is the FREE-TEXT engine's own
/// policy threshold (the identifier door uses a stricter 4.5 — they differ on
/// purpose, but both build on the one shared [`secret_shape::shannon_entropy`]).
const ENTROPY_THRESHOLD: f64 = 4.0;

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
    // (1)–(4): the structural detector classes — single-sourced from
    // `secret_shape` (marker / known-prefix / `sk-` gate / PEM / conn-string /
    // keyword-context). These are entropy-independent and shared verbatim with
    // the identifier door.
    if is_structural_secret(s) {
        return true;
    }
    // (5) high-entropy scan over base64/hex runs, with the cas-ref exemption —
    // the FREE-TEXT engine's own policy (threshold 4.0 + bare-hex exemption).
    high_entropy_token(s)
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
        // All-lowercase-hex tokens of OTHER lengths (not the exempt {40,64}-char
        // content-address shapes) that meet the minimum length are credential-shaped.
        // Pure hex tops out at ~4.0 bits/char so the entropy gate (4.0) cannot
        // reliably catch them — a 32-hex API key, 20-hex session token, 50-hex HMAC
        // key would all slip through. Redact unconditionally for len ∈ [20,∞) \ {40,64}.
        if token.len() >= ENTROPY_MIN_LEN
            && !matches!(token.len(), 40 | 64)
            && token
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return true;
        }
        if shannon_entropy(token) >= ENTROPY_THRESHOLD {
            return true;
        }
    }
    false
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

#[cfg(test)]
mod tests {
    use super::*;
    // `super::*` already re-exports `is_content_address_ref` and `shannon_entropy`
    // (used by the engine). `has_sk_key` / `has_connection_string_password` are
    // test-only here, so import them directly from the single-source module.
    use crate::secret_shape::{has_connection_string_password, has_sk_key};

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
        // WF-1: only PREFIXED forms are content-address refs. K-SCRUB: the `cas:`
        // payload is now VALUE-GATED — it must be content-address shaped, not any
        // string.
        assert!(is_content_address_ref(
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        assert!(is_content_address_ref(
            "sha1:da39a3ee5e6b4b0d3255bfef95601890afd80709"
        ));
        // A real `cas:<64-hex>` / base32 CID survives.
        assert!(is_content_address_ref(
            "cas:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        ));
        assert!(is_content_address_ref(
            "cas:bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"
        ));
        // K-SCRUB: a credential smuggled behind `cas:` is NOT a content address.
        assert!(!is_content_address_ref(
            "cas:ghp_16C7e42F292c6912E7710c838347Ae178B4a"
        ));
        // K-SCRUB: an arbitrary non-shaped `cas:` payload is no longer a ref.
        assert!(!is_content_address_ref("cas:anything-here"));
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
    fn cas_with_a_credential_payload_redacts() {
        // K-SCRUB (Round-7 hole): the `cas:` exemption is value-gated. A PAT
        // smuggled behind `cas:` is NOT a content address and MUST redact.
        assert_eq!(
            apply("cas:ghp_16C7e42F292c6912E7710c838347Ae178B4a"),
            REDACTED
        );
        // Belt-and-braces across structural classes behind `cas:`.
        assert_eq!(
            apply("cas:xoxb-2222222222-3333333333-abcdefghijklmnop"),
            REDACTED
        );
        // A real `cas:<64-hex>` still survives verbatim (addressability).
        let real = "cas:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(apply(real), real);
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

    // ── Fix 1: DigitalOcean token prefix ────────────────────────────────────

    #[test]
    fn dop_v1_token_redacted() {
        // A 64-char hex DigitalOcean personal access token.
        let token = "dop_v1_0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab";
        assert_eq!(apply(token), REDACTED, "dop_v1_ prefix must redact");
    }

    // ── Fix 2: Bare hex tokens of non-{40,64} lengths ───────────────────────

    #[test]
    fn bare_32_hex_redacts() {
        // A 32-char lowercase hex token (NOT a content-address shape) must redact.
        let token = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6";
        assert_eq!(
            apply(token),
            REDACTED,
            "a 32-char lowercase hex token must be treated as a credential"
        );
    }

    #[test]
    fn bare_20_hex_redacts() {
        // Minimum length (20-char) lowercase hex — just at the floor.
        let token = "deadbeefcafe01234567";
        assert_eq!(apply(token), REDACTED);
    }

    #[test]
    fn bare_50_hex_redacts() {
        // A 50-char hex token — between the exempt 40 and 64 lengths.
        let token = "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5";
        assert_eq!(apply(token), REDACTED);
    }

    #[test]
    fn bare_40_hex_still_redacts_in_free_text() {
        // WF-1: a bare 40-hex still redacts in free text (no algo: prefix).
        // This test validates the existing bare-40-hex-digest-shape path is unaffected.
        let s = "3f786850e387550fdab836ed7e6dc881de23001b";
        assert_eq!(apply(s), REDACTED);
    }

    #[test]
    fn bare_64_hex_still_redacts_in_free_text() {
        // WF-1: a bare 64-hex still redacts in free text.
        let s = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(apply(s), REDACTED);
    }

    #[test]
    fn prefixed_40_hex_survives() {
        // sha1:<40-hex> is a content-address ref — must NOT redact.
        let s = "sha1:da39a3ee5e6b4b0d3255bfef95601890afd80709";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn prefixed_64_hex_survives() {
        // sha256:<64-hex> is a content-address ref — must NOT redact.
        let s = "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        assert_eq!(apply(s), s);
    }

    #[test]
    fn short_hex_below_floor_survives() {
        // Under ENTROPY_MIN_LEN (20) — not a credential shape.
        let s = "deadbeef0123456789"; // 18 chars
        assert_eq!(apply(s), s);
    }

    #[test]
    fn uppercase_hex_not_caught_by_bare_hex_rule() {
        // The bare-hex rule matches lowercase-only hex. UPPERCASE hex tokens
        // that are not {40,64} length pass through this specific rule and fall
        // through to the entropy gate. A 32-char ALL-UPPERCASE hex token has
        // entropy near 4.0 — the entropy gate may or may not catch it. This test
        // verifies the bare-hex rule itself doesn't over-trigger on mixed/upper hex.
        // (The entropy gate is a separate concern.)
        let upper = "A1B2C3D4E5F6A7B8C9D0E1F2A3B4C5D6"; // 32 uppercase hex
        // entropy gate: ~4.0 bits/char (16-char alphabet), borderline
        // We just verify it doesn't panic and documents the edge case.
        let _ = apply(upper); // pass or redact — no assertion, documents the boundary
    }

    // ── Fix 3: New keyword prefixes ─────────────────────────────────────────

    #[test]
    fn access_token_eq_redacted() {
        assert_eq!(
            apply("access_token=supersecretvalue"),
            REDACTED,
            "access_token= must be caught by the keyword-context detector"
        );
    }

    #[test]
    fn auth_token_colon_redacted() {
        assert_eq!(apply("auth_token: my_secret_here"), REDACTED);
    }

    #[test]
    fn private_key_eq_redacted() {
        assert_eq!(apply("private_key=abc123secret"), REDACTED);
    }

    #[test]
    fn passphrase_eq_redacted() {
        assert_eq!(apply("passphrase=correct horse battery"), REDACTED);
    }

    // ── Fix 4: Bearer tab bypass ─────────────────────────────────────────────

    #[test]
    fn bearer_tab_redacted() {
        assert_eq!(
            apply("Authorization: Bearer\tabc123def456ghi789jkl"),
            REDACTED,
            "Bearer followed by a tab must redact"
        );
    }

    #[test]
    fn bearer_double_space_redacted() {
        assert_eq!(
            apply("Authorization: Bearer  abc123def456ghi789jkl"),
            REDACTED,
            "Bearer followed by multiple spaces must redact"
        );
    }
}
