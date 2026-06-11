//! The on-disk intent store — the local hermetic spine `new` writes and `show`
//! reads (WP-PC2).
//!
//! Hermetic-first by mandate: the intent porcelain operates on a LOCAL event-log
//! file through the **real** refstore append/projection paths
//! ([`EventLog`] · [`import_sidecar`] · [`intents_from_log`]) — exactly the path
//! the dogfood wave drives. The live DO/CAS binding stays the P2 disclosed seam;
//! here the store is a single JSON file the caller points `--store` at (same
//! file-seam shape `hugit why`/`export` use for their `--log`).
//!
//! ## On-disk shape (the store's wire contract)
//!
//! ```json
//! {
//!   "events":   [ <EventRecord>, … ],   // the hash-chained spine (canonical)
//!   "sidecars": [ <IntentSidecar>, … ], // the B6 corpus, keyed by intent_id
//!   "envelopes": { "<intent_id>": "<context_ref>", … }, // disclosed P2 refs
//!   "verdicts":  { "<intent_id>": <Verdict>, … }        // captured verdicts
//! }
//! ```
//!
//! The `events` array IS the authoritative record — it is loaded back through
//! [`EventLog::push_record`] (which re-checks the monotonic-seq invariant) and
//! the chain is re-verified with [`verify_chain`], so a tampered store fails
//! closed rather than projecting a forged intent. The `sidecars` corpus is the
//! frozen, **non-authoritative** [`IntentSidecar`] metadata B6 stores by
//! `intent_id`; the event log never disagrees with it because the event carries
//! the same `intent_id`. `envelopes`/`verdicts` are honestly absent until
//! captured — never invented.
//!
//! [`import_sidecar`]: hugit_refstore::intent::import_sidecar
//! [`intents_from_log`]: hugit_refstore::intent::intents_from_log

use std::collections::BTreeMap;
use std::path::Path;

use hugit_contracts::IntentSidecar;
use hugit_contracts::event_record::EventRecord;
use hugit_refstore::intent::{Intent, intents_from_log};
use hugit_refstore::{EventLog, verify_chain};
use serde::{Deserialize, Serialize};

/// A captured adversarial verdict on an intent (honestly absent until a panel
/// records one — never fabricated). Minimal, machine-stable shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verdict {
    /// The reviewer lens / panel that issued the verdict.
    pub lens: String,
    /// The decision token (e.g. `"approve"` / `"reject"` / `"needs-changes"`).
    pub decision: String,
    /// Optional free-text rationale (absent when not captured).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rationale: Option<String>,
}

/// The full on-disk store: the hash-chained event spine plus the
/// non-authoritative sidecar corpus and the disclosed envelope/verdict seams.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IntentStoreFile {
    /// The append-only event records (the canonical, hash-chained spine).
    #[serde(default)]
    pub events: Vec<EventRecord>,
    /// The B6 [`IntentSidecar`] corpus, one per landed intent_id.
    #[serde(default)]
    pub sidecars: Vec<IntentSidecar>,
    /// Disclosed P2 seam: intent_id → context envelope ref, when captured.
    #[serde(default)]
    pub envelopes: BTreeMap<String, String>,
    /// Captured verdicts: intent_id → verdict, when an adversarial panel ran.
    #[serde(default)]
    pub verdicts: BTreeMap<String, Verdict>,
}

