//! THE single source of truth for the structural-secret, content-address-shape,
//! and entropy primitives shared by the redaction layers (R9-3, Wave M / M-2).
//!
//! # Why this module exists
//!
//! Redaction lives in TWO policy layers with DIFFERENT top-level rules:
//!
//! - the **free-text engine** ([`crate::redact`]) — redacts on five detector
//!   classes, with a free-text entropy threshold (4.0) and a {40,64}-hex
//!   exemption; AND
//! - the **identifier door / boundary** (`hugit_cli::porcelain`) — deny-by-default
//!   (L-A): an identifier value survives verbatim ONLY if it proves a bounded
//!   safe-address shape, with a stricter identifier entropy threshold (4.5) and
//!   the PS-14 hybrid (only {40,64}-hex/numeric survive; odd-length hex and
//!   long-numeric redact).
//!
//! Historically BOTH layers hand-rolled the SAME low-level predicates (the
//! credential-prefix table, the `sk-` length gate, the connection-string scan,
//! the keyword-context scan, the content-address shape/`cas:` value-gate, the
//! Shannon-entropy function, the ULID shape). They were kept identical "in
//! lockstep" BY HAND — the comments literally said so — and the Round-8 / Round-9
//! audits (F4 / R9-3) flagged this hand-duplication as a standing DRIFT risk: a
//! fix to one copy that misses the other silently re-opens the class.
//!
//! This module hoists the SHARED PRIMITIVES into ONE canonical location in the
//! engine crate (`hugit-ledger`, which `hugit-cli` already depends on — no
//! crate cycle). Each layer KEEPS its own top-level policy (the thresholds and
//! the survive/redact decision differ on purpose) but BUILDS it on these shared
//! primitives, so a primitive can no longer drift between the two engines.
//!
//! Nothing here is weakened relative to the prior hand-mirrored copies — the
//! bodies are the exact union of the two previous (byte-identical) definitions.

// ── Shared constants ─────────────────────────────────────────────────────────

/// The planted-secret prefix that triggers redaction (the marker path).
pub const SECRET_MARKER: &str = "SECRET:";

/// Minimum length of a base64/hex token run before the entropy scan considers
/// it (short runs are noise — variable names, hex colours, etc.). Shared by the
/// free-text engine ([`crate::redact`]) and the identifier address gate.
pub const ENTROPY_MIN_LEN: usize = 20;

/// Minimum run of base64/hex chars that must follow `sk-` for the prefix to be
/// treated as a real API key rather than a short identifier or prose word.
pub const SK_MIN_SUFFIX_LEN: usize = 20;

/// Keyword prefixes (lowercase) that gate the keyword-context detector. Each
/// keyword is followed by `=`/`:` and a value — the value is a credential
/// regardless of entropy.
pub const KEYWORD_PREFIXES: &[&str] = &[
    "password",
    "passwd",
    "secret",
    "token",
    "api_key",
    "pwd",
    "auth_token",
    "access_token",
    "private_key",
    "credential",
    "passphrase",
];

/// Known credential prefixes that are secrets by construction (except `sk-`,
/// which is handled separately with a length gate). A field containing any of
/// these (case-sensitive, as the issuers mint them) is a structural secret.
pub const KNOWN_PREFIXES: &[&str] = &[
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
    "dop_v1_", // DigitalOcean personal access token
    "glpat-",  // GitLab personal access token
    // NOTE: "Bearer" is intentionally NOT here — Bearer matching requires a
    // trailing whitespace check (space OR tab) handled by `has_bearer_token`.
    "eyJ", // JWT header (base64 of `{"`)
];

// ── Structural-secret detector (the four entropy-independent classes) ────────

