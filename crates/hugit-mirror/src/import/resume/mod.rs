//! Resumable import state and idempotency engine (WP-E2a, items ④ ⑦).
//!
//! # Resumable (④)
//! A repo whose import exceeds one timeout resumes from the last persisted
//! cursor, never restarting from zero. The cursor records the last successfully
//! imported OID and the current event sequence.
//!
//! # Idempotency (⑦)
//! - **Unchanged source** (cursor matches, nothing re-written): **no-op**.
//! - **Changed source** (new commits beyond the cursor): **incremental re-sync**
//!   of the delta only — **never duplicates**.
//!
//! The diff key is: source object-hash vs the cursor's last imported OID.

use serde::{Deserialize, Serialize};

/// A checkpoint cursor for resumable import.
///
/// Persisted after each successfully imported batch. On resume, import
/// continues from `next_oid_index` in the ordered OID list, and the
/// event chain continues from `next_seq` / `last_event_hash`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportCursor {
    /// The repository being imported (owner/repo).
    pub repo: String,
    /// The ref being imported (e.g. `"refs/heads/main"`).
    pub ref_name: String,
    /// The OID of the last successfully imported commit.
    pub last_imported_oid: String,
    /// The event sequence number to use for the NEXT event.
    pub next_seq: u64,
    /// The `this_hash` of the last successfully imported event (used as
    /// `prev_hash` for the next batch).
    pub last_event_hash: String,
    /// Unix epoch ms when this cursor was last updated.
    pub updated_at: u64,
}

impl ImportCursor {
    /// Create a new cursor at the start of an import (nothing imported yet).
    pub fn new_initial(repo: &str, ref_name: &str, start_prev_hash: &str) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        Self {
            repo: repo.to_string(),
            ref_name: ref_name.to_string(),
            last_imported_oid: String::new(),
            next_seq: 0,
            last_event_hash: start_prev_hash.to_string(),
            updated_at: now,
        }
    }

    /// Advance the cursor after successfully importing a batch.
    pub fn advance(&mut self, last_oid: &str, next_seq: u64, last_event_hash: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.last_imported_oid = last_oid.to_string();
        self.next_seq = next_seq;
        self.last_event_hash = last_event_hash.to_string();
        self.updated_at = now;
    }

    /// Returns `true` if the cursor has already imported `oid`.
    ///
    /// This is the idempotency no-op check (⑦): if the source OID matches
    /// `last_imported_oid`, and no new commits follow it, the import is a no-op.
    pub fn already_imported(&self, oid: &str) -> bool {
        self.last_imported_oid == oid
    }
}

/// The outcome of an idempotency check (⑦).
#[derive(Debug, Clone, PartialEq)]
pub enum IdempotencyOutcome {
    /// Unchanged source — cursor matches, no work to do.
    NoOp,
    /// Changed source — the given OIDs are the delta to import.
    IncrementalSync { new_oids: Vec<String> },
    /// First import (cursor is empty).
    FirstImport { all_oids: Vec<String> },
}

/// Compute the idempotency outcome given the current cursor and the full
/// ordered list of OIDs from the source repository.
///
/// # Idempotency rules (⑦)
/// - If `cursor.last_imported_oid` is empty (first import): `FirstImport`.
/// - If `cursor.last_imported_oid` is the last OID in `source_oids`: `NoOp`.
/// - If `cursor.last_imported_oid` is in `source_oids` but not the last:
///   `IncrementalSync` with the OIDs after the cursor position.
/// - If `cursor.last_imported_oid` is NOT in `source_oids` (force-push/rewrite):
///   treat as `FirstImport` (full re-import — the caller decides dedup policy).
///
/// **Never duplicates**: OIDs at or before the cursor are excluded from the delta.
pub fn compute_idempotency(cursor: &ImportCursor, source_oids: &[String]) -> IdempotencyOutcome {
    if cursor.last_imported_oid.is_empty() {
        // First import: import all OIDs.
        return IdempotencyOutcome::FirstImport {
            all_oids: source_oids.to_vec(),
        };
    }

    // Find the cursor position in the source OID list.
    if let Some(pos) = source_oids
        .iter()
        .position(|oid| oid == &cursor.last_imported_oid)
    {
        let remaining = &source_oids[pos + 1..];
        if remaining.is_empty() {
            IdempotencyOutcome::NoOp
        } else {
            IdempotencyOutcome::IncrementalSync {
                new_oids: remaining.to_vec(),
            }
        }
    } else {
        // Cursor OID not in source — treat as full re-import.
        IdempotencyOutcome::FirstImport {
            all_oids: source_oids.to_vec(),
        }
    }
}

