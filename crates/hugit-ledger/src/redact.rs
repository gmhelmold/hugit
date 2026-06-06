//! View-boundary redaction filter (④).
//!
//! A planted secret is any string that matches the canonical marker prefix
//! `SECRET:`.  At the view boundary every such string is replaced with the
//! sentinel "[REDACTED]" so secrets never appear in ledger/verdict output bytes.
//!
//! The redaction rule is:
//! - Any field value that **contains** the substring `SECRET:` is replaced with
//!   "[REDACTED]" wholesale.
//! - This is applied to every string field before it is placed in a
//!   `LedgerEntry` or `VerdictView`.

/// The planted-secret prefix that triggers redaction.
pub const SECRET_MARKER: &str = "SECRET:";

/// The redaction sentinel rendered in ledger/verdict views.
pub const REDACTED: &str = "[REDACTED]";

/// Apply view-boundary redaction to a string field.
///
/// Returns "[REDACTED]" if the input contains the secret marker; otherwise
/// returns the input unchanged (as an owned `String`).
pub fn apply(s: &str) -> String {
    if s.contains(SECRET_MARKER) {
        REDACTED.to_string()
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_secret_passthrough() {
        assert_eq!(apply("normal text"), "normal text");
    }

    #[test]
    fn secret_becomes_redacted() {
        assert_eq!(apply("SECRET:my-api-key-12345"), REDACTED);
    }

    #[test]
    fn embedded_secret_becomes_redacted() {
        assert_eq!(apply("prefix SECRET: suffix"), REDACTED);
    }
}
