//! Disaster recovery controller (WP-E1c, items ⑨ + ⑪).
//!
//! Two failure envelopes, one law: **fail-CLOSED** — an unverifiable recovery
//! is an [`Incident`], never a silent "recovered".
//!
//! - **⑨ GitHub-side loss**: detect App revocation (auth-failure class) and
//!   mirror-repo deletion/rename (404/redirect class). Each → an incident whose
//!   **recovery-source is pinned** (the hugit substrate is authoritative;
//!   re-seed via the [`crate::bootstrap`] cold-seed path), whose **completeness
//!   criterion** is byte-identity of the re-seeded mirror, and which **asserts
//!   resume-from-recovered-state** (continuous verified sync resumes from the
//!   recovered mirror, no data loss).
//! - **⑪ substrate-loss DR** (the marketed reason): induce forge/substrate loss;
//!   prove the continuously-verified **mirror is a byte-complete, WORKING git
//!   repo**; prove full **recovery/resume end-to-end** into a rebuilt substrate;
//!   prove recovered content **imports as change-events** (the E2a boundary),
//!   **never fabricated intents**.

use crate::bootstrap::{SeedProgress, SeedUnit};
use hugit_contracts::EventRecord;
use serde::{Deserialize, Serialize};

// ───────────────────────────── recovery-source pinning ──────────────────────

/// The pinned, authoritative source a recovery re-seeds **from**.
///
/// Pinning is explicit and stated per the contract: every DR path declares its
/// recovery-source so recovery is never ambiguous about authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoverySource {
    /// The hugit substrate (CAS/event-log) is authoritative; re-seed the mirror
    /// from it. This is the pin for GitHub-side loss (⑨).
    HugitSubstrate,
    /// The continuously-verified mirror is authoritative; rebuild the substrate
    /// from it. This is the pin for substrate loss (⑪).
    VerifiedMirror,
}

/// The completeness criterion a recovery must satisfy before it is declared
/// recovered. Byte-identity is the only acceptable proof (fail-CLOSED).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompletenessCriterion {
    /// Recovered side is byte-identical to the pinned source (object-hash equal).
    ByteIdentity,
}

// ───────────────────────────── incidents ────────────────────────────────────

/// Classes of loss the controller detects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum LossKind {
    /// GitHub App installation revoked — auth-failure class (⑨).
    AppRevocation,
    /// Mirror repo deleted or renamed — 404 / redirect (not-found) class (⑨).
    RepoDeletionOrRename,
    /// Forge/substrate lost — the marketed DR trigger (⑪).
    SubstrateLoss,
}

/// A DR incident. Emitting one is the **fail-CLOSED** action: detection never
/// silently self-heals; recovery is an explicit, source-pinned, byte-verified
/// procedure tracked through the incident.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Incident {
    /// What was lost.
    pub kind: LossKind,
    /// Where recovery re-seeds/rebuilds **from** (pinned, explicit).
    pub recovery_source: RecoverySource,
    /// The criterion that must hold before "recovered" may be declared.
    pub completeness: CompletenessCriterion,
    /// Human/audit detail of the detection trigger.
    pub detail: String,
}

// ───────────────────────────── GitHub-side detection (⑨) ────────────────────

/// Observed outcome of a mirror-side GitHub operation, classified for DR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GithubProbe {
    /// Operation succeeded.
    Ok,
    /// Authentication failed (App installation revoked / token rejected).
    AuthFailure { detail: String },
    /// Target not found / moved (repo deleted or renamed; 404 or redirect).
    NotFoundOrRedirect { detail: String },
}

