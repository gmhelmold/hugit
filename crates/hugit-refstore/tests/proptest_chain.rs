//! O-1 (T-8) — PROPERTY tests for the event-log hash chain security spine.
//!
//! An adversarial review sweep (T-8) found the
//! hash chain had ONLY hand-crafted tamper fixtures. This suite generates random
//! chains of N records AND a random single mutation, and asserts:
//!
//!   1. a freshly-built valid chain ALWAYS verifies (`verify_chain` == Ok), and
//!   2. ANY single mutation that does not recompute downstream hashes — a payload
//!      byte-flip, a dropped record, a reorder, a duplicate — ALWAYS breaks
//!      verification (the partial-corruption guarantee, the chain's actual
//!      promise: tamper-EVIDENT against an incomplete rewrite).
//!
//! The chain is built through the SAME `EventLog::append_for_test` producer the
//! engine uses (`test-support` feature), so producer and verifier cannot drift in
//! the test the way they cannot in production.
//!
//! Property failure => a real bug; reported, not papered over.

use hugit_contracts::event_record::EventRecord;
use hugit_refstore::log::EventLog;
use hugit_refstore::tamper::verify_chain;
use proptest::prelude::*;

/// A single appended event: (kind, principal_chain, payload). The payload is
/// kept JSON-ish-canonical (a quoted string) so it round-trips the hash formula
/// unchanged; the chain promise is independent of payload contents.
fn event() -> impl Strategy<Value = (String, Vec<String>, String)> {
    (
        proptest::string::string_regex("[a-z]{1,8}(\\.[a-z]{1,8}){0,2}").unwrap(),
        proptest::collection::vec(
            proptest::string::string_regex("[a-zA-Z0-9_@./-]{1,12}").unwrap(),
            0..=3,
        ),
        proptest::string::string_regex("\"[a-zA-Z0-9 _:,{}-]{0,30}\"").unwrap(),
    )
}

/// Build a valid chain of `events.len()` records via the real producer.
fn build_chain(events: &[(String, Vec<String>, String)]) -> Vec<EventRecord> {
    let mut log = EventLog::new();
    for (i, (kind, principals, payload)) in events.iter().enumerate() {
        log.append_for_test(kind.clone(), principals.clone(), payload.clone(), i as u64);
    }
    log.records().to_vec()
}

proptest! {
    // Bounded, deterministic for CI; no on-disk regression-seed persistence (the
    // suite is hermetic — a counterexample surfaces in the failure message).
    #![proptest_config(ProptestConfig {
        cases: 384,
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// A freshly-built valid chain of N random records always verifies.
    #[test]
    fn valid_chain_verifies(events in proptest::collection::vec(event(), 0..=12)) {
        let records = build_chain(&events);
        prop_assert!(
            verify_chain(&records).is_ok(),
            "a freshly-built valid chain failed verification: {records:?}"
        );
    }

    /// Tamper detection — flip one byte of one record's payload WITHOUT
    /// recomputing downstream hashes. `verify_chain` must return an error.
    #[test]
    fn payload_byteflip_is_detected(
        events in proptest::collection::vec(event(), 1..=12),
        idx in any::<prop::sample::Index>(),
    ) {
        let mut records = build_chain(&events);
        let i = idx.index(records.len());
        // Mutate the payload (append a char) but leave this_hash/prev_hash as the
        // producer wrote them — exactly the incomplete-rewrite the chain catches.
        records[i].payload.push('!');
        prop_assert!(
            verify_chain(&records).is_err(),
            "a payload byte-flip at idx {i} was NOT detected: {records:?}"
        );
    }

    /// Tamper detection — drop one NON-TAIL record. The seq becomes non-gap-free
    /// and/or the linkage breaks mid-stream. `verify_chain` must error.
    ///
    /// NOTE — the drop index is constrained to `[0, len-2]` (never the LAST
    /// record) BY DESIGN. Dropping the tail record leaves a still-valid PREFIX:
    /// `verify_chain` re-checks `seq == position` + the prev-hash linkage from
    /// genesis forward, and a shorter valid prefix satisfies both. Detecting a
    /// truncated TAIL requires a published length / head-hash anchor, which is
    /// exactly the unkeyed-chain honesty caveat documented on `verify_chain`
    /// (tamper-EVIDENT against an incomplete mid-stream rewrite; the
    /// append-immutability + length anchor is the P2 server-side seam, PS-8). So
    /// this property asserts the guarantee the local chain actually makes:
    /// any MID-STREAM drop is caught.
    #[test]
    fn dropped_non_tail_record_is_detected(
        events in proptest::collection::vec(event(), 2..=12),
        idx in any::<prop::sample::Index>(),
    ) {
        let mut records = build_chain(&events);
        // index in [0, len-2] — never the last record.
        let i = idx.index(records.len() - 1);
        records.remove(i);
        prop_assert!(
            verify_chain(&records).is_err(),
            "dropping non-tail record at idx {i} was NOT detected: {records:?}"
        );
    }

    /// Tamper detection — reorder two adjacent records. Their seqs / linkage no
    /// longer match position. `verify_chain` must error.
    #[test]
    fn reorder_is_detected(
        events in proptest::collection::vec(event(), 2..=12),
        idx in any::<prop::sample::Index>(),
    ) {
        let mut records = build_chain(&events);
        // Choose a swap position in [0, len-2] so i+1 is valid.
        let i = idx.index(records.len() - 1);
        records.swap(i, i + 1);
        prop_assert!(
            verify_chain(&records).is_err(),
            "reordering records at idx {i}/{} was NOT detected: {records:?}",
            i + 1
        );
    }

    /// Tamper detection — duplicate one record (insert a clone right after it).
    /// The duplicate's seq collides with the following position. `verify_chain`
    /// must error.
    #[test]
    fn duplicate_record_is_detected(
        events in proptest::collection::vec(event(), 1..=12),
        idx in any::<prop::sample::Index>(),
    ) {
        let mut records = build_chain(&events);
        let i = idx.index(records.len());
        let clone = records[i].clone();
        records.insert(i + 1, clone);
        prop_assert!(
            verify_chain(&records).is_err(),
            "duplicating record at idx {i} was NOT detected: {records:?}"
        );
    }
}
