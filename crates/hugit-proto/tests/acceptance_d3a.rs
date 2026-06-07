//! WP-D3a acceptance oracle — git push WRITE path (receive-pack → CAS + log).
//!
//! Owned item (VERBATIM from decomposition v2.0 — D3a):
//!   ① push→clone round-trip identical
//!
//! Red-team items (D3a source-of-truth bar, owned by D3a):
//!   redteam_malformed_pack_rejected
//!   redteam_oversized_pack_rejected
//!   redteam_ref_update_tamper_rejected
//!   redteam_unreachable_target_rejected   (target present in odb but not
//!     reachable from the pushed pack — present ≠ delivered)
//!   redteam_decompression_bomb_rejected   (tiny compressed pack inflates huge)
//!
//! Plus the structural no-fake-intent proof (the D3⑤ leg that lives in D3a's
//! source-of-truth bar): a raw push records a `ref.update` external-change and
//! the intent altitude stays empty — no synthetic intent is fabricated.
//!
//! Every ingest is admitted through the **flag gate** (the write path is OFF
//! unless `self-hosted-alpha`) and applies the ref move as **compare-and-append**
//! (the pusher declares the tip it expects; a stale expectation is rejected with
//! no lost update).
//!
//! Driven by `tests/acceptance/wp-d3a/run.sh`. The push side is built with the
//! system git binary (the libgit2-class pack engine the write path wires to);
//! ingest runs through `hugit_proto::write` and the clone-back is materialized
//! from CAS and cloned with system git, then compared byte-for-byte.

#[path = "push_core/mod.rs"]
mod push_core;

use hugit_proto::write::flag::FlagGate;
use hugit_proto::write::receive::{
    DEFAULT_MAX_PACK_BYTES, ReceiveError, ReceiveRequest, RecvLimits, RefUpdate,
    materialize_bare_repo, receive_pack,
};
use hugit_proto::write::store::{InMemoryCas, REF_UPDATE_KIND};
use hugit_refstore::intent::{INTENT_LANDED_KIND, intents_from_log};
use hugit_refstore::log::EventLog;
use push_core::Fixture;

/// The gate every push in D3a runs under: the self-hosted-alpha flag IS set, so
/// the write path is reachable. (The flag-off refusal is its own oracle below.)
fn enabled_gate() -> FlagGate {
    FlagGate::self_hosted_alpha()
}

/// ① push→clone round-trip identical.
///
/// Build a real repo, pack its objects, ingest the push through the write path
/// (objects → CAS, ref move → D1 event), materialize the bare repo back from
/// CAS, clone it with git, and assert the cloned commit is byte-identical to the
/// pushed one — git is never broken; the write path produces a valid repo.
#[test]
fn item_1_push_clone_roundtrip_identical() {
    let fx = Fixture::build_repo(&[
        ("README.md", "hello hugit\n"),
        ("src.txt", "fn main() {}\n"),
    ]);

    let mut cas = InMemoryCas::new();
    let mut log = EventLog::new();

    let req = ReceiveRequest {
        pack: fx.pack.clone(),
        update: RefUpdate {
            ref_name: fx.ref_name.clone(),
            // create: the ref is currently absent (compare-and-append create).
            expected: None,
            new_oid: fx.head_oid.clone(),
        },
        principal_chain: vec!["user:gustavo".into()],
        recorded_at: 1_717_000_000_000,
    };

    let receipt = receive_pack(
        &enabled_gate(),
        &req,
        &mut cas,
        &mut log,
        RecvLimits::default(),
    )
    .expect("ingest succeeds");

    // every pushed object landed in CAS.
    assert!(!cas.is_empty(), "CAS received objects");
    assert!(
        receipt.stored_oids.contains(&fx.head_oid),
        "head commit stored to CAS"
    );

    // the ref move is one append-only raw-push event (no provenance).
    assert_eq!(log.len(), 1, "exactly one event appended for the push");
    assert_eq!(receipt.event.kind, REF_UPDATE_KIND);

    // materialize back from CAS and clone with system git.
    let served = materialize_bare_repo(&cas, &receipt.stored_oids, &fx.ref_name, &fx.head_oid)
        .expect("materialize from CAS");
    let cloned_head = push_core::clone_and_head(served.path());

    // byte-identical: same head oid AND same raw commit object bytes.
    assert_eq!(cloned_head.oid, fx.head_oid, "cloned head oid identical");
    assert_eq!(
        cloned_head.commit_bytes, fx.head_commit_bytes,
        "cloned commit object byte-identical to the pushed one"
    );

    // structural no-fake-intent proof (D3⑤ leg): the intent altitude is empty —
    // the raw push was recorded as an external change, not synthesised into an
    // intent. There is no `intent.landed` event on the log.
    let intents = intents_from_log(&log).expect("intent fold");
    assert!(
        intents.is_empty(),
        "no synthetic intent fabricated for a raw push"
    );
    assert!(
        log.records().iter().all(|r| r.kind != INTENT_LANDED_KIND),
        "no intent.landed event exists for a raw push"
    );
}

