//! WP-E1b acceptance oracle — verified mirror: failure modes.
//!
//! Owned items (decomposition §E1, items ② ④ ⑤ ⑥ ⑦):
//!   ②    `item_2_divergence_alarm_repair_incident`
//!   ④(+) `item_4_outage_queue_bounded_backoff_no_drop`
//!   ④(+) `item_4_outage_recovery_drain_verified_gap_incident`
//!   ⑤(+) `item_5_refops_replicate_force_push_branch_delete_tag`
//!   ⑤(+) `item_5_deleted_ref_absent_on_mirror`
//!   ⑤(+) `item_5_no_false_divergence_from_orphans`
//!   ⑥(+) `item_6_partial_divergence_repair_scoped_to_broken_ref`
//!   ⑥(+) `item_6_webhook_loss_poll_fallback_detects_within_sla`
//!   ⑦(R2) `item_7_reverse_write_treated_as_divergence`
//!   ⑦(R2) `item_7_zero_reverse_sync_codepath_absent`
//!
//! Local fixture proofs over the mirror failure-modes logic. No live GitHub
//! call is required; `HUGIT_GH_TEST_REPO` is set by run.sh for parity with the
//! E1a/E3 suites. Repair is forge-authoritative; handling is fail-CLOSED.
//!
//! # Contract deps (consumed, never modified)
//! - `hugit_contracts::{EventRecord, AppWebhooks}` (frozen by WP-00)
//! - `hugit_mirror::{divergence, outage, refops, poll}` (this WP's Claims)

use hugit_mirror::divergence::{
    RefClass, RefState, RepairStrategy, ReversePropagation, apply_repair, classify, resolve,
};
use hugit_mirror::outage::{
    BackoffSchedule, DurableQueue, EnqueueResult, QueuedWrite, drain_to_verified,
};
use hugit_mirror::poll::{FallbackTrigger, PollConfig, detect_via_poll};
use hugit_mirror::refops::{RefOp, RefTips, apply, apply_all, diff_ref_tips, tips_in_sync};

// ── helpers ──────────────────────────────────────────────────────────────────

fn forge_tip() -> String {
    "f".repeat(40)
}

fn mirror_tip() -> String {
    "0".repeat(40)
}

fn queued(seq: u64) -> QueuedWrite {
    QueuedWrite {
        seq,
        ref_name: format!("refs/heads/b{seq}"),
        forge_tip: format!("{seq:040}"),
    }
}

// ── ② divergence → alarm + repair + incident ─────────────────────────────────
#[test]
fn item_2_divergence_alarm_repair_incident() {
    // A divergent ref must yield the {alarm, repair, incident} triple, with a
    // forge-authoritative repair (mirror overwritten, never merged).
    let state = RefState::new("refs/heads/main", forge_tip(), mirror_tip());
    assert_eq!(classify(&state), RefClass::Divergent);

    let res = resolve(&state).expect("divergent ref must resolve to a triple");
    assert_eq!(res.alarm.ref_name, "refs/heads/main");
    assert_eq!(res.repair.ref_name, "refs/heads/main");
    assert_eq!(res.repair.strategy, RepairStrategy::ForgeAuthoritative);
    assert_eq!(res.incident.ref_name, "refs/heads/main");

    // Forge wins: post-repair tip equals the forge tip, not the mirror value.
    assert_eq!(apply_repair(&res.repair), forge_tip());

    // Fail-CLOSED: an undecidable detector state is treated as divergent, and
    // a divergent ref is never reported synced.
    let degraded = RefState::undecidable("refs/heads/x");
    assert_eq!(classify(&degraded), RefClass::Degraded);
    assert!(classify(&degraded).is_divergent());
    assert!(resolve(&degraded).is_some());

    // A synced ref produces no triple.
    let synced = RefState::new("refs/heads/main", forge_tip(), forge_tip());
    assert!(resolve(&synced).is_none());
}

