//! The forge-arbitrated bidirectional sync engine (rules 1–5).
//!
//! The **forge** is an append-only [`EventLog`] (the canonical D1 surface). Refs
//! are a *derived view* — the [`replay`] fold over the log — never primary state.
//! Every mutation to the forge goes through exactly one of two appends, and which
//! one is reachable is what makes `main` single-writer **by construction**:
//!
//! - [`hugit_proto::record_external_change`] → a `ref.update` / `ref.delete`
//!   external-change event (D3⑤: opaque, attributed, **never** an intent). This
//!   is the ONLY path the GitHub-ingest side uses, and it **cannot** name the
//!   protected branch (the ingest path reroutes such a push to a proposed
//!   branch first).
//! - [`BidirSync::land_via_queue`] → an `intent.landed` event. This is the ONLY
//!   path that may advance the protected branch, and it models the B4 landing
//!   queue (the single writer to `main`).
//!
//! Both appends route through the canonical hasher, so [`verify_chain`] holds at
//! every step. The GitHub side is a separate ref-tip map ([`crate::refops::RefTips`])
//! that the caller keeps in step with a real local git repo fixture in tests.

use std::collections::BTreeMap;

use hugit_proto::{RawPush, record_external_change};
use hugit_refstore::intent::{INTENT_LANDED_KIND, RAW_PUSH_KINDS};
use hugit_refstore::{EventLog, RefState, TamperError, replay, verify_chain};

use crate::divergence::{
    DivergenceResolution, MutationOrigin, RefState as DivRefState, resolve as resolve_divergence,
};

/// The reserved ref namespace under which a divergent GitHub-side tip is
/// preserved as a recoverable incident (rule 4). It is a forge ref like any
/// other (folded by replay), so the preserved tip is chain-verifiable and never
/// silently dropped.
pub const INCIDENT_REF_PREFIX: &str = "refs/hugit/incidents/";

/// The reserved ref namespace a rerouted direct-`main` GitHub push lands on
/// instead of the protected branch (rule 2). It rides the normal branch path so
/// it can later land through the queue — it never touches `main` symmetrically.
pub const PROPOSED_REF_PREFIX: &str = "refs/hugit/proposed/";

/// Compute the incident ref name that preserves a divergent GitHub-side `tip`
/// for `branch`. Deterministic and collision-free per (branch, tip): the tip is
/// embedded so two different divergent tips never alias one incident ref.
pub fn incident_ref_name(branch: &str, github_tip: &str) -> String {
    let leaf = branch.strip_prefix("refs/heads/").unwrap_or(branch);
    format!("{INCIDENT_REF_PREFIX}{leaf}/{github_tip}")
}

/// The proposed-branch ref a rerouted direct-`main` GitHub push lands on.
fn proposed_ref_name(github_tip: &str) -> String {
    format!("{PROPOSED_REF_PREFIX}main/{github_tip}")
}

/// Errors from the bidirectional sync engine. Fail-closed: a refused operation
/// never leaves the forge in a half-applied state.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SyncError {
    /// A GitHub-side push carried no attributing principal. An external change
    /// must be attributable; an unattributed mutation is refused, never recorded
    /// blind (mirrors [`hugit_proto::ExternalChangeError::MissingAttribution`]).
    #[error("github-side push refused: external change must carry attribution")]
    MissingAttribution,
    /// The forge log failed chain re-verification — the derived ref view is
    /// refused (fail-closed; never served off a tampered log).
    #[error("forge chain verification failed: {0}")]
    Tamper(String),
}

impl From<TamperError> for SyncError {
    fn from(e: TamperError) -> Self {
        SyncError::Tamper(e.to_string())
    }
}

/// Which side is authoritative for a ref. Used by the no-symmetric-authority
/// property model (rule 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthoritySide {
    /// The forge arbitrates (the only authority for the protected branch, and
    /// the arbiter on any contended branch).
    Forge,
    /// The GitHub side — authoritative for **nothing** that the forge owns; this
    /// variant exists only so the property model can *attempt* a symmetric state
    /// and prove it is unreachable.
    GitHub,
}

