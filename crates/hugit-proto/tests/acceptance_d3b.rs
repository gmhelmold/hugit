//! WP-D3b acceptance oracle — push concurrency, total order, external-change,
//! flag, and negatives. The dedicated D3 **write-path red-team** rides here: the
//! write path is the source-of-truth bar.
//!
//! Owned items (VERBATIM from decomposition v2.0 — D3b):
//!   ② concurrent pushes: total order, correct stale rejection
//!   ③ raw push = external-change w/ attribution
//!   ④ flag off unless self-hosted-alpha
//!   ⑤(+) negative: NO synthetic intent fabricated for a raw push (intent log clean)
//!
//! Driven by `tests/acceptance/wp-d3b/run.sh`.
//!
//! Strategy: exercise the in-crate write path (`write/{order,external,flag}`)
//! against the D1 single-writer append point and the D4 intent surface (consumed
//! read-only). Concurrency proofs use **real thread overlap** held at a barrier —
//! never instant serial calls — so the total order is proven under genuine
//! contention. The negative (⑤) asserts the *absence* of any fabricated intent.

use std::collections::BTreeSet;
use std::sync::Arc;

use hugit_proto::write::receive::{
    ReceiveError, ReceiveRequest, RecvLimits, RefUpdate as RecvRefUpdate, SerializedReceiver,
};
use hugit_proto::write::store::InMemoryCas;
use hugit_proto::{
    Attribution, ExternalChangeError, FlagGate, PushOutcome, RawPush, RefUpdate, SerializedWriter,
    WritePathDisabled, is_external_change_kind, record_external_change,
};

use hugit_refstore::EventLog;
use hugit_refstore::intent::{INTENT_LANDED_KIND, RAW_PUSH_KINDS, intents_from_log};

#[path = "push_concurrency_negatives/mod.rs"]
mod fixtures;
use fixtures::{
    advance, create, drive_overlapping, drive_receivers, oid, real_advance_pack, real_pack,
};

// ─── ② concurrent pushes: total order + correct stale rejection ──────────────

/// Concurrent pushes serialized through the D1 single-writer point get a strict
/// **total order**, and a push against a **stale tip** is correctly rejected
/// (compare-and-append), with no lost update and no false accept. The pushes are
/// driven from real overlapping threads (barrier-released), so the order holds
/// under genuine contention, not by luck of instant serial calls.
#[test]
fn item_2_concurrent_pushes_total_order_stale_rejection() {
    // --- 2a. total order: N concurrent creates of distinct refs all land at
    //         distinct, contiguous, monotonically increasing seqs. ---
    let writer = Arc::new(SerializedWriter::new());
    let n = 8usize;
    let updates: Vec<RefUpdate> = (0..n)
        .map(|i| {
            create(
                &format!("refs/heads/b{i}"),
                &oid(&format!("c{i}")),
                &format!("agent-{i}"),
                1_000 + i as u64,
            )
        })
        .collect();

    let outcomes = drive_overlapping(Arc::clone(&writer), updates);

    // Every concurrent create landed (distinct refs ⇒ none stale).
    let seqs: Vec<u64> = outcomes
        .iter()
        .map(|o| o.landed_seq().expect("distinct-ref create must land"))
        .collect();
    let unique: BTreeSet<u64> = seqs.iter().copied().collect();
    assert_eq!(unique.len(), n, "every push got a DISTINCT total-order seq");
    // The total order is exactly the contiguous slot range 0..n — no gap, no
    // collision, no lost update under real overlap.
    assert_eq!(
        unique,
        (0..n as u64).collect::<BTreeSet<u64>>(),
        "concurrent pushes occupy a contiguous, gap-free total order"
    );
    assert_eq!(writer.len(), n, "log holds exactly the N landed pushes");
    // The single-writer log is itself strictly monotonic in seq (the spine of the
    // total order).
    let log = writer.snapshot();
    for (i, rec) in log.records().iter().enumerate() {
        assert_eq!(rec.seq, i as u64, "record {i} out of total order");
    }

    // --- 2b. stale rejection: two pushes RACE the SAME ref from the same base;
    //         exactly one wins, the other is correctly rejected as stale. ---
    let writer = Arc::new(SerializedWriter::new());
    let base = oid("base");
    // Seed the contended ref at `base` (a landed create).
    match writer.push(create("refs/heads/main", &base, "seed", 1)) {
        PushOutcome::Landed { .. } => {}
        PushOutcome::Stale(s) => panic!("seed create must land, got {s}"),
    }

    // Two agents both believe main is at `base` and try to advance it — a real
    // race on the single ref, driven from overlapping threads.
    let racers = vec![
        advance("refs/heads/main", &base, &oid("winA"), "agentA", 10),
        advance("refs/heads/main", &base, &oid("winB"), "agentB", 11),
    ];
    let outcomes = drive_overlapping(Arc::clone(&writer), racers);

    let landed: Vec<&PushOutcome> = outcomes
        .iter()
        .filter(|o| matches!(o, PushOutcome::Landed { .. }))
        .collect();
    let stale: Vec<&PushOutcome> = outcomes
        .iter()
        .filter(|o| matches!(o, PushOutcome::Stale(_)))
        .collect();

    // Exactly one winner, exactly one correct stale rejection — no false accept,
    // no lost update.
    assert_eq!(landed.len(), 1, "exactly one racer wins the contended ref");
    assert_eq!(stale.len(), 1, "the loser is correctly rejected as STALE");

    // The stale rejection names the value that actually won (the loser saw the
    // winner's tip, not its own expectation).
    if let PushOutcome::Stale(s) = stale[0] {
        let winner_tip = writer
            .ref_view()
            .get("refs/heads/main")
            .unwrap()
            .to_string();
        assert_eq!(
            s.actual.as_deref(),
            Some(winner_tip.as_str()),
            "stale rejection reports the winning tip"
        );
        assert_eq!(
            s.expected.as_deref(),
            Some(base.as_str()),
            "stale rejection reports the (now-stale) expected tip"
        );
        assert_ne!(s.expected, s.actual, "rejection is because the tip moved");
    }

    // The winner is durable: the ref shows exactly the winner's target, and the
    // log grew by exactly one (the loser appended NOTHING — no lost update).
    assert_eq!(
        writer.len(),
        2,
        "seed + one winner; the stale push appended nothing"
    );
    let tip = writer
        .ref_view()
        .get("refs/heads/main")
        .unwrap()
        .to_string();
    assert!(
        tip == oid("winA") || tip == oid("winB"),
        "the surviving tip is one of the two racers' targets"
    );
}

