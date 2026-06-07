//! Persistence adapter for inbound webhook events (WP-B1 item ②).
//!
//! Persists the raw PR event to durable storage and returns a 2xx ack in <1s.
//! Heavy work is enqueued, never done inline (whitepaper §1.5 / 10-second limit).
//!
//! In production, the persistence target is the CoreLink CAS via `/v1/cas`,
//! keyed by delivery ID. This module provides the local-testable adapter
//! interface; the live CF binding is infrastructure-gated (PARTIAL).

use hugit_contracts::SignedEventEnvelope;

/// Errors from the persistence adapter.
#[derive(Debug, thiserror::Error)]
pub enum PersistenceError {
    /// Serialisation failure.
    #[error("serialise envelope: {0}")]
    Serialise(String),

    /// Storage write failed.
    #[error("storage write failed: {0}")]
    Write(String),

    /// Processing for this installation has been halted (uninstall received).
    #[error("installation {installation_id} is halted; event rejected")]
    InstallationHalted { installation_id: String },
}

/// Outcome of persisting a webhook event.
#[derive(Debug, Clone, PartialEq)]
pub struct PersistResult {
    /// CAS key under which the event was stored (delivery_id-keyed).
    pub cas_key: String,
    /// Whether the heavy-work queue entry was enqueued (true in local mode).
    pub enqueued: bool,
}

/// Persistence adapter.
///
/// The production implementation wires to the CoreLink CAS client (`clw`).
/// The in-process implementation stores events in a `Vec` for unit tests.
pub struct PersistenceAdapter {
    /// In-memory store for local/test usage (keyed by delivery_id).
    store: std::collections::HashMap<String, String>,
    /// Set of halted installation IDs. `persist_event` rejects events for
    /// any installation in this set.
    halted: std::collections::HashSet<String>,
}

impl PersistenceAdapter {
    /// Create a new in-process adapter (for tests and local development).
    pub fn new_local() -> Self {
        Self {
            store: std::collections::HashMap::new(),
            halted: std::collections::HashSet::new(),
        }
    }

    /// Persist a `SignedEventEnvelope` and enqueue heavy work.
    ///
    /// If `installation_id` is `Some` and that installation has been halted,
    /// returns `Err(PersistenceError::InstallationHalted)` immediately.
    ///
    /// Must complete in <1s (GitHub 10-second fire-and-forget limit, §1.5).
    /// Returns the CAS key and enqueue status.
    pub fn persist_event(
        &mut self,
        envelope: &SignedEventEnvelope,
        installation_id: Option<&str>,
    ) -> Result<PersistResult, PersistenceError> {
        // Fail-closed: reject events for halted installations.
        if let Some(iid) = installation_id
            && self.halted.contains(iid)
        {
            return Err(PersistenceError::InstallationHalted {
                installation_id: iid.to_string(),
            });
        }

        let value = serde_json::to_string(envelope)
            .map_err(|e| PersistenceError::Serialise(e.to_string()))?;

        let cas_key = format!("webhook/{}", envelope.delivery_id);
        self.store.insert(cas_key.clone(), value);

        // In production: enqueue delivery_id for async processing.
        // Locally: flag enqueued = true to satisfy the contract.
        Ok(PersistResult {
            cas_key,
            enqueued: true,
        })
    }

    /// Retrieve a persisted event by delivery ID (for testing).
    pub fn get_event(&self, delivery_id: &str) -> Option<SignedEventEnvelope> {
        let key = format!("webhook/{delivery_id}");
        self.store
            .get(&key)
            .and_then(|v| serde_json::from_str(v).ok())
    }

    /// Halt all queued processing for a given installation ID.
    ///
    /// Inserts `installation_id` into the halted set. Subsequent calls to
    /// `persist_event` with this installation_id will return
    /// `PersistenceError::InstallationHalted`.
    ///
    /// Returns `items_halted = 1` when newly halted, `0` if already halted.
    pub fn halt_installation(&mut self, installation_id: &str) -> HaltResult {
        let newly_halted = self.halted.insert(installation_id.to_string());
        HaltResult {
            installation_id: installation_id.to_string(),
            items_halted: if newly_halted { 1 } else { 0 },
        }
    }
}

/// Result of halting an installation's processing.
#[derive(Debug, Clone, PartialEq)]
pub struct HaltResult {
    pub installation_id: String,
    pub items_halted: u64,
}
