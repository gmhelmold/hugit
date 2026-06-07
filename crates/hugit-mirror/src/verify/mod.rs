//! Per-push content-hash verification (WP-E1a item ①).
//!
//! Every outbound push to the GitHub mirror carries the **content hash it
//! expects to observe post-push**. After the push, the writer re-reads the
//! mirror ref's object hash and compares it to that locally-computed expected
//! hash. The comparison is **byte-identity** over the git object id (the
//! content address): if they match, the ref is verified-synced; if they
//! differ, the push is a divergence.
//!
//! Invariants (all FAIL-CLOSED):
//!
//! - Verification is **per push**, never batched — each ref's identity is
//!   proven the moment it lands on the mirror.
//! - A mismatch is a **hard fail**: the writer raises a [`DivergenceSignal`]
//!   and NEVER marks the ref synced. (The alarm routing and repair are E1b's;
//!   the *detection* is here.) There is no "best-effort accept" path.
//! - The expected hash is the git oid the read path (D2) computed for the
//!   object — the same content address both stores key by, which is exactly
//!   what makes a verified mirror byte-identical to hugit.

use serde::{Deserialize, Serialize};

/// A content hash = a git object id, as a lowercase 40-hex SHA-1 string.
///
/// This is the byte-identity key: two refs are identical iff their tip oids are
/// equal. The mirror writer computes the expected oid locally (from the D2 read
/// path) and re-reads the observed oid from GitHub after the push.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ContentHash(pub String);

impl ContentHash {
    /// Construct from any hex string, normalised to lowercase.
    pub fn new(hex: impl Into<String>) -> Self {
        Self(hex.into().to_ascii_lowercase())
    }

    /// The raw hex digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ContentHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A divergence signal raised when a post-push verify finds the mirror ref does
/// NOT byte-match the expected content hash (fail-CLOSED).
///
/// This is the detection contract handed to E1b's alarm/repair seam. The
/// writer NEVER marks the ref synced when this is raised.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DivergenceSignal {
    /// The ref whose mirror tip diverged from the expected hash.
    pub ref_name: String,
    /// The hash the writer expected to observe on the mirror (locally computed).
    pub expected: ContentHash,
    /// The hash actually observed on the mirror after the push.
    pub observed: ContentHash,
    /// Human-readable detail for the divergence log.
    pub detail: String,
}

/// The outcome of a per-push content-hash verification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// The mirror ref byte-matches the expected hash — verified-synced.
    Verified {
        /// The ref that was verified.
        ref_name: String,
        /// The content hash both sides agree on.
        hash: ContentHash,
    },
    /// The mirror ref did NOT match — fail-CLOSED divergence (never synced).
    Diverged(DivergenceSignal),
}

impl VerifyOutcome {
    /// Whether the push verified to byte-identity.
    pub fn is_verified(&self) -> bool {
        matches!(self, VerifyOutcome::Verified { .. })
    }

    /// The divergence signal, if this is a fail-CLOSED mismatch.
    pub fn divergence(&self) -> Option<&DivergenceSignal> {
        match self {
            VerifyOutcome::Diverged(d) => Some(d),
            VerifyOutcome::Verified { .. } => None,
        }
    }
}

/// Verify a single push by comparing the locally-computed `expected` content
/// hash against the `observed` hash re-read from the GitHub mirror (item ①).
///
/// **Fail-CLOSED**: any mismatch yields [`VerifyOutcome::Diverged`] carrying a
/// [`DivergenceSignal`] — the caller must NOT mark the ref synced. Only an
/// exact byte-identity match yields [`VerifyOutcome::Verified`].
pub fn verify_push(
    ref_name: &str,
    expected: &ContentHash,
    observed: &ContentHash,
) -> VerifyOutcome {
    if expected == observed {
        VerifyOutcome::Verified {
            ref_name: ref_name.to_string(),
            hash: expected.clone(),
        }
    } else {
        // Hash mismatch → divergence. Fail-CLOSED: never synced.
        VerifyOutcome::Diverged(DivergenceSignal {
            ref_name: ref_name.to_string(),
            expected: expected.clone(),
            observed: observed.clone(),
            detail: format!(
                "post-push content-hash mismatch on {ref_name}: expected {expected}, \
                 mirror observed {observed} (fail-CLOSED, not marked synced)"
            ),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_hash_verifies() {
        let h = ContentHash::new("ABC123");
        let out = verify_push("refs/heads/main", &h, &ContentHash::new("abc123"));
        assert!(out.is_verified());
        assert!(out.divergence().is_none());
    }

    #[test]
    fn mismatch_is_fail_closed_divergence() {
        let exp = ContentHash::new(format!("{:040x}", 1));
        let obs = ContentHash::new(format!("{:040x}", 2));
        let out = verify_push("refs/heads/main", &exp, &obs);
        assert!(!out.is_verified());
        let d = out.divergence().expect("must raise divergence");
        assert_eq!(d.expected, exp);
        assert_eq!(d.observed, obs);
    }
}