/// Red-team / DEFECT-1 oracle: with the write-path flag OFF, the REAL ingest
/// (`receive_pack`) REFUSES the push — no object reaches CAS, no event reaches
/// the log. (Before the fix `receive_pack` never consulted the flag at all, so a
/// flag-off push was wrongly accepted; this turns that RED.)
#[test]
fn flag_off_real_ingest_refuses_push() {
    let fx = Fixture::build_repo(&[("f", "x\n")]);
    let mut cas = InMemoryCas::new();
    let mut log = EventLog::new();
    let req = ReceiveRequest {
        pack: fx.pack.clone(),
        update: RefUpdate {
            ref_name: fx.ref_name.clone(),
            expected: None,
            new_oid: fx.head_oid.clone(),
        },
        principal_chain: vec!["user:gustavo".into()],
        recorded_at: 1,
    };

    // Flag OFF (the production default): the real ingest must refuse fail-closed.
    let off = FlagGate::default();
    let err = receive_pack(&off, &req, &mut cas, &mut log, RecvLimits::default())
        .expect_err("flag-off push must be refused by the real ingest");
    assert!(
        matches!(err, ReceiveError::WritePathDisabled),
        "expected WritePathDisabled, got {err:?}"
    );
    // Fail-closed: the gate is step 0 — nothing was unpacked, stored, or logged.
    assert!(cas.is_empty(), "flag off ⇒ NO object committed to CAS");
    assert_eq!(log.len(), 0, "flag off ⇒ NO event appended to the log");

    // Sanity: the SAME push succeeds once the flag is on (the gate is the only
    // difference — proves the refusal is the flag, not a broken pack).
    let receipt = receive_pack(
        &enabled_gate(),
        &req,
        &mut cas,
        &mut log,
        RecvLimits::default(),
    )
    .expect("flag-on push succeeds");
    assert!(!cas.is_empty(), "flag on ⇒ objects committed");
    assert_eq!(log.len(), 1, "flag on ⇒ one event appended");
    assert!(receipt.stored_oids.contains(&fx.head_oid));
}

/// Red-team: a malformed (truncated / corrupt) pack is rejected; nothing is
/// committed to CAS or the log.
#[test]
fn redteam_malformed_pack_rejected() {
    let fx = Fixture::build_repo(&[("f", "x\n")]);
    // truncate the pack mid-stream → corrupt.
    let mut bad = fx.pack.clone();
    bad.truncate(bad.len() / 2);

    let mut cas = InMemoryCas::new();
    let mut log = EventLog::new();
    let req = ReceiveRequest {
        pack: bad,
        update: RefUpdate {
            ref_name: fx.ref_name.clone(),
            expected: None,
            new_oid: fx.head_oid.clone(),
        },
        principal_chain: vec!["user:gustavo".into()],
        recorded_at: 1,
    };

    let err = receive_pack(
        &enabled_gate(),
        &req,
        &mut cas,
        &mut log,
        RecvLimits::default(),
    )
    .expect_err("malformed pack must be rejected");
    assert!(
        matches!(err, ReceiveError::MalformedPack { .. }),
        "expected MalformedPack, got {err:?}"
    );
    // fail-closed: no side effects.
    assert!(cas.is_empty(), "no objects committed on malformed pack");
    assert_eq!(log.len(), 0, "no event appended on malformed pack");
}

/// Red-team: an oversized pack is rejected up-front, before any unpack work.
#[test]
fn redteam_oversized_pack_rejected() {
    let fx = Fixture::build_repo(&[("f", "x\n")]);
    let mut cas = InMemoryCas::new();
    let mut log = EventLog::new();
    let req = ReceiveRequest {
        pack: fx.pack.clone(),
        update: RefUpdate {
            ref_name: fx.ref_name.clone(),
            expected: None,
            new_oid: fx.head_oid.clone(),
        },
        principal_chain: vec!["user:gustavo".into()],
        recorded_at: 1,
    };
    // set the COMPRESSED ceiling below the real pack size; keep the inflated/count
    // ceilings at their defaults so the rejection is unambiguously about size.
    let tiny = RecvLimits {
        max_pack_bytes: fx.pack.len().saturating_sub(1),
        ..RecvLimits::default()
    };
    let err = receive_pack(&enabled_gate(), &req, &mut cas, &mut log, tiny)
        .expect_err("oversized pack must be rejected");
    assert!(
        matches!(err, ReceiveError::OversizedPack { .. }),
        "expected OversizedPack, got {err:?}"
    );
    assert!(cas.is_empty(), "no objects committed on oversized pack");
    assert_eq!(log.len(), 0, "no event appended on oversized pack");
    // sanity: the default ceiling would have accepted this pack.
    assert!(fx.pack.len() <= DEFAULT_MAX_PACK_BYTES);
}