// ─── ② DEFECT-2/3: total order + stale rejection on the REAL ingest path ──────

/// The previous test proves the order *module*. This one proves the REAL ingest
/// (`receive_pack` via the single-writer `SerializedReceiver`) routes through
/// compare-and-append: concurrent REAL pushes (genuine packs, overlapping
/// threads) get a strict contiguous total order, and two racers advancing the
/// SAME ref from the same expected tip yield exactly one winner + one stale
/// rejection — no lost update. Before the fix `receive_pack` ignored the expected
/// tip entirely, so both racers would have "succeeded" (lost-update); that makes
/// this RED on main.
#[test]
fn item_2_real_ingest_concurrent_total_order_stale_rejection() {
    let gate = FlagGate::self_hosted_alpha();

    // --- total order: N concurrent CREATES of distinct refs all land at
    //     distinct, contiguous, monotonically increasing log seqs. ---
    let n = 5usize;
    let receiver = Arc::new(SerializedReceiver::new(
        InMemoryCas::new(),
        gate,
        RecvLimits::default(),
    ));
    let packs: Vec<_> = (0..n).map(|i| real_pack(&format!("c{i}"))).collect();
    let requests: Vec<ReceiveRequest> = packs
        .iter()
        .enumerate()
        .map(|(i, rp)| ReceiveRequest {
            pack: rp.pack.clone(),
            update: RecvRefUpdate {
                ref_name: format!("refs/heads/b{i}"),
                expected: None,
                new_oid: rp.head_oid.clone(),
            },
            principal_chain: vec![format!("agent-{i}")],
            recorded_at: 1_000 + i as u64,
        })
        .collect();

    let results = drive_receivers(Arc::clone(&receiver), requests);
    // Every distinct-ref create landed.
    let seqs: BTreeSet<u64> = results
        .iter()
        .map(|r| r.as_ref().expect("distinct-ref create must land").event.seq)
        .collect();
    assert_eq!(
        seqs.len(),
        n,
        "every concurrent push got a DISTINCT total-order seq"
    );
    assert_eq!(
        seqs,
        (0..n as u64).collect::<BTreeSet<u64>>(),
        "concurrent real pushes occupy a contiguous, gap-free total order"
    );
    assert_eq!(
        receiver.log_len(),
        n,
        "log holds exactly the N landed pushes"
    );
    // Spine is strictly monotonic in seq.
    let log = receiver.snapshot_log();
    for (i, rec) in log.records().iter().enumerate() {
        assert_eq!(rec.seq, i as u64, "record {i} out of total order");
    }

    // --- stale rejection: two pushes RACE the SAME ref from the same base. ---
    let receiver = Arc::new(SerializedReceiver::new(
        InMemoryCas::new(),
        gate,
        RecvLimits::default(),
    ));
    // Seed `main` at a base commit.
    let base = real_pack("base");
    let seed = receiver
        .receive(&ReceiveRequest {
            pack: base.pack.clone(),
            update: RecvRefUpdate {
                ref_name: "refs/heads/main".to_string(),
                expected: None,
                new_oid: base.head_oid.clone(),
            },
            principal_chain: vec!["seed".to_string()],
            recorded_at: 1,
        })
        .expect("seed create lands");
    assert_eq!(seed.event.seq, 0);

    // Two agents both believe main is at `base` and advance it — a real race.
    let adv_a = real_advance_pack("base", "winA");
    let adv_b = real_advance_pack("base", "winB");
    let racers = vec![
        ReceiveRequest {
            pack: adv_a.pack.clone(),
            update: RecvRefUpdate {
                ref_name: "refs/heads/main".to_string(),
                expected: Some(base.head_oid.clone()),
                new_oid: adv_a.head_oid.clone(),
            },
            principal_chain: vec!["agentA".to_string()],
            recorded_at: 10,
        },
        ReceiveRequest {
            pack: adv_b.pack.clone(),
            update: RecvRefUpdate {
                ref_name: "refs/heads/main".to_string(),
                expected: Some(base.head_oid.clone()),
                new_oid: adv_b.head_oid.clone(),
            },
            principal_chain: vec!["agentB".to_string()],
            recorded_at: 11,
        },
    ];
    let results = drive_receivers(Arc::clone(&receiver), racers);
    let landed = results.iter().filter(|r| r.is_ok()).count();
    let stale = results
        .iter()
        .filter(|r| matches!(r, Err(ReceiveError::StaleRef { .. })))
        .count();
    assert_eq!(landed, 1, "exactly one racer wins the contended ref");
    assert_eq!(
        stale, 1,
        "the loser is correctly rejected as STALE (no lost update)"
    );

    // The winner is durable; the log grew by exactly one (loser appended nothing).
    assert_eq!(
        receiver.log_len(),
        2,
        "seed + one winner; the stale push appended NOTHING"
    );
    let tip = receiver
        .ref_view()
        .get("refs/heads/main")
        .unwrap()
        .to_string();
    assert!(
        tip == adv_a.head_oid || tip == adv_b.head_oid,
        "the surviving tip is one of the two racers' targets"
    );
    assert_ne!(
        tip, base.head_oid,
        "the ref genuinely advanced off the base"
    );
}

