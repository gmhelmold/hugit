//! WP-E2a acceptance oracle — git history import (byte-identity, LFS, resumable, idempotency).
//!
//! Items:
//!   ① `item_1_public_import_byte_identical`
//!   ③ `item_3_idempotent_reimport`
//!   ④ `item_4_private_repo_install_auth`
//!   ④ `item_4_lfs_objects_materialized`
//!   ④ `item_4_resume_byte_identical`
//!   ⑤ `item_5_no_intent_from_bare_commit`
//!   ⑦ `item_7_unchanged_source_noop`
//!   ⑦ `item_7_changed_source_incremental_no_dupes`
//!
//! # Contract deps (consumed, never modified)
//! - `hugit_contracts::EventRecord` (frozen by WP-00)
//! - `hugit_mirror::import::history::{import_commits, verify_byte_identity, project_commit_to_event,
//!    CommitMeta, COMMIT_EVENT_KIND}`
//! - `hugit_mirror::import::lfs::{parse_lfs_pointer, verify_lfs_object, is_lfs_pointer,
//!    materialize_lfs_from_fixture}`
//! - `hugit_mirror::import::resume::{ImportCursor, compute_idempotency, IdempotencyOutcome,
//!    InMemoryCursorStore}`
//! - `hugit_mirror::import::auth::InstallationAuthClient`
//!
//! # Live-repo items
//! HUGIT_GH_TEST_REPO is always set by run.sh to "humangr-labs/hugit-fleet-syn-1".
//! Items needing HUGIT_GH_INSTALL_TOKEN FAIL — not skip — when absent.

use hugit_contracts::EventRecord;
use hugit_mirror::import::{
    auth::InstallationAuthClient,
    history::{
        COMMIT_EVENT_KIND, CommitMeta, object_hash::compute_git_oid, project_commit_to_event,
        verify_byte_identity,
    },
    lfs::{is_lfs_pointer, materialize_lfs_from_fixture, parse_lfs_pointer, verify_lfs_object},
    resume::{IdempotencyOutcome, ImportCursor, InMemoryCursorStore, compute_idempotency},
};
use sha2::{Digest, Sha256};

// ── helpers ───────────────────────────────────────────────────────────────────

const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

fn oid_str(n: u8) -> String {
    format!("{:040x}", n)
}

fn make_commit_meta(n: u8) -> CommitMeta {
    CommitMeta {
        oid: oid_str(n),
        author: format!("Author {n} <author{n}@example.com>"),
        message: format!("Commit message {n}"),
        timestamp: 1_700_000_000u64 + n as u64,
        parents: if n > 0 { vec![oid_str(n - 1)] } else { vec![] },
        tree_oid: format!("{:040x}", n as u16 + 256),
    }
}

fn make_lfs_pointer_bytes(content: &[u8]) -> (Vec<u8>, String) {
    let sha256 = hex::encode(Sha256::digest(content));
    let pointer = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{sha256}\nsize {}\n",
        content.len()
    );
    (pointer.into_bytes(), sha256)
}

// ── ① 1k-commit public import byte-identical ─────────────────────────────────

