//! Redaction-at-export + the redaction manifest (E5④/⑦).
//!
//! The context/journal redaction policy is applied **at export time**: seeded
//! secrets appear **nowhere** in the git artifact or the JSON envelope, and
//! every removal is **manifested** (recorded, never silent). The redacted
//! artifact still passes the exit proof.
//!
//! ## Single source of truth (WF-4)
//!
//! Detection is delegated to the ONE redaction law,
//! [`hugit_ledger::redact::apply`] — the SAME engine the ledger/envelope write
//! path uses. Export can therefore never be *weaker* than the write path: a
//! class the ledger engine catches (JWT, `gho_`/`ghs_`, `clp_`, PEM blocks,
//! connection strings, keyword-context, high-entropy / bare-digest-shape runs)
//! is caught here too. The old static 16-pattern list — which missed 9 of those
//! classes — is gone.
//!
//! ## Field-wholesale redaction + the manifest
//!
//! The ledger engine is *wholesale*: a field that trips any detector is replaced
//! in full with the [`REDACTED_TOKEN`] sentinel, so no fragment of a secret
//! survives. Export records each such wholesale removal in the manifest with the
//! SHA-256 digest of the ORIGINAL field text (audit without re-exposure) — the
//! removal is recorded, never silent.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The fixed replacement token a redacted span is rewritten to. It is itself
/// secret-free and stable, so a redacted artifact is deterministic. Single-sourced
/// from the canonical [`hugit_contracts::REDACTED_MARKER`] so it cannot drift from
/// the ledger/verdict redaction sentinel — and identical to the ledger engine's
/// [`hugit_ledger::redact::REDACTED`], which is the same constant.
pub const REDACTED_TOKEN: &str = hugit_contracts::REDACTED_MARKER;

/// The policy label recorded for every wholesale removal. Because detection is
/// now the unified ledger engine (not a per-pattern table), there is one honest
/// label: the engine fired on the field.
const LEDGER_POLICY_LABEL: &str = "ledger-engine";

/// One recorded redaction: the policy label that fired and the SHA-256 digest of
/// the removed material (so the removal is auditable WITHOUT re-exposing the
/// secret — the manifest never carries plaintext).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Removal {
    /// Where the secret was found (logical location, e.g. `journals/<id>.body`).
    pub location: String,
    /// The policy label of the pattern that matched.
    pub policy: String,
    /// SHA-256 hex digest of the removed token (audit without re-exposure).
    pub removed_digest: String,
}

/// The redaction manifest: every removal, plus a content hash of the whole
/// manifest used as the [`ExportSchema::redaction_manifest`] ref. An empty
/// removal set is still a positive, hashed manifest (a no-op redaction is
/// asserted, never assumed).
///
/// [`ExportSchema::redaction_manifest`]: hugit_contracts::ExportSchema::redaction_manifest
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionManifest {
    /// Every removal performed during the export, in encounter order.
    pub removals: Vec<Removal>,
}

impl RedactionManifest {
    /// A fresh, empty manifest.
    pub fn new() -> Self {
        Self::default()
    }

    /// The content-addressed ref for this manifest: `redaction-<sha256>` over
    /// the canonical JSON of the manifest. Stable and secret-free.
    ///
    /// Returns the serialization error rather than swallowing it: a fixed hash
    /// on serialize-failure (the old `unwrap_or_default()`) would make a
    /// corrupt/empty manifest indistinguishable from a real one and pass schema
    /// validation with a meaningless ref.
    pub fn content_ref(&self) -> Result<String, serde_json::Error> {
        let bytes = serde_json::to_vec(self)?;
        let digest = Sha256::digest(&bytes);
        Ok(format!("redaction-{}", hex::encode(digest)))
    }

    /// Whether anything was redacted.
    pub fn is_empty(&self) -> bool {
        self.removals.is_empty()
    }
}

