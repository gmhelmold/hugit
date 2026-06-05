//! WP-D1b acceptance oracle — hugit-refstore compaction/cold-tier + recovery + undo.
//!
//! Owned items (VERBATIM from decomposition v2.0 — D1):
//!   ③ compaction replay-equivalent, hot log bounded
//!   ④ undo restores + preserves history
//!   ⑥(+) recovery: hot-DO loss → full ref state rebuilt from cold tier
//!        (and/or mirror) replay-identical
//!
//! Driven by `tests/acceptance/wp-d1b/run.sh`. Builds strictly on D1a's frozen
//! pub API (append/replay/verify_chain) — never rewrites it.

use hugit_refstore::coldtier::InMemoryColdStore;
use hugit_refstore::compaction::compact;
use hugit_refstore::log::EventLog;
use hugit_refstore::recovery::{MirrorSource, NoMirror};
use hugit_refstore::recovery::{RecoverySource, recover_from_cold, recover_with_mirror};
use hugit_refstore::replay::{RefState, replay, replay_unchecked};
use hugit_refstore::tamper::verify_chain;
use hugit_refstore::undo::{compute_compensation, undo};

use hugit_contracts::event_record::EventRecord;

/// Deterministically build an N-event log of ref mutations interleaved with
/// inert events (mirrors D1a's generator so the projected state is non-trivial).
fn build_log(n: u64) -> EventLog {
    let mut log = EventLog::new();
    for i in 0..n {
        let principal_chain = vec![format!("agent:runner-{:02}", i % 7), "user:gustavo".into()];
        let recorded_at = 1_717_000_000_000 + i;
        match i % 5 {
            0..=2 => {
                let bucket = i % 64;
                let payload =
                    format!(r#"{{"ref":"refs/heads/branch-{bucket}","target":"oid-{i:064x}"}}"#);
                log.append("ref.update", principal_chain, payload, recorded_at);
            }
            3 => {
                let bucket = i % 64;
                let payload = format!(r#"{{"ref":"refs/heads/branch-{bucket}"}}"#);
                log.append("ref.delete", principal_chain, payload, recorded_at);
            }
            _ => {
                let payload = format!(r#"{{"note":"checkpoint-{i}"}}"#);
                log.append("checkpoint.noted", principal_chain, payload, recorded_at);
            }
        }
    }
    log
}

/// Recombine a cold tier's ranges + a hot remainder into the full record chain,
/// in seq order, the way a live DO would after compaction.
fn recombine(cold: &InMemoryColdStore, hot: &[EventRecord]) -> Vec<EventRecord> {
    use hugit_refstore::coldtier::ColdStore;
    let mut all: Vec<EventRecord> = Vec::new();
    for key in cold.list() {
        if let Some(range) = cold.get(&key) {
            all.extend(range.records);
        }
    }
    all.extend_from_slice(hot);
    all.sort_by_key(|r| r.seq);
    all
}

/// ③ compaction replay-equivalent, hot log bounded.
///
/// After compacting a log down to a hot bound, (a) the hot remainder is bounded,
/// (b) the cold tier holds the sealed prefix verbatim, and (c) the ref state
/// derived from (cold tier + hot remainder) is byte-for-byte identical to
/// replaying the full uncompacted log. History is relocated, never dropped.
#[test]
fn item_3_compaction_replay_equivalent() {
    let n = 5_000u64;
    let hot_bound = 256usize;

    let log = build_log(n);
    assert_eq!(log.len() as u64, n, "log holds all events");

    // Reference: the ref state of the full uncompacted log.
    let reference = replay(&log).expect("intact log replays");
    assert!(!reference.is_empty(), "non-trivial ref state");

    // Compact: D1a log is read-only; we get a bounded hot remainder + cold tier.
    let mut cold = InMemoryColdStore::new();
    let report = compact(&log, hot_bound, &mut cold).expect("compaction succeeds on intact log");

    // (a) hot log is BOUNDED.
    assert_eq!(
        report.hot.len(),
        hot_bound,
        "hot remainder bounded to hot_bound"
    );
    assert!(
        report.hot.len() <= hot_bound,
        "hot log never exceeds the bound"
    );
    assert_eq!(
        report.sealed_count as u64,
        n - hot_bound as u64,
        "all over-bound records were sealed"
    );

    // (b) cold tier holds the sealed prefix; nothing dropped.
    assert_eq!(report.sealed_start_seq, 0);
    assert_eq!(report.sealed_end_seq, n - hot_bound as u64);
    assert_eq!(cold.len(), 1, "one sealed cold range");

    // (c) REPLAY-EQUIVALENT: (cold + hot) recombines into the exact chain and
    //     re-verifies + re-projects byte-for-byte identically to the original.
    let recombined = recombine(&cold, report.hot.records());
    assert_eq!(
        recombined.len() as u64,
        n,
        "no record lost across compaction"
    );
    verify_chain(&recombined).expect("recombined chain re-verifies (relocated, not rewritten)");

    let after = replay_unchecked(&recombined).expect("recombined chain replays");
    assert_eq!(after, reference, "compaction is replay-equivalent");
    assert_eq!(
        after.canonical_bytes(),
        reference.canonical_bytes(),
        "compacted projection is byte-for-byte identical"
    );

    // Records keep their original identity (seq + hashes), proving relocation.
    for (i, r) in recombined.iter().enumerate() {
        assert_eq!(r.seq, i as u64, "seq preserved exactly across compaction");
    }

    // No-op compaction when already within the bound.
    let small = build_log(100);
    let mut cold2 = InMemoryColdStore::new();
    let rep2 = compact(&small, 256, &mut cold2).expect("no-op compaction");
    assert_eq!(rep2.sealed_count, 0, "nothing sealed when within bound");
    assert!(cold2.is_empty(), "cold tier untouched on no-op");
    assert_eq!(rep2.hot.len(), 100);
}

/// ④ undo restores + preserves history.
///
/// Undo is a COMPENSATING EVENT: it restores the prior ref state AND leaves the
/// original event (and its compensator) addressable in the chain. Never a
/// rewrite, never a deletion.
#[test]
fn item_4_undo_restores_preserves_history() {
    // (1) Undo an UPDATE that overwrote an existing ref → restores prior target.
    let mut log = EventLog::new();
    let pc = vec!["user:gustavo".to_string()];
    log.append(
        "ref.update",
        pc.clone(),
        r#"{"ref":"refs/heads/main","target":"oid-A"}"#,
        1,
    );
    log.append(
        "ref.update",
        pc.clone(),
        r#"{"ref":"refs/heads/main","target":"oid-B"}"#,
        2,
    );

    let before_overwrite = {
        // state after only the first append
        let mut l = EventLog::new();
        l.append(
            "ref.update",
            pc.clone(),
            r#"{"ref":"refs/heads/main","target":"oid-A"}"#,
            1,
        );
        replay(&l).unwrap()
    };
    let len_before_undo = log.len();
    let state_with_overwrite = replay(&log).unwrap();
    assert_eq!(state_with_overwrite.get("refs/heads/main"), Some("oid-B"));

    // Undo the overwrite (seq 1).
    let comp = undo(&mut log, 1, pc.clone(), 3).expect("undo overwrite");
    assert_eq!(comp.kind, "ref.update", "compensator restores prior value");

    // RESTORES: state now equals the state before the overwrite.
    let restored = replay(&log).unwrap();
    assert_eq!(restored.get("refs/heads/main"), Some("oid-A"));
    assert_eq!(
        restored.canonical_bytes(),
        before_overwrite.canonical_bytes(),
        "undo restored the prior ref state byte-for-byte"
    );

    // PRESERVES HISTORY: nothing removed — the log GREW by exactly one (the
    // compensator), the original overwrite event is still addressable, and the
    // chain still verifies.
    assert_eq!(
        log.len(),
        len_before_undo + 1,
        "undo appends, never deletes"
    );
    assert_eq!(
        log.records()[1].payload,
        r#"{"ref":"refs/heads/main","target":"oid-B"}"#,
        "the original overwrite event is untouched & addressable"
    );
    verify_chain(log.records()).expect("chain intact after compensating append");

    // (2) Undo a CREATE (ref did not exist before) → compensator DELETES it.
    let mut log2 = EventLog::new();
    log2.append(
        "ref.update",
        pc.clone(),
        r#"{"ref":"refs/heads/feature","target":"oid-X"}"#,
        1,
    );
    assert_eq!(
        replay(&log2).unwrap().get("refs/heads/feature"),
        Some("oid-X")
    );
    let comp2 = undo(&mut log2, 0, pc.clone(), 2).expect("undo create");
    assert_eq!(comp2.kind, "ref.delete", "undoing a create deletes the ref");
    assert_eq!(
        replay(&log2).unwrap().get("refs/heads/feature"),
        None,
        "ref restored to absent"
    );
    assert_eq!(
        log2.len(),
        2,
        "create event preserved + compensator appended"
    );

    // (3) Undo a DELETE → compensator restores the deleted ref's prior target.
    let mut log3 = EventLog::new();
    log3.append(
        "ref.update",
        pc.clone(),
        r#"{"ref":"refs/heads/x","target":"oid-1"}"#,
        1,
    );
    log3.append("ref.delete", pc.clone(), r#"{"ref":"refs/heads/x"}"#, 2);
    assert_eq!(replay(&log3).unwrap().get("refs/heads/x"), None);
    undo(&mut log3, 1, pc.clone(), 3).expect("undo delete");
    assert_eq!(
        replay(&log3).unwrap().get("refs/heads/x"),
        Some("oid-1"),
        "delete undone"
    );

    // (4) Undo is itself undoable (it's just another event) → force-push-style
    //     irreversible loss is unexpressible.
    let comp_payload = compute_compensation(&log3, 2).expect("undo is itself undoable");
    assert_eq!(comp_payload.kind, "ref.delete");

    // (5) An inert event has nothing to compensate (fail-loud, not silent).
    let mut log4 = EventLog::new();
    log4.append("checkpoint.noted", pc.clone(), r#"{"note":"hi"}"#, 1);
    assert!(
        compute_compensation(&log4, 0).is_err(),
        "inert event: nothing to undo"
    );
}

/// ⑥ recovery: hot-DO loss → full ref state rebuilt from cold tier
///   (and/or mirror) replay-identical.
#[test]
fn item_6_recovery_hot_do_loss_rebuild() {
    let n = 3_000u64;
    let log = build_log(n);
    let pre_loss = replay(&log).expect("intact log replays");
    assert!(!pre_loss.is_empty());

    // Simulate steady-state compaction: seal EVERYTHING to the cold tier so the
    // cold tier alone is a complete record of history (hot bound 0).
    let mut cold = InMemoryColdStore::new();
    let report = compact(&log, 0, &mut cold).expect("seal full history to cold tier");
    assert_eq!(report.sealed_count as u64, n);
    assert!(report.hot.is_empty(), "nothing left hot after full seal");

    // HOT-DO LOSS: the hot DO (and `log`) is gone. Recover from the cold tier.
    let recovered = recover_from_cold(&cold).expect("recovery from cold tier succeeds");

    // REPLAY-IDENTICAL to the pre-loss state.
    assert_eq!(recovered.source, RecoverySource::ColdTier);
    assert_eq!(
        recovered.state, pre_loss,
        "recovered state equals pre-loss state"
    );
    assert_eq!(
        recovered.state.canonical_bytes(),
        pre_loss.canonical_bytes(),
        "recovery is replay-identical (byte-for-byte)"
    );
    assert_eq!(
        recovered.records.len() as u64,
        n,
        "full chain reconstructed"
    );
    verify_chain(&recovered.records).expect("recovered chain verifies");

    // Recovery from a partial cold tier (hot bound > 0) + the recombined hot is
    // also identical — proves cold-tier recovery composes with a live hot tail.
    let mut cold2 = InMemoryColdStore::new();
    let rep2 = compact(&log, 500, &mut cold2).expect("partial seal");
    // The hot remainder survives; recovery of the cold portion + re-seeding the
    // hot tail reconstructs the full chain.
    let recombined = recombine(&cold2, rep2.hot.records());
    let after = replay_unchecked(&recombined).unwrap();
    assert_eq!(after.canonical_bytes(), pre_loss.canonical_bytes());

    // MIRROR as SECONDARY source: cold tier missing a prefix, mirror fills it.
    // Seal only the tail to cold (drop the head range), supply the head via a
    // mirror; recovery (cold first, mirror second) must still be replay-identical.
    let head_seq = 1_000u64;
    let mut cold3 = InMemoryColdStore::new();
    {
        use hugit_refstore::coldtier::{ColdRange, ColdStore};
        // Cold tier holds only [head_seq, n).
        let tail: Vec<EventRecord> = log
            .records()
            .iter()
            .filter(|r| r.seq >= head_seq)
            .cloned()
            .collect();
        cold3.put(ColdRange::from_records(tail));
    }
    // The mirror holds the missing head [0, head_seq).
    struct HeadMirror {
        head: Vec<EventRecord>,
    }
    impl MirrorSource for HeadMirror {
        fn records_in(&self, start: u64, end: u64) -> Vec<EventRecord> {
            self.head
                .iter()
                .filter(|r| r.seq >= start && r.seq < end)
                .cloned()
                .collect()
        }
    }
    let mirror = HeadMirror {
        head: log
            .records()
            .iter()
            .filter(|r| r.seq < head_seq)
            .cloned()
            .collect(),
    };
    let recovered_m = recover_with_mirror(&cold3, &mirror).expect("recovery with mirror fallback");
    assert_eq!(
        recovered_m.source,
        RecoverySource::ColdTierWithMirror,
        "mirror is the secondary source for the cold-tier gap"
    );
    assert_eq!(
        recovered_m.state.canonical_bytes(),
        pre_loss.canonical_bytes(),
        "cold+mirror recovery is replay-identical"
    );

    // Cold-tier-FIRST precedence + fail-closed on an unfillable gap.
    let recovered_no_mirror = recover_with_mirror(&cold3, &NoMirror);
    assert!(
        recovered_no_mirror.is_err(),
        "without the mirror, the missing head makes recovery fail-closed"
    );

    // A trivial sanity that RefState type is what we think.
    let _: &RefState = &recovered.state;
}
