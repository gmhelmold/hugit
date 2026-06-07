//! Git object hash computation for byte-identity verification (WP-E2a, item ①).
//!
//! Git computes object identifiers (OIDs) as:
//!   `SHA-1("<kind> <size-in-bytes>\0<raw-bytes>")`
//!
//! This module replicates that formula so imported bytes can be verified
//! byte-identical to the source without re-cloning.

use sha1::{Digest, Sha1};

/// Compute the git object OID (SHA-1 hex) for a given object kind and raw bytes.
///
/// Formula: `SHA-1("<kind> <len>\0<bytes>")` where `<len>` is the byte length
/// of `raw_bytes` as a decimal ASCII string.
///
/// Returns a 40-character lowercase hex string.
pub fn compute_git_oid(kind: &str, raw_bytes: &[u8]) -> String {
    let header = format!("{} {}\0", kind, raw_bytes.len());
    let mut hasher = Sha1::new();
    hasher.update(header.as_bytes());
    hasher.update(raw_bytes);
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_blob_oid_matches_git_canonical() {
        // The empty blob OID is a well-known git constant.
        // `git hash-object /dev/null` → e69de29bb2d1d6434b8b29ae775ad8c2e48c5391
        let oid = compute_git_oid("blob", &[]);
        assert_eq!(oid, "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
    }

    #[test]
    fn hello_world_blob_oid() {
        // `printf 'hello world' | git hash-object --stdin` (no trailing newline)
        // → 95d09f2b10159347eece71399a7e2e907ea3df4f
        let oid = compute_git_oid("blob", b"hello world");
        assert_eq!(oid, "95d09f2b10159347eece71399a7e2e907ea3df4f");
    }

    #[test]
    fn oid_is_40_hex_chars() {
        let oid = compute_git_oid("blob", b"some content");
        assert_eq!(oid.len(), 40);
        assert!(oid.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