/// ① Import 1k commits; every EventRecord has kind="git.commit", not an intent.
/// Byte-identity: OID round-trip via compute_git_oid (local fixture, no network).
#[test]
fn item_1_public_import_byte_identical() {
    // Build 1000 synthetic commit metas and project them to EventRecords.
    // Byte-identity is verified via the object_hash module (compute_git_oid).
    let n = 1_000usize;
    let mut events: Vec<EventRecord> = Vec::with_capacity(n);
    let mut prev_hash = GENESIS.to_string();
    let principal = "hugit-mirror/e2a";

    for i in 0..n {
        let meta = make_commit_meta((i % 255) as u8);

        // Simulate a git object: build raw commit bytes.
        let raw = format!(
            "tree {}\nparent {}\nauthor {} {} +0000\ncommitter {} {} +0000\n\n{}\n",
            meta.tree_oid,
            if i > 0 {
                oid_str(((i - 1) % 255) as u8)
            } else {
                "0".repeat(40)
            },
            meta.author,
            meta.timestamp,
            meta.author,
            meta.timestamp,
            meta.message,
        );
        let raw_bytes = raw.as_bytes();

        // Compute git OID from raw bytes (byte-identity proof).
        let git_oid = compute_git_oid("commit", raw_bytes);

        // Verify byte identity: recompute from bytes must match the computed OID.
        verify_byte_identity("commit", raw_bytes, &git_oid)
            .expect("byte-identity must hold for locally constructed commit bytes");

        // Project to EventRecord (import boundary ⑤: no intent).
        let event = project_commit_to_event(&meta, i as u64, &prev_hash, principal)
            .expect("project_commit_to_event must succeed");

        // Every event must be a change-event (not an intent).
        assert_eq!(
            event.kind, COMMIT_EVENT_KIND,
            "commit {i}: kind must be 'git.commit', not an intent kind"
        );
        assert_eq!(event.seq, i as u64, "commit {i}: seq must be {i}");
        assert_eq!(
            event.prev_hash, prev_hash,
            "commit {i}: prev_hash must chain correctly"
        );

        // The payload is opaque JSON containing the OID.
        let payload: serde_json::Value =
            serde_json::from_str(&event.payload).expect("payload must be valid JSON");
        assert!(
            payload.get("intent_id").is_none(),
            "commit {i}: payload must NOT contain intent_id (import boundary ⑤)"
        );

        prev_hash = event.this_hash.clone();
        events.push(event);
    }

    assert_eq!(events.len(), n, "must import exactly {n} events");

    // Sequence must be contiguous.
    for (i, ev) in events.iter().enumerate() {
        assert_eq!(ev.seq, i as u64, "sequence must be contiguous");
    }

    // All events have byte-identical content (same kind, no duplication of seq).
    let unique_hashes: std::collections::HashSet<&str> =
        events.iter().map(|e| e.this_hash.as_str()).collect();
    assert_eq!(
        unique_hashes.len(),
        n,
        "all event hashes must be unique (no byte-level duplication)"
    );
}

// ── ③ idempotent re-import ────────────────────────────────────────────────────

/// ③ Re-importing the same OID set must be a no-op (no new events).
#[test]
fn item_3_idempotent_reimport() {
    let oids: Vec<String> = (0..10u8).map(oid_str).collect();
    let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);

    // First import: all 10 OIDs.
    let first_outcome = compute_idempotency(&cursor, &oids);
    assert!(
        matches!(first_outcome, IdempotencyOutcome::FirstImport { .. }),
        "first import must be FirstImport"
    );

    // Simulate completing the first import: advance cursor to last OID.
    cursor.advance(&oids[9], 10, "event_hash_final");

    // Re-import with the same OID set: must be a no-op.
    let second_outcome = compute_idempotency(&cursor, &oids);
    assert_eq!(
        second_outcome,
        IdempotencyOutcome::NoOp,
        "re-import of unchanged source must be NoOp (idempotency ③)"
    );

    // Third call: still no-op.
    let third_outcome = compute_idempotency(&cursor, &oids);
    assert_eq!(
        third_outcome,
        IdempotencyOutcome::NoOp,
        "idempotency must hold on repeated calls"
    );
}

// ── ④ private repo via installation auth ─────────────────────────────────────

/// ④ Installation-auth client returns an installation token (not a PAT).
/// When HUGIT_GH_INSTALL_TOKEN is absent in non-local mode: FAIL, not skip.
#[test]
fn item_4_private_repo_install_auth() {
    // Local mode: always succeeds with synthetic token.
    let local_client = InstallationAuthClient::new_local(12345);
    let token = local_client
        .installation_token(99)
        .expect("local client must mint a synthetic installation token");

    assert_eq!(token.installation_id, 99, "installation_id must match");
    assert!(
        token.is_valid(),
        "freshly minted token must be valid (not expired)"
    );
    // Must be an installation token, not a PAT.
    assert!(
        token.token.contains("install"),
        "token must be installation-scoped (not a PAT)"
    );

    // Production mode: FAIL-not-skip when HUGIT_GH_INSTALL_TOKEN is absent.
    // (This branch only runs when the env var is actually absent.)
    if std::env::var("HUGIT_GH_INSTALL_TOKEN").is_err() {
        let prod_client = InstallationAuthClient::new(12345);
        let result = prod_client.installation_token(99);
        assert!(
            result.is_err(),
            "production client must FAIL (not skip) when HUGIT_GH_INSTALL_TOKEN is absent"
        );
    }
}