/// True iff `s` trips a STRUCTURAL secret detector — the credential-prefix / PEM /
/// connection-string / keyword-context classes that do NOT depend on entropy or
/// bare-hex shape.
///
/// This is the shared definition both layers use:
///
/// - the free-text engine consults it as the value-gate guard for the `cas:`
///   exemption (a content-address-charset payload that nonetheless carries a
///   credential prefix is NOT a content address), and
/// - the identifier door / boundary uses it as the first deny rule (a structural
///   credential shape is never an address).
///
/// The bare-hex/high-entropy scan (detector 5) is intentionally NOT consulted
/// here — that is each layer's own policy (the free-text engine adds it in
/// [`crate::redact`]; the identifier gate adds a stricter entropy/hex rule).
pub fn is_structural_secret(s: &str) -> bool {
    s.contains(SECRET_MARKER)
        || KNOWN_PREFIXES.iter().any(|p| s.contains(p))
        || has_bearer_token(s)
        || has_sk_key(s)
        || (s.contains("-----BEGIN") && s.contains("PRIVATE KEY"))
        || has_connection_string_password(s)
        || has_keyword_context_secret(s)
}

/// A character that can appear inside a base64/hex token run.
pub fn is_token_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '+' || c == '/' || c == '=' || c == '-' || c == '_'
}

/// True iff `s` contains an `sk-` API key. The suffix run after `sk-` is the
/// maximal run of [`is_token_char`] chars — which INCLUDES `-` and `_`, so the
/// modern hyphenated `sk-proj-<id>` format is one continuous run. The key fires
/// when EITHER:
///
/// - the suffix run is ≥ [`SK_MIN_SUFFIX_LEN`] token chars (a dense classic
///   `sk-<base64>` key), OR
/// - the suffix is the OpenAI **project-key** format `proj-<id>` with a
///   non-empty `<id>` — a strong structural marker, so a SHORT `sk-proj-…`
///   redacts even though its run sits just under the generic length gate.
///
/// Short non-key `sk-` words (`sk-256`, `sk-learn`, bare `sk-`) do NOT match.
pub fn has_sk_key(s: &str) -> bool {
    let needle = "sk-";
    let mut search = s;
    while let Some(pos) = search.find(needle) {
        let after = &search[pos + needle.len()..];
        // The suffix run includes `-`/`_` (is_token_char), so `proj-…` is one run.
        let run: String = after.chars().take_while(|&c| is_token_char(c)).collect();
        if run.chars().count() >= SK_MIN_SUFFIX_LEN {
            return true;
        }
        if let Some(id) = run.strip_prefix("proj-")
            && !id.is_empty()
        {
            return true;
        }
        let advance = pos + needle.len();
        if advance >= search.len() {
            break;
        }
        search = &search[advance..];
    }
    false
}

/// True iff `s` contains a `Bearer` token (case-insensitive) — the word
/// `bearer` (any case: `Bearer`, `BEARER`, `bearer`) immediately followed by
/// one or more ASCII whitespace characters (space OR tab). This covers both
/// `Bearer <token>` and `Bearer\t<token>` (and multiple spaces), while NOT
/// firing on bare prose use of the word "Bearer" with no whitespace after it.
///
/// The match is case-insensitive so `bearer <token>` / `BEARER <token>` do
/// not bypass the detector (they did before this fix).
///
/// The `Bearer` prefix is separated from `KNOWN_PREFIXES` precisely because
/// the whitespace-following check cannot be expressed as a plain `contains`.
pub fn has_bearer_token(s: &str) -> bool {
    // Case-fold the input once; all scanning runs on the lowercase copy.
    // The whitespace byte check is still correct because ASCII whitespace is
    // unchanged by lowercasing.
    let lower = s.to_ascii_lowercase();
    let needle = "bearer";
    let mut search = lower.as_str();
    while let Some(pos) = search.find(needle) {
        let after_bearer = pos + needle.len();
        if search
            .as_bytes()
            .get(after_bearer)
            .is_some_and(|b| *b == b' ' || *b == b'\t')
        {
            return true;
        }
        let advance = after_bearer;
        if advance >= search.len() {
            break;
        }
        search = &search[advance..];
    }
    false
}