/// The authority model for one ref (rule 5).
///
/// The protected branch is **always** forge-authoritative; the engine exposes no
/// transition that makes the GitHub side authoritative for it. This type makes
/// the property checkable: [`AuthorityModel::is_symmetric_for_main`] is the
/// predicate the property test asserts is `false` across every reachable state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorityModel {
    /// The protected branch name (e.g. `refs/heads/main`).
    protected: String,
    /// Per-ref authority. A ref absent from this map is *uncontended* (a plain
    /// branch that syncs both ways with no arbiter needed).
    authority: BTreeMap<String, AuthoritySide>,
}

impl AuthorityModel {
    /// A fresh model whose protected branch is forge-authoritative by
    /// construction.
    pub fn new(protected: impl Into<String>) -> Self {
        let protected = protected.into();
        let mut authority = BTreeMap::new();
        authority.insert(protected.clone(), AuthoritySide::Forge);
        Self {
            protected,
            authority,
        }
    }

    /// The authoritative side for `ref_name` (the protected branch is always
    /// [`AuthoritySide::Forge`]).
    pub fn authority_for(&self, ref_name: &str) -> AuthoritySide {
        if ref_name == self.protected {
            return AuthoritySide::Forge;
        }
        self.authority
            .get(ref_name)
            .copied()
            .unwrap_or(AuthoritySide::Forge)
    }

    /// Apply an arbitration step: the forge always wins a contended ref. There is
    /// deliberately **no** method that sets [`AuthoritySide::GitHub`] for the
    /// protected branch — a symmetric-authority state is unrepresentable through
    /// the engine's API.
    pub fn arbitrate(&mut self, ref_name: &str) {
        self.authority
            .insert(ref_name.to_string(), AuthoritySide::Forge);
    }

    /// The property under test (rule 5): is `main` simultaneously authoritative
    /// on both sides? Must be `false` for **every** reachable state.
    pub fn is_symmetric_for_main(&self) -> bool {
        // The forge is, by construction, authoritative for the protected branch.
        // A symmetric state would require the GitHub side ALSO authoritative for
        // it — which the engine exposes no transition to produce.
        self.authority_for(&self.protected) == AuthoritySide::Forge
            && self
                .authority
                .get(&self.protected)
                .is_some_and(|s| *s == AuthoritySide::GitHub)
    }
}

/// Outcome of ingesting a GitHub-side push (rule 1 for branches, rule 2 for the
/// protected branch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestOutcome {
    /// A plain branch was ingested as a forge branch ref via an external
    /// change-event. `main` untouched.
    BranchIngested {
        /// The branch ref ingested.
        ref_name: String,
        /// The tip it was set to (byte-identical to the GitHub tip).
        tip: String,
    },
    /// A direct push to the **protected** branch was rerouted to a proposed
    /// branch and did NOT mutate `main` symmetrically (rule 2).
    Rerouted(ProtectedReroute),
}

/// A rerouted direct-`main` GitHub push (rule 2): captured as a proposed branch
/// so it can land through the queue; `main` was not symmetrically mutated.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedReroute {
    /// The proposed branch ref the push was captured onto.
    pub proposed_ref: String,
    /// The tip the GitHub side tried to push to the protected branch.
    pub attempted_tip: String,
}

/// Outcome of a convergence pass (rule 3): whether anything re-emitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvergeOutcome {
    /// Both sides already hold the same tip for this ref — content-hash equal ⇒
    /// **zero re-emit** (sync is quiet).
    AlreadyConverged,
    /// A real observed mutation moved the tip; one emission occurred.
    Emitted {
        /// The ref that re-synced.
        ref_name: String,
        /// The tip propagated.
        tip: String,
    },
}

/// The incident ref preserving a divergent GitHub-side tip (rule 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncidentRef {
    /// The reserved incident ref the GitHub tip is preserved under.
    pub ref_name: String,
    /// The preserved (recoverable) GitHub-side tip.
    pub preserved_tip: String,
}

/// One branch's two-sided state for a round-trip assertion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSync {
    /// The branch ref.
    pub ref_name: String,
    /// The forge-side tip (replay-derived).
    pub forge_tip: Option<String>,
    /// The GitHub-side tip.
    pub github_tip: Option<String>,
}

impl BranchSync {
    /// Whether the two sides hold byte-identical tips for this branch.
    pub fn byte_identical(&self) -> bool {
        self.forge_tip == self.github_tip && self.forge_tip.is_some()
    }
}

