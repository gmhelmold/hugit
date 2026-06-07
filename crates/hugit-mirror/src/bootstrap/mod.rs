//! Cold-seed bootstrap (WP-E1c, item ⑧).
//!
//! Replicates **full existing history** from the hugit substrate to a **fresh**
//! GitHub mirror repo via the E1a push+verify pipeline, **resumable mid-seed**,
//! with the final state **hash-verified to byte-identity**.
//!
//! Design (pre-decided by the contract):
//! - Seed proceeds ref-by-ref / pack-window-by-window; after each verified unit
//!   the [`SeedProgress`] cursor (last verified ref + pack offset) is persisted
//!   so an interrupted seed **resumes from the cursor**, never from scratch.
//! - Every seeded unit is **byte-identity hash-verified** against the source
//!   before the cursor advances. An unverifiable unit is fail-CLOSED: the seed
//!   stops and surfaces the mismatch — never a silent "seeded".
//! - The push+verify transport itself is E1a's pipeline; this module models the
//!   seed *driver* (ordering, resume cursor, byte-identity gate) and consumes a
//!   [`SeedTransport`] so the E1a outbound writer can be plugged in unchanged.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One unit of source history to seed: a ref pointing at a content hash, with
/// the ordered object/pack bytes that ref transitively requires.
///
/// `object_hash` is the byte-identity criterion — the SHA of the content git
/// stores for this ref. Cold-seed declares the mirror complete only when the
/// mirror reports the **same** `object_hash` back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedUnit {
    /// Fully-qualified ref name, e.g. `refs/heads/main`.
    pub refname: String,
    /// Byte-identity content hash of the object graph this ref points at.
    pub object_hash: String,
    /// Offset (in packed bytes) of this unit within the full seed stream — the
    /// resume coordinate persisted alongside the ref.
    pub pack_offset: u64,
}

/// Persisted seed cursor — the resume position of an in-flight cold-seed.
///
/// Holds the **last verified ref** and the **pack offset** reached, so a seed
/// interrupted at any point resumes from exactly here. Persisted (serde) after
/// every verified unit; on restart the driver replays only units **after** the
/// cursor.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeedProgress {
    /// Last ref proven byte-identical on the mirror (`None` = nothing seeded).
    pub last_verified: Option<String>,
    /// Pack offset reached for the last verified unit.
    pub pack_offset: u64,
    /// Refs proven byte-identical, in seed order (idempotency / audit trail).
    pub verified_refs: Vec<String>,
}

impl SeedProgress {
    /// True once `refname` has been proven byte-identical on the mirror.
    pub fn is_verified(&self, refname: &str) -> bool {
        self.verified_refs.iter().any(|r| r == refname)
    }

    /// Advance the cursor after a unit is byte-identity verified.
    fn record(&mut self, unit: &SeedUnit) {
        if !self.is_verified(&unit.refname) {
            self.verified_refs.push(unit.refname.clone());
        }
        self.last_verified = Some(unit.refname.clone());
        self.pack_offset = unit.pack_offset;
    }
}

/// Push+verify transport seam (E1a's outbound writer, plugged in here).
///
/// `push` replicates a unit to the fresh mirror and returns the content hash
/// the **mirror** reports for it. Cold-seed compares it against the source
/// `object_hash` to assert byte-identity — verification is by the receiver's
/// own readback, never asserted by fiat.
pub trait SeedTransport {
    /// Push one unit to the mirror; return the mirror-side content hash.
    fn push(&mut self, unit: &SeedUnit) -> Result<String, SeedError>;
}

/// An in-memory transport used by fixtures: records pushes and echoes the
/// source hash, optionally corrupting one ref to exercise the fail-CLOSED gate
/// and failing once at a chosen ref to exercise resume.
#[derive(Debug, Default)]
pub struct MockSeedTransport {
    /// Refs the mirror would report with a *wrong* hash (corruption).
    pub corrupt: BTreeMap<String, String>,
    /// Push exactly once at this ref before succeeding (transient outage).
    pub fail_once_at: Option<String>,
    /// Refs actually delivered to the mirror, in order (observability).
    pub delivered: Vec<String>,
    failed: bool,
}

