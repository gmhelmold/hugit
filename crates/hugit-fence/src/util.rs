//! Crate-private utilities for the `broker` (shared, before the WP-R4
//! transfer, with the `enforce`/`materialize` halves that moved to
//! corelink-runners).

/// Normalize a relative path into canonical, slash-separated segments — **the
/// fence traversal rule**.
///
/// Returns `None` if the path escapes the workspace root — an absolute path
/// or any `..` component is treated as an escape, because such a path can
/// resolve outside the materialized set. `.` and empty segments are dropped;
/// `Some(vec![])` is a path that collapses to the root (`"."`, `"./"`, `""`).
///
/// Relocated verbatim from `enforce::normalize_path` when the enforcement
/// half (`materialize`/`enforce`) transferred to corelink-runners (WP-R4):
/// the broker's result-delivery guard keeps applying the EXACT rule the
/// transferred materialize layer's `place_file` re-guard applies on its side
/// of the wire, so the broker delivery path cannot diverge from the fence's
/// traversal policy.
#[must_use]
pub(crate) fn normalize_path(path: &str) -> Option<Vec<&str>> {
    if path.starts_with('/') {
        return None; // absolute → escapes the workspace root
    }
    let mut out = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}       // drop empty / current-dir segments
            ".." => return None, // parent traversal → escape
            s => out.push(s),
        }
    }
    Some(out)
}

/// POSIX single-quote a string for safe interpolation into a remote `sh -c`.
///
/// Every path the fence places or probes on the box passes through this before
/// appearing in a shell command, so shell injection via a crafted path is
/// defeated.  Note: this blocks injection but **not** traversal — callers must
/// validate traversal separately before quoting.
#[must_use]
pub(crate) fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// Minimal, dependency-free standard base64 (standard alphabet, padded).
///
/// Used to ship file content and credential-scan needles over a shell transport
/// without raw bytes appearing on any command line.
#[must_use]
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as usize;
        let b1 = chunk.get(1).copied().unwrap_or(0) as usize;
        let b2 = chunk.get(2).copied().unwrap_or(0) as usize;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(A[(n >> 18) & 63] as char);
        out.push(A[(n >> 12) & 63] as char);
        out.push(if chunk.len() > 1 {
            A[(n >> 6) & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            A[n & 63] as char
        } else {
            '='
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quote_escapes_single_quotes() {
        assert_eq!(shell_quote("a'b"), r"'a'\''b'");
        assert_eq!(shell_quote("no-quotes"), "'no-quotes'");
    }

    #[test]
    fn base64_known_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn normalize_path_traversal_rule_preserved() {
        // The exact assertions the rule carried in its enforce/ home (WP-R4
        // relocation — rule unchanged): absolute and `..`-bearing paths
        // escape (None); `.`/empty segments drop; root-collapse is Some([]).
        assert_eq!(normalize_path("/etc/passwd"), None);
        assert_eq!(normalize_path("../x"), None);
        assert_eq!(normalize_path("src/../../x"), None);
        assert_eq!(normalize_path("src/../src/main.rs"), None);
        assert_eq!(
            normalize_path("./src/./main.rs"),
            Some(vec!["src", "main.rs"])
        );
        assert_eq!(normalize_path("src/main.rs"), Some(vec!["src", "main.rs"]));
        assert_eq!(normalize_path("./"), Some(vec![]));
        assert_eq!(normalize_path("."), Some(vec![]));
        assert_eq!(normalize_path(""), Some(vec![]));
    }
}