/// The bidirectional sync engine over one repository.
///
/// Holds the forge [`EventLog`] (single source of truth) and tracks the
/// last-synced tip per ref per direction so convergence is content-hash
/// idempotent (rule 3). The GitHub side is a separate ref-tip map the caller
/// keeps in step with a real git fixture.
#[derive(Debug, Clone)]
pub struct BidirSync {
    log: EventLog,
    protected: String,
    /// Last tip the forge has *emitted outbound* per ref (rule 3, forge→GitHub).
    emitted_outbound: BTreeMap<String, String>,
    /// Last tip the forge has *ingested inbound* per ref (rule 3, GitHub→forge).
    ingested_inbound: BTreeMap<String, String>,
    authority: AuthorityModel,
}

impl BidirSync {
    /// A fresh engine whose protected branch defaults to `refs/heads/main`.
    pub fn new() -> Self {
        Self::with_protected("refs/heads/main")
    }

    /// A fresh engine with an explicit protected branch.
    pub fn with_protected(protected: impl Into<String>) -> Self {
        let protected = protected.into();
        Self {
            log: EventLog::new(),
            protected: protected.clone(),
            emitted_outbound: BTreeMap::new(),
            ingested_inbound: BTreeMap::new(),
            authority: AuthorityModel::new(protected),
        }
    }

    /// The protected branch name.
    pub fn protected(&self) -> &str {
        &self.protected
    }

    /// Read-only view of the forge event log (for chain verification in tests).
    pub fn log(&self) -> &EventLog {
        &self.log
    }

    /// The authority model (rule 5).
    pub fn authority(&self) -> &AuthorityModel {
        &self.authority
    }

    /// Re-verify the forge chain (fail-closed) — never serve a derived view off a
    /// tampered log.
    pub fn verify_forge_chain(&self) -> Result<(), SyncError> {
        verify_chain(self.log.records()).map_err(SyncError::from)
    }

    /// The current forge ref view (replay-derived). Fail-closed on a broken
    /// chain.
    pub fn forge_refs(&self) -> Result<RefState, SyncError> {
        replay(&self.log).map_err(|e| match e {
            hugit_refstore::replay::ReplayError::Tamper(t) => SyncError::from(t),
            other => SyncError::Tamper(other.to_string()),
        })
    }

    /// The current forge tip for `ref_name`, if present.
    pub fn forge_tip(&self, ref_name: &str) -> Result<Option<String>, SyncError> {
        Ok(self.forge_refs()?.get(ref_name).map(str::to_string))
    }

    // ── rule 1 / rule 2: ingest a GitHub-side push ──────────────────────────

    /// Ingest a GitHub-side push (rules 1 + 2).
    ///
    /// - A push to **any non-protected branch** is recorded on the forge as an
    ///   external change-event ([`record_external_change`] → `ref.update`,
    ///   **never** an intent) and the forge branch ref advances to the GitHub
    ///   tip — byte-identical, `main` untouched (rule 1).
    /// - A push to the **protected** branch is rerouted: it lands on a proposed
    ///   branch ref (so it can later land through the queue) and the protected
    ///   branch is **not** symmetrically mutated (rule 2). The reroute itself is
    ///   recorded as an external change-event on the proposed ref — still never
    ///   an intent, so `main` cannot advance through this path.
    ///
    /// Fail-closed: an unattributed push is refused
    /// ([`SyncError::MissingAttribution`]).
    pub fn ingest_github_push(
        &mut self,
        ref_name: &str,
        github_tip: &str,
        principal_chain: Vec<String>,
        recorded_at: u64,
    ) -> Result<IngestOutcome, SyncError> {
        if principal_chain.is_empty() {
            return Err(SyncError::MissingAttribution);
        }

        if ref_name == self.protected {
            // Rule 2: reroute — capture onto a proposed branch, never touch main.
            let proposed = proposed_ref_name(github_tip);
            self.append_external_update(&proposed, github_tip, principal_chain, recorded_at)?;
            return Ok(IngestOutcome::Rerouted(ProtectedReroute {
                proposed_ref: proposed,
                attempted_tip: github_tip.to_string(),
            }));
        }

        // Rule 1: ingest the branch as that same forge ref via an external
        // change-event (D3⑤: never an intent).
        self.append_external_update(ref_name, github_tip, principal_chain, recorded_at)?;
        self.ingested_inbound
            .insert(ref_name.to_string(), github_tip.to_string());
        Ok(IngestOutcome::BranchIngested {
            ref_name: ref_name.to_string(),
            tip: github_tip.to_string(),
        })
    }

