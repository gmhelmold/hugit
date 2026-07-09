//! Redaction-with-hash-preservation — the tamper-evident primitive that lets an
//! Art.17 erasure render already-written cleartext PII **unrecoverable** while the
//! append-only chain STILL verifies (ADR-0004, follow-up leg 4).
//!
//! ## The constraint
//!
//! `this_hash` is a frozen SHA-256 over `(prev_hash, kind, principal_chain, payload,
//! seq)` and [`verify_chain`](super::verify_chain) recomputes it (check #3). Existing
//! records were hashed with the subject's CLEARTEXT `principal_chain`/`payload`
//! bytes, so those bytes cannot simply be scrubbed in place — the recompute would no
//! longer match the stored `this_hash` and the chain would fail closed.
//!
//! ## The mechanism (in-band redaction records, so `verify_chain` needs NO new args)
//!
//! A redaction rewrites record N's `principal_chain`/`payload` to the pseudonym
//! **while PRESERVING record N's original `this_hash`** (so the `prev_hash` linkage of
//! record N+1 — check #2, the append-immutability spine — is untouched). To keep the
//! REDACTED content itself tamper-evident, an APPEND-ONLY `provenance.redaction`
//! record is added to the same log carrying a [`RedactionCommitment`]:
//!
//! - `original_this_hash` — record N's frozen `this_hash` (binds the preserved hash to
//!   the redaction claim), and
//! - `redacted_this_hash` — `compute_this_hash` over record N's REDACTED fields (so the
//!   redacted content cannot be altered afterwards without detection).
//!
//! [`verify_chain`](super::verify_chain) does a first pass collecting these markers,
//! then for a redacted record checks #1 (seq) + #2 (linkage, via the preserved
//! `this_hash`) as always, and REPLACES check #3 with: stored `this_hash ==
//! original_this_hash` AND recompute-over-current-fields `== redacted_this_hash`.
//!
//! ## What is preserved, and what is deliberately reduced
//!
//! Preserved for EVERY record: #1 (gap-free monotonic seq — insertion/drop/reorder
//! detection) and #2 (`prev_hash` linkage — splice/append-immutability). Non-redacted
//! records keep FULL check #3. What is deliberately reduced for a redacted record: its
//! ORIGINAL cleartext content is no longer independently re-derivable from `this_hash`
//! (by design — that cleartext is GONE), but the redacted content is still bound by
//! `redacted_this_hash`. This is a bounded, documented reduction that lives WITHIN the
//! chain's existing threat model (tamper-EVIDENT, not tamper-PROOF against a competent
//! rewriter — see [`super::verify_chain`]'s honesty caveat). A log with NO redaction
//! records verifies **byte-for-byte identically** to before (zero regression).

use crate::log::canonical_json;
use hugit_contracts::event_record::EventRecord;
use std::collections::BTreeMap;

/// The event kind of an APPEND-ONLY redaction marker. Its payload is a
/// [`RedactionCommitment`] naming the `redacted_seq` it covers. Carries NO cleartext.
pub const PROVENANCE_REDACTION_KIND: &str = "provenance.redaction";

/// The commitment a [`PROVENANCE_REDACTION_KIND`] record carries for the record it
/// redacts: the preserved original hash (chain linkage) + the redacted-content hash
/// (forward tamper-evidence of the pseudonymised bytes).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionCommitment {
    /// Record N's FROZEN `this_hash` — must still equal the stored `this_hash` of the
    /// redacted record (so its `prev_hash` linkage forward is intact).
    pub original_this_hash: String,
    /// `compute_this_hash` over record N's REDACTED (pseudonymised) fields — so the
    /// redacted content is itself tamper-evident going forward.
    pub redacted_this_hash: String,
}

impl RedactionCommitment {
    /// Serialise the marker payload (canonical JSON; carries only the seq + two hex
    /// hashes — never any cleartext). `redacted_seq` is the position of the record this
    /// marker redacts.
    #[must_use]
    pub fn to_payload(&self, redacted_seq: u64) -> String {
        let v = serde_json::json!({
            "redacted_seq": redacted_seq,
            "original_this_hash": self.original_this_hash,
            "redacted_this_hash": self.redacted_this_hash,
        });
        canonical_json(&v.to_string()).unwrap_or_else(|| v.to_string())
    }