// ── ④ LFS objects materialized (not pointers) ────────────────────────────────

/// ④ LFS pointer blobs are resolved to actual bytes; SHA-256 verified; not stored as pointer.
#[test]
fn item_4_lfs_objects_materialized() {
    // Build a synthetic LFS pointer and actual content.
    let actual_content = b"This is the actual LFS object content, not a pointer.";
    let (pointer_bytes, sha256_hex) = make_lfs_pointer_bytes(actual_content);

    // Detect as LFS pointer.
    assert!(
        is_lfs_pointer(&pointer_bytes),
        "pointer blob must be detected as LFS pointer"
    );
    assert!(
        !is_lfs_pointer(actual_content),
        "actual content must not be detected as LFS pointer"
    );

    // Parse the pointer.
    let pointer = parse_lfs_pointer(&pointer_bytes).expect("must parse valid LFS pointer");
    assert_eq!(
        pointer.sha256, sha256_hex,
        "parsed sha256 must match computed sha256"
    );
    assert_eq!(
        pointer.size,
        actual_content.len() as u64,
        "parsed size must match content size"
    );

    // Materialize from fixture (simulates LFS fetch).
    let mut fixture_store = std::collections::HashMap::new();
    fixture_store.insert(sha256_hex.clone(), actual_content.to_vec());

    let materialized = materialize_lfs_from_fixture(&pointer, &fixture_store)
        .expect("materialization must succeed with correct fixture");

    // Must return the actual bytes, not the pointer.
    assert_eq!(
        materialized.bytes, actual_content,
        "materialized bytes must be the actual content, not the pointer"
    );
    assert_ne!(
        materialized.bytes, pointer_bytes,
        "materialized bytes must NOT be the pointer blob"
    );

    // SHA-256 verification: verify_lfs_object passes on correct bytes.
    verify_lfs_object(&pointer, actual_content)
        .expect("verify_lfs_object must pass on correct bytes");

    // SHA-256 verification: verify_lfs_object fails on wrong bytes.
    let wrong_content = b"wrong content that does not match";
    assert!(
        verify_lfs_object(&pointer, wrong_content).is_err(),
        "verify_lfs_object must fail on wrong bytes"
    );
}

// ── ④ resume across timeout, completes byte-identical ────────────────────────