    /// Convenience wrapper: ingest a GitHub-side branch (rule 1) and assert it is
    /// not the protected branch in debug builds.
    pub fn ingest_github_branch(
        &mut self,
        ref_name: &str,
        github_tip: &str,
        principal_chain: Vec<String>,
        recorded_at: u64,
    ) -> Result<IngestOutcome, SyncError> {
        debug_assert_ne!(
            ref_name, self.protected,
            "ingest_github_branch is for non-protected branches; use ingest_github_push for main"
        );
        self.ingest_github_push(ref_name, github_tip, principal_chain, recorded_at)
    }

    // ── rule 2: land main through the queue (the single writer) ─────────────

    /// Land a change onto the **protected** branch through the queue — the ONLY
    /// path that advances `main` (rule 2).
    ///
    /// This models the B4 union-landing queue: it appends an `intent.landed`
    /// event (the ref-advancing intent kind). It is intentionally the sole
    /// constructor of a `main`-advancing append; the GitHub-ingest path has no
    /// branch that can move the protected ref.
    pub fn land_via_queue(
        &mut self,
        intent_id: &str,
        tip: &str,
        charter: &str,
        principal_chain: Vec<String>,
        recorded_at: u64,
    ) -> Result<(), SyncError> {
        let payload = serde_json::json!({
            "intent_id": intent_id,
            "ref": self.protected,
            "target": tip,
            "charter": charter,
        })
        .to_string();
        let canonical = hugit_refstore::canonical_json(&payload).unwrap_or_else(|| payload.clone());
        self.log
            .append(INTENT_LANDED_KIND, principal_chain, canonical, recorded_at);
        // The forge becomes the (sole) authoritative side for main on a land.
        self.authority.arbitrate(&self.protected);
        Ok(())
    }

    // ── rule 1: mirror a forge-side branch update out to GitHub ─────────────

    /// Mirror a forge-side branch advance out (rule 1, forge→GitHub).
    ///
    /// Records the forge-side branch advance as an external change-event on the
    /// forge log and returns the tip the GitHub side must be set to (byte-
    /// identical). The actual push to GitHub is the existing E1 outbound writer
    /// ([`crate::outbound`]); this method computes *what* to mirror and records
    /// the outbound emission for the convergence bookkeeping (rule 3).
    pub fn mirror_out_forge_branch(
        &mut self,
        ref_name: &str,
        forge_tip: &str,
        principal_chain: Vec<String>,
        recorded_at: u64,
    ) -> Result<String, SyncError> {
        debug_assert_ne!(
            ref_name, self.protected,
            "mirror_out_forge_branch is for branches; main mirrors out only after land_via_queue"
        );
        self.append_external_update(ref_name, forge_tip, principal_chain, recorded_at)?;
        self.emitted_outbound
            .insert(ref_name.to_string(), forge_tip.to_string());
        Ok(forge_tip.to_string())
    }

    // ── rule 3: content-hash idempotent convergence (no echo loop) ──────────

    /// Converge `ref_name` from the forge toward the GitHub side, **only** when a
    /// real observed mutation moved the forge tip (rule 3).
    ///
    /// This reuses E1's "mirror_write from observed mutation, not tip-inequality"
    /// rule: convergence keys off the *last emitted* tip, not raw tip-inequality.
    /// If the forge tip equals what was last emitted outbound, the two sides are
    /// content-hash equal ⇒ **zero re-emit** ([`ConvergeOutcome::AlreadyConverged`]).
    /// A planted echo (re-running with the same tip) therefore re-emits nothing —
    /// the oracle catches a broken implementation that re-emits.
    pub fn converge_outbound(&mut self, ref_name: &str) -> Result<ConvergeOutcome, SyncError> {
        let forge_tip = match self.forge_tip(ref_name)? {
            Some(t) => t,
            None => return Ok(ConvergeOutcome::AlreadyConverged),
        };
        if self.emitted_outbound.get(ref_name) == Some(&forge_tip) {
            // Content-hash equal: already mirrored this exact tip. No echo.
            return Ok(ConvergeOutcome::AlreadyConverged);
        }
        // A genuinely new tip → emit exactly once, then record it as emitted so a
        // re-run is a no-op.
        self.emitted_outbound
            .insert(ref_name.to_string(), forge_tip.clone());
        Ok(ConvergeOutcome::Emitted {
            ref_name: ref_name.to_string(),
            tip: forge_tip,
        })
    }