/// An error operating the on-disk intent store.
#[derive(Debug)]
pub enum StoreError {
    /// The store file could not be read (and `--store` was not a fresh path).
    Read {
        /// The store path that failed to read.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The store file held invalid JSON.
    Parse {
        /// The store path that failed to parse.
        path: String,
        /// The parse error message.
        msg: String,
    },
    /// Loading the persisted chain violated the append-only seq invariant.
    Rehydrate(String),
    /// The persisted chain failed integrity verification (tamper / corruption).
    ChainBroken(String),
    /// The store could not be written back to disk.
    Write {
        /// The store path that failed to write.
        path: String,
        /// The underlying I/O error.
        source: std::io::Error,
    },
    /// The store could not be serialised to JSON.
    Serialize(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Read { path, source } => write!(f, "read store {path}: {source}"),
            StoreError::Parse { path, msg } => write!(f, "parse store {path}: {msg}"),
            StoreError::Rehydrate(m) => write!(f, "rehydrate event log: {m}"),
            StoreError::ChainBroken(m) => write!(f, "store chain failed verification: {m}"),
            StoreError::Write { path, source } => write!(f, "write store {path}: {source}"),
            StoreError::Serialize(m) => write!(f, "serialize store: {m}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// The in-memory, loaded store: the rehydrated [`EventLog`] (verified) plus the
/// side corpora. `new` mutates this and persists with [`IntentStore::save`];
/// `show` only reads it.
#[derive(Debug, Clone)]
pub struct IntentStore {
    /// The rehydrated, chain-verified event log (the real refstore spine).
    pub log: EventLog,
    /// The non-authoritative sidecar corpus, indexed by `intent_id`.
    pub sidecars: BTreeMap<String, IntentSidecar>,
    /// Disclosed P2 envelope refs by `intent_id`.
    pub envelopes: BTreeMap<String, String>,
    /// Captured verdicts by `intent_id`.
    pub verdicts: BTreeMap<String, Verdict>,
}

impl IntentStore {
    /// Load the store at `path`. A non-existent path is an EMPTY store (so the
    /// first `new` bootstraps it) — any OTHER read error fails closed.
    ///
    /// The persisted events are rehydrated through [`EventLog::push_record`]
    /// (re-checks the monotonic seq) and the whole chain is re-verified with
    /// [`verify_chain`], so a tampered store is rejected, never projected.
    pub fn load(path: &Path) -> Result<Self, StoreError> {
        let file = match std::fs::read(path) {
            Ok(bytes) => {
                let path_s = path.display().to_string();
                serde_json::from_slice::<IntentStoreFile>(&bytes).map_err(|e| {
                    StoreError::Parse {
                        path: path_s,
                        msg: e.to_string(),
                    }
                })?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => IntentStoreFile::default(),
            Err(e) => {
                return Err(StoreError::Read {
                    path: path.display().to_string(),
                    source: e,
                });
            }
        };

        let mut log = EventLog::new();
        for record in file.events {
            log.push_record(record)
                .map_err(|e| StoreError::Rehydrate(e.to_string()))?;
        }
        // Fail closed on a tampered / corrupt chain.
        verify_chain(log.records()).map_err(|e| StoreError::ChainBroken(e.to_string()))?;

        let sidecars = file
            .sidecars
            .into_iter()
            .map(|s| (s.intent_id.clone(), s))
            .collect();

        Ok(Self {
            log,
            sidecars,
            envelopes: file.envelopes,
            verdicts: file.verdicts,
        })
    }

    /// The native intent altitude over the current log (the real projection).
    pub fn intent_for(&self, intent_id: &str) -> Result<Option<Intent>, StoreError> {
        let intents =
            intents_from_log(&self.log).map_err(|e| StoreError::ChainBroken(e.to_string()))?;
        Ok(intents.by_id(intent_id).cloned())
    }

    /// All native intents on the log, in log order (the full altitude projection).
    pub fn intent_for_all(&self) -> Result<Vec<Intent>, StoreError> {
        let intents =
            intents_from_log(&self.log).map_err(|e| StoreError::ChainBroken(e.to_string()))?;
        Ok(intents.intents().to_vec())
    }

    /// Persist the store back to `path` (events + corpora), pretty JSON so the
    /// on-disk artifact stays inspectable.
    pub fn save(&self, path: &Path) -> Result<(), StoreError> {
        let mut sidecars: Vec<IntentSidecar> = self.sidecars.values().cloned().collect();
        sidecars.sort_by(|a, b| a.intent_id.cmp(&b.intent_id));
        let file = IntentStoreFile {
            events: self.log.records().to_vec(),
            sidecars,
            envelopes: self.envelopes.clone(),
            verdicts: self.verdicts.clone(),
        };
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| StoreError::Serialize(e.to_string()))?;
        std::fs::write(path, json).map_err(|e| StoreError::Write {
            path: path.display().to_string(),
            source: e,
        })
    }
}