impl MockSeedTransport {
    /// A transport that fails exactly once at `refname` (then succeeds) — used
    /// to exercise resumable mid-seed.
    pub fn fail_once_at(refname: impl Into<String>) -> Self {
        MockSeedTransport {
            fail_once_at: Some(refname.into()),
            ..Default::default()
        }
    }

    /// A transport that reports a wrong hash for `refname` — used to exercise
    /// the fail-CLOSED byte-identity gate.
    pub fn with_corruption(refname: impl Into<String>, wrong_hash: impl Into<String>) -> Self {
        let mut t = MockSeedTransport::default();
        t.corrupt.insert(refname.into(), wrong_hash.into());
        t
    }
}

impl SeedTransport for MockSeedTransport {
    fn push(&mut self, unit: &SeedUnit) -> Result<String, SeedError> {
        if self.fail_once_at.as_deref() == Some(unit.refname.as_str()) && !self.failed {
            self.failed = true;
            return Err(SeedError::Transport {
                refname: unit.refname.clone(),
                detail: "transient push failure (resumable)".into(),
            });
        }
        self.delivered.push(unit.refname.clone());
        Ok(self
            .corrupt
            .get(&unit.refname)
            .cloned()
            .unwrap_or_else(|| unit.object_hash.clone()))
    }
}

/// Cold-seed outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeedOutcome {
    /// Full history seeded and **byte-identity verified** to the source.
    Complete {
        /// Total refs proven byte-identical.
        refs_verified: usize,
    },
    /// Seed was interrupted; `progress` is the resume cursor (no data lost).
    Interrupted {
        /// Persisted resume cursor.
        progress: SeedProgress,
        /// The ref the interruption occurred at.
        at: String,
    },
}

/// Cold-seed errors. Byte-identity mismatch is **fail-CLOSED** — never silent.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum SeedError {
    #[error(
        "byte-identity verification FAILED for {refname}: source={source_hash} mirror={mirror_hash} (fail-closed)"
    )]
    ByteIdentityMismatch {
        refname: String,
        source_hash: String,
        mirror_hash: String,
    },
    #[error("transport failure at {refname}: {detail}")]
    Transport { refname: String, detail: String },
}

/// Drive a cold-seed of `units` (full source history, in seed order) into a
/// fresh mirror via `transport`, **resuming from** `progress`.
///
/// Returns [`SeedOutcome::Complete`] only when every unit is proven
/// byte-identical on the mirror. A transport failure returns
/// [`SeedOutcome::Interrupted`] with the persisted cursor (caller re-invokes
/// with that cursor to resume). A byte-identity mismatch is fail-CLOSED:
/// [`SeedError::ByteIdentityMismatch`].
pub fn cold_seed<T: SeedTransport>(
    units: &[SeedUnit],
    transport: &mut T,
    progress: &mut SeedProgress,
) -> Result<SeedOutcome, SeedError> {
    for unit in units {
        // Resume: skip units already proven byte-identical.
        if progress.is_verified(&unit.refname) {
            continue;
        }
        let mirror_hash = match transport.push(unit) {
            Ok(h) => h,
            Err(SeedError::Transport { refname, .. }) => {
                return Ok(SeedOutcome::Interrupted {
                    progress: progress.clone(),
                    at: refname,
                });
            }
            Err(e) => return Err(e),
        };
        // Byte-identity gate (fail-CLOSED): mirror readback must equal source.
        if mirror_hash != unit.object_hash {
            return Err(SeedError::ByteIdentityMismatch {
                refname: unit.refname.clone(),
                source_hash: unit.object_hash.clone(),
                mirror_hash,
            });
        }
        progress.record(unit);
    }
    Ok(SeedOutcome::Complete {
        refs_verified: progress.verified_refs.len(),
    })
}