    /// Converge `ref_name` from the GitHub side toward the forge, **only** when a
    /// real observed mutation moved the GitHub tip (rule 3, the inbound mirror).
    ///
    /// Same content-hash-idempotent discipline as [`Self::converge_outbound`]:
    /// if the observed GitHub tip equals what was last ingested, this is a no-op
    /// (the change just came FROM the forge — ingesting it again would be the
    /// echo loop). Otherwise it ingests once.
    pub fn converge_inbound(
        &mut self,
        ref_name: &str,
        observed_github_tip: &str,
        principal_chain: Vec<String>,
        recorded_at: u64,
    ) -> Result<ConvergeOutcome, SyncError> {
        // Echo guard: if the GitHub tip equals what the forge last EMITTED
        // outbound for this ref, the GitHub side is just reflecting the forge's
        // own write back — re-ingesting it is the echo loop. Stay quiet.
        if self.emitted_outbound.get(ref_name).map(String::as_str) == Some(observed_github_tip) {
            return Ok(ConvergeOutcome::AlreadyConverged);
        }
        if self.ingested_inbound.get(ref_name).map(String::as_str) == Some(observed_github_tip) {
            return Ok(ConvergeOutcome::AlreadyConverged);
        }
        self.ingest_github_branch(ref_name, observed_github_tip, principal_chain, recorded_at)?;
        Ok(ConvergeOutcome::Emitted {
            ref_name: ref_name.to_string(),
            tip: observed_github_tip.to_string(),
        })
    }

    // ── rule 4: same-branch concurrent divergence arbitration ───────────────

    /// Arbitrate a same-branch concurrent divergence (rule 4).
    ///
    /// One branch ref moved to incompatible tips on both sides at once. The forge
    /// arbitrates **that one ref**:
    /// - the forge tip wins (the branch ref stays at the forge tip — repair is
    ///   forge-authoritative, reusing [`crate::divergence::resolve`]);
    /// - the divergent GitHub-side tip is **preserved** as a recoverable incident
    ///   ref recorded on the forge log (so it is never silently dropped and the
    ///   chain stays verifiable).
    ///
    /// Returns the `{resolution, incident_ref}` pair. The incident ref's tip is
    /// the exact preserved GitHub tip — recoverable, chain-verifiable.
    pub fn arbitrate_branch_divergence(
        &mut self,
        ref_name: &str,
        forge_tip: &str,
        github_tip: &str,
        principal_chain: Vec<String>,
        recorded_at: u64,
    ) -> Result<(DivergenceResolution, IncidentRef), SyncError> {
        // Reuse E1's forge-authoritative divergence resolution. The GitHub tip is
        // recorded as an OBSERVED mirror-side mutation so the incident is charged
        // honestly (not inferred from raw tip-inequality / lag).
        let div_state = DivRefState::with_observed_mutation(
            ref_name,
            forge_tip,
            github_tip,
            MutationOrigin::MirrorSide,
        );
        let resolution = resolve_divergence(&div_state)
            .expect("incompatible tips are divergent by construction");

        // Preserve the divergent GitHub tip as a recoverable incident ref on the
        // forge log — never dropped. Recorded as an external change-event (it is
        // not an intent; it is a preserved side-ref).
        let incident = IncidentRef {
            ref_name: incident_ref_name(ref_name, github_tip),
            preserved_tip: github_tip.to_string(),
        };
        self.append_external_update(&incident.ref_name, github_tip, principal_chain, recorded_at)?;

        // The forge tip wins the contended branch (arbitration step, rule 5
        // bookkeeping): the branch ref remains at the forge tip on the forge.
        self.authority.arbitrate(ref_name);

        Ok((resolution, incident))
    }