    /// Parse a marker payload back to `(redacted_seq, commitment)`. `None` if the payload
    /// is not a well-formed redaction descriptor (a malformed marker contributes no
    /// redaction, so the record it claims to cover falls back to a FULL check #3 — a
    /// tampered/garbage marker therefore fails the chain closed, never opens it).
    #[must_use]
    pub fn parse(payload: &str) -> Option<(u64, Self)> {
        let v: serde_json::Value = serde_json::from_str(payload).ok()?;
        let redacted_seq = v.get("redacted_seq")?.as_u64()?;
        let original_this_hash = v.get("original_this_hash")?.as_str()?.to_string();
        let redacted_this_hash = v.get("redacted_this_hash")?.as_str()?.to_string();
        if original_this_hash.is_empty() || redacted_this_hash.is_empty() {
            return None;
        }
        Some((
            redacted_seq,
            Self {
                original_this_hash,
                redacted_this_hash,
            },
        ))
    }
}

/// Rebuild `record` with its `principal_chain`/`payload` replaced by their redacted
/// (pseudonymised) forms while PRESERVING every hash field — `seq`, `prev_hash`,
/// `this_hash`, `kind`, `recorded_at` are untouched, so the chain linkage is intact and
/// only the CLEARTEXT bytes change. Returns the redacted record plus the
/// [`RedactionCommitment`] the append-only marker must carry.
///
/// This is the primitive an Art.17 erasure calls per record it must scrub; the caller
/// (the erasure executor) supplies the pseudonymised replacements (it owns the
/// cleartext→pseudonym mapping via its key store). `original_this_hash` is taken from the
/// record's own frozen `this_hash`; `redacted_this_hash` is recomputed over the redacted
/// fields here so the two are always self-consistent.
#[must_use]
pub fn redact_record(
    record: &EventRecord,
    redacted_principal_chain: Vec<String>,
    redacted_payload: String,
) -> (EventRecord, RedactionCommitment) {
    let redacted_this_hash = crate::log::compute_this_hash(
        &record.prev_hash,
        &record.kind,
        &redacted_principal_chain,
        &redacted_payload,
        record.seq,
    );
    let commitment = RedactionCommitment {
        original_this_hash: record.this_hash.clone(),
        redacted_this_hash,
    };
    let redacted = EventRecord {
        seq: record.seq,
        prev_hash: record.prev_hash.clone(),
        // PRESERVED — the chain links through this; the redaction is verified via the marker.
        this_hash: record.this_hash.clone(),
        kind: record.kind.clone(),
        principal_chain: redacted_principal_chain,
        payload: redacted_payload,
        recorded_at: record.recorded_at,
    };
    (redacted, commitment)
}