/// Detect GitHub-side loss from a probe. Returns an [`Incident`] (fail-CLOSED)
/// for any non-`Ok` probe; `None` only when the probe is clean.
///
/// Both loss classes pin [`RecoverySource::HugitSubstrate`] (the substrate is
/// authoritative — re-seed via cold-seed) with the byte-identity completeness
/// criterion.
pub fn detect_github_loss(probe: &GithubProbe) -> Option<Incident> {
    match probe {
        GithubProbe::Ok => None,
        GithubProbe::AuthFailure { detail } => Some(Incident {
            kind: LossKind::AppRevocation,
            recovery_source: RecoverySource::HugitSubstrate,
            completeness: CompletenessCriterion::ByteIdentity,
            detail: format!("app revocation / auth failure: {detail}"),
        }),
        GithubProbe::NotFoundOrRedirect { detail } => Some(Incident {
            kind: LossKind::RepoDeletionOrRename,
            recovery_source: RecoverySource::HugitSubstrate,
            completeness: CompletenessCriterion::ByteIdentity,
            detail: format!("repo deletion/rename (404/redirect): {detail}"),
        }),
    }
}

// ───────────────────────────── recovery outcome ─────────────────────────────

/// The proven end-state of a recovery. `Recovered` is only reachable through a
/// byte-identity check; anything else is `Unverifiable` (fail-CLOSED).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryOutcome {
    /// Byte-identity proven; the recovered side may resume verified sync.
    Recovered {
        /// Resume cursor the recovered side continues verified sync from.
        resume_from: SeedProgress,
        /// Refs proven byte-identical during recovery.
        refs_verified: usize,
    },
    /// Could not prove byte-identity — stays an open incident, never "recovered".
    Unverifiable { reason: String },
}

impl RecoveryOutcome {
    /// True only when recovery proved byte-identity end-to-end.
    pub fn is_recovered(&self) -> bool {
        matches!(self, RecoveryOutcome::Recovered { .. })
    }
}

/// Recover from a **GitHub-side** loss (⑨): re-seed the fresh/renamed mirror
/// from the pinned hugit substrate (`source_units`), then assert byte-identity
/// completeness and a resume cursor.
///
/// `incident` carries the pinned source + criterion. `mirror_readback` is the
/// hash the rebuilt mirror reports per ref — recovery is verified by readback,
/// never by fiat. Any mismatch → [`RecoveryOutcome::Unverifiable`] (fail-CLOSED).
pub fn recover_github_loss(
    incident: &Incident,
    source_units: &[SeedUnit],
    mut mirror_readback: impl FnMut(&SeedUnit) -> String,
) -> RecoveryOutcome {
    debug_assert!(matches!(
        incident.recovery_source,
        RecoverySource::HugitSubstrate
    ));
    let mut progress = SeedProgress::default();
    for unit in source_units {
        let got = mirror_readback(unit);
        if got != unit.object_hash {
            return RecoveryOutcome::Unverifiable {
                reason: format!(
                    "re-seeded mirror not byte-identical at {}: source={} mirror={} (fail-closed)",
                    unit.refname, unit.object_hash, got
                ),
            };
        }
        progress.verified_refs.push(unit.refname.clone());
        progress.last_verified = Some(unit.refname.clone());
        progress.pack_offset = unit.pack_offset;
    }
    RecoveryOutcome::Recovered {
        refs_verified: progress.verified_refs.len(),
        resume_from: progress,
    }
}

// ───────────────────────────── substrate-loss DR (⑪) ────────────────────────

/// A change-event projected from recovered git content during substrate-loss
/// recovery — the E2a import boundary, modeled here for the DR driver.
///
/// Recovered commit history materializes as **opaque change-events**
/// ([`EventRecord`] with [`CHANGE_EVENT_KIND`]). It is a hard error to fabricate
/// an *intent* from a bare recovered commit (cf. E2⑤, the no-fake-intents law).
pub const CHANGE_EVENT_KIND: &str = "mirror.recovered.change_event";

/// Kinds that would be a forbidden fabrication if minted from a bare commit.
const FORBIDDEN_FABRICATED_KINDS: &[&str] = &["intent", "proposed_intent", "synthesized_intent"];

/// Error from the substrate-loss import boundary.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ImportBoundaryError {
    #[error(
        "fabricated intent from bare recovered commit is forbidden (kind={kind}); cf. E2⑤ no-fake-intents law"
    )]
    FabricatedIntent { kind: String },
}