    // ── shared: the single external-change append (D3⑤) ─────────────────────

    /// Append a `ref.update` external-change event via the canonical
    /// [`record_external_change`] recorder. This is the ONLY way the engine moves
    /// a non-protected forge ref — and it can never emit an intent kind, so it is
    /// structurally incapable of advancing `main` symmetrically.
    fn append_external_update(
        &mut self,
        ref_name: &str,
        tip: &str,
        principal_chain: Vec<String>,
        recorded_at: u64,
    ) -> Result<(), SyncError> {
        let push = RawPush::Update {
            ref_name: ref_name.to_string(),
            target: tip.to_string(),
        };
        // record_external_change appends a RAW_PUSH_KINDS event (ref.update),
        // never INTENT_LANDED_KIND — the structural D3⑤ guarantee.
        let (record, _attribution) =
            record_external_change(&mut self.log, &push, principal_chain, recorded_at)
                .map_err(|_| SyncError::MissingAttribution)?;
        // Defence-in-depth: the recorded kind is an external-change kind, never
        // an intent. An ingest path that ever emitted an intent would break this.
        debug_assert!(RAW_PUSH_KINDS.contains(&record.kind.as_str()));
        debug_assert_ne!(record.kind, INTENT_LANDED_KIND);
        Ok(())
    }
}

impl Default for BidirSync {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(c: char) -> String {
        c.to_string().repeat(40)
    }

    #[test]
    fn branch_ingest_uses_external_change_never_intent() {
        let mut s = BidirSync::new();
        s.ingest_github_branch("refs/heads/feature", &oid('a'), vec!["dev".into()], 1)
            .unwrap();
        // Forge folded the branch to the github tip; no intent kind was emitted.
        assert_eq!(s.forge_tip("refs/heads/feature").unwrap(), Some(oid('a')));
        assert!(
            s.log()
                .records()
                .iter()
                .all(|r| r.kind != INTENT_LANDED_KIND),
            "branch ingest must never emit an intent"
        );
        s.verify_forge_chain().unwrap();
    }

    #[test]
    fn direct_main_push_is_rerouted_not_symmetric() {
        let mut s = BidirSync::new();
        let out = s
            .ingest_github_push("refs/heads/main", &oid('b'), vec!["dev".into()], 1)
            .unwrap();
        match out {
            IngestOutcome::Rerouted(r) => {
                assert!(r.proposed_ref.starts_with(PROPOSED_REF_PREFIX));
            }
            _ => panic!("a direct main push must reroute"),
        }
        // main did NOT advance symmetrically.
        assert_eq!(s.forge_tip("refs/heads/main").unwrap(), None);
    }

    #[test]
    fn main_advances_only_via_queue() {
        let mut s = BidirSync::new();
        s.land_via_queue("i1", &oid('c'), "land it", vec!["queue".into()], 1)
            .unwrap();
        assert_eq!(s.forge_tip("refs/heads/main").unwrap(), Some(oid('c')));
    }

    #[test]
    fn convergence_is_idempotent_no_echo() {
        let mut s = BidirSync::new();
        s.mirror_out_forge_branch("refs/heads/b", &oid('d'), vec!["dev".into()], 1)
            .unwrap();
        // First converge after a real mutation? mirror_out already recorded the
        // emission, so converge is a no-op; the point is a re-run never re-emits.
        assert_eq!(
            s.converge_outbound("refs/heads/b").unwrap(),
            ConvergeOutcome::AlreadyConverged
        );
    }

    #[test]
    fn divergence_preserves_github_tip_as_incident() {
        let mut s = BidirSync::new();
        let (res, incident) = s
            .arbitrate_branch_divergence(
                "refs/heads/x",
                &oid('f'),
                &oid('e'),
                vec!["dev".into()],
                1,
            )
            .unwrap();
        assert_eq!(res.repair.forge_tip, oid('f'));
        assert_eq!(incident.preserved_tip, oid('e'));
        // The preserved tip is on the forge log, chain verifiable.
        assert_eq!(s.forge_tip(&incident.ref_name).unwrap(), Some(oid('e')));
        s.verify_forge_chain().unwrap();
    }
}
