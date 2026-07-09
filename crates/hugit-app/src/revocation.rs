//! Durable installation-revocation ledger (WP-B1 item ⑤, stateful revoke).
//!
//! When an `installation.deleted` webhook arrives, the installation's token is
//! zeroed in-memory ([`crate::webhook::WebhookProcessor::handle_uninstall`]) —
//! but an in-memory tombstone does **not** survive a process restart, and it
//! does not by itself refuse a *subsequent* token mint. This ledger closes both
//! gaps: it records a revoked installation **durably on disk** and is consulted,
//! fail-closed, by
//!
//! - the token-mint path ([`AppAuth`](../../hugit_mirror/outbound/auth) in
//!   `hugit-mirror` refuses to mint for a revoked installation), and
//! - the event-persistence path ([`crate::PersistenceAdapter`] seeds its halted
//!   set from the ledger on boot, so a restart re-halts).
//!
//! ## Durability model
//!
//! The ledger is a directory. Each revoked installation is a **tombstone file**
//! named by the (validated) installation id. Revocation = create the tombstone
//! (idempotent). Presence of the file ⇒ revoked. On [`RevocationLedger::open`]
//! the directory is scanned once and the tombstones are loaded into an in-memory
//! index; subsequent [`RevocationLedger::revoke`] updates both disk and index.
//! This makes revocation **monotonic and restart-surviving** — a revoked
//! installation stays revoked across reboots because the tombstone is on disk.
//!
//! ## Fail-closed
//!
//! Installation ids are validated to a safe slug (`[A-Za-z0-9_-]+`, GitHub ids
//! are numeric) before ever becoming a path component — an id that cannot be
//! safely persisted is refused ([`RevocationError::UnsafeId`]), never written to
//! an attacker-influenced path. A revoke that cannot be durably written is an
//! error (the caller must treat a failed revoke as a failure, never silently
//! "revoked").

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Errors from the revocation ledger.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RevocationError {
    /// The installation id is not a safe slug (`[A-Za-z0-9_-]+`) and therefore
    /// cannot be a durable tombstone path component. Fail-closed — never coerced.
    #[error("unsafe installation id (must be [A-Za-z0-9_-]+)")]
    UnsafeId,

    /// The ledger directory could not be created / read, or a tombstone could
    /// not be written. Carries a short, path-free reason (never secret material).
    #[error("revocation ledger I/O failed: {0}")]
    Io(String),
}

/// A durable, restart-surviving set of revoked installation ids.
///
/// Cheaply cloneable (shares the on-disk directory + the in-memory index via an
/// `Arc`), so the webhook processor and the token-mint gate can hold the same
/// ledger.
#[derive(Debug, Clone)]
pub struct RevocationLedger {
    dir: PathBuf,
    /// In-memory index of revoked ids, seeded from disk at [`Self::open`] and
    /// kept in step by [`Self::revoke`]. Read-mostly; a `Mutex` is ample.
    index: Arc<Mutex<HashSet<String>>>,
}

