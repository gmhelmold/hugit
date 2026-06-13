//! O-1 (T-8) — PROPERTY tests for the redaction scrubber security spine.
//!
//! The review sweep (`docs/review/sweep-2026-06-12/tests.md`, T-8) found the
//! scrubber / `is_structural_secret` / `is_safe_identifier_shape` had ONLY
//! hand-crafted fixtures and ZERO property/fuzz coverage. This suite asserts the
//! redaction invariants hold over GENERATED inputs — random strings AND
//! structured credential / address shapes — so a regression that slips past the
//! fixtures (a UTF-8 boundary, a length edge, a false-positive on a benign id)
//! is caught by a generated counterexample, not by luck.
//!
//! The "scrub" under test is the engine's free-text view-boundary filter
//! `hugit_ledger::redact::apply` (single-sourced sentinel + the shared
//! `secret_shape` primitives) — the redaction door that lives in THIS crate. The
//! shape predicates `is_structural_secret` / `is_safe_identifier_shape` are
//! tested directly from `hugit_ledger::secret_shape`.
//!
//! Property failure => a real bug. Per the WP, we do NOT weaken a property to
//! make it pass; a counterexample is reported, not papered over.

use hugit_ledger::redact::{REDACTED, apply};
use hugit_ledger::secret_shape::{is_safe_identifier_shape, is_structural_secret};
use proptest::prelude::*;

// ── Generators ───────────────────────────────────────────────────────────────

/// Arbitrary noise around a planted credential — exercises the SUBSTRING
/// detectors (the credential need not be the whole string). Bounded to printable
/// + some control to also stress UTF-8 / whitespace boundaries.
fn noise() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[ -~\\t\\n]{0,40}").unwrap()
}

/// A token run of credential-body characters (base64/hex-ish), >= a length so the
/// generated credential clears the issuer-shape / entropy floors.
fn token_body(min: usize, max: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(
        prop::sample::select(
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"
                .chars()
                .collect::<Vec<_>>(),
        ),
        min..=max,
    )
    .prop_map(|v| v.into_iter().collect())
}

/// The known credential prefixes that are secret BY CONSTRUCTION. Each, when
/// followed by a real body, MUST be classified as a structural secret and MUST
/// be scrubbed. (Mirrors `secret_shape::KNOWN_PREFIXES` + the `sk-` / PEM /
/// conn-string / JWT classes named in the WP.)
fn credential_shape() -> impl Strategy<Value = String> {
    prop_oneof![
        // Prefixed issuer tokens: <prefix><>=20 body chars> (clears length gates).
        (
            prop::sample::select(vec![
                "ghp_",
                "gho_",
                "ghs_",
                "github_pat_",
                "AKIA",
                "xoxb-",
                "xoxp-",
                "xoxo-",
                "xoxa-",
                "xoxs-",
                "clp_",
                "Bearer ",
                "sk-",
            ]),
            token_body(24, 48),
        )
            .prop_map(|(p, body)| format!("{p}{body}")),
        // JWT: three base64url segments joined by '.', leading `eyJ` header.
        (token_body(8, 16), token_body(8, 16), token_body(8, 16))
            .prop_map(|(a, b, c)| format!("eyJ{a}.{b}.{c}")),
        // PEM private-key block.
        token_body(16, 40).prop_map(|b| format!("-----BEGIN RSA PRIVATE KEY-----\n{b}")),
        // Connection string with embedded password `://user:pass@host`.
        (token_body(3, 10), token_body(12, 30))
            .prop_map(|(u, p)| format!("postgres://{u}:{p}@db.example.com:5432/app")),
        // The planted SECRET: marker.
        token_body(4, 16).prop_map(|b| format!("SECRET:{b}")),
    ]
}

/// Address / identifier shapes that MUST survive (no over-scrub). Respects PS-14:
/// hex is exempt ONLY at the {40,64} digest lengths; we never generate a bare
/// non-{40,64} hex >=20 here and call it safe.
fn safe_address() -> impl Strategy<Value = String> {
    fn hex_of(n: usize) -> impl Strategy<Value = String> {
        proptest::collection::vec(
            prop::sample::select("0123456789abcdef".chars().collect::<Vec<_>>()),
            n..=n,
        )
        .prop_map(|v| v.into_iter().collect())
    }
    prop_oneof![
        // 40-hex (sha-1) and 64-hex (sha-256) digests.
        hex_of(40),
        hex_of(64),
        // cas:<digest> content-address ref.
        hex_of(64).prop_map(|h| format!("cas:{h}")),
        hex_of(40).prop_map(|h| format!("sha1:{h}")),
        // ULID — 26-char Crockford base32.
        proptest::collection::vec(
            prop::sample::select(
                "0123456789ABCDEFGHJKMNPQRSTVWXYZ"
                    .chars()
                    .collect::<Vec<_>>()
            ),
            26..=26,
        )
        .prop_map(|v| v.into_iter().collect()),
        // kebab / snake / dotted slug (bounded charset, low entropy).
        proptest::string::string_regex("[a-z][a-z0-9]{0,6}([-_./][a-z0-9]{1,6}){0,4}").unwrap(),
        // Small integer (a `--pr 7`, `--run-id 12345`).
        proptest::string::string_regex("[1-9][0-9]{0,7}").unwrap(),
    ]
}

