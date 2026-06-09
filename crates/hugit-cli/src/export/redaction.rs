//! Redaction-at-export + the redaction manifest (E5④/⑦).
//!
//! The context/journal redaction policy is applied **at export time**: seeded
//! secrets appear **nowhere** in the git artifact or the JSON envelope, and
//! every removal is **manifested** (recorded, never silent). The redacted
//! artifact still passes the exit proof.
//!
//! The secret signatures mirror the house CI secrets gate
//! ([`hugit_checks`]/policy) so the export redactor and the landing-time gate
//! can never drift on what counts as a secret.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The fixed replacement token a redacted span is rewritten to. It is itself
/// secret-free and stable, so a redacted artifact is deterministic. Single-sourced
/// from the canonical [`hugit_contracts::REDACTED_MARKER`] so it cannot drift from
/// the ledger/verdict redaction sentinel.
pub const REDACTED_TOKEN: &str = hugit_contracts::REDACTED_MARKER;

/// Known secret signatures redacted at export. Substring matches, no regex — the
/// same portable approach the secrets gate uses. Order matters only for the
/// manifest label; every match is removed.
pub static SECRET_PATTERNS: &[(&str, &str)] = &[
    ("aws-access-key", "AKIA"),
    ("aws-secret-key", "aws_secret_access_key"),
    ("api-key", "api_key="),
    ("api-key-upper", "API_KEY="),
    ("bearer-token", "Bearer "),
    ("rsa-private-key", "-----BEGIN RSA PRIVATE KEY-----"),
    ("ec-private-key", "-----BEGIN EC PRIVATE KEY-----"),
    ("openssh-private-key", "-----BEGIN OPENSSH PRIVATE KEY-----"),
    ("password", "password="),
    ("password-upper", "PASSWORD="),
    ("token", "token="),
    ("token-upper", "TOKEN="),
    ("github-token", "ghp_"),
    ("github-actions-token", "ghs_"),
    ("slack-bot-token", "xoxb-"),
    ("slack-user-token", "xoxp-"),
];

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

/// Redact a single field's text against the policy, recording every removal into
/// `manifest`. Returns the redacted text. Idempotent: redacting clean text is a
/// no-op; redacting already-redacted text adds no removals.
pub fn redact_field(location: &str, text: &str, manifest: &mut RedactionManifest) -> String {
    let mut out = text.to_string();

    for (label, pattern) in SECRET_PATTERNS {
        while let Some(idx) = out.find(pattern) {
            // Remove the secret token: the matched pattern plus the contiguous
            // run of non-whitespace, non-quote material that follows it (the
            // secret value), so the value itself is gone, not just the prefix.
            let start = idx;
            let after = &out[idx + pattern.len()..];
            let value_len = after
                .find(|c: char| c.is_whitespace() || c == '"' || c == '\'')
                .unwrap_or(after.len());
            let end = idx + pattern.len() + value_len;
            let removed = &out[start..end];

            let digest = hex::encode(Sha256::digest(removed.as_bytes()));
            manifest.removals.push(Removal {
                location: location.to_string(),
                policy: (*label).to_string(),
                removed_digest: digest,
            });

            out.replace_range(start..end, REDACTED_TOKEN);
        }
    }

    out
}

/// Scan text for any residual secret signature. Used by the red-team proof
/// (E5⑦): after export, NO secret pattern may survive anywhere in git or JSON.
pub fn contains_secret(text: &str) -> bool {
    SECRET_PATTERNS.iter().any(|(_, p)| text.contains(p))
}