/// Redact a single field's text through the UNIFIED ledger engine (WF-4),
/// recording every wholesale removal into `manifest`. Returns the redacted text.
///
/// Detection is [`hugit_ledger::redact::apply`] — the single source of truth.
/// When the engine fires, the field is replaced WHOLESALE with
/// [`REDACTED_TOKEN`] and one [`Removal`] is recorded carrying the SHA-256 of
/// the original field text (auditable, never re-exposing plaintext). Idempotent:
/// clean text is a no-op; already-redacted text trips no detector and adds no
/// removal.
pub fn redact_field(location: &str, text: &str, manifest: &mut RedactionManifest) -> String {
    let redacted = hugit_ledger::redact::apply(text);
    if redacted != text {
        // The engine fired: record the wholesale removal (digest of the ORIGINAL
        // field, so the audit trail proves WHAT was removed without re-exposing).
        let digest = hex::encode(Sha256::digest(text.as_bytes()));
        manifest.removals.push(Removal {
            location: location.to_string(),
            policy: LEDGER_POLICY_LABEL.to_string(),
            removed_digest: digest,
        });
    }
    redacted
}

/// Scan text for any residual secret per the UNIFIED ledger engine (WF-4). Used
/// by the red-team proof (E5⑦): after export, NO secret may survive anywhere in
/// git or JSON.
///
/// Detection is the SAME law `redact_field` applies on the way out
/// ([`hugit_ledger::redact::apply`]) — so any secret class the field-level path
/// catches is caught here. ONE artifact-level adjustment: bare 40/64-hex runs
/// are neutralised before the scan. At the field level a bare hex run is treated
/// as secret-shaped (WF-1, err toward redaction), but a serialized EXPORT
/// ARTIFACT legitimately carries content-address digests in typed positions
/// (the manifest's `removed_digest`, ref targets, the manifest ref) — those are
/// not secrets, and `contains_secret` is a residual-plaintext assertion over the
/// concatenated artifact, not a field. Neutralising bare hex keeps the
/// red-team's "no seeded secret survived" check honest without re-flagging the
/// export's own digests. Every NON-digest secret class (prefix tokens, PEM,
/// conn-strings, keyword-context, JWT, high-entropy keys) still fires.
pub fn contains_secret(text: &str) -> bool {
    let neutralised = neutralise_bare_hex_digests(text);
    hugit_ledger::redact::apply(&neutralised) != neutralised
}

/// Replace every content-address-shaped token with a benign placeholder so the
/// artifact-level residual scan does not flag the export's OWN content addresses
/// (see [`contains_secret`]). A token is content-address-shaped when it is, or
/// ends in (after an optional `<algo>:` or `<label>-` prefix), a 40/64-char hex
/// run — i.e. a bare digest, a `sha256:<hex>`/`cas:<hex>` ref, or the manifest
/// ref `redaction-<hex>`. Tokens that are NOT digest-shaped (a real key of other
/// length/alphabet — `ghp_…`, a dense base64 blob) are left intact so the engine
/// still catches them.
fn neutralise_bare_hex_digests(text: &str) -> String {
    // `:` counts as a token char so `sha256:<hex>` / `cas:<hex>` stay ONE token
    // (the manifest ref and CAS refs carry their algorithm/label inline).
    fn is_tok(b: u8) -> bool {
        b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'=' | b'-' | b'_' | b':')
    }
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if !is_tok(bytes[i]) {
            // Copy the maximal NON-token span verbatim (UTF-8 safe — separators
            // may be multi-byte chars; never push `byte as char`).
            let sep_start = i;
            while i < bytes.len() && !is_tok(bytes[i]) {
                i += 1;
            }
            out.push_str(&text[sep_start..i]);
            continue;
        }
        let start = i;
        while i < bytes.len() && is_tok(bytes[i]) {
            i += 1;
        }
        let token = &text[start..i];
        if is_content_address_shaped(token) {
            out.push_str("contentaddressplaceholder");
        } else {
            out.push_str(token);
        }
    }
    out
}