impl RevocationLedger {
    /// Open (creating if absent) the ledger rooted at `dir`, loading any existing
    /// tombstones into the in-memory index.
    ///
    /// The one-time directory scan is what makes revocation survive a restart:
    /// tombstones written in a prior process are re-loaded here.
    pub fn open(dir: impl Into<PathBuf>) -> Result<Self, RevocationError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|e| RevocationError::Io(e.kind().to_string()))?;
        let mut index = HashSet::new();
        for entry in
            std::fs::read_dir(&dir).map_err(|e| RevocationError::Io(e.kind().to_string()))?
        {
            let entry = entry.map_err(|e| RevocationError::Io(e.kind().to_string()))?;
            if let Some(name) = entry.file_name().to_str()
                && is_safe_id(name)
            {
                index.insert(name.to_string());
            }
        }
        Ok(Self {
            dir,
            index: Arc::new(Mutex::new(index)),
        })
    }

    /// Resolve the default ledger directory (`~/.hugit/state/revoked`), honouring
    /// the `HUGIT_REVOCATION_DIR` override. Returns `None` only when no override
    /// is set AND `$HOME` is absent.
    pub fn resolve_dir() -> Option<PathBuf> {
        if let Ok(p) = std::env::var("HUGIT_REVOCATION_DIR") {
            let p = p.trim().to_string();
            if !p.is_empty() {
                return Some(PathBuf::from(p));
            }
        }
        std::env::var_os("HOME").map(|home| {
            Path::new(&home)
                .join(".hugit")
                .join("state")
                .join("revoked")
        })
    }

    /// Durably record `installation_id` as revoked (idempotent).
    ///
    /// Writes a tombstone file and updates the in-memory index. Returns
    /// `newly_revoked = true` iff the installation was not already revoked. A
    /// failed disk write is an [`RevocationError::Io`] — the caller must treat a
    /// failed revoke as a failure, never as a silent success.
    pub fn revoke(&self, installation_id: &str) -> Result<bool, RevocationError> {
        if !is_safe_id(installation_id) {
            return Err(RevocationError::UnsafeId);
        }
        // Write the durable tombstone FIRST — the on-disk fact is the source of
        // truth. Only after it lands do we update the in-memory index, so a
        // crash between the two never reports "revoked" without the tombstone.
        let path = self.dir.join(installation_id);
        std::fs::write(&path, b"revoked\n")
            .map_err(|e| RevocationError::Io(e.kind().to_string()))?;
        let mut index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        Ok(index.insert(installation_id.to_string()))
    }

    /// Whether `installation_id` is revoked (durably). Consults the in-memory
    /// index (seeded from disk at open + updated on revoke). An unsafe id can
    /// never have been persisted, so it reads back `false`.
    pub fn is_revoked(&self, installation_id: &str) -> bool {
        if !is_safe_id(installation_id) {
            return false;
        }
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        index.contains(installation_id)
    }

    /// A snapshot of all revoked installation ids (for seeding the halted set).
    pub fn revoked_ids(&self) -> Vec<String> {
        let index = self.index.lock().unwrap_or_else(|e| e.into_inner());
        let mut ids: Vec<String> = index.iter().cloned().collect();
        ids.sort();
        ids
    }
}

/// A safe installation-id slug: non-empty, only `[A-Za-z0-9_-]`, and never a
/// path-traversal token. GitHub installation ids are numeric, so this is
/// permissive enough for real ids while rejecting anything path-dangerous.
fn is_safe_id(id: &str) -> bool {
    !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let mut p = std::env::temp_dir();
        p.push(format!(
            "hugit-revocation-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        p
    }

    #[test]
    fn revoke_is_durable_across_reopen() {
        let dir = tmp_dir("durable");
        {
            let ledger = RevocationLedger::open(&dir).unwrap();
            assert!(!ledger.is_revoked("12345"));
            assert!(ledger.revoke("12345").unwrap(), "first revoke is new");
            assert!(ledger.is_revoked("12345"));
        }
        // A FRESH ledger over the same dir (models a process restart) must still
        // see the revocation — this is the durability guarantee.
        {
            let reopened = RevocationLedger::open(&dir).unwrap();
            assert!(
                reopened.is_revoked("12345"),
                "revocation must survive a restart"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn revoke_is_idempotent() {
        let dir = tmp_dir("idem");
        let ledger = RevocationLedger::open(&dir).unwrap();
        assert!(ledger.revoke("77").unwrap(), "first is newly-revoked");
        assert!(!ledger.revoke("77").unwrap(), "second is a no-op");
        assert!(ledger.is_revoked("77"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unsafe_id_is_refused_never_written() {
        let dir = tmp_dir("unsafe");
        let ledger = RevocationLedger::open(&dir).unwrap();
        for bad in ["../escape", "a/b", ".", "..", "", "with space"] {
            assert_eq!(ledger.revoke(bad).unwrap_err(), RevocationError::UnsafeId);
            assert!(!ledger.is_revoked(bad));
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn revoked_ids_snapshot_is_sorted() {
        let dir = tmp_dir("snap");
        let ledger = RevocationLedger::open(&dir).unwrap();
        ledger.revoke("30").unwrap();
        ledger.revoke("10").unwrap();
        ledger.revoke("20").unwrap();
        assert_eq!(ledger.revoked_ids(), vec!["10", "20", "30"]);
        std::fs::remove_dir_all(&dir).ok();
    }
}