/// First pass of the redaction-aware verify: collect every well-formed redaction marker
/// in the log into `seq -> commitment`. A later marker for the same seq wins (a
/// re-redaction). Empty for any log without redaction records — the common case — so the
/// verifier's redacted branch is never entered and behaviour is unchanged.
#[must_use]
pub fn collect_redactions(records: &[EventRecord]) -> BTreeMap<u64, RedactionCommitment> {
    let mut out = BTreeMap::new();
    for r in records {
        if r.kind != PROVENANCE_REDACTION_KIND {
            continue;
        }
        if let Some((seq, commitment)) = RedactionCommitment::parse(&r.payload) {
            out.insert(seq, commitment);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::EventLog;
    use crate::tamper::verify_chain;

    /// Build a 3-record log with cleartext identity in record 1 (a subject-authored
    /// event), plus a genesis and a trailer, via the crate-internal raw append.
    fn cleartext_log() -> EventLog {
        let mut log = EventLog::new();
        log.append(
            "repo.meta",
            vec!["orchestrator:hugit".into()],
            "{\"visibility\":\"public\"}",
            1,
        );
        log.append(
            "pr.opened",
            vec!["clerk:acme:user-1".into()],
            "{\"account\":\"acme\",\"subject\":\"acme\"}",
            2,
        );
        log.append(
            "pr.queued",
            vec!["orchestrator:hugit".into()],
            "{\"pr\":1}",
            3,
        );
        log
    }

    #[test]
    fn a_log_with_no_redaction_markers_verifies_exactly_as_before() {
        let log = cleartext_log();
        assert!(collect_redactions(log.records()).is_empty());
        verify_chain(log.records()).expect("a plain chain verifies");
    }

    #[test]
    fn a_redacted_record_plus_marker_verifies() {
        let log = cleartext_log();
        // Redact record 1 (the subject-authored event): pseudonymise principal + payload,
        // preserving this_hash.
        let (redacted, commitment) = redact_record(
            &log.records()[1],
            vec!["subj:deadbeef".into()],
            crate::log::canonical_json(
                "{\"account\":\"subj:deadbeef\",\"subject\":\"subj:deadbeef\"}",
            )
            .unwrap(),
        );
        // Rebuild the log with the redacted record swapped in, then append the marker.
        let mut rebuilt = EventLog::new();
        for (i, r) in log.records().iter().enumerate() {
            let rec = if i == 1 { redacted.clone() } else { r.clone() };
            rebuilt.push_record(rec).expect("seq is preserved");
        }
        rebuilt.append(
            PROVENANCE_REDACTION_KIND,
            vec!["subj:deadbeef".into()],
            commitment.to_payload(1),
            4,
        );
        // The cleartext is GONE, yet the chain STILL verifies.
        for r in rebuilt.records() {
            assert!(
                !r.payload.contains("acme"),
                "no cleartext account slug survives"
            );
            assert!(
                r.principal_chain.iter().all(|p| p != "clerk:acme:user-1"),
                "no cleartext clerk principal survives"
            );
        }
        verify_chain(rebuilt.records()).expect("redacted chain verifies via the marker");
    }

    #[test]
    fn tampering_the_redacted_content_after_redaction_is_detected() {
        let log = cleartext_log();
        let (redacted, commitment) = redact_record(
            &log.records()[1],
            vec!["subj:deadbeef".into()],
            crate::log::canonical_json("{\"x\":1}").unwrap(),
        );
        let mut rebuilt = EventLog::new();
        for (i, r) in log.records().iter().enumerate() {
            let mut rec = if i == 1 { redacted.clone() } else { r.clone() };
            // ATTACK: after redaction, mutate the redacted record's payload without
            // updating its marker.
            if i == 1 {
                rec.payload = crate::log::canonical_json("{\"x\":999}").unwrap();
            }
            rebuilt.push_record(rec).expect("seq preserved");
        }
        rebuilt.append(
            PROVENANCE_REDACTION_KIND,
            vec!["subj:deadbeef".into()],
            commitment.to_payload(1),
            4,
        );
        // The redacted content no longer matches the committed redacted hash → fail-closed.
        assert!(
            matches!(
                verify_chain(rebuilt.records()),
                Err(crate::tamper::TamperError::ThisHashMismatch { seq: 1, .. })
            ),
            "a post-redaction content change must be detected"
        );
    }

    #[test]
    fn a_forged_marker_claiming_a_different_original_hash_is_detected() {
        let log = cleartext_log();
        let (redacted, mut commitment) = redact_record(
            &log.records()[1],
            vec!["subj:deadbeef".into()],
            crate::log::canonical_json("{\"x\":1}").unwrap(),
        );
        // ATTACK: the marker claims a bogus original hash (≠ the record's preserved hash).
        commitment.original_this_hash = "0".repeat(64);
        let mut rebuilt = EventLog::new();
        for (i, r) in log.records().iter().enumerate() {
            let rec = if i == 1 { redacted.clone() } else { r.clone() };
            rebuilt.push_record(rec).expect("seq preserved");
        }
        rebuilt.append(
            PROVENANCE_REDACTION_KIND,
            vec!["subj:x".into()],
            commitment.to_payload(1),
            4,
        );
        assert!(
            verify_chain(rebuilt.records()).is_err(),
            "a marker whose original hash ≠ the preserved this_hash must fail closed"
        );
    }

    #[test]
    fn dropping_a_redacted_record_still_breaks_the_chain() {
        // Redaction must NOT weaken insertion/drop detection (#1/#2). Drop record 2 from a
        // redacted log and confirm it still fails closed.
        let log = cleartext_log();
        let (redacted, commitment) = redact_record(
            &log.records()[1],
            vec!["subj:deadbeef".into()],
            crate::log::canonical_json("{\"x\":1}").unwrap(),
        );
        let mut rebuilt = EventLog::new();
        for (i, r) in log.records().iter().enumerate() {
            let rec = if i == 1 { redacted.clone() } else { r.clone() };
            rebuilt.push_record(rec).expect("seq preserved");
        }
        rebuilt.append(
            PROVENANCE_REDACTION_KIND,
            vec!["subj:x".into()],
            commitment.to_payload(1),
            4,
        );
        // Splice out the trailing non-redacted record → seq gap → fail closed.
        let spliced: Vec<_> = rebuilt
            .records()
            .iter()
            .filter(|r| r.seq != 2)
            .cloned()
            .collect();
        assert!(
            verify_chain(&spliced).is_err(),
            "dropping a record from a redacted log is still detected"
        );
    }
}