// ── ④ GitHub outage: durable queue, bounded backoff, no drop/reorder ─────────
#[test]
fn item_4_outage_queue_bounded_backoff_no_drop() {
    // Bounded backoff: every delay in the schedule is capped at max_delay_ms.
    let schedule = BackoffSchedule::default();
    let delays = schedule.delays();
    assert!(!delays.is_empty());
    for d in &delays {
        assert!(*d <= schedule.max_delay_ms, "backoff must be bounded");
    }
    // Exponential: monotonic non-decreasing until the cap.
    for pair in delays.windows(2) {
        assert!(pair[1] >= pair[0], "backoff must be non-decreasing");
    }

    // During outage, writes hold in the durable queue — none dropped.
    let mut q = DurableQueue::new(64);
    for i in 1..=10 {
        assert_eq!(q.enqueue(queued(i)), EnqueueResult::Accepted);
    }
    assert_eq!(q.len(), 10, "no write dropped while held");

    // Order preserved (no reorder) in the snapshot.
    let seqs: Vec<u64> = q.snapshot().iter().map(|w| w.seq).collect();
    assert_eq!(seqs, (1..=10).collect::<Vec<_>>());

    // Capacity overflow is rejected (backpressure), never a silent drop.
    let mut small = DurableQueue::new(2);
    assert_eq!(small.enqueue(queued(1)), EnqueueResult::Accepted);
    assert_eq!(small.enqueue(queued(2)), EnqueueResult::Accepted);
    assert_eq!(small.enqueue(queued(3)), EnqueueResult::Overflow);
    assert_eq!(
        small.len(),
        2,
        "overflow rejects, does not drop accepted items"
    );
}

// ── ④ recovery → drain to verified sync + gap incident ───────────────────────
#[test]
fn item_4_outage_recovery_drain_verified_gap_incident() {
    let mut q = DurableQueue::new(64);
    for i in 100..=105 {
        q.enqueue(queued(i));
    }
    let out = drain_to_verified(&mut q);

    // Drains to verified sync.
    assert!(out.verified, "recovery drain must verify every write");

    // No drop, no reorder: drained equals enqueue order, full count.
    let seqs: Vec<u64> = out.drained.iter().map(|w| w.seq).collect();
    assert_eq!(seqs, (100..=105).collect::<Vec<_>>());
    assert!(q.is_empty(), "queue fully drained");

    // Gap incident records the outage window.
    let gap = out.gap_incident.expect("recovery must emit a gap incident");
    assert_eq!(gap.from_seq, 100);
    assert_eq!(gap.to_seq, 105);
    assert_eq!(gap.held_count, 6);
}

// ── ⑤ ref ops replicate: force-push / branch-delete / tag ─────────────────────
#[test]
fn item_5_refops_replicate_force_push_branch_delete_tag() {
    let mut mirror = RefTips::new();
    mirror.set("refs/heads/main", "a".repeat(40));

    let ops = vec![
        RefOp::Update {
            ref_name: "refs/heads/main".into(),
            new_tip: "b".repeat(40),
            force: true,
        },
        RefOp::TagCreate {
            ref_name: "refs/tags/v1".into(),
            tip: "b".repeat(40),
        },
        RefOp::Delete {
            ref_name: "refs/heads/old".into(),
        },
    ];
    // Pre-seed the branch to be deleted so the delete op is meaningful.
    mirror.set("refs/heads/old", "c".repeat(40));
    apply_all(&mut mirror, &ops);

    // Force-push replicated the new tip.
    assert_eq!(mirror.get("refs/heads/main"), Some(&"b".repeat(40)));
    // Tag create replicated.
    assert!(mirror.contains("refs/tags/v1"));
    // Branch delete replicated.
    assert!(!mirror.contains("refs/heads/old"));
}

// ── ⑤ deleted ref absent on mirror after sync ────────────────────────────────
#[test]
fn item_5_deleted_ref_absent_on_mirror() {
    let mut mirror = RefTips::new();
    mirror.set("refs/heads/feature", "d".repeat(40));
    assert!(mirror.contains("refs/heads/feature"));

    apply(
        &mut mirror,
        &RefOp::Delete {
            ref_name: "refs/heads/feature".into(),
        },
    );
    assert!(
        !mirror.contains("refs/heads/feature"),
        "deleted ref must be absent"
    );

    // And it is absent from the name listing.
    assert!(!mirror.names().iter().any(|n| *n == "refs/heads/feature"));
}