// ─── ③ raw push = external-change WITH attribution ───────────────────────────

/// A raw `git push` is recorded as an OPAQUE external-change event carrying its
/// attribution (who / when / which ref). It is one of the external-change kinds
/// D1/D4 recognise — never an intent kind — and an unattributed push is refused.
#[test]
fn item_3_raw_push_external_change_with_attribution() {
    let mut log = EventLog::new();

    let push = RawPush::Update {
        ref_name: "refs/heads/main".to_string(),
        target: oid("deadbeef"),
    };
    let who = vec!["pat:alice".to_string(), "session:42".to_string()];
    let when = 1_717_000_123_456u64;

    let (record, attribution): (_, Attribution) =
        record_external_change(&mut log, &push, who.clone(), when)
            .expect("attributed raw push records");

    // Attribution: WHO (full principal chain, in order), WHEN, WHICH ref.
    assert!(
        attribution.is_attributed(),
        "external change carries a pusher"
    );
    assert_eq!(
        attribution.principal_chain, who,
        "who: principal chain preserved in order"
    );
    assert_eq!(
        attribution.recorded_at, when,
        "when: recorded timestamp preserved"
    );
    assert_eq!(
        attribution.ref_name, "refs/heads/main",
        "which ref: preserved"
    );

    // The recorded event is an EXTERNAL-CHANGE kind (opaque), never an intent.
    assert!(
        is_external_change_kind(&record.kind),
        "raw push records an external-change kind, got {:?}",
        record.kind
    );
    assert!(
        RAW_PUSH_KINDS.contains(&record.kind.as_str()),
        "the kind is exactly a D1/D4 raw-push kind"
    );
    assert_ne!(
        record.kind, INTENT_LANDED_KIND,
        "an external change is NEVER an intent"
    );

    // The same attribution is on the appended log record itself (not just the
    // returned struct) — the event log is the source of truth.
    assert_eq!(record.principal_chain, who);
    assert_eq!(record.recorded_at, when);

    // Fail-closed: an UNATTRIBUTED raw push is refused, never recorded blind.
    let before = log.len();
    let unattributed = record_external_change(&mut log, &push, vec![], when);
    assert_eq!(
        unattributed,
        Err(ExternalChangeError::MissingAttribution),
        "an unattributed external change is refused"
    );
    assert_eq!(log.len(), before, "the refused push appended NOTHING");
}