/// True iff `s` contains a URL with an embedded password in the authority.
///
/// Hand-scan (no regex, std only): find `://`, then within the authority (up to
/// the next `/`, `?`, `#`, or end-of-string) look for `:` followed by `@` — the
/// segment between `:` and `@` is the password. An empty password segment
/// (`user:@host`) is not treated as a secret.
pub fn has_connection_string_password(s: &str) -> bool {
    let mut search = s;
    while let Some(scheme_end) = search.find("://") {
        let authority_start = scheme_end + 3;
        if authority_start >= search.len() {
            break;
        }
        let authority_str = &search[authority_start..];
        let authority_len = authority_str
            .find(['/', '?', '#'])
            .unwrap_or(authority_str.len());
        let authority = &authority_str[..authority_len];
        if let Some(at_pos) = authority.find('@') {
            let userinfo = &authority[..at_pos];
            if let Some(colon_pos) = userinfo.find(':') {
                let password = &userinfo[colon_pos + 1..];
                if !password.is_empty() {
                    return true;
                }
            }
        }
        let advance = authority_start;
        if advance >= search.len() {
            break;
        }
        search = &search[advance..];
    }
    false
}

/// True iff `s` contains a keyword from [`KEYWORD_PREFIXES`] (case-insensitive)
/// immediately followed by `=`/`:` and at least one non-whitespace character.
pub fn has_keyword_context_secret(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for kw in KEYWORD_PREFIXES {
        let mut pos = 0usize;
        while pos < lower.len() {
            let Some(kw_pos) = lower[pos..].find(kw) else {
                break;
            };
            let abs_kw_start = pos + kw_pos;
            let after_kw = abs_kw_start + kw.len();
            // The char BEFORE the keyword must be a word boundary (underscore is
            // an allowed separator — `DB_PASSWORD`/`APP_SECRET`); block only an
            // alphanumeric immediately preceding (`notapassword=x` must NOT fire).
            let before_ok = abs_kw_start == 0 || !bytes[abs_kw_start - 1].is_ascii_alphanumeric();
            if before_ok {
                let mut sep_idx = after_kw;
                while bytes
                    .get(sep_idx)
                    .is_some_and(|b| *b == b' ' || *b == b'\t')
                {
                    sep_idx += 1;
                }
                if let Some(sep_byte) = bytes.get(sep_idx)
                    && (*sep_byte == b'=' || *sep_byte == b':')
                {
                    // Validate the value in the ORIGINAL string (case preserved).
                    let value = s.get(sep_idx + 1..).unwrap_or("").trim_start();
                    if !value.is_empty() {
                        return true;
                    }
                }
            }
            let advance = abs_kw_start + kw.len();
            if advance <= pos {
                break; // safety: ensure progress
            }
            pos = advance;
        }
    }
    false
}

// ── Content-address SHAPE primitives (the digest / `cas:` exemption) ──────────