/// Project one recovered commit (`refname` @ `object_hash`, ordered by `seq`)
/// into an opaque change-event [`EventRecord`].
///
/// This is the import boundary: it can only ever emit a change-event, never an
/// intent. Callers that try to import recovered content under an intent kind hit
/// [`import_as_change_event`]'s guard.
pub fn project_recovered_commit(
    seq: u64,
    prev_hash: &str,
    object_hash: &str,
    principal_chain: Vec<String>,
    payload: String,
    recorded_at: u64,
) -> EventRecord {
    EventRecord {
        seq,
        prev_hash: prev_hash.to_string(),
        this_hash: object_hash.to_string(),
        kind: CHANGE_EVENT_KIND.to_string(),
        principal_chain,
        payload,
        recorded_at,
    }
}

/// Import a recovered record through the change-event boundary.
///
/// Accepts only change-events. If the record's `kind` is an intent-class kind,
/// this is a fabrication attempt and is rejected (fail-CLOSED) — recovered
/// content is **never** allowed in as a synthesized intent.
pub fn import_as_change_event(record: &EventRecord) -> Result<(), ImportBoundaryError> {
    if FORBIDDEN_FABRICATED_KINDS.iter().any(|k| record.kind == *k) {
        return Err(ImportBoundaryError::FabricatedIntent {
            kind: record.kind.clone(),
        });
    }
    Ok(())
}

/// A minimal model of the mirror as a **working git repository** — the
/// substrate-loss invariant: after forge loss the mirror alone supports
/// clone/log/checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkingGitRepo {
    /// Path on disk to a real, complete `.git` (clone target).
    pub path: std::path::PathBuf,
    /// Refs present (clone advertises these).
    pub refs: Vec<String>,
    /// Whether HEAD resolves (checkout works).
    pub head_resolves: bool,
}

impl WorkingGitRepo {
    /// The mirror is a usable working repo iff a clone/log/checkout would all
    /// succeed: it has refs and a resolvable HEAD.
    pub fn is_working(&self) -> bool {
        !self.refs.is_empty() && self.head_resolves
    }
}

/// Drive **substrate-loss recovery end-to-end** (⑪): from the pinned verified
/// mirror, rebuild the substrate, prove byte-identity, and yield a resume
/// cursor — fail-CLOSED on any unverifiable unit.
///
/// `mirror`: the surviving working-git mirror (must be working).
/// `source_units`: the mirror's refs+hashes (the recovery source).
/// `substrate_readback`: hash the rebuilt substrate reports per ref.
pub fn recover_substrate_loss(
    mirror: &WorkingGitRepo,
    source_units: &[SeedUnit],
    mut substrate_readback: impl FnMut(&SeedUnit) -> String,
) -> RecoveryOutcome {
    // Precondition: the recovery source must itself be a working git repo, else
    // recovery is unverifiable from the start (fail-CLOSED).
    if !mirror.is_working() {
        return RecoveryOutcome::Unverifiable {
            reason: "mirror is not a working git repo; cannot recover (fail-closed)".into(),
        };
    }
    let mut progress = SeedProgress::default();
    for unit in source_units {
        let got = substrate_readback(unit);
        if got != unit.object_hash {
            return RecoveryOutcome::Unverifiable {
                reason: format!(
                    "rebuilt substrate not byte-identical at {}: mirror={} substrate={} (fail-closed)",
                    unit.refname, unit.object_hash, got
                ),
            };
        }
        progress.verified_refs.push(unit.refname.clone());
        progress.last_verified = Some(unit.refname.clone());
        progress.pack_offset = unit.pack_offset;
    }
    RecoveryOutcome::Recovered {
        refs_verified: progress.verified_refs.len(),
        resume_from: progress,
    }
}

/// Build the substrate-loss incident with its pinned source + criterion.
pub fn substrate_loss_incident(detail: impl Into<String>) -> Incident {
    Incident {
        kind: LossKind::SubstrateLoss,
        recovery_source: RecoverySource::VerifiedMirror,
        completeness: CompletenessCriterion::ByteIdentity,
        detail: detail.into(),
    }
}
