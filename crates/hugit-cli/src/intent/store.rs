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

use crate::pr::filelock::{self, FileLock, LockError};

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
    /// The store file is locked by another live `hugit` verb (the advisory
    /// exclusive lock — see [`crate::pr::filelock`]). Retry-able; never a
    /// silent clobber.
    Busy {
        /// The store path that is locked.
        path: String,
    },
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
            StoreError::Busy { path } => {
                write!(f, "store {path} is locked by another hugit verb")
            }
        }
    }
}

impl std::error::Error for StoreError {}

/// Map a [`LockError`] into the store's [`StoreError`] (a live holder →
/// [`StoreError::Busy`], an I/O fault → [`StoreError::Write`]).
fn store_lock_error(path: &Path, e: LockError) -> StoreError {
    match e {
        LockError::Busy { .. } => StoreError::Busy {
            path: path.display().to_string(),
        },
        LockError::Io { .. } => StoreError::Write {
            path: path.display().to_string(),
            source: std::io::Error::other(e.to_string()),
        },
    }
}

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

    /// Acquire the advisory exclusive lock for `path` and THEN load the store
    /// under it — the WP-WC1 lock-before-load discipline (the same one
    /// `hugit pr`/`hugit intent new --log` use on the canonical seam).
    ///
    /// Returns the held [`FileLock`] guard alongside the freshly-loaded store;
    /// the caller (`intent new --store`) holds the guard across its idempotent
    /// intent-id pre-check + [`save_locked`](Self::save_locked) so the whole
    /// load→mutate→save is a single serialized critical section. Two concurrent
    /// `intent new --store` for DIFFERENT intents on the same store now serialize
    /// or fail structured ([`StoreError::Busy`]) — they no longer each load the
    /// same chain and clobber on save (WF-CLI2 bug 2: the load→lock inversion the
    /// per-write lock left open between two distinct `new`s).
    ///
    /// A missing `--store` bootstraps a fresh empty store (the `new` posture —
    /// [`load`](Self::load), NOT [`load_existing`](Self::load_existing)).
    pub fn lock_and_load(path: &Path) -> Result<(FileLock, Self), StoreError> {
        let lock = FileLock::acquire(path).map_err(|e| store_lock_error(path, e))?;
        let store = IntentStore::load(path)?;
        Ok((lock, store))
    }

    /// Persist the store back to `path` (events + corpora), pretty JSON so the
    /// on-disk artifact stays inspectable.
    ///
    /// **Lock + atomic discipline (WP-WC1).** The write acquires the advisory
    /// exclusive lock for the duration of the write (serializing concurrent
    /// saves — a racing verb gets [`StoreError::Busy`], never a clobber) and
    /// lands the bytes via an **atomic** temp-file-then-rename, so a reader or a
    /// crash sees the whole old store or the whole new one — never a truncated
    /// file. The lock is released the moment the write returns.
    ///
    /// This is the SELF-LOCKING entry point (used where no broader lock is held);
    /// `intent new --store` instead holds one lock across the whole
    /// load→mutate→save via [`lock_and_load`](Self::lock_and_load) +
    /// [`save_locked`](Self::save_locked) so two distinct concurrent `new`s
    /// cannot clobber (WF-CLI2 bug 2).
    pub fn save(&self, path: &Path) -> Result<(), StoreError> {
        // Serialize the write itself behind the advisory lock (the guard drops
        // when this function returns, releasing it).
        let _lock = FileLock::acquire(path).map_err(|e| store_lock_error(path, e))?;
        self.write_to(path)
    }

    /// Persist under a lock the CALLER already holds (acquired via
    /// [`lock_and_load`](Self::lock_and_load)): the load→mutate→save runs as one
    /// serialized critical section, killing the store-seam load→lock inversion
    /// (WF-CLI2 bug 2). The `_lock` is borrowed only to make the held-lock
    /// invariant a compile-time obligation — it is never released here (it drops
    /// when the caller's guard does).
    pub fn save_locked(&self, _lock: &FileLock, path: &Path) -> Result<(), StoreError> {
        self.write_to(path)
    }

    /// Serialize the store and land it via the **atomic** temp-then-rename write.
    /// Lock-free by itself — the lock is the caller's ([`save`](Self::save)
    /// self-locks; [`save_locked`](Self::save_locked) relies on the caller-held
    /// lock).
    fn write_to(&self, path: &Path) -> Result<(), StoreError> {
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
        filelock::atomic_write(path, json.as_bytes()).map_err(|e| store_lock_error(path, e))
    }
}