// ── Properties ─────────────────────────────────────────────────────────────

proptest! {
    // Bounded, deterministic for CI; no on-disk regression-seed persistence (the
    // suite is hermetic — a counterexample surfaces in the failure message).
    #![proptest_config(ProptestConfig {
        cases: 512,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// A value built to contain a known credential shape is ALWAYS classified a
    /// structural secret AND the scrub CHANGES it — a credential never passes
    /// through verbatim. (Substring placement: noise on either side.)
    #[test]
    fn credential_is_always_structural_and_scrubbed(
        pre in noise(),
        cred in credential_shape(),
        post in noise(),
    ) {
        let s = format!("{pre}{cred}{post}");
        prop_assert!(
            is_structural_secret(&s),
            "credential not detected as structural secret: {s:?}"
        );
        let scrubbed = apply(&s);
        prop_assert_ne!(
            &scrubbed, &s,
            "scrub PASSED A CREDENTIAL THROUGH unchanged: {:?}", s
        );
        prop_assert_eq!(
            &scrubbed, REDACTED,
            "credential-bearing field not fully redacted: {:?} -> {:?}", s, scrubbed
        );
        // And the scrubbed output never itself looks like a credential.
        prop_assert!(!is_structural_secret(&scrubbed));
    }

    /// `is_structural_secret(x)` ⟹ `!is_safe_identifier_shape(x)` — a structural
    /// secret is NEVER a "safe" identifier. Generated over BOTH credential shapes
    /// and arbitrary strings.
    #[test]
    fn structural_secret_is_never_a_safe_identifier(
        s in prop_oneof![credential_shape(), any::<String>(), noise()],
    ) {
        if is_structural_secret(&s) {
            prop_assert!(
                !is_safe_identifier_shape(&s),
                "value is a structural secret YET passed the safe-identifier gate: {s:?}"
            );
        }
    }

    /// Idempotence: scrubbing twice equals scrubbing once, for ANY input
    /// (credential, address, or arbitrary UTF-8). Also proves `apply` never
    /// panics on arbitrary input.
    #[test]
    fn scrub_is_idempotent(
        s in prop_oneof![credential_shape(), safe_address(), any::<String>(), noise()],
    ) {
        let once = apply(&s);
        let twice = apply(&once);
        prop_assert_eq!(once, twice, "scrub is not idempotent for {:?}", s);
    }

    /// Address shapes that MUST survive are recognised as safe identifiers (no
    /// over-scrub). Encodes the PS-14 boundary by construction (only {40,64} hex
    /// is generated as hex; slugs/integers/ULIDs/cas-refs are the rest).
    #[test]
    fn safe_addresses_survive_the_identifier_gate(addr in safe_address()) {
        prop_assert!(
            is_safe_identifier_shape(&addr),
            "a legitimate address shape was NOT recognised as safe (over-scrub risk): {addr:?}"
        );
        // A safe address is, by the prior property's contrapositive, not a
        // structural secret.
        prop_assert!(
            !is_structural_secret(&addr),
            "a legitimate address shape was classified a structural secret: {addr:?}"
        );
    }

    /// PS-14 boundary, the NEGATIVE direction: a BARE hex run of length >= 20 that
    /// is NOT exactly 40 or 64 is NOT a safe identifier (it would otherwise leak —
    /// the entropy gate cannot catch hex). Encodes the boundary, does not
    /// contradict it.
    #[test]
    fn non_digest_length_bare_hex_is_not_safe(
        digits in proptest::collection::vec(
            prop::sample::select("0123456789abcdef".chars().collect::<Vec<_>>()),
            20..=80,
        ).prop_filter(
            "exclude the {40,64} digest lengths",
            |v| v.len() != 40 && v.len() != 64,
        ),
    ) {
        let hex: String = digits.into_iter().collect();
        prop_assert!(
            !is_safe_identifier_shape(&hex),
            "a non-{{40,64}} bare hex run of len {} was treated as safe (PS-14 violation)",
            hex.len()
        );
    }
}
