//! Journal object persistence — tenant-private, bound to a workspace/intent.
//!
//! A session journal is a **tenant-private first-class object** bound to a
//! specific `(workspace_id, intent_id)` pair. It is structurally impossible
//! to read another tenant's journal: the tenant id is part of the key and is
//! always checked before any record is returned.
//!
//! No I/O: this module owns the **model** (the in-memory object) and the
//! **binding assertion** (key derivation + scope check). Persistence to an
//! external store (D-tier DO/R2) is a transport concern outside D11's claims.
//!
//! ## Tenant-private scope (D11①)
//!
//! Cross-tenant access is X3's law — D11 *honours the scope* and never
//! re-implements isolation. Concretely: every `JournalKey` carries
//! `tenant_id`; `JournalStore::open` refuses to return a journal whose
//! `JournalKey.tenant_id` differs from the caller-supplied `tenant_id`.

use serde::{Deserialize, Serialize};

use crate::journal::horizon::{DEFAULT_HORIZON_MS, HorizonResult, check_horizon};

/// The composite key that uniquely and privately addresses one session journal.
///
/// A journal is **tenant-private**: the `tenant_id` is always part of the key.
/// Two journals with the same `workspace_id` + `intent_id` but different
/// `tenant_id` values are *different, inaccessible objects* to each other.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct JournalKey {
    /// Tenant identifier — the isolation boundary (X3's law).
    pub tenant_id: String,
    /// Workspace the session was running inside.
    pub workspace_id: String,
    /// Native intent the session was executing.
    pub intent_id: String,
}

impl JournalKey {
    /// Construct a new journal key.
    pub fn new(
        tenant_id: impl Into<String>,
        workspace_id: impl Into<String>,
        intent_id: impl Into<String>,
    ) -> Self {
        JournalKey {
            tenant_id: tenant_id.into(),
            workspace_id: workspace_id.into(),
            intent_id: intent_id.into(),
        }
    }
}

/// One entry in the session journal — a recorded event with provenance.
///
/// Journals are best-effort reconstruction; they are NOT a bitwise context
/// replay (replay was cut per catalog v2). Each entry is an attributed note
/// on what the session did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Monotonically increasing sequence within this journal (1-based).
    pub seq: u32,
    /// Unix epoch milliseconds when this entry was recorded.
    pub recorded_at: u64,
    /// The principal (agent/user) that recorded this entry.
    pub principal: String,
    /// Human-readable note describing what the session did at this point.
    pub note: String,
}

/// A session journal — tenant-private first-class object bound to
/// `(workspace_id, intent_id)`.
///
/// The binding is structural: the journal carries its `key` and every
/// operation on the journal verifies the tenant matches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Journal {
    /// The composite binding key — carries tenant, workspace, and intent.
    pub key: JournalKey,
    /// Ordered log of session entries, newest last.
    pub entries: Vec<JournalEntry>,
}

impl Journal {
    /// Create a new, empty journal bound to the given key.
    pub fn new(key: JournalKey) -> Self {
        Journal {
            key,
            entries: Vec::new(),
        }
    }

    /// Append one entry, assigning the next sequence number.
    pub fn append(
        &mut self,
        recorded_at: u64,
        principal: impl Into<String>,
        note: impl Into<String>,
    ) {
        let seq = self.entries.len() as u32 + 1;
        self.entries.push(JournalEntry {
            seq,
            recorded_at,
            principal: principal.into(),
            note: note.into(),
        });
    }

    /// Unix epoch milliseconds of the most recent entry, or `None` if the
    /// journal is empty.
    pub fn last_recorded_at(&self) -> Option<u64> {
        self.entries.last().map(|e| e.recorded_at)
    }

    /// Check whether this journal is within the supported resume horizon.
    ///
    /// An empty journal (no entries) is always considered within-horizon.
    pub fn horizon_check(&self, now_ms: u64) -> HorizonResult {
        match self.last_recorded_at() {
            None => HorizonResult::WithinHorizon,
            Some(last) => check_horizon(last, now_ms, DEFAULT_HORIZON_MS),
        }
    }
}

/// Error variants for journal store operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JournalError {
    /// The tenant id in the key does not match the caller-supplied tenant.
    /// Fail-closed: cross-tenant access is never silently permitted.
    TenantMismatch { expected: String, found: String },
    /// No journal exists for this key.
    NotFound { key: JournalKey },
}

impl std::fmt::Display for JournalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JournalError::TenantMismatch { expected, found } => write!(
                f,
                "tenant mismatch: caller is `{expected}`, journal is `{found}`"
            ),
            JournalError::NotFound { key } => write!(
                f,
                "no journal for tenant={} ws={} intent={}",
                key.tenant_id, key.workspace_id, key.intent_id
            ),
        }
    }
}

impl std::error::Error for JournalError {}

/// In-memory journal store.
///
/// Represents one tenant's private journal collection. In production this
/// would be backed by a DO/R2 store; here it is an in-memory map for testing
/// and offline use.
///
/// The store holds journals for exactly **one tenant**. All open() calls must
/// supply the same `tenant_id`; any mismatch is refused fail-closed.
#[derive(Debug, Default)]
pub struct JournalStore {
    journals: std::collections::HashMap<String, Journal>,
}

impl JournalStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Persist (insert or replace) a journal. The journal's `key.tenant_id`
    /// is the canonical tenant; the caller must own the key.
    pub fn put(&mut self, journal: Journal) {
        let store_key = Self::storage_key(&journal.key);
        self.journals.insert(store_key, journal);
    }

    /// Open (retrieve) a journal for `(tenant_id, workspace_id, intent_id)`.
    ///
    /// Returns `Err(JournalError::NotFound)` if no such journal exists.
    /// Returns `Err(JournalError::TenantMismatch)` if the stored journal's
    /// tenant does not match — this is a structural invariant violation and
    /// must never happen in a well-formed store, but is checked defensively
    /// (fail-closed).
    pub fn open(
        &self,
        tenant_id: &str,
        workspace_id: &str,
        intent_id: &str,
    ) -> Result<&Journal, JournalError> {
        let key = JournalKey::new(tenant_id, workspace_id, intent_id);
        let store_key = Self::storage_key(&key);
        let journal = self
            .journals
            .get(&store_key)
            .ok_or_else(|| JournalError::NotFound { key: key.clone() })?;
        // Defensive tenant assertion (fail-closed).
        if journal.key.tenant_id != tenant_id {
            return Err(JournalError::TenantMismatch {
                expected: tenant_id.to_string(),
                found: journal.key.tenant_id.clone(),
            });
        }
        Ok(journal)
    }

    /// The storage key is derived deterministically from the binding triple.
    /// Cross-tenant collisions are impossible because the tenant_id is always
    /// included.
    fn storage_key(key: &JournalKey) -> String {
        format!("{}:{}:{}", key.tenant_id, key.workspace_id, key.intent_id)
    }
}