/// In-memory cursor store for tests.
///
/// Production would persist to D1/R2. This store provides the same API
/// so tests can exercise the full resume/idempotency lifecycle.
#[derive(Debug, Default)]
pub struct InMemoryCursorStore {
    cursors: std::collections::HashMap<String, ImportCursor>,
}

impl InMemoryCursorStore {
    /// Cursor store key: `"{repo}:{ref_name}"`.
    fn key(repo: &str, ref_name: &str) -> String {
        format!("{repo}:{ref_name}")
    }

    /// Load the cursor for `(repo, ref_name)`.
    pub fn load(&self, repo: &str, ref_name: &str) -> Option<&ImportCursor> {
        self.cursors.get(&Self::key(repo, ref_name))
    }

    /// Persist (upsert) the cursor for `(repo, ref_name)`.
    pub fn save(&mut self, cursor: ImportCursor) {
        let key = Self::key(&cursor.repo, &cursor.ref_name);
        self.cursors.insert(key, cursor);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GENESIS: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn oid(n: u8) -> String {
        format!("{:040x}", n)
    }

    #[test]
    fn first_import_returns_all_oids() {
        let cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);
        let oids: Vec<String> = (0..5).map(oid).collect();
        let outcome = compute_idempotency(&cursor, &oids);
        assert_eq!(
            outcome,
            IdempotencyOutcome::FirstImport {
                all_oids: oids.clone()
            }
        );
    }

    #[test]
    fn unchanged_source_is_noop() {
        let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);
        let oids: Vec<String> = (0..5).map(oid).collect();
        cursor.advance(&oids[4], 5, "some_hash");

        let outcome = compute_idempotency(&cursor, &oids);
        assert_eq!(outcome, IdempotencyOutcome::NoOp);
    }

    #[test]
    fn changed_source_incremental_no_dupes() {
        let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);
        let oids_v1: Vec<String> = (0..5).map(oid).collect();
        cursor.advance(&oids_v1[4], 5, "some_hash");

        // Three new commits added.
        let oids_v2: Vec<String> = (0..8).map(oid).collect();
        let outcome = compute_idempotency(&cursor, &oids_v2);

        match outcome {
            IdempotencyOutcome::IncrementalSync { new_oids } => {
                // Must only include the new OIDs, not duplicates.
                assert_eq!(new_oids.len(), 3);
                assert_eq!(new_oids, oids_v2[5..]);
                // None of the already-imported OIDs appear.
                for old_oid in &oids_v1 {
                    assert!(!new_oids.contains(old_oid));
                }
            }
            other => panic!("expected IncrementalSync, got {other:?}"),
        }
    }

    #[test]
    fn cursor_advance_updates_state() {
        let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);
        assert!(cursor.last_imported_oid.is_empty());
        assert_eq!(cursor.next_seq, 0);

        cursor.advance("abc123", 10, "event_hash_abc");
        assert_eq!(cursor.last_imported_oid, "abc123");
        assert_eq!(cursor.next_seq, 10);
        assert_eq!(cursor.last_event_hash, "event_hash_abc");
        assert!(cursor.already_imported("abc123"));
        assert!(!cursor.already_imported("different_oid"));
    }

    #[test]
    fn in_memory_store_save_load_round_trip() {
        let mut store = InMemoryCursorStore::default();
        let cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);

        store.save(cursor.clone());
        let loaded = store.load("owner/repo", "refs/heads/main").unwrap();
        assert_eq!(loaded.repo, cursor.repo);
        assert_eq!(loaded.ref_name, cursor.ref_name);
        assert_eq!(loaded.next_seq, cursor.next_seq);
    }

    #[test]
    fn resume_continues_from_cursor_not_zero() {
        let mut store = InMemoryCursorStore::default();
        let oids: Vec<String> = (0..100).map(oid).collect();

        // Simulate: imported 50, timeout. Cursor saved at position 49.
        let mut cursor = ImportCursor::new_initial("owner/repo", "refs/heads/main", GENESIS);
        cursor.advance(&oids[49], 50, "hash_at_50");
        store.save(cursor);

        // Resume: load cursor, compute delta.
        let loaded = store.load("owner/repo", "refs/heads/main").unwrap();
        let outcome = compute_idempotency(loaded, &oids);

        match outcome {
            IdempotencyOutcome::IncrementalSync { new_oids } => {
                // Must resume from position 50 (not restart from 0).
                assert_eq!(new_oids.len(), 50);
                assert_eq!(new_oids[0], oids[50]);
                // No restart-from-zero: oid(0) through oid(49) must not be in new_oids.
                for old in oids.iter().take(50) {
                    assert!(!new_oids.contains(old));
                }
            }
            other => panic!("expected IncrementalSync, got {other:?}"),
        }
    }
}