// ─── ④ flag off unless self-hosted-alpha ─────────────────────────────────────

/// The write path is OFF by default and admits a write ONLY under the
/// self-hosted-alpha flag. Flag off ⇒ the write path is refused (fail-closed).
#[test]
fn item_4_flag_off_unless_self_hosted_alpha() {
    // Default / off: write path disabled, admission refused.
    let off = FlagGate::default();
    assert!(!off.write_path_enabled(), "write path is OFF by default");
    assert_eq!(
        off.admit_write(),
        Err(WritePathDisabled),
        "flag off ⇒ write path refused (fail-closed)"
    );
    assert_eq!(FlagGate::new(), off, "new() is the off-by-default gate");

    // Only the self-hosted-alpha flag enables the write path.
    let on = FlagGate::self_hosted_alpha();
    assert!(
        on.write_path_enabled(),
        "self-hosted-alpha enables the write path"
    );
    assert_eq!(on.admit_write(), Ok(()), "flag on ⇒ write path admitted");

    // The gate is the single admission point: off and on are distinct states and
    // nothing other than the flag flips it.
    assert_ne!(off, on, "off and on are distinct gate states");
}

// ─── ⑤(+) NEGATIVE: no synthetic intent fabricated for a raw push ────────────

/// After any sequence of raw pushes (including a concurrent race and a delete),
/// the intent log is provably CLEAN: not one synthetic intent was fabricated.
/// This is the D3 leg of the on-record no-fake-intents adjudication. Also a
/// red-team check: an attacker-supplied payload that *looks* like an intent
/// cannot smuggle an intent in through the raw-push path.
#[test]
fn item_5_no_synthetic_intent_for_raw_push() {
    let mut log = EventLog::new();

    // A normal raw push.
    record_external_change(
        &mut log,
        &RawPush::Update {
            ref_name: "refs/heads/main".to_string(),
            target: oid("aa"),
        },
        vec!["pat:bob".to_string()],
        1,
    )
    .unwrap();

    // A raw push whose payload TRIES to look like an intent (attribution/charter
    // smuggling). The recorder must still emit an external-change kind — the kind
    // is structurally fixed by the push variant, not by caller-controlled bytes.
    record_external_change(
        &mut log,
        &RawPush::Update {
            ref_name: r#"refs/heads/x","intent_id":"forged","charter":"pwn"#.to_string(),
            target: oid("bb"),
        },
        vec!["pat:mallory".to_string()],
        2,
    )
    .unwrap();

    // A raw delete is also external, also never an intent.
    record_external_change(
        &mut log,
        &RawPush::Delete {
            ref_name: "refs/heads/main".to_string(),
        },
        vec!["pat:bob".to_string()],
        3,
    )
    .unwrap();

    // The write path emitted exactly three events, and NONE is an intent kind.
    assert_eq!(log.len(), 3, "three raw pushes recorded");
    for rec in log.records() {
        assert_ne!(
            rec.kind, INTENT_LANDED_KIND,
            "no raw push ever emits an intent.landed event"
        );
        assert!(
            is_external_change_kind(&rec.kind),
            "every raw-push event is an external-change kind, got {:?}",
            rec.kind
        );
    }

    // The intent altitude (D4's derived view, consumed read-only) is CLEAN: the
    // write path fabricated zero intents for these raw pushes.
    let intents = intents_from_log(&log).expect("intent projection over a clean log");
    assert!(
        intents.is_empty(),
        "INTENT LOG CLEAN: no synthetic intent fabricated for any raw push (got {})",
        intents.len()
    );
    // Specifically, the forged "intent_id" the attacker stuffed into the payload
    // did NOT materialise as an intent.
    assert!(
        intents.by_id("forged").is_none(),
        "a forged intent_id in a raw-push payload is NOT projected as an intent"
    );
}