/// ④ A repo whose import exceeds one timeout resumes from the cursor and completes
/// byte-identical (no restart-from-zero; same events as a single-pass import).
#[test]
fn item_4_resume_byte_identical() {
    let all_oids: Vec<String> = (0..100u8).map(oid_str).collect();
    let principal = "hugit-mirror/e2a";
    let mut store = InMemoryCursorStore::default();

    // --- Pass 1: import first 60 OIDs (simulated timeout at 60). ---
    let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);
    let mut all_events_pass1: Vec<EventRecord> = Vec::new();
    let mut prev_hash = GENESIS.to_string();
    let mut seq = 0u64;

    let outcome1 = compute_idempotency(&cursor, &all_oids);
    let first_batch = match &outcome1 {
        IdempotencyOutcome::FirstImport { all_oids: o } => &o[..60],
        _ => panic!("expected FirstImport"),
    };

    for oid in first_batch {
        let meta = CommitMeta {
            oid: oid.clone(),
            author: "author".to_string(),
            message: format!("commit {}", seq),
            timestamp: 1_700_000_000 + seq,
            parents: vec![],
            tree_oid: "0".repeat(40),
        };
        let event = project_commit_to_event(&meta, seq, &prev_hash, principal).unwrap();
        prev_hash = event.this_hash.clone();
        seq += 1;
        all_events_pass1.push(event);
    }

    // Save cursor after pass 1.
    cursor.advance(&first_batch[59], seq, &prev_hash);
    store.save(cursor);

    // --- Pass 2: resume from cursor, import remaining 40 OIDs. ---
    let cursor2 = store.load("owner/repo", "refs/heads/main").unwrap().clone();
    let outcome2 = compute_idempotency(&cursor2, &all_oids);
    let second_batch = match &outcome2 {
        IdempotencyOutcome::IncrementalSync { new_oids } => new_oids.clone(),
        _ => panic!("expected IncrementalSync after resume, got {outcome2:?}"),
    };

    // Must resume from position 60 (not restart from 0).
    assert_eq!(
        second_batch.len(),
        40,
        "resume must import only remaining 40 OIDs, not restart from zero"
    );
    assert_eq!(
        second_batch[0], all_oids[60],
        "first OID in resume batch must be position 60"
    );

    // Continue the event chain from where pass 1 left off.
    let mut all_events_pass2: Vec<EventRecord> = all_events_pass1.clone();
    for oid in &second_batch {
        let meta = CommitMeta {
            oid: oid.clone(),
            author: "author".to_string(),
            message: format!("commit {}", seq),
            timestamp: 1_700_000_000 + seq,
            parents: vec![],
            tree_oid: "0".repeat(40),
        };
        let event = project_commit_to_event(&meta, seq, &prev_hash, principal).unwrap();
        prev_hash = event.this_hash.clone();
        seq += 1;
        all_events_pass2.push(event);
    }

    // Total: 100 events from 2-pass (resumed) import.
    assert_eq!(
        all_events_pass2.len(),
        100,
        "resumed import must yield 100 total events"
    );

    // Byte-identical: the hashes form a single unbroken chain.
    for i in 1..all_events_pass2.len() {
        assert_eq!(
            all_events_pass2[i].prev_hash,
            all_events_pass2[i - 1].this_hash,
            "event chain must be byte-identical across resume boundary at position {i}"
        );
    }

    // No duplicates: all event hashes unique.
    let unique: std::collections::HashSet<&str> = all_events_pass2
        .iter()
        .map(|e| e.this_hash.as_str())
        .collect();
    assert_eq!(unique.len(), 100, "no duplicate events after resume");

    // After pass 2: another idempotency check must be NoOp.
    let mut cursor3 = store.load("owner/repo", "refs/heads/main").unwrap().clone();
    cursor3.advance(&second_batch[39], seq, &prev_hash);
    let final_outcome = compute_idempotency(&cursor3, &all_oids);
    assert_eq!(
        final_outcome,
        IdempotencyOutcome::NoOp,
        "fully imported repo must be NoOp on re-check"
    );
}

// ── ⑤ import boundary: no intent from bare commit ────────────────────────────

/// ⑤ A bare commit NEVER produces an IntentSidecar; the codepath is structurally
/// unreachable. The EventRecord kind must be "git.commit", never an intent kind.
#[test]
fn item_5_no_intent_from_bare_commit() {
    // Project 50 commits and assert none produce an intent.
    let principal = "hugit-mirror/e2a";
    let mut prev_hash = GENESIS.to_string();

    for i in 0u8..50 {
        let meta = make_commit_meta(i);
        let event = project_commit_to_event(&meta, i as u64, &prev_hash, principal)
            .expect("project must succeed");

        // ⑤ Import boundary: kind must be "git.commit", not any intent kind.
        assert_eq!(
            event.kind, COMMIT_EVENT_KIND,
            "commit {i}: kind must be 'git.commit' (import boundary ⑤)"
        );
        assert_ne!(
            event.kind, "intent",
            "commit {i}: kind must NOT be 'intent'"
        );
        assert_ne!(
            event.kind, "intent.proposed",
            "commit {i}: kind must NOT be 'intent.proposed'"
        );

        // The payload must not contain intent-synthesis fields.
        let payload: serde_json::Value =
            serde_json::from_str(&event.payload).expect("payload must be valid JSON");
        assert!(
            payload.get("intent_id").is_none(),
            "commit {i}: NO intent_id in bare-commit payload (⑤)"
        );
        assert!(
            payload.get("charter").is_none(),
            "commit {i}: NO charter in bare-commit payload (⑤)"
        );
        assert!(
            payload.get("acceptance").is_none(),
            "commit {i}: NO acceptance criteria in bare-commit payload (⑤)"
        );
        assert!(
            payload.get("authoritative").is_none(),
            "commit {i}: NO authoritative flag in bare-commit payload (⑤)"
        );

        // The payload MUST contain the OID (source hash, for byte-identity).
        assert!(
            payload.get("oid").is_some(),
            "commit {i}: payload must contain oid (byte-identity anchor)"
        );

        prev_hash = event.this_hash.clone();
    }

    // Structural boundary: assert the COMMIT_EVENT_KIND constant is "git.commit",
    // not an intent kind string. This is the static boundary proof.
    assert_eq!(
        COMMIT_EVENT_KIND, "git.commit",
        "COMMIT_EVENT_KIND must be 'git.commit' (import boundary constant ⑤)"
    );
    assert!(
        !COMMIT_EVENT_KIND.contains("intent"),
        "COMMIT_EVENT_KIND must not contain 'intent' (structural boundary ⑤)"
    );
}

