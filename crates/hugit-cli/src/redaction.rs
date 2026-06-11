//! Porcelain redaction parity (Wave E, P-REDACT-SURFACE).
//!
//! The hardened engine [`hugit_ledger::redact`] is the ONE redaction law. The
//! ledger/export views already route through it; the flow porcelain
//! (`intent`/`campaign`) did NOT — it persisted user-supplied free text
//! (charter / acceptance / campaign / owner / reason) verbatim into
//! `.hugit/intents.json` AND the hash-chained `--log` payload, and echoed it raw
//! on the read surfaces. The adversary proved a real `ghp_…` PAT survived
//! verbatim and re-emitted.
//!
//! This module is the single seam every porcelain WRITE and READ surface scrubs
//! through, so the engine's detector set (known-prefix credentials, PEM blocks,
//! JWTs, high-entropy runs) applies uniformly. The replacement is the engine's
//! wholesale [`REDACTED`](hugit_ledger::redact::REDACTED) sentinel — a string
//! that trips any detector is replaced in full, so no fragment of a secret
//! survives.
//!
//! ## Write-before-append is the only fix
//!
//! The canonical event log is hash-chained and append-only: a secret appended
//! verbatim is unredactable forever (re-hashing would break the chain). The
//! write surfaces therefore scrub the user text BEFORE building the sidecar /
//! payload, so the secret never reaches the persisted record. The read surfaces
//! scrub again as defence-in-depth — a pre-existing (pre-Wave-E) log or a field
//! a future write path forgets still echoes redacted.

use hugit_ledger::redact;

/// Scrub one free-text field through the hardened engine: the engine's
/// [`REDACTED`](hugit_ledger::redact::REDACTED) sentinel when any detector
/// fires, the input unchanged otherwise.
pub fn scrub(s: &str) -> String {
    redact::apply(s)
}

/// Scrub each item of a free-text list (e.g. an intent's acceptance criteria).
pub fn scrub_all(items: &[String]) -> Vec<String> {
    items.iter().map(|s| scrub(s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_ledger::redact::REDACTED;

    #[test]
    fn non_secret_passes_through() {
        assert_eq!(
            scrub("Add the JSON porcelain for intents"),
            "Add the JSON porcelain for intents"
        );
    }

    #[test]
    fn github_pat_is_redacted_wholesale() {
        let charter = "Wire the deploy using token ghp_16C7e42F292c6912E7710c838347Ae178B4a now";
        assert_eq!(scrub(charter), REDACTED);
    }

    #[test]
    fn connection_string_is_redacted() {
        // The engine's high-entropy + prefix detectors catch credentials; a
        // long random-looking secret trips the entropy scan.
        let s = "postgres://user:S3cr3tP4ssw0rdVeryLongRandomToken9999@db.internal:5432/app";
        assert_eq!(scrub(s), REDACTED);
    }

    #[test]
    fn scrub_all_redacts_per_item() {
        let items = vec![
            "build the parser".to_string(),
            "use ghp_16C7e42F292c6912E7710c838347Ae178B4a".to_string(),
        ];
        let out = scrub_all(&items);
        assert_eq!(out[0], "build the parser");
        assert_eq!(out[1], REDACTED);
    }
}
