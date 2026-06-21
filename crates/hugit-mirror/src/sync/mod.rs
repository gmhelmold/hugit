//! Seamless **forge-arbitrated bidirectional** GitHub ↔ hugit sync
//! (supersedes E6).
//!
//! The old E6 was a *naive symmetric mirror* (deliberately deferred). This
//! module replaces it with a model that feels like instant two-way sync to the
//! user yet keeps **the forge as the single source of truth**, so it never
//! corrupts and never violates the house principle *"embrace, don't assault /
//! never naive symmetric sync"*.
//!
//! ## The five rules (the binding contract)
//!
//! 1. **Branch refs sync both ways, seamlessly.** A branch pushed on the GitHub
//!    side is ingested into the forge as that same branch ref — recorded as an
//!    **external change-event** via [`hugit_proto::record_external_change`] (per
//!    D3⑤: never a fabricated intent, never touching `main`); a forge-side
//!    branch update mirrors out through the existing E1 outbound writer.
//!    Independent branches ⇒ no conflict ⇒ both sides converge with no human
//!    step. ([`engine::BidirSync::ingest_github_branch`] /
//!    [`engine::BidirSync::mirror_out_forge_branch`].)
//! 2. **`main` is single-writer = the forge.** A direct GitHub-side push to the
//!    protected default branch is **never applied symmetrically**. It is
//!    rerouted onto the normal path (auto-converted to a proposed branch) so it
//!    lands through the queue. `main` advances ONLY via landing. This is enforced
//!    *structurally*: the only constructor that appends a `main`-advancing event
//!    is [`engine::BidirSync::land_via_queue`]; the GitHub-ingest path has no
//!    branch that can move the protected ref.
//!    ([`engine::BidirSync::ingest_github_push`].)
//! 3. **No echo loop.** A change synced in one direction must not bounce back
//!    and re-emit. Convergence is **content-hash idempotent** (reuse E1's
//!    "mirror_write from observed mutation, not tip-inequality"): once both sides
//!    hold the same tip, sync goes quiet.
//!    ([`engine::BidirSync::converge`].)
//! 4. **Same-branch concurrent divergence** (the rare residual). If one branch
//!    ref is moved to incompatible tips on both sides at once, the forge
//!    arbitrates that one ref: the forge tip wins, and the divergent GitHub-side
//!    tip is **preserved as a recoverable incident/side-ref** — never silently
//!    dropped, never corrupting, chain stays verifiable.
//!    ([`engine::BidirSync::arbitrate_branch_divergence`].)
//! 5. **No symmetric-authority state.** There is no reachable state in which
//!    both sides are authoritative for `main` simultaneously — the forge is
//!    always the arbiter. ([`engine::AuthorityModel`].)
//!
//! ## Reuse, never re-transcribe
//!
//! This module is purely additive. It **consumes** the frozen surfaces and never
//! forks them:
//! - the canonical hash-chained event log + `verify_chain` + replay
//!   ([`hugit_refstore`]);
//! - the raw-push→external-change-event recorder
//!   ([`hugit_proto::record_external_change`] — *the* D3⑤ no-fake-intent path);
//! - the E1 divergence machinery (forge-authoritative repair, observed-origin)
//!   in [`crate::divergence`] and the orphan-aware ref-tip diff in
//!   [`crate::refops`].
//!
//! ## Hermetic vs. live
//!
//! All arbitration / convergence / single-writer logic is proven **hermetically**
//! in-process against the real forge surfaces. The one piece that genuinely needs
//! live infrastructure — *detecting* a GitHub-side change (webhook/poll) — is the
//! documented P2 seam in [`detect`], gated behind `HUGIT_GH_TEST_REPO`
//! (run-not-skip when set; asserted "not wired" in the bare gate so it cannot rot
//! to green).

pub mod detect;
pub mod engine;

pub use detect::{GitHubDetectOutcome, detect_github_change};
pub use engine::{
    AuthorityModel, AuthoritySide, BidirSync, BranchSync, ConvergeOutcome, IncidentRef,
    IngestOutcome, ProtectedReroute, SyncError, incident_ref_name,
};