/// True iff `token` is, or ends in, a 40/64-char hex digest (a content address):
/// a bare digest, `<algo>:<hex>` (`cas:`/`sha256:`/…), or `<label>-<hex>` (the
/// `redaction-<hex>` manifest ref). The hex suffix is found after the last `:`
/// or `-` separator; the prefix (if any) must be a plain identifier.
fn is_content_address_shaped(token: &str) -> bool {
    // Candidate hex suffix: everything after the last `:` or `-` (or the whole
    // token when there is no separator). `rsplit` always yields ≥1 item.
    let hex = token.rsplit([':', '-']).next().unwrap_or(token);
    if !matches!(hex.len(), 40 | 64) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    // The prefix (token minus the hex suffix and its single separator) must be a
    // plain identifier — letters/digits/`_`/`-`/`:` — so we don't neutralise a
    // token that merely happens to end in hex after arbitrary symbols.
    let prefix_len = token.len() - hex.len();
    let prefix = &token[..prefix_len];
    prefix
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WF-4: the 9 classes the OLD static 16-pattern export list missed are now
    /// caught — because detection is the unified ledger engine. Each is fed
    /// through the EXPORT redaction path (`redact_field`) and proven redacted +
    /// manifested.
    #[test]
    fn export_catches_previously_missed_classes() {
        // (class label, a specimen the old static list did NOT catch)
        let specimens = [
            ("jwt", "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.abc123def"),
            ("gho", "gho_16C7e42F292c6912E7710c838347Ae178B4a"),
            ("ghs", "ghs_16C7e42F292c6912E7710c838347Ae178B4a"),
            (
                "clp-corelink-pat",
                "clp_live_9f8e7d6c5b4a3210fedcba9876543210",
            ),
            (
                "pem-ec-private-key-body",
                "-----BEGIN EC PRIVATE KEY-----\nMHcCAQEE...\n-----END EC PRIVATE KEY-----",
            ),
            (
                "conn-string",
                "postgres://user:s3cr3tlongpasswordvalue@db.internal:5432/app",
            ),
            ("keyword-context-spaces", "db_password = hunter2"),
            ("high-entropy", "creds 8Kp2mZ9qLx4vTn7wRj3sYb6cFd1gHe0 done"),
            (
                "bare-hex-secret",
                "SECRET_KEY 9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
            ),
        ];

        for (label, specimen) in specimens {
            let mut manifest = RedactionManifest::new();
            let out = redact_field(&format!("test/{label}"), specimen, &mut manifest);
            assert_eq!(
                out, REDACTED_TOKEN,
                "class '{label}' must redact wholesale through the export path"
            );
            assert_eq!(
                manifest.removals.len(),
                1,
                "class '{label}' must be MANIFESTED (recorded, not silent)"
            );
            // The manifest carries the digest of the ORIGINAL, never plaintext.
            assert_ne!(
                manifest.removals[0].removed_digest, specimen,
                "manifest stores a digest, not plaintext"
            );
            assert!(
                !contains_secret(&out),
                "no residual secret survives for class '{label}'"
            );
        }
    }

    /// The export `contains_secret` is now backed by the same engine — a class
    /// the old static list MISSED is no longer a false negative.
    #[test]
    fn contains_secret_uses_unified_engine() {
        assert!(contains_secret(
            "a jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiJ4In0.sig"
        ));
        assert!(contains_secret("gho_16C7e42F292c6912E7710c838347Ae178B4a"));
        assert!(!contains_secret("a perfectly ordinary commit message"));
        // The sentinel itself is clean (idempotent on already-redacted text).
        assert!(!contains_secret(REDACTED_TOKEN));
    }

    /// Idempotence: redacting clean text is a no-op (no removal); redacting the
    /// sentinel adds no removal.
    #[test]
    fn redact_field_idempotent_on_clean_and_sentinel() {
        let mut m = RedactionManifest::new();
        assert_eq!(redact_field("loc", "clean text", &mut m), "clean text");
        assert!(m.is_empty(), "clean text records no removal");
        assert_eq!(redact_field("loc", REDACTED_TOKEN, &mut m), REDACTED_TOKEN);
        assert!(m.is_empty(), "the sentinel trips no detector");
    }
}