/// True iff `token` is a bare 40- or 64-char run of hex digits (the sha-1 /
/// sha-256 shapes). No prefix logic — purely shape.
pub fn is_bare_hex_digest_shape(token: &str) -> bool {
    matches!(token.len(), 40 | 64) && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Recognised content-address algorithm tags for the prefixed-digest exemption.
/// Case-insensitive. A bare unknown prefix does NOT exempt.
pub fn is_digest_algo(algo: &str) -> bool {
    matches!(
        algo.to_ascii_lowercase().as_str(),
        "sha1" | "sha256" | "sha-1" | "sha-256" | "sha512" | "sha-512" | "blake3" | "cas" | "oid"
    )
}

/// True iff `payload` (the part after a `cas:` prefix) is a genuine
/// content-address SHAPE: a bare 40/64-char hex run (sha-1 / sha-256) or a
/// base32 content-id (lowercase RFC-4648 `[a-z2-7]`, of a content-address-
/// plausible length). A credential smuggled behind `cas:` does NOT match this
/// charset/length.
pub fn is_cas_payload_shaped(payload: &str) -> bool {
    if matches!(payload.len(), 40 | 64) && payload.bytes().all(|b| b.is_ascii_hexdigit()) {
        return true;
    }
    matches!(payload.len(), 32..=64)
        && payload
            .bytes()
            .all(|b| b.is_ascii_lowercase() || (b'2'..=b'7').contains(&b))
}

/// True for content-address refs that carry an explicit algorithm prefix:
/// `cas:<payload>` or `<algo>:<40|64-hex>` where `<algo>` names a recognised
/// hash family. A **bare** hex run (no `:`) is deliberately NOT a ref here.
///
/// VALUE-GATED (K-SCRUB, the Round-7 hole): the `cas:` prefix does NOT
/// blanket-exempt ANY payload. The payload must be a genuine content-address
/// SHAPE ([`is_cas_payload_shaped`]) AND must not trip a structural-secret
/// detector ([`is_structural_secret`]). A `cas:ghp_…` / `cas:<JWT>` is NOT a
/// content address → not a ref. A real `cas:<64-hex>` (or base32 CID) survives.
pub fn is_content_address_ref(token: &str) -> bool {
    if let Some(payload) = token.strip_prefix("cas:") {
        return is_cas_payload_shaped(payload) && !is_structural_secret(payload);
    }
    let Some((algo, hex)) = token.split_once(':') else {
        return false;
    };
    is_digest_algo(algo)
        && matches!(hex.len(), 40 | 64)
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
}

/// True iff `value` is digest-SHAPED — a bare 40/64-hex run OR a prefixed
/// content-address ref ([`is_content_address_ref`]). The value gate for the
/// digest-field exemption AND the canonical high-entropy address shape the
/// identifier gate must let survive.
pub fn is_digest_shaped(value: &str) -> bool {
    is_bare_hex_digest_shape(value) || is_content_address_ref(value)
}

// ── ULID shape + entropy ─────────────────────────────────────────────────────

/// True iff `s` is a ULID — exactly 26 chars of Crockford base32
/// (`0-9A-HJKMNP-TV-Z`, i.e. no `I`/`L`/`O`/`U`). The canonical hugit intent-id
/// shape; high-entropy by construction so it needs an EXPLICIT survival path in
/// the identifier gate (the generic entropy gate would otherwise redact it).
pub fn is_ulid_shaped(s: &str) -> bool {
    s.len() == 26
        && s.bytes().all(|b| {
            b.is_ascii_digit()
                || matches!(b,
                    b'A'..=b'H' | b'J' | b'K' | b'M' | b'N' | b'P'..=b'T' | b'V'..=b'Z')
        })
}

/// Shannon entropy of a token in bits per character. Shared by the free-text
/// entropy scan and the identifier address gate.
pub fn shannon_entropy(token: &str) -> f64 {
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

// ── The identifier-door safe-shape policy (deny-by-default, PS-14 hybrid) ─────

/// Shannon-entropy threshold (bits/char) above which a long identifier run is
/// treated as a credential blob rather than an address. A ULID (Crockford
/// base32) and human slugs sit well under; a dense random base64 AWS/SendGrid/
/// Stripe key clears it. Set slightly above the engine's free-text 4.0 so a
/// ULID's structured base32 still SURVIVES (it is a real address) while a
/// 40-char dense mixed-case base64 key redacts.
pub const IDENT_ENTROPY_THRESHOLD: f64 = 4.5;

/// The CLOSED set of address shapes an identifier may legitimately take and
/// still survive unredacted (L-A deny-by-default; PS-14 hybrid tuning). NOT on
/// this list ⇒ redact.
///
/// This is the IDENTIFIER-door policy — distinct from the free-text engine,
/// which has its own threshold (4.0) and bare-hex exemption. It is hosted here
/// (next to the shared primitives it is built on) so the door
/// (`hugit_cli::porcelain::structural_secret_scrub` / `ident.rs`) and the
/// payload boundary call ONE implementation; they cannot drift.
///
/// Accepts (→ survive): a 40/64-hex digest / `cas:`/`<algo>:` content-address
/// ref ([`is_digest_shaped`]); a ULID ([`is_ulid_shaped`]); the bounded
/// identifier charset (`[A-Za-z0-9._@:/+-]`) — git short-hashes, integers,
/// kebab/snake/dotted slugs, emails, branch refs — gated so it does NOT admit a
/// high-entropy credential blob.
///
/// Rejects (→ redact): a STRUCTURAL secret ([`is_structural_secret`]); a long
/// BARE all-hex/all-numeric run that is NOT a {40,64}-hex digest (PS-14 hybrid:
/// odd-length hex / long numeric secrets the entropy gate cannot catch); and any
/// value that is BOTH ≥ the credential length floor AND high Shannon entropy AND
/// not digest-shaped (a 40-char dense base64 AWS/SendGrid/Stripe key, a 32-char
/// dense base64 blob). When in doubt we prefer redaction (security).
pub fn is_safe_identifier_shape(s: &str) -> bool {
    let t = s.trim();
    if t.is_empty() {
        // An empty/whitespace value carries no secret and MUST pass through
        // UNCHANGED (return safe → the scrub is a no-op). Turning `""` into the
        // sentinel would mask emptiness from downstream checks (e.g. `pr open`'s
        // empty-`--run-id` binding). Emptiness is the door's separate
        // `invalid_argument` rule, not the secret scrub's job.
        return true;
    }
    // A structural credential shape is never an address — redact.
    if is_structural_secret(t) {
        return false;
    }
    // A 40/64-hex digest or a `cas:`/`<algo>:` content-address ref is the
    // canonical high-entropy address that MUST survive (exit early before the
    // entropy gate below would reject it).
    if is_digest_shaped(t) {
        return true;
    }
    // A ULID is the canonical intent-id shape — 26-char Crockford base32 — and is
    // high-entropy by construction, so recognise it EXPLICITLY as a structured
    // address before the generic entropy gate.
    if is_ulid_shaped(t) {
        return true;
    }
    // Bounded identifier charset only — a value outside it is not an address the
    // flow uses. `@`, `:`, `/`, `.` are the address punctuation (emails, branch
    // refs, scoped ids); `_`/`-` are slug separators.
    if !t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '@' | ':' | '/' | '+' | '-'))
    {
        return false;
    }
    // PS-14 (Round-9 C1 tuning — owner-chosen HYBRID: hex exemption pinned to the
    // known digest lengths {40,64}, integers and slugs left generous). The 40/64-hex
    // digests and `cas:` refs already survived above. A long BARE hex run of ANY
    // OTHER length — equivalently any long all-`[0-9a-fA-F]` run, which subsumes a
    // long all-NUMERIC run — is not a known address shape, and the generic entropy
    // gate below CANNOT catch it (hex tops out at 4.0 bits/char, decimal at ~3.32,
    // both under the threshold). So a 32/50-hex API key or a 24-digit numeric secret
    // would otherwise leak verbatim. Treat such a value as a credential → redact.
    // SHORT numerics (`--pr 7`, a CI `--run-id 12345`) stay safe (len < the floor);
    // prefixed ids (`intent-<16hex>`), UUIDs (hyphens → not all-hex), and slugs are
    // unaffected. (A LOW-entropy base32 value remains the accepted physics residual.)
    if t.len() >= ENTROPY_MIN_LEN && t.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    // The one place the entropy signal belongs for identifiers: an identifier is
    // not allowed to BE a long, dense, high-entropy non-hex blob (an AWS /
    // SendGrid / Stripe key). A long LOW-entropy slug survives; a long dense
    // random run redacts.
    if t.len() >= ENTROPY_MIN_LEN && shannon_entropy(t) >= IDENT_ENTROPY_THRESHOLD {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    // GHP and friends — recognisable specimens.
    const GHP: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
    const DIGEST_64: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    const DIGEST_40: &str = "da39a3ee5e6b4b0d3255bfef95601890afd80709";

    #[test]
    fn structural_secret_classes() {
        assert!(is_structural_secret(GHP));
        assert!(is_structural_secret("xo\x78b-2222-3333-abcdefghij"));
        assert!(is_structural_secret(
            "clp_live_9f8e7d6c5b4a3210fedcba9876543210"
        ));
        assert!(is_structural_secret("Bearer abc123def456ghi789jkl"));
        assert!(is_structural_secret(
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.abc"
        ));
        assert!(is_structural_secret(
            "postgres://u:S3cr3tP4ssw0rdVeryLongRandomToken9999@h:5432/d"
        ));
        assert!(is_structural_secret("token=hunter2"));
        assert!(is_structural_secret(
            "sk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"
        ));
        assert!(is_structural_secret(
            "-----BEGIN RSA PRIVATE KEY-----\nMIIE"
        ));
        // Addresses + benign ids are NOT structural secrets (entropy/hex skipped).
        assert!(!is_structural_secret(DIGEST_64));
        assert!(!is_structural_secret(DIGEST_40));
        assert!(!is_structural_secret("8Kp2mZ9qLx4vTn7wRj3sYb6cFd1gHe0"));
        assert!(!is_structural_secret("run-0011-2233"));
        assert!(!is_structural_secret("sk-256"));
    }

    #[test]
    fn sk_key_gate() {
        assert!(has_sk_key("sk-abcdefghijklmnopqrstuvwxyz0123456789ABCDEF"));
        assert!(has_sk_key("sk-proj-leaklens99999"));
        assert!(!has_sk_key("sk-256"));
        assert!(!has_sk_key("sk-learn"));
        assert!(!has_sk_key("sk-proj-"));
    }

    #[test]
    fn content_address_shapes() {
        assert!(is_digest_shaped(DIGEST_64));
        assert!(is_digest_shaped(DIGEST_40));
        assert!(is_digest_shaped(&format!("sha256:{DIGEST_64}")));
        assert!(is_digest_shaped(&format!("cas:{DIGEST_64}")));
        assert!(is_digest_shaped(
            "cas:bafybeigdyrzt5sfp7udm7hu76uh7y26nf3efuylqabf3oclgtqy55fbzdi"
        ));
        assert!(!is_digest_shaped(&format!("cas:{GHP}")));
        assert!(!is_digest_shaped("cas:anything-here"));
        assert!(!is_digest_shaped(GHP));
        assert!(!is_digest_shaped("deadbeef"));
        assert!(!is_digest_shaped(&format!("token:{DIGEST_64}")));
    }

    #[test]
    fn ulid_and_entropy() {
        assert!(is_ulid_shaped("01HQXW8ZK4M9P2N7R3T5V6Y8BC"));
        assert!(!is_ulid_shaped("01HQXW8ZK4M9P2N7R3T5V6Y8B")); // 25 chars
        let random = shannon_entropy("8Kp2mZ9qLx4vTn7wRj3sYb6cFd1gHe0");
        let repeated = shannon_entropy("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert!(random > repeated);
    }

    #[test]
    fn safe_identifier_shape_splits_addresses_from_blobs() {
        // Addresses survive.
        assert!(is_safe_identifier_shape(DIGEST_40));
        assert!(is_safe_identifier_shape(DIGEST_64));
        assert!(is_safe_identifier_shape("01HQXW8ZK4M9P2N7R3T5V6Y8BC"));
        assert!(is_safe_identifier_shape("auth-hardening"));
        assert!(is_safe_identifier_shape("feature/login"));
        assert!(is_safe_identifier_shape("42"));
        assert!(is_safe_identifier_shape("owner@example.com"));
        assert!(is_safe_identifier_shape("sk-256"));
        assert!(is_safe_identifier_shape(
            "550e8400-e29b-41d4-a716-446655440000"
        ));
        assert!(is_safe_identifier_shape("intent-a1b2c3d4e5f6a7b8"));
        assert!(is_safe_identifier_shape("a1b2c3d4e5f6a7b8"));
        assert!(is_safe_identifier_shape("123456789"));
        // Blobs / credentials redact.
        assert!(!is_safe_identifier_shape(GHP));
        assert!(!is_safe_identifier_shape(
            "wJalrXUtnFEMIK7MDENGbPxRfiCYEXAMPLEKEY123"
        ));
        assert!(!is_safe_identifier_shape(
            "aB3xZ9qL2mK7pR4tY8wN6vC1dF5gH0jS"
        ));
        // PS-14 hybrid: odd-length hex / long numeric redact.
        assert!(!is_safe_identifier_shape(
            "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6"
        )); // 32-hex
        assert!(!is_safe_identifier_shape(
            "a1b2c3d4e5f6a7b8c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5"
        )); // 50-hex
        assert!(!is_safe_identifier_shape("123456789012345678901234")); // 24-digit
    }

    // ── Fix 1: DigitalOcean token prefix ────────────────────────────────────

    #[test]
    fn dop_v1_token_is_structural_secret() {
        // A realistic DigitalOcean personal access token (dop_v1_ + 64 hex chars).
        let token = "dop_v1_0123456789abcdef0123456789abcdef0123456789abcdef0123456789ab";
        assert!(
            is_structural_secret(token),
            "dop_v1_ prefix must be a structural secret"
        );
    }

    // ── Fix 3: Extended keyword prefixes ────────────────────────────────────

    #[test]
    fn access_token_keyword_is_caught() {
        assert!(
            has_keyword_context_secret("access_token=supersecretvalue"),
            "access_token= must trigger the keyword-context detector"
        );
        assert!(is_structural_secret("access_token=supersecretvalue"));
    }

    #[test]
    fn auth_token_keyword_is_caught() {
        assert!(has_keyword_context_secret("auth_token=my_secret_here"));
        assert!(is_structural_secret("auth_token=my_secret_here"));
    }

    #[test]
    fn private_key_keyword_is_caught() {
        assert!(has_keyword_context_secret("private_key=abc123secret"));
        assert!(is_structural_secret("private_key=abc123secret"));
    }

    #[test]
    fn credential_keyword_is_caught() {
        assert!(has_keyword_context_secret("credential=mysecretcred"));
        assert!(is_structural_secret("credential=mysecretcred"));
    }

    #[test]
    fn passphrase_keyword_is_caught() {
        assert!(has_keyword_context_secret(
            "passphrase=correct horse battery"
        ));
        assert!(is_structural_secret("passphrase=correct horse battery"));
    }

    // ── Fix 4: Bearer tab bypass ─────────────────────────────────────────────

    #[test]
    fn bearer_space_is_structural_secret() {
        assert!(has_bearer_token(
            "Authorization: Bearer abc123def456ghi789jkl"
        ));
        assert!(is_structural_secret(
            "Authorization: Bearer abc123def456ghi789jkl"
        ));
    }

    #[test]
    fn bearer_tab_is_structural_secret() {
        assert!(
            has_bearer_token("Authorization: Bearer\tabc123def456ghi789jkl"),
            "Bearer followed by tab must be detected"
        );
        assert!(is_structural_secret(
            "Authorization: Bearer\tabc123def456ghi789jkl"
        ));
    }

    #[test]
    fn bearer_double_space_is_structural_secret() {
        // "Bearer" followed by multiple spaces also fires (the first space triggers it).
        assert!(has_bearer_token(
            "Authorization: Bearer  abc123def456ghi789jkl"
        ));
        assert!(is_structural_secret(
            "Authorization: Bearer  abc123def456ghi789jkl"
        ));
    }

    #[test]
    fn bearer_without_whitespace_does_not_fire() {
        // "Bearer" immediately followed by a non-whitespace char does NOT fire.
        assert!(!has_bearer_token("BearerScheme"));
        assert!(!has_bearer_token("Bearer:nospace"));
    }

    // ── Fix: Bearer case-sensitivity ────────────────────────────────────────

    #[test]
    fn bearer_lowercase_is_caught() {
        // `bearer <token>` (all-lowercase) must be detected — was a bypass before
        // the case-fold fix.
        assert!(
            has_bearer_token("authorization: bearer abc123def456ghi789jkl"),
            "lowercase 'bearer' followed by space must be detected"
        );
        assert!(is_structural_secret(
            "authorization: bearer abc123def456ghi789jkl"
        ));
    }

    #[test]
    fn bearer_uppercase_is_caught() {
        // `BEARER <token>` (all-uppercase) must be detected — was a bypass before.
        assert!(
            has_bearer_token("AUTHORIZATION: BEARER abc123def456ghi789jkl"),
            "uppercase 'BEARER' followed by space must be detected"
        );
        assert!(is_structural_secret(
            "AUTHORIZATION: BEARER abc123def456ghi789jkl"
        ));
    }

    // ── Fix: glpat- GitLab PAT prefix ────────────────────────────────────────

    #[test]
    fn glpat_token_is_structural_secret() {
        // A realistic GitLab personal access token (glpat- + alphanumeric suffix).
        let token = "glpat-abcdefghijklmnopqrst";
        assert!(
            is_structural_secret(token),
            "glpat- prefix must be a structural secret"
        );
        assert!(
            KNOWN_PREFIXES.contains(&"glpat-"),
            "glpat- must be in KNOWN_PREFIXES"
        );
    }
}