/// Red-team: a ref update pointing at an oid the push never delivered is
/// rejected as tampering — before any object or event is committed.
#[test]
fn redteam_ref_update_tamper_rejected() {
    let fx = Fixture::build_repo(&[("f", "x\n")]);
    let mut cas = InMemoryCas::new();
    let mut log = EventLog::new();
    // valid pack, but the requested ref target is an oid the pack never carries.
    let bogus = "0123456789abcdef0123456789abcdef01234567".to_string();
    assert_ne!(bogus, fx.head_oid);
    let req = ReceiveRequest {
        pack: fx.pack.clone(),
        update: RefUpdate {
            ref_name: fx.ref_name.clone(),
            expected: None,
            new_oid: bogus.clone(),
        },
        principal_chain: vec!["user:gustavo".into()],
        recorded_at: 1,
    };
    let err = receive_pack(
        &enabled_gate(),
        &req,
        &mut cas,
        &mut log,
        RecvLimits::default(),
    )
    .expect_err("tampered ref update must be rejected");
    assert!(
        matches!(err, ReceiveError::RefUpdateTampered { .. }),
        "expected RefUpdateTampered, got {err:?}"
    );
    // fail-closed: rejected before any write.
    assert!(
        cas.is_empty(),
        "no objects committed on tampered ref update"
    );
    assert_eq!(log.len(), 0, "no event appended on tampered ref update");
}

/// Red-team / DEFECT-4 oracle: a pack whose declared ref target IS present in the
/// scratch odb but is NOT reachable from the pushed objects (its closure is
/// incomplete — the blob is missing) must be rejected. A "present anywhere in the
/// odb" check would wrongly accept it; reachability validation rejects it.
#[test]
fn redteam_unreachable_target_rejected() {
    let fx = Fixture::build_incomplete_pack();
    let mut cas = InMemoryCas::new();
    let mut log = EventLog::new();
    let req = ReceiveRequest {
        pack: fx.pack.clone(),
        update: RefUpdate {
            ref_name: fx.ref_name.clone(),
            expected: None,
            // the head commit IS in the pack (present), but its tree's blob is NOT
            // → the target is present-but-not-reachable.
            new_oid: fx.head_oid.clone(),
        },
        principal_chain: vec!["user:gustavo".into()],
        recorded_at: 1,
    };
    let err = receive_pack(
        &enabled_gate(),
        &req,
        &mut cas,
        &mut log,
        RecvLimits::default(),
    )
    .expect_err("unreachable (present-but-incomplete) target must be rejected");
    assert!(
        matches!(err, ReceiveError::UnreachableTarget { .. }),
        "expected UnreachableTarget, got {err:?}"
    );
    // fail-closed: rejected before any write.
    assert!(cas.is_empty(), "no objects committed on unreachable target");
    assert_eq!(log.len(), 0, "no event appended on unreachable target");
}

/// Red-team / DEFECT-5 oracle: a decompression bomb — a pack that is small on the
/// wire (under the compressed ceiling) yet inflates far beyond the inflated
/// ceiling — is rejected fail-closed. An uncapped unpack would inflate it to disk
/// and accept; the inflated cap rejects it before any CAS write.
#[test]
fn redteam_decompression_bomb_rejected() {
    // 8 MiB of zeros: compresses to a few KiB, inflates to 8 MiB.
    let inflated = 8 * 1024 * 1024usize;
    let fx = Fixture::build_bomb_pack(inflated);
    // The compressed pack is tiny — it passes the (default) compressed ceiling.
    assert!(
        fx.pack.len() < 1024 * 1024,
        "the bomb is small on the wire ({} bytes)",
        fx.pack.len()
    );

    let mut cas = InMemoryCas::new();
    let mut log = EventLog::new();
    let req = ReceiveRequest {
        pack: fx.pack.clone(),
        update: RefUpdate {
            ref_name: fx.ref_name.clone(),
            expected: None,
            new_oid: fx.head_oid.clone(),
        },
        principal_chain: vec!["user:gustavo".into()],
        recorded_at: 1,
    };
    // Compressed ceiling is generous (the bomb fits); the INFLATED ceiling (1 MiB)
    // is what catches the 8 MiB expansion.
    let limits = RecvLimits {
        max_pack_bytes: DEFAULT_MAX_PACK_BYTES,
        max_inflated_bytes: 1024 * 1024,
        ..RecvLimits::default()
    };
    let err = receive_pack(&enabled_gate(), &req, &mut cas, &mut log, limits)
        .expect_err("decompression bomb must be rejected");
    assert!(
        matches!(err, ReceiveError::DecompressionBomb { .. }),
        "expected DecompressionBomb, got {err:?}"
    );
    // fail-closed: the bomb committed nothing.
    assert!(
        cas.is_empty(),
        "no objects committed on a decompression bomb"
    );
    assert_eq!(log.len(), 0, "no event appended on a decompression bomb");
}