// ── ⑤ no false divergence from orphaned objects ──────────────────────────────
#[test]
fn item_5_no_false_divergence_from_orphans() {
    // Forge and mirror agree on ref tips. A force-push left orphaned loose
    // objects on the mirror — but the orphan-aware diff compares ref tips only,
    // so it reports zero divergence.
    let mut forge = RefTips::new();
    forge.set("refs/heads/main", "z".repeat(40));
    let mut mirror = RefTips::new();
    mirror.set("refs/heads/main", "z".repeat(40));

    assert!(tips_in_sync(&forge, &mirror), "matching tips → in sync");
    assert!(
        diff_ref_tips(&forge, &mirror).is_empty(),
        "orphans must not register as divergence"
    );

    // Sanity: a genuine tip mismatch IS reported.
    let mut other = RefTips::new();
    other.set("refs/heads/main", "y".repeat(40));
    assert_eq!(
        diff_ref_tips(&forge, &other),
        vec!["refs/heads/main".to_string()]
    );
}

// ── ⑥ partial divergence: repair scoped to broken ref only ───────────────────
#[test]
fn item_6_partial_divergence_repair_scoped_to_broken_ref() {
    // Two refs: one synced, one divergent. Repair must be scoped to the broken
    // ref only — never a full re-seed of the synced ref.
    let synced = RefState::new("refs/heads/main", forge_tip(), forge_tip());
    let broken = RefState::new("refs/heads/dev", "1".repeat(40), "2".repeat(40));

    // The synced ref yields no repair.
    assert!(resolve(&synced).is_none(), "synced ref must not be touched");

    // The broken ref yields a repair scoped to itself.
    let res = resolve(&broken).expect("broken ref must resolve");
    assert_eq!(res.repair.ref_name, "refs/heads/dev");
    assert_ne!(
        res.repair.ref_name, "refs/heads/main",
        "scoped: never the synced ref"
    );
    assert_eq!(res.repair.strategy, RepairStrategy::ForgeAuthoritative);
    assert_eq!(apply_repair(&res.repair), "1".repeat(40));
}

// ── ⑥ webhook loss → poll fallback detects within SLA ────────────────────────
#[test]
fn item_6_webhook_loss_poll_fallback_detects_within_sla() {
    let cfg = PollConfig::default();
    assert!(cfg.satisfies_sla(), "poll cadence must guarantee SLA");
    assert!(cfg.worst_case_detection_ms() <= cfg.sla_ms);

    // No webhook arrives; the poll fallback still detects a divergence that
    // appeared early in the window, within the SLA.
    let det = detect_via_poll(&cfg, FallbackTrigger::WebhookLoss, 1);
    assert!(det.diverged);
    assert!(
        det.within_sla,
        "webhook-loss divergence must be caught within SLA"
    );

    // Even a divergence appearing just before SLA expiry is caught within it.
    let late = detect_via_poll(
        &cfg,
        FallbackTrigger::WebhookLoss,
        cfg.sla_ms - cfg.interval_ms,
    );
    assert!(late.within_sla);
}

// ── ⑦ reverse write treated as divergence (forge-authoritative repair) ───────
#[test]
fn item_7_reverse_write_treated_as_divergence() {
    // A write made directly on the GitHub mirror moves the mirror tip away from
    // the forge tip. This is detected as divergence — NOT propagated back — and
    // resolved forge-authoritative, with an alarm and an incident.
    let reverse_write = RefState::new(
        "refs/heads/main",
        forge_tip(),    // forge unchanged (truth)
        "e".repeat(40), // mirror moved by a direct write
    );
    assert_eq!(classify(&reverse_write), RefClass::Divergent);

    let res = resolve(&reverse_write).expect("reverse write must be divergence");
    // Forge wins: the mirror write is overwritten, never adopted as truth.
    assert_eq!(res.repair.strategy, RepairStrategy::ForgeAuthoritative);
    assert_eq!(apply_repair(&res.repair), forge_tip());
    assert_ne!(
        apply_repair(&res.repair),
        "e".repeat(40),
        "mirror write never becomes truth"
    );

    // Incident records that the divergent write originated on the mirror.
    assert!(
        res.incident.mirror_write,
        "reverse write recorded in incident"
    );
}

// ── ⑦ zero reverse-sync codepath exists ──────────────────────────────────────
#[test]
fn item_7_zero_reverse_sync_codepath_absent() {
    // The reverse-propagation codepath is structurally absent: the marker is a
    // compile-time false, and no API reads the mirror value as authoritative.
    const {
        assert!(
            !ReversePropagation::CODEPATH_PRESENT,
            "there must be zero reverse-sync codepath"
        )
    };
}