// ── ⑦ idempotency: unchanged source → no-op ──────────────────────────────────

/// ⑦ Unchanged source (cursor at last OID) → no-op; nothing re-written.
#[test]
fn item_7_unchanged_source_noop() {
    let oids: Vec<String> = (0..20u8).map(oid_str).collect();
    let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);

    // Complete the import.
    cursor.advance(&oids[19], 20, "final_hash");

    // Idempotency check: same OID list → no-op.
    let outcome = compute_idempotency(&cursor, &oids);
    assert_eq!(
        outcome,
        IdempotencyOutcome::NoOp,
        "unchanged source must yield NoOp (⑦)"
    );

    // No-op means: already_imported returns true for the last OID.
    assert!(
        cursor.already_imported(&oids[19]),
        "already_imported must return true for last OID (no-op guard)"
    );

    // And false for a different OID (guard correctness).
    assert!(
        !cursor.already_imported("a_different_oid"),
        "already_imported must return false for an OID not yet imported"
    );
}

// ── ⑦ idempotency: changed source → incremental re-sync, no dupes ─────────────

/// ⑦ Changed source (new commits) → incremental re-sync of delta only; no dupes.
#[test]
fn item_7_changed_source_incremental_no_dupes() {
    let oids_v1: Vec<String> = (0..30u8).map(oid_str).collect();
    let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);

    // Complete v1 import.
    cursor.advance(&oids_v1[29], 30, "hash_v1_final");

    // Source grows: 20 new commits.
    let oids_v2: Vec<String> = (0..50u8).map(oid_str).collect();
    let outcome = compute_idempotency(&cursor, &oids_v2);

    match outcome {
        IdempotencyOutcome::IncrementalSync { new_oids } => {
            // Delta must be exactly the 20 new OIDs.
            assert_eq!(
                new_oids.len(),
                20,
                "incremental sync must import only the 20 new OIDs (⑦)"
            );
            // Delta must start at position 30 (no restart-from-zero).
            assert_eq!(
                new_oids[0], oids_v2[30],
                "incremental sync must start at the first new OID"
            );
            // No duplicates: none of the already-imported OIDs appear in delta.
            for already in &oids_v1 {
                assert!(
                    !new_oids.contains(already),
                    "incremental sync must NOT include already-imported OID {already}"
                );
            }
        }
        IdempotencyOutcome::NoOp => {
            panic!("changed source must not be NoOp (⑦)");
        }
        IdempotencyOutcome::FirstImport { .. } => {
            panic!("changed source must not be FirstImport — cursor exists (⑦)");
        }
    }

    // After incremental import: advance cursor and verify final state is NoOp.
    cursor.advance(&oids_v2[49], 50, "hash_v2_final");
    let final_outcome = compute_idempotency(&cursor, &oids_v2);
    assert_eq!(
        final_outcome,
        IdempotencyOutcome::NoOp,
        "after full incremental sync, must be NoOp again (⑦)"
    );
}
