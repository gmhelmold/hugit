//! GDPR1 Part 2 — the erasure EXECUTION cascade, slice 1: the read-only PLANNER.
//!
//! See `docs/design/2026-07-04-gdpr1-erasure-execution-cascade.md`. This module
//! computes WHAT an account erasure would tombstone/purge/disclose — it MUTATES
//! NOTHING. It is the safe "what" the future EXECUTOR drives (the "do", gated behind
//! clw's independent adversarial audit — never enabled live from here).
//!
//! ## Why a read-only planner first
//!
//! The executor is the product's ONLY irreversible-deletion path. Building the plan
//! as a pure, fully-testable projection FIRST means: (a) clw can audit exactly what
//! would be erased before any store-mutating code exists, and (b) the executor
//! becomes a low-ambiguity drive over an already-reviewed plan (the Part-1 discipline
//! — design → seam → verb → route — carried into the irreversible kernel).
//!
//! ## The law this planner encodes (from X7/X12, proven hermetically)
//!
//! Erasure operates on the OBJECT/content store (tombstone by content hash), NEVER on
//! the append-only provenance chain. A repo is tombstoned by appending a TERMINAL
//! [`REPO_ERASED_KIND`] record to its log (never a rewrite) — the projection is
//! terminal, so the read/authz gate serves 404 and the git wire serves nothing.
//!
//! ## CAS erasure — premise CORRECTED 2026-07-05 (corelink-server TL)
//!
//! An earlier model here assumed the CoreLink CAS was cross-tenant content-deduplicated,
//! so a "shared" object could not be physically deleted → a residual-risk disclosure.
//! **That premise was WRONG.** CoreLink CAS is keyed PER-TENANT
//! (`<region>/HMAC(TDK,tenant)/<digest>`), so cross-CoreLink-tenant physical sharing is
//! impossible by construction — a delete under hugit's tenant can never touch another
//! CoreLink tenant. Two consequences:
//! - **Exclusivity is INTRA-hugit + ours:** the only sharing is two of hugit's OWN users
//!   deduping to one object, answerable ONLY from hugit's manifest graph. An object
//!   referenced by a SURVIVING user is a **legitimate retention** (that user still owns
//!   it) — NOT a residual-risk disclosure.
//! - **Account-EXCLUSIVE objects are physically deletable** via corelink-server's LIVE
//!   seam `POST /_internal/cas/<tenant>/<hash>/erase` → 410 Gone (see [`CasEraseTransport`]).
//!   The executor erases each exclusive digest + asserts the 410, then claims `executed`;
//!   until a transport is wired it claims `partial` (never over-claims).

use hugit_refstore::{Endpoint, EventLog};

use crate::error::EngineErr;
use crate::state::AppState;
use crate::writes::{AccountLogSink, LogSink, MAX_CAS_ATTEMPTS, asserted_class};

/// The terminal event kind that tombstones a repo (GDPR1). Appended to a repo's
/// append-only log by the future executor; the read/authz projection treats a
/// `repo.erased`-terminal repo as tombstoned (404, no content, no clone). Defined
/// here so the planner (already-erased detection) and the executor share ONE source.
pub const REPO_ERASED_KIND: &str = "repo.erased";

/// One repo leg of the plan: a repo owned by the subject account that the executor
/// would tombstone. `already_erased` = it already carries a terminal [`REPO_ERASED_KIND`]
/// record, so re-tombstoning it is a no-op (the idempotency the planner surfaces).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoErasureLeg {
    /// The repo slug to tombstone.
    pub repo: String,
    /// True iff the repo is ALREADY tombstoned (a terminal `repo.erased` record) —
    /// the executor skips it (idempotent replay = no-op).
    pub already_erased: bool,
}

/// Which non-repo leg a [`ResidualDisclosure`] covers. A closed enum (not a bare
/// `&'static str`) so the planner and the future executor can never drift on the leg
/// set (audit nit) — `cas-shared` is the SHARED-object CAS leg (defensible disclose),
/// distinct from the account-EXCLUSIVE CAS leg (a [`CasGcObligation`], which must be
/// physically GC'd, never disclosed away).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisclosureLeg {
    /// SHARED (cross-tenant-deduplicated) CAS objects — cannot be unilaterally deleted
    /// (would erase another tenant's data); tombstone + disclose is the honest maximum.
    CasShared,
    /// The GitHub mirror (out of hugit's physical control) — the P2 mirror-erase seam.
    GithubMirror,
    /// Account-scoped context/journal bytes — purged where an engine API exists.
    ContextStore,
}

impl DisclosureLeg {
    /// The stable wire label.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            DisclosureLeg::CasShared => "cas-shared",
            DisclosureLeg::GithubMirror => "github-mirror",
            DisclosureLeg::ContextStore => "context-store",
        }
    }
}

/// A non-repo leg whose erasure the engine cannot UNILATERALLY prove-complete in v0,
/// surfaced as an honest, NON-EMPTY residual-risk disclosure (the X7/X12 mirror-leg
/// discipline — an empty disclosure is an omission masquerading as one, so the plan
/// invariant [`ErasurePlan::is_honest`] rejects it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResidualDisclosure {
    /// Which leg.
    pub leg: DisclosureLeg,
    /// The honest, non-empty statement of what may persist and why it cannot be
    /// unilaterally guaranteed erased in v0.
    pub disclosure: String,
}

/// The **account-EXCLUSIVE** CAS objects obligation (GDPR1 audit #4 — the CAS-leg
/// split). An object referenced by NO other tenant is pure subject data with zero
/// cross-tenant collateral: it MUST be physically GC'd (a manifest tombstone leaves the
/// raw content-addressed object fetchable by digest — a real Art.17 gap, NOT closed by
/// disclosure). This is DISTINCT from the shared-object leg ([`DisclosureLeg::CasShared`],
/// legitimately disclose-only). The read-only planner cannot run the reachability check
/// (that needs the CAS) — it EMITS the obligation so the executor drives the correct
/// behavior; `seam_wired=false` means the CoreLink CAS reachability+GC seam is not yet
/// available, so this is a LOUD, separately-tracked go-live blocker (never folded into
/// the soft disclosure).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CasGcObligation {
    /// The account owns content-addressed objects that require the exclusive-vs-shared
    /// partition + physical GC of the exclusive ones (true iff it owns ≥1 repo).
    pub required: bool,
    /// Whether the CoreLink CAS reachability + physical-GC seam is available to the
    /// executor. `false` in v0 → the required physical GC cannot be performed →
    /// [`ErasurePlan::is_launch_blocked`].
    pub seam_wired: bool,
    /// The tracked-blocker note (named owner / cross-repo seam).
    pub note: String,
}

/// Whether the CoreLink CAS reachability + physical-GC seam is wired (v0: NOT — it is a
/// cross-repo obligation on the server/CAS TL, relayed by clw). Flip to a real
/// capability probe when the seam lands.
const CAS_GC_SEAM_WIRED: bool = false;

/// The full, read-only plan for erasing `account`: the repo legs to tombstone, the
/// account-exclusive CAS physical-GC obligation, and the per-non-repo-leg honest
/// residual-risk disclosures. MUTATES NOTHING.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ErasurePlan {
    /// The subject account slug.
    pub account: String,
    /// Every owned repo the executor would tombstone (with idempotency state).
    pub repos: Vec<RepoErasureLeg>,
    /// The account-exclusive CAS physical-GC obligation (split from the shared disclose).
    pub cas_gc: CasGcObligation,
    /// The v0 residual-risk disclosures (cas-SHARED, github-mirror, context-store).
    pub disclosures: Vec<ResidualDisclosure>,
}

impl ErasurePlan {
    /// The count of repos that still need tombstoning (not already erased).
    #[must_use]
    pub fn pending_repo_count(&self) -> usize {
        self.repos.iter().filter(|r| !r.already_erased).count()
    }

    /// True iff every residual-risk disclosure is NON-EMPTY (the honest-resolution
    /// predicate — an empty disclosure fails closed, mirroring X7/X12's
    /// `is_honestly_resolved`). A plan that would ship an empty disclosure is refused.
    #[must_use]
    pub fn is_honest(&self) -> bool {
        self.disclosures
            .iter()
            .all(|d| !d.disclosure.trim().is_empty())
    }

    /// True iff a go-live blocker stands: the account owns content-addressed objects
    /// whose account-EXCLUSIVE subset MUST be physically GC'd, but the CAS reachability
    /// and GC seam is not wired (audit #4). The executor MUST NOT claim full erasure
    /// while this holds — a loud, separately-tracked blocker, never a soft disclosure.
    #[must_use]
    pub fn is_launch_blocked(&self) -> bool {
        self.cas_gc.required && !self.cas_gc.seam_wired
    }
}

/// Whether `log` is a repo already TOMBSTONED — it carries a terminal
/// [`REPO_ERASED_KIND`] record. The append-only projection: once erased, always
/// erased (the executor's idempotency + the irreversibility guard).
#[must_use]
pub fn repo_is_erased(log: &EventLog) -> bool {
    log.records().iter().any(|r| r.kind == REPO_ERASED_KIND)
}

/// The v0 honest residual-risk disclosures for the non-repo legs. Each is NON-EMPTY
/// by construction (the plan's honesty invariant). These name the CoreLink-owned /
/// P2 seams the engine cannot unilaterally prove-complete today. NOTE the CAS leg here
/// is the **SHARED** (cross-tenant-deduplicated) objects ONLY — the account-EXCLUSIVE
/// objects are a physical-GC obligation ([`CasGcObligation`]), never disclosed away.
fn v0_residual_disclosures() -> Vec<ResidualDisclosure> {
    vec![
        ResidualDisclosure {
            leg: DisclosureLeg::CasShared,
            disclosure: "SHARED (cross-tenant-deduplicated) CAS objects — content also \
                         referenced by another tenant — cannot be physically deleted without \
                         erasing that tenant's data. The subject's repos are manifest/repo \
                         tombstoned (content unreachable + unserved); the shared objects persist \
                         in the CAS by design (unreachable via the subject's severed manifests). \
                         Account-EXCLUSIVE objects are NOT covered here — they are a physical-GC \
                         obligation, not a disclosure."
                .to_string(),
        },
        ResidualDisclosure {
            leg: DisclosureLeg::GithubMirror,
            disclosure: "Data replicated to the GitHub mirror is outside hugit's physical \
                         control; a live mirror-side erase is the documented P2 seam. Until \
                         wired, any mirrored copy is disclosed residual risk, not proven erased."
                .to_string(),
        },
        ResidualDisclosure {
            leg: DisclosureLeg::ContextStore,
            disclosure: "Account-scoped context/journal bytes are purged where an engine \
                         context-store erasure API exists; absent that API in v0, any residual \
                         context datum is disclosed residual risk, not proven purged."
                .to_string(),
        },
    ]
}

/// Compute the read-only erasure plan for `account`. MUTATES NOTHING.
///
/// Enumerates every repo owned by `account` via the DURABLE authoritative set
/// ([`AppState::authoritative_owned_repo_logs`] — the durable store listing ∪ the
/// in-memory loaded set), so a durable-but-unloaded repo can NEVER be missed (audit B1).
/// Marks each `already_erased` or pending; splits the CAS leg into the account-exclusive
/// physical-GC obligation (audit #4) + the shared-object disclosure; attaches the v0
/// honest residual disclosures. Self-checks honesty: a plan that would ship an empty
/// disclosure is REFUSED (fail-closed, audit nit).
///
/// # Errors
/// `503 ENGINE_UNAVAILABLE` — the durable ownership enumeration could not be determined
/// (any listing/load fault → fail-closed; the executor must NEVER erase from an
/// under-reported set), or the constructed plan is not honest (defense-in-depth).
pub fn plan_account_erasure(state: &AppState, account: &str) -> Result<ErasurePlan, EngineErr> {
    let owned = state.authoritative_owned_repo_logs(account)?;
    let repos: Vec<RepoErasureLeg> = owned
        .into_iter()
        .map(|(repo, log)| RepoErasureLeg {
            repo,
            already_erased: repo_is_erased(&log),
        })
        .collect();
    // The account-exclusive CAS physical-GC obligation is REQUIRED iff the account owns
    // content-addressed objects (i.e. owns ≥1 repo). The read-only planner cannot run
    // the reachability partition (needs the CAS) — it emits the obligation for the
    // executor + surfaces `seam_wired` so `is_launch_blocked` fires loudly in v0.
    let cas_gc = CasGcObligation {
        required: !repos.is_empty(),
        seam_wired: CAS_GC_SEAM_WIRED,
        note: "account-exclusive CAS objects require the reachability partition + physical GC \
               (a CoreLink server/CAS-TL cross-repo seam, relayed by clw); until wired this is a \
               tracked go-live blocker, NOT a disclosure"
            .to_string(),
    };
    let plan = ErasurePlan {
        account: account.to_string(),
        repos,
        cas_gc,
        disclosures: v0_residual_disclosures(),
    };
    // Defense-in-depth (audit nit): never return a plan an executor could drive with an
    // empty disclosure.
    if !plan.is_honest() {
        return Err(EngineErr::unavailable(
            "plano de apagamento com disclosure vazio — recusado (fail-closed)",
        ));
    }
    Ok(plan)
}

// ── the exclusive-digest PARTITION (slice 2 — the physical-GC blast-radius) ────
//
// The premise-corrected model: git-CAS content lands under a SINGLE shared tenant
// (`HUGIT_SERVE_CAS_TENANT_ID` = `d863fafb`; verified from the write path), so multiple
// hugit users' objects coexist, content-deduped. Erasing an account is therefore NOT a
// whole-tenant wipe — the executor may physically erase ONLY the account-EXCLUSIVE
// digests (referenced by the subject but by NO surviving user); a shared digest is a
// legitimate retention. Getting this partition wrong deletes a surviving user's data —
// it is slice-2's #1 blast-radius, so it is pure + fail-closed + hermetically tested here.

/// Enumerates the set of CAS content digests (blake3, 64-hex) a repo REFERENCES — the
/// VALUES of its `<tenant>/<repo>/oid-index.json` map. Abstracted so the exclusive-digest
/// partition is HERMETICALLY testable; the real impl reads the R2 oid-index (slice-2
/// wiring). MUST be authoritative + complete for a repo, or the partition below is unsafe.
pub trait RepoDigestSource {
    /// The blake3 digests `slug` references under `tenant`. `Err` on ANY fault — the
    /// partition treats an indeterminate read as fail-closed (never a partial set).
    fn repo_digests(
        &self,
        tenant: &str,
        slug: &str,
    ) -> Result<std::collections::BTreeSet<String>, EngineErr>;
}

/// Compute the subject-EXCLUSIVE CAS digests: referenced by the erased account's repos
/// but by **NO surviving (non-subject) repo**. This is the ONLY set the executor may
/// physically erase — a digest a surviving user still references is a legitimate
/// retention (the premise-corrected model: intra-hugit dedup in the SHARED tenant).
///
/// FAIL-CLOSED, and the fail-closed DIRECTION matters: an enumeration fault on ANY repo
/// (subject OR surviving) → `Err` (abort, erase NOTHING). A missing SURVIVING repo's
/// digests would shrink the surviving set → wrongly classify a shared digest as exclusive
/// → **delete a retained object** — the worst outcome, so we never proceed on a partial
/// surviving set. (A missing SUBJECT repo only shrinks the exclusive set — a safe
/// under-erase — but we still abort for a clean all-or-nothing contract.) The CALLER owns
/// passing a COMPLETE + authoritative `surviving_repos` = EVERY repo owned by a non-subject
/// account (the durable enumeration, like the planner's B1 fix); under-reporting THAT list
/// is the real blast-radius, upstream of this function.
///
/// # Errors
/// `503` — any `repo_digests` fault (fail-closed; nothing erased).
pub fn partition_exclusive_digests(
    src: &dyn RepoDigestSource,
    cas_tenant: &str,
    subject_repos: &[String],
    surviving_repos: &[String],
) -> Result<std::collections::BTreeSet<String>, EngineErr> {
    let mut subject = std::collections::BTreeSet::new();
    for slug in subject_repos {
        subject.extend(src.repo_digests(cas_tenant, slug)?);
    }
    // The surviving set MUST be complete — a fault here aborts (see the fail-closed note).
    let mut surviving = std::collections::BTreeSet::new();
    for slug in surviving_repos {
        surviving.extend(src.repo_digests(cas_tenant, slug)?);
    }
    Ok(subject.difference(&surviving).cloned().collect())
}

/// The production [`RepoDigestSource`] (clw bar #2): reads a repo's `oid-index.json` from
/// R2 (`<tenant>/<repo>/oid-index.json`, the git-oid→blake3 map) and collects its blake3
/// VALUES = the CAS digests the repo references. Holds a `&dyn R2Get` so it is testable
/// against a double.
///
/// FAIL-CLOSED, the partition's safety depends on it: an R2 read fault OR a malformed
/// index → `Err` (503; NEVER a partial digest set — a shrunk set on a surviving repo
/// would delete a retained object). An ABSENT index (`get_object` → `None`, i.e. a genuine
/// 404 — a repo with no pushed objects) → the EMPTY set: it references nothing, which is
/// correct + safe on both sides (a surviving repo with no objects retains nothing; a
/// subject repo with no objects contributes no exclusive digests).
pub struct R2OidIndexDigests<'a> {
    /// The R2 store the tenant's oid-indexes live in.
    pub r2: &'a dyn crate::cas::R2Get,
}

impl RepoDigestSource for R2OidIndexDigests<'_> {
    fn repo_digests(
        &self,
        tenant: &str,
        slug: &str,
    ) -> Result<std::collections::BTreeSet<String>, EngineErr> {
        let key = crate::cas::oid_index_key(tenant, slug);
        let bytes = self
            .r2
            .get_object(&key)
            .map_err(|e| EngineErr::unavailable(format!("oid-index read failed: {e}")))?;
        let Some(bytes) = bytes else {
            // Genuine absence (404) — the repo references no CAS objects. Fault → Err above.
            return Ok(std::collections::BTreeSet::new());
        };
        let index = crate::cas::parse_oid_index(&bytes)
            .map_err(|e| EngineErr::unavailable(format!("oid-index malformed: {e}")))?;
        Ok(index.into_values().collect())
    }
}

// ── the EXECUTOR (slice 2 — the irreversible legs) ────────────────────────────
//
// This drives the plan against the REAL stores. `execute_account_erasure_composed` IS
// route-wired via the operator execute route (server.rs `POST /v1/account/erase/execute`
// → `dispatch_account_erase_execute`); that route stays 404-DISABLED unless the erase seam
// is configured (`CORELINK_ERASE_URL`/`AUTH_KEY` → `state.erase_config`) and is gated
// operator-only + step-up + post-grace + DSR-legitimacy, so the physical CAS transport
// stays inert until that env is set (clw-audited before live enablement). It tombstones
// each owned repo (append-only terminal `repo.erased` → the authz projection serves 404)
// idempotently, then records the terminal claim on the account log — `erasure.executed`
// ONLY when the cascade is COMPLETE, else `erasure.partial` (fail-closed against an over-claim).

/// The terminal claim appended to the ACCOUNT log when the cascade COMPLETED — every
/// leg durably tombstoned/purged/GC'd. Emitted ONLY when the plan is not launch-blocked.
pub const ERASURE_EXECUTED_KIND: &str = "erasure.executed";

/// The claim appended when the cascade made progress but CANNOT complete yet (the
/// account-exclusive CAS physical GC is unmet — `is_launch_blocked`). Records what WAS
/// done + the outstanding obligation. NEVER an over-claim of full erasure.
pub const ERASURE_PARTIAL_KIND: &str = "erasure.partial";

/// A self-serve cancellation of a standing erasure request (GDPR1 withdrawal, go-live gap
/// G12a). The subject (or an operator on their behalf) WITHDRAWS a still-pending erasure
/// during the cooling-off — an append-only terminal that supersedes the standing
/// `erasure.requested` so the operator-execute cascade will NOT run. Produced by
/// [`cancel_account_erasure`] (route `POST /v1/account/erase/cancel`); consumed by
/// [`read_standing_erasure_request`] (which already drops the standing request on this kind)
/// and by [`classify_erase_cancel`].
pub const ERASURE_CANCELLED_KIND: &str = "erasure.cancelled";

/// A standing, still-governing erasure request read off an account log — the subject-staged
/// legitimacy the operator-execute route validates BEFORE driving the irreversible cascade.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandingErasureRequest {
    /// The subject account — captured from the AUTHENTICATED requester at REQUEST time
    /// (Part-1's `derive_owner_tenant`, subject == author), read back here at EXECUTE time.
    /// The executor erases THIS subject; it is NEVER a caller-supplied arg (no god-erase).
    pub subject: String,
    /// The DSR legitimacy id (githugr's `/_internal/dsr/anchor` originates it; hugit
    /// CONSUMES). `None` when the request predates the anchor wiring — physical erase is
    /// refused without it (the route fails closed; a delete needs a legitimacy id).
    pub dsr_id: Option<String>,
    /// Unix-ms the request was staged — the grace window is measured from here.
    pub requested_at: u64,
}

/// Read the LATEST standing `erasure.requested` off an account log, IF it is still the
/// governing lifecycle state (NOT superseded by a later [`ERASURE_CANCELLED_KIND`] or
/// [`ERASURE_EXECUTED_KIND`]). `None` when there is no lawful standing request → the route
/// 404s (no god-erase: the operator can only execute a subject-staged request, never mint).
///
/// Fail-safe: a `requested` record with an unparseable/`subject`-less payload is SKIPPED
/// (not trusted → never erased on a corrupt record); a later valid `requested` still governs.
pub fn read_standing_erasure_request(log: &EventLog) -> Option<StandingErasureRequest> {
    let mut standing: Option<StandingErasureRequest> = None;
    for r in log.records() {
        if r.kind == crate::writes::verbs::write_account_erase::ERASURE_REQUESTED_KIND {
            if let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.payload)
                && let Some(subject) = v.get("subject").and_then(|s| s.as_str())
            {
                let dsr_id = v
                    .get("dsr_id")
                    .and_then(|s| s.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from);
                standing = Some(StandingErasureRequest {
                    subject: subject.to_string(),
                    dsr_id,
                    requested_at: r.recorded_at,
                });
            }
        } else if r.kind == ERASURE_CANCELLED_KIND || r.kind == ERASURE_EXECUTED_KIND {
            standing = None; // superseded — no standing request to execute
        }
    }
    standing
}

/// PURE maturity predicate for the GDPR1 auto-executor (no I/O). A governing
/// `erasure.requested` is ripe for the irreversible cascade IFF it is NOT superseded (a
/// later `executed`/`cancelled` in its lifecycle) AND the cooling-off grace has FULLY
/// elapsed (`now >= requested_at + grace_ms`). `superseded` is what the standing-request
/// reader already encodes ([`read_standing_erasure_request`] returns `None` on a superseded
/// lifecycle), surfaced here as an explicit arg so the predicate is testable in isolation.
/// Saturating add so a huge grace / clock skew can NEVER wrap the threshold DOWN into a
/// false "matured".
#[must_use]
pub fn is_matured_governing_request(
    requested_at: u64,
    grace_ms: u64,
    now: u64,
    superseded: bool,
) -> bool {
    if superseded {
        return false;
    }
    now >= requested_at.saturating_add(grace_ms)
}

/// PURE per-account auto-execute DECISION (the sweep's selection logic, no I/O): drive the
/// cascade IFF there is a governing (non-superseded) standing request whose grace has matured
/// AND it carries a non-empty DSR legitimacy id (a physical erase is fail-closed-REFUSED
/// without one, mirroring the operator route's 403 — a delete needs an anchored legitimacy
/// id). `standing == None` (never requested, OR superseded by `executed`/`cancelled`) → never
/// (idempotent: an already-executed account is a no-op the sweep must skip).
#[must_use]
pub fn should_auto_execute(
    standing: Option<&StandingErasureRequest>,
    grace_ms: u64,
    now: u64,
) -> bool {
    match standing {
        Some(s) => {
            is_matured_governing_request(s.requested_at, grace_ms, now, false)
                && s.dsr_id.as_deref().is_some_and(|d| !d.is_empty())
        }
        None => false,
    }
}

/// The cancelability verdict of an account's erasure lifecycle at a given instant — the
/// projection the self-serve cancel route ([`cancel_account_erasure`]) switches on. Read
/// off the append-only account log by scanning to the LAST governing erasure-lifecycle
/// record (requested/cancelled/executed/partial). FAIL-CLOSED: only a still-standing
/// request that is provably WITHIN grace is cancelable; every other state is terminal and
/// never claims a cancel it did not perform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EraseCancelState {
    /// A standing `erasure.requested` still within the grace window — CANCELABLE.
    InGrace {
        /// Unix-ms the standing request was staged (grace is measured from here).
        requested_at: u64,
    },
    /// A standing `erasure.requested` whose grace has ELAPSED — the operator-execute
    /// cascade may already run; a withdrawal is too late (not cancelable).
    PastGrace,
    /// The cascade already ran (`erasure.executed`/`erasure.partial`) — irreversible.
    AlreadyExecuted,
    /// Already withdrawn (`erasure.cancelled` is the governing state) — a re-cancel is an
    /// idempotent success (there is nothing left to run).
    AlreadyCancelled,
    /// No standing request was ever staged (or the last state is neither requested nor a
    /// terminal) — nothing to cancel.
    NonePending,
}

/// Classify an account log's erasure lifecycle for the cancel route — a PURE projection
/// (env-free, exhaustively testable, mirroring [`read_standing_erasure_request`]). Scans to
/// the LAST governing erasure record: a `requested` (with its `recorded_at`) unless a later
/// `cancelled`/`executed`/`partial` supersedes it. A `requested` still governing is split
/// on the grace boundary (`now < requested_at + grace_ms`). FAIL-CLOSED at every arm — an
/// account with no erasure history is [`EraseCancelState::NonePending`], never a false
/// "cancelable".
#[must_use]
pub fn classify_erase_cancel(log: &EventLog, now: u64, grace_ms: u64) -> EraseCancelState {
    use crate::writes::verbs::write_account_erase::ERASURE_REQUESTED_KIND;
    // The last governing erasure-lifecycle record wins (append-only ⇒ last supersedes).
    let mut governing_requested_at: Option<u64> = None;
    let mut last_terminal: Option<&str> = None;
    for r in log.records() {
        if r.kind == ERASURE_REQUESTED_KIND {
            governing_requested_at = Some(r.recorded_at);
            last_terminal = None; // a fresh request re-opens the lifecycle
        } else if r.kind == ERASURE_CANCELLED_KIND
            || r.kind == ERASURE_EXECUTED_KIND
            || r.kind == ERASURE_PARTIAL_KIND
        {
            governing_requested_at = None; // superseded — no standing request
            last_terminal = Some(r.kind.as_str());
        }
    }
    if let Some(requested_at) = governing_requested_at {
        return if now < requested_at.saturating_add(grace_ms) {
            EraseCancelState::InGrace { requested_at }
        } else {
            EraseCancelState::PastGrace
        };
    }
    match last_terminal {
        Some(ERASURE_CANCELLED_KIND) => EraseCancelState::AlreadyCancelled,
        // `executed` OR `partial` — the irreversible cascade already ran (partial still
        // physically tombstoned repos), so a withdrawal is impossible.
        Some(_) => EraseCancelState::AlreadyExecuted,
        None => EraseCancelState::NonePending,
    }
}

/// The outcome of the self-serve cancel route — a 1:1 map to the frozen HTTP contract
/// (`200 {canceled:true}` vs `409 not_cancelable{reason}`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EraseCancelOutcome {
    /// A standing in-grace request was cleared THIS call (`erasure.cancelled` appended).
    Canceled,
    /// The request was ALREADY withdrawn — idempotent success (no second record).
    AlreadyCanceled,
    /// Terminal: the grace elapsed (the executor may run) → `409 past_grace`.
    PastGrace,
    /// Terminal: the cascade already executed → `409 already_executed`.
    AlreadyExecuted,
    /// Terminal: nothing was ever staged → `409 none_pending`.
    NonePending,
}

/// Self-serve WITHDRAW a standing erasure request (GDPR1 gap G12a) — the account-scoped
/// analogue of the operator-execute drive, but SAFE (it only STOPS a deletion, never
/// performs one). Over an [`AccountLogSink`], in a bounded CAS loop so classify + append are
/// atomic against the loaded head:
///
/// - [`EraseCancelState::InGrace`] → append the terminal `erasure.cancelled` (as the acting
///   principal, append-only ⇒ the provenance chain still verifies) and CAS-persist →
///   [`EraseCancelOutcome::Canceled`]. A concurrent head move → reload + re-classify (never
///   last-writer-wins).
/// - [`EraseCancelState::AlreadyCancelled`] → [`EraseCancelOutcome::AlreadyCanceled`] (no
///   second record; idempotent double-cancel).
/// - `PastGrace`/`AlreadyExecuted`/`NonePending` → the matching terminal outcome, NO write.
///
/// The DSR anchor row is LEFT intact (audit trail): cancel voids the STAGED request via the
/// superseding `erasure.cancelled`; it does not rewrite or drop the `erasure.requested`
/// (the log is append-only). `dsr_id`, when supplied, is captured on the cancel record for
/// provenance. FAIL-CLOSED: any load/verify/persist fault is an `Err`, never a false cancel.
pub fn cancel_account_erasure(
    sink: &dyn AccountLogSink,
    subject: &str,
    dsr_id: Option<&str>,
    principal_chain: &[String],
    now: u64,
    grace_ms: u64,
) -> Result<EraseCancelOutcome, EngineErr> {
    let class = asserted_class(principal_chain)?;
    for _attempt in 0..MAX_CAS_ATTEMPTS {
        let (mut log, token) = sink.load_account(subject)?;
        match classify_erase_cancel(&log, now, grace_ms) {
            EraseCancelState::InGrace { .. } => {
                let mut payload_value = serde_json::json!({
                    "account": subject,
                    "subject": subject,
                    "state": "canceled",
                });
                if let Some(d) = dsr_id.filter(|s| !s.is_empty()) {
                    payload_value["dsr_id"] = serde_json::json!(d);
                }
                let payload = hugit_refstore::canonical_json(&payload_value.to_string())
                    .unwrap_or_else(|| payload_value.to_string());
                log.append_authorized(
                    class,
                    Endpoint::Land,
                    ERASURE_CANCELLED_KIND,
                    principal_chain.to_vec(),
                    payload,
                    now,
                )
                .map_err(|d| {
                    EngineErr::unavailable(format!(
                        "erasure.cancelled append denied: {}",
                        d.reason.code()
                    ))
                })?;
                match sink.persist_account(subject, &log, &token) {
                    Ok(()) => return Ok(EraseCancelOutcome::Canceled),
                    Err(e) if e.is_cas_conflict() => continue, // head moved — reload + retry
                    Err(e) => return Err(e),
                }
            }
            EraseCancelState::AlreadyCancelled => return Ok(EraseCancelOutcome::AlreadyCanceled),
            EraseCancelState::PastGrace => return Ok(EraseCancelOutcome::PastGrace),
            EraseCancelState::AlreadyExecuted => return Ok(EraseCancelOutcome::AlreadyExecuted),
            EraseCancelState::NonePending => return Ok(EraseCancelOutcome::NonePending),
        }
    }
    Err(EngineErr::unavailable(
        "cancelamento de apagamento sob contenção — tente novamente",
    ))
}

/// The outcome of driving an account erasure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErasureOutcome {
    /// The cascade COMPLETED — every leg durable; `erasure.executed` recorded.
    Executed { repos_tombstoned: usize },
    /// The cascade tombstoned the repos but CANNOT claim full erasure (a go-live blocker
    /// stands — the account-exclusive CAS physical GC is unmet); `erasure.partial` recorded.
    Partial {
        repos_tombstoned: usize,
        blocked_reason: String,
    },
    /// A no-op replay — the account was already `erasure.executed`. Irreversible +
    /// idempotent: never re-tombstones, never resurrects.
    AlreadyExecuted,
    /// A grace-boundary cancellation SUPERSEDED the request BEFORE any irreversible action —
    /// the executor re-read the FRESH account log before a destructive leg (or the
    /// terminal-claim CAS append) and found a governing [`ERASURE_CANCELLED_KIND`] with
    /// NOTHING yet destroyed. STRICTLY NON-DESTRUCTIVE: zero repos tombstoned, zero CAS bytes
    /// GC'd, NO terminal claim recorded — the callers map this to a no-op. This variant is
    /// returned ONLY when `tombstoned==0 && digests_erased==0`; once ANY destruction has
    /// occurred the executor reports the TRUTHFUL `Executed`/`Partial` instead (MF1 — it never
    /// asserts `erased:false` over real, irreversible destruction). The data-loss race fix: an
    /// in-grace cancel can never be silently overrun, and a false "executed" is never recorded
    /// over a standing withdrawal that touched no data.
    Cancelled,
}

/// Whether the account log already carries a terminal [`ERASURE_EXECUTED_KIND`] — the
/// executor's idempotency + irreversibility guard (a completed erasure never re-runs).
fn account_already_executed(log: &EventLog) -> bool {
    log.records()
        .iter()
        .any(|r| r.kind == ERASURE_EXECUTED_KIND)
}

/// Whether the account's erasure lifecycle is CURRENTLY superseded by a cancellation — a
/// governing [`ERASURE_CANCELLED_KIND`] stands that is NOT re-opened by a later
/// [`ERASURE_REQUESTED_KIND`] and NOT completed by a later [`ERASURE_EXECUTED_KIND`].
///
/// This is the executor's fail-closed re-check at the POINT OF NO RETURN. The cancel guard
/// otherwise lives ONLY in the callers (the auto-executor selection + the operator route),
/// strictly BEFORE dispatch — a TOCTOU: the ~5-min auto-sweep can read the account log
/// BEFORE an in-grace `erasure.cancelled` persists, then physically tombstone every repo +
/// GC the CAS bytes while the cancel returns `200 {canceled:true}`. Re-reading the FRESH
/// account log with this predicate before EACH destructive leg (each tombstone + before the
/// byte-level GC) and in the terminal-claim CAS append closes that race per-leg. Mirrors the
/// last-governing scan of [`classify_erase_cancel`], but answers ONLY the one question the
/// executor needs: "is a cancellation the standing terminal right now?"
///
/// MF3 (crucial): [`ERASURE_PARTIAL_KIND`] DELIBERATELY does not clear a standing cancel. A
/// partial is progress, not completion — a cancel that halted the cascade mid-flight leaves
/// `[requested, cancelled, partial]`, and the withdrawal MUST keep governing the REMAINING
/// (un-destroyed) legs so a later auto-sweep can never silently finish the erasure the
/// subject withdrew. Only a full `executed` (the cascade truly completed) or a fresh
/// `requested` (a NEW erasure lifecycle) clears it.
fn account_erasure_superseded_by_cancel(log: &EventLog) -> bool {
    use crate::writes::verbs::write_account_erase::ERASURE_REQUESTED_KIND;
    let mut superseded_by_cancel = false;
    for r in log.records() {
        if r.kind == ERASURE_REQUESTED_KIND {
            superseded_by_cancel = false; // a fresh request re-opens the lifecycle
        } else if r.kind == ERASURE_CANCELLED_KIND {
            superseded_by_cancel = true; // a cancellation supersedes the standing request
        } else if r.kind == ERASURE_EXECUTED_KIND {
            superseded_by_cancel = false; // a COMPLETED erasure is the governing terminal
            // NOTE: ERASURE_PARTIAL_KIND is intentionally NOT handled here — a partial must
            // NEVER clear a standing cancellation (MF3); the withdrawal still governs the
            // remaining legs.
        }
    }
    superseded_by_cancel
}

/// Tombstone ONE repo: append the terminal [`REPO_ERASED_KIND`] to its append-only log
/// via a bounded compare-and-swap (mirrors the write-door's CAS loop). IDEMPOTENT — a
/// repo already carrying `repo.erased` is a no-op. FAIL-CLOSED — a durable-persist fault
/// propagates (the caller must NOT then claim `executed`). Appends AS the executing
/// principal (chain-derived class), on the append-only log (never a rewrite → the
/// provenance chain still verifies, X7/X12).
fn tombstone_repo(
    sink: &dyn LogSink,
    repo: &str,
    principal_chain: &[String],
    at: u64,
) -> Result<(), EngineErr> {
    let class = asserted_class(principal_chain)?;
    for _attempt in 0..MAX_CAS_ATTEMPTS {
        let (mut log, token) = sink.load(repo)?;
        if repo_is_erased(&log) {
            return Ok(()); // already tombstoned — idempotent no-op
        }
        let payload_value = serde_json::json!({ "reason": "erasure", "state": "erased" });
        let payload = hugit_refstore::canonical_json(&payload_value.to_string())
            .unwrap_or_else(|| payload_value.to_string());
        log.append_authorized(
            class,
            Endpoint::Land,
            REPO_ERASED_KIND,
            principal_chain.to_vec(),
            payload,
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!("repo.erased append denied: {}", d.reason.code()))
        })?;
        match sink.persist(repo, &log, &token) {
            Ok(()) => return Ok(()),
            Err(e) if e.is_cas_conflict() => continue, // head moved — reload + retry
            Err(e) => return Err(e),
        }
    }
    Err(EngineErr::unavailable(
        "tombstone de repositório sob contenção — tente novamente",
    ))
}

/// Append the terminal erasure claim (`executed`|`partial`) to the ACCOUNT log via a
/// bounded CAS. Idempotent on the SAME kind (a matching terminal record already present
/// → no-op). Fail-closed on a durable fault.
///
/// Returns `Ok(true)` when the claim is recorded (or was already present — idempotent), and
/// `Ok(false)` ONLY for a REFUSED `executed`: the CAS hardening for the data-loss race — when
/// `allow_over_cancel` is FALSE (the completion was CLEAN, i.e. NOTHING was physically
/// destroyed) this append re-reads the FRESH account log inside the CAS loop and REFUSES to
/// stamp `erasure.executed` OVER a superseding `erasure.cancelled` (mirrors the If-Match
/// discipline so the log can never record a false executed while a withdrawal stands and no
/// data was touched).
///
/// `allow_over_cancel` is TRUE only when irreversible destruction ALREADY occurred
/// (`tombstoned>0 || digests_erased>0`): recording `executed` is then the TRUTHFUL,
/// coherent-with-the-repo-logs outcome (the data really is gone — asserting `cancelled`/
/// `erased:false` over it would be the dishonest response MF1 forbids), so the refusal is
/// bypassed. Only `executed` is ever guarded — a `partial` is not a completion over-claim
/// and, per MF3, can never clear a standing cancel via the predicate.
fn append_account_claim(
    sink: &dyn AccountLogSink,
    account: &str,
    kind: &str,
    principal_chain: &[String],
    at: u64,
    payload_value: &serde_json::Value,
    allow_over_cancel: bool,
) -> Result<bool, EngineErr> {
    let class = asserted_class(principal_chain)?;
    for _attempt in 0..MAX_CAS_ATTEMPTS {
        let (mut log, token) = sink.load_account(account)?;
        // CAS HARDENING (data-loss race, defense-in-depth): refuse to record `executed` over a
        // superseding cancellation on the FRESH head — UNLESS destruction already occurred
        // (`allow_over_cancel`), in which case `executed` is the truthful record (MF1).
        if kind == ERASURE_EXECUTED_KIND
            && !allow_over_cancel
            && account_erasure_superseded_by_cancel(&log)
        {
            return Ok(false);
        }
        if log.records().iter().any(|r| r.kind == kind) {
            return Ok(true); // this terminal claim already recorded — idempotent
        }
        let payload = hugit_refstore::canonical_json(&payload_value.to_string())
            .unwrap_or_else(|| payload_value.to_string());
        log.append_authorized(
            class,
            Endpoint::Land,
            kind,
            principal_chain.to_vec(),
            payload,
            at,
        )
        .map_err(|d| {
            EngineErr::unavailable(format!("{kind} append denied: {}", d.reason.code()))
        })?;
        match sink.persist_account(account, &log, &token) {
            Ok(()) => return Ok(true),
            Err(e) if e.is_cas_conflict() => continue,
            Err(e) => return Err(e),
        }
    }
    Err(EngineErr::unavailable(
        "registro de apagamento sob contenção — tente novamente",
    ))
}

/// The physical CAS-erase seam — corelink-server's LIVE internal route
/// `POST /_internal/cas/<tenant>/<hash>/erase` (physically deletes the R2 bytes + writes
/// a `cas_tombstone`, so a subsequent fetch-by-digest returns **410 Gone**). Abstracted
/// so the executor is HERMETICALLY testable against a mock; the REAL HTTP impl + the
/// least-privilege `CORELINK_ERASE_AUTH_KEY` + the DSR legitimacy step are slice-2 (built
/// behind clw's re-audit before any live enablement).
///
/// Every method is FAIL-CLOSED: an uncertain erase/verify is an `Err`, NEVER a false
/// "gone" — the executor must never claim `executed` on an unproven physical delete.
pub trait CasEraseTransport {
    /// Physically erase `digest` under `tenant`, authorized by the DSR `dsr_id`.
    /// IDEMPOTENT (an already-erased digest → `Ok`, mirrors the seam's `AlreadyErased`).
    /// `Err` on any uncertain outcome (never a silent success).
    fn erase(
        &self,
        tenant: &str,
        digest: &str,
        dsr_id: &str,
        reason: &str,
    ) -> Result<(), EngineErr>;

    /// Verify `digest` is physically GONE (a fetch-by-digest returns 410, not 200). The
    /// executor asserts this PER digest before claiming `executed` — 410-not-404 is the
    /// affirmative physical-GC proof, distinct from an unreachable-named-path.
    fn is_gone(&self, tenant: &str, digest: &str) -> Result<bool, EngineErr>;
}

// ── the REAL HTTP CasEraseTransport (slice-2 network seam; NOT route-wired) ─────
//
// The ONLY code here that opens a socket. Mirrors the ureq + SSRF-allowlist discipline
// of the session-exchange client (`token.rs`). It is inert until the route wires it AND
// `CORELINK_ERASE_URL`/`CORELINK_ERASE_AUTH_KEY` are set AND clw re-audits — it deletes
// nothing on its own.

/// SSRF allowlist for the CoreLink internal-erase base URL — same posture as the
/// session-exchange client: only a `.humangr.com` host (or a loopback for tests) may
/// ever receive the internal-auth key. Fail-closed at config parse time.
const ERASE_URL_TRUSTED_SUFFIXES: &[&str] = &[".humangr.com", "localhost", "127.0.0.1", "[::1]"];

/// Bounded per-call timeout (a single small POST/GET).
const ERASE_TIMEOUT_SECS: u64 = 15;

/// Config for the real erase transport, read FAIL-CLOSED from env.
///
/// - `CORELINK_ERASE_URL` — the CoreLink internal API base (e.g.
///   `https://corelink-api.humangr.com`). ABSENT → `Ok(None)`: the erase seam is not
///   configured, so the executor has no transport and stays `partial` (never live).
/// - `CORELINK_ERASE_AUTH_KEY` — the internal-auth secret sent RAW in the
///   `x-corelink-internal-auth` header (NEVER a Bearer — a Bearer is 401 on this seam).
///
/// `Err` (set-but-invalid) on: an empty/scheme-less URL, an untrusted host (SSRF), or a
/// URL present WITHOUT a key (a half-configured erase seam must never boot).
#[derive(Clone)]
pub struct EraseConfig {
    base_url: String,
    auth_key: String,
}

/// REDACTING Debug — the `auth_key` is a secret and must never appear in a log/panic.
impl std::fmt::Debug for EraseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EraseConfig")
            .field("base_url", &self.base_url)
            .field("auth_key", &"<redacted>")
            .finish()
    }
}

impl EraseConfig {
    /// Read the erase seam config from env (fail-closed; see the type doc).
    pub fn from_env() -> Result<Option<Self>, String> {
        Self::validate(
            std::env::var("CORELINK_ERASE_URL").ok(),
            std::env::var("CORELINK_ERASE_AUTH_KEY").ok(),
        )
    }

    /// The PURE validation (testable without touching the process env): the SSRF allowlist
    /// plus the half-configured-seam fail-close. `base_url` ABSENT → `Ok(None)` (no
    /// transport). `pub(crate)` so the route builds a config from a loopback mock in tests.
    pub(crate) fn validate(
        base_url: Option<String>,
        auth_key: Option<String>,
    ) -> Result<Option<Self>, String> {
        let base_url = match base_url {
            Some(v) => v.trim().to_string(),
            None => return Ok(None), // not configured → no transport → executor stays partial
        };
        if base_url.is_empty() {
            return Err("CORELINK_ERASE_URL is empty (fail-closed)".to_string());
        }
        if !base_url.starts_with("http://") && !base_url.starts_with("https://") {
            return Err(format!(
                "CORELINK_ERASE_URL must start with http(s)://; got: {base_url}"
            ));
        }
        let host = crate::token::extract_host(&base_url)
            .ok_or_else(|| format!("CORELINK_ERASE_URL: cannot parse host from `{base_url}`"))?;
        if !ERASE_URL_TRUSTED_SUFFIXES
            .iter()
            .any(|s| host == *s || host.ends_with(s))
        {
            return Err(format!(
                "CORELINK_ERASE_URL host `{host}` is not on the trusted allowlist \
                 (suffixes: {ERASE_URL_TRUSTED_SUFFIXES:?}); set a *.humangr.com endpoint"
            ));
        }
        // A URL WITHOUT a key is a half-configured seam — fail-closed (never send an erase
        // with a missing/empty internal-auth key: it would 401 silently, or worse).
        let auth_key =
            auth_key.ok_or("CORELINK_ERASE_URL set but CORELINK_ERASE_AUTH_KEY missing")?;
        if auth_key.trim().is_empty() {
            return Err("CORELINK_ERASE_AUTH_KEY is empty (fail-closed)".to_string());
        }
        Ok(Some(EraseConfig {
            // Trim the trailing slash so `{base}/_internal/...` is well-formed; the key is
            // used verbatim (already rejected all-whitespace above).
            base_url: base_url.trim_end_matches('/').to_string(),
            auth_key,
        }))
    }

    /// Build the client.
    pub fn into_client(self) -> HttpCasErase {
        HttpCasErase::new(self.base_url, self.auth_key)
    }
}

/// The REAL HTTP [`CasEraseTransport`] against CoreLink's internal erase seam.
///
/// `erase` POSTs `POST {base}/_internal/cas/<tenant>/<digest>/erase` with the RAW
/// `x-corelink-internal-auth` key and body `{tenant, dsr_id, reason}`. The confirmed seam
/// contract (corelink-server #634) answers **410 Gone** on a fresh physical delete and
/// **200 AlreadyErased** on an idempotent replay — BOTH mean the bytes are gone, so both
/// record the digest as CONFIRMED-gone. Any other outcome (401/403 auth, 404, 5xx,
/// transport fault) is FAIL-CLOSED `Err` — never a silent success, never a recorded-gone.
///
/// `is_gone` reflects that recorded gone-truth directly — NO independent network read. The
/// corelink-server TL confirmed (2026-07-05) that the erase's **410 is durably
/// read-consistent** (the handler deletes the R2 bytes + upserts the `cas_tombstone` D1 row
/// BEFORE returning, and the CAS read consults that same primary D1 — read-your-write), so
/// the erase response IS the durable physical-GC proof; a separate fetch-by-digest verify
/// would be redundant (and would need a `cas:r` PAT on a different data-plane route
/// `GET /v1/cas/<tenant>/<hash>`, not the internal-auth key). We therefore do NOT open a
/// second socket — `is_gone` is membership in `confirmed_gone`, so a proven physical delete
/// is never downgraded to a false `partial`, and an un-erased digest is honestly not-gone.
pub struct HttpCasErase {
    base_url: String,
    auth_key: String,
    /// Digests this client has CONFIRMED gone from the erase seam's own 410/200 response —
    /// the durable gone-truth the confirmed contract already carries. Per-erasure-run
    /// local (a `HttpCasErase` is built per execute call), single-threaded accept loop.
    confirmed_gone: std::cell::RefCell<std::collections::BTreeSet<String>>,
}

impl HttpCasErase {
    /// Construct from an already-validated base URL + key (the config path is
    /// [`EraseConfig::from_env`]).
    pub fn new(base_url: String, auth_key: String) -> Self {
        Self {
            base_url,
            auth_key,
            confirmed_gone: std::cell::RefCell::new(std::collections::BTreeSet::new()),
        }
    }

    fn agent() -> ureq::Agent {
        ureq::AgentBuilder::new()
            .timeout(std::time::Duration::from_secs(ERASE_TIMEOUT_SECS))
            .build()
    }
}

impl CasEraseTransport for HttpCasErase {
    fn erase(
        &self,
        tenant: &str,
        digest: &str,
        dsr_id: &str,
        reason: &str,
    ) -> Result<(), EngineErr> {
        let url = format!("{}/_internal/cas/{tenant}/{digest}/erase", self.base_url);
        let body = serde_json::json!({
            "tenant": tenant,
            "dsr_id": dsr_id,
            "reason": reason,
        })
        .to_string();
        // The internal-auth key is sent RAW in `x-corelink-internal-auth` (NOT a Bearer —
        // a Bearer is 401 on this seam) and NEVER logged/echoed.
        let resp = Self::agent()
            .post(&url)
            .set("x-corelink-internal-auth", &self.auth_key)
            .set("Content-Type", "application/json")
            .send_string(&body);
        match resp {
            // ureq: <400 → Ok. 200 AlreadyErased ⇒ gone (idempotent).
            Ok(_r) => {
                self.confirmed_gone.borrow_mut().insert(digest.to_string());
                Ok(())
            }
            // 410 Gone is the SUCCESS signal (ureq surfaces >=400 as Err(Status)).
            Err(ureq::Error::Status(410, _)) => {
                self.confirmed_gone.borrow_mut().insert(digest.to_string());
                Ok(())
            }
            // Everything else FAILS CLOSED — never a recorded-gone, never an over-claim.
            // 401/403 = the internal-auth key is wrong/missing (the RAW-vs-Bearer trap);
            // 404 = the seam did not confirm a tombstone; 5xx = retryable.
            Err(ureq::Error::Status(code, _)) => Err(EngineErr::unavailable(format!(
                "cas-erase seam returned {code} (not 410/200)"
            ))),
            Err(ureq::Error::Transport(_)) => {
                Err(EngineErr::unavailable("cas-erase seam transport fault"))
            }
        }
    }

    fn is_gone(&self, _tenant: &str, digest: &str) -> Result<bool, EngineErr> {
        // The erase's 410/200 is the DURABLE physical-GC proof (read-your-write, confirmed
        // by the corelink-server TL 2026-07-05) — so a digest recorded gone by `erase` above
        // IS gone, and one that was not is honestly not-gone. NO independent network read
        // (a separate verify would be redundant + would need a different `cas:r`-PAT route).
        Ok(self.confirmed_gone.borrow().contains(digest))
    }
}

/// EXECUTE an account erasure (GDPR1 Part 2 — the irreversible legs). NOT route-wired;
/// gated behind clw's re-audit before any live enablement. NO CAS erase transport → the
/// account-exclusive physical GC is unmet, so the claim is `partial` (never over-claims).
/// The transport path is [`execute_account_erasure_with_erase`].
pub fn execute_account_erasure(
    state: &AppState,
    account: &str,
    principal_chain: Vec<String>,
    at: u64,
) -> Result<ErasureOutcome, EngineErr> {
    execute_account_erasure_inner(state, account, principal_chain, at, None, "", &[], "")
}

/// EXECUTE an account erasure WITH the physical CAS-erase transport (the completed
/// cost-of-erasure path). After tombstoning the owned repos, it erases EACH
/// account-exclusive digest via `erase` and asserts each is 410-GONE; ONLY when every
/// exclusive digest is erased + verified does it claim `erasure.executed`. Any un-erased
/// or un-verified digest → `erasure.partial` (records progress, never over-claims).
///
/// `cas_tenant` is the CoreLink CAS tenant the digests live under (hugit's git tenant).
/// `exclusive_digests` MUST be the subject-EXCLUSIVE set (referenced ONLY by the erased
/// account — a surviving user's object is a legitimate retention, NEVER passed here); its
/// computation from the manifest graph + the real HTTP transport + the DSR `dsr_id`
/// origin are slice-2 (behind clw's re-audit). Passing a surviving-user digest here would
/// delete a retained object — so the CALLER owns that partition's correctness.
#[allow(clippy::too_many_arguments)]
pub fn execute_account_erasure_with_erase(
    state: &AppState,
    account: &str,
    principal_chain: Vec<String>,
    at: u64,
    erase: &dyn CasEraseTransport,
    cas_tenant: &str,
    exclusive_digests: &[String],
    dsr_id: &str,
) -> Result<ErasureOutcome, EngineErr> {
    execute_account_erasure_inner(
        state,
        account,
        principal_chain,
        at,
        Some(erase),
        cas_tenant,
        exclusive_digests,
        dsr_id,
    )
}

/// The COMPOSITION (clw bar #3) — the single call the route drives: derive the durable
/// subject/surviving repo partition (bar #1), read each repo's referenced digests
/// (`digests`, bar #2 in production = [`R2OidIndexDigests`]), compute the subject-EXCLUSIVE
/// set (`subject − surviving`), and drive the executor over EXACTLY that set.
///
/// **The exact-superset guarantee bar #3 requires:** the digests physically erased are
/// PRECISELY `partition_exclusive_digests(subject, surviving)` — never a digest outside
/// `subject − surviving`. Concretely, the `exclusive` this computes is the *same slice*
/// handed to [`execute_account_erasure_with_erase`], which erases each and no other. So a
/// surviving user's shared object is structurally impossible to erase (it is subtracted
/// before the drive ever sees it), and an EMPTY exclusive set is a LEGITIMATE `executed`
/// erasure (a subject whose every object is shared with a surviving user, or that
/// references none: tombstone the repos, delete nothing physical, claim executed honestly).
///
/// **Fail-closed end to end:** the partition aborts on an indeterminate durable listing
/// (bar #1, `503`) and the digest read aborts on any R2 fault or malformed index (bar #2,
/// `503`) — nothing is erased under an incomplete surviving set. This is why `digests` is a
/// `&dyn RepoDigestSource` (not constructed inline): the route wires the real
/// `R2OidIndexDigests`, tests a hermetic double, and the exact-superset contract holds for
/// either.
///
/// `cas_tenant` is the CoreLink CAS tenant hugit's git content lives under (the single
/// shared `HUGIT_SERVE_CAS_TENANT_ID`); the partition is meaningful precisely because that
/// tenant is shared across accounts (per-tenant CAS keying — verified), so a naïve
/// whole-tenant wipe would delete surviving users' objects.
///
/// # Errors
/// `503` — indeterminate durable enumeration (bar #1), any digest-read fault/malformed
/// index (bar #2), or any tombstone/erase/claim durable fault (the drive). `401` —
/// unclassifiable principal (the drive). Fail-closed throughout; nothing over-claimed and
/// nothing outside the exclusive set is ever erased.
#[allow(clippy::too_many_arguments)]
pub fn execute_account_erasure_composed(
    state: &AppState,
    digests: &dyn RepoDigestSource,
    account: &str,
    principal_chain: Vec<String>,
    at: u64,
    erase: &dyn CasEraseTransport,
    cas_tenant: &str,
    dsr_id: &str,
) -> Result<ErasureOutcome, EngineErr> {
    // Bar #1 — the durable subject-vs-surviving partition (fail-closed on an indeterminate
    // listing → 503; a partial surviving set NEVER proceeds).
    let (subject_repos, surviving_repos) = state.erasure_repo_partition(account)?;

    // Partition (bars #2 → exclusive) — the subject-EXCLUSIVE digests, `subject − surviving`,
    // fail-closed on any read fault (503). This slice is EXACTLY what the drive erases.
    let exclusive: Vec<String> =
        partition_exclusive_digests(digests, cas_tenant, &subject_repos, &surviving_repos)?
            .into_iter()
            .collect();

    // Bar #3 — drive the executor over EXACTLY the exclusive set (exact-superset: what is
    // erased == what was computed exclusive; empty → executed legitimately).
    execute_account_erasure_with_erase(
        state,
        account,
        principal_chain,
        at,
        erase,
        cas_tenant,
        &exclusive,
        dsr_id,
    )
}

/// The shared executor drive (fail-closed `executed ⇒ every leg durable + physically GC'd`,
/// the receive-pack discipline):
/// 1. Plan (fail-closed on an indeterminate durable enumeration — never under-erase).
/// 2. Already `erasure.executed` → NO-OP (idempotent + irreversible).
/// 3. Tombstone EVERY pending owned repo durably (append-only terminal `repo.erased`,
///    CAS-guarded + idempotent). ANY failure aborts BEFORE the claim (503).
/// 4. The account-exclusive CAS physical GC:
///    - no transport → the obligation is unmet → NOT complete (→ `partial`);
///    - transport → erase EACH exclusive digest + assert 410-gone; ALL gone → complete;
///      an erase fault propagates (503, retry converges — idempotent); a post-erase
///      not-gone → NOT complete (→ `partial`, never a silent over-claim).
/// 5. The terminal claim on the account log, LAST: `erasure.executed` iff complete, else
///    `erasure.partial` (progress + the outstanding obligation).
///
/// # Errors
/// `503` — indeterminate enumeration, any repo-tombstone or erase durable fault, or the
/// claim append fault (fail-closed; nothing over-claimed). `401` — unclassifiable principal.
#[allow(clippy::too_many_arguments)]
fn execute_account_erasure_inner(
    state: &AppState,
    account: &str,
    principal_chain: Vec<String>,
    at: u64,
    erase: Option<&dyn CasEraseTransport>,
    cas_tenant: &str,
    exclusive_digests: &[String],
    dsr_id: &str,
) -> Result<ErasureOutcome, EngineErr> {
    let plan = plan_account_erasure(state, account)?;

    // Idempotent + irreversible: a completed account never re-runs.
    let (account_log, _) = state.load_account(account)?;
    if account_already_executed(&account_log) {
        return Ok(ErasureOutcome::AlreadyExecuted);
    }

    // DATA-LOSS RACE FIX — the FIRST fail-closed re-check (the cheap early-out). The callers
    // gate on cancellation strictly BEFORE dispatch, but a grace-boundary `erasure.cancelled`
    // can persist in the TOCTOU window between that gate and here (the auto-sweep read the log
    // ~5 min ago). If a cancellation already supersedes the request, ABORT before touching
    // anything → non-destructive `Cancelled`. This is only the first of the per-leg re-checks:
    // each `tombstone_repo` and the byte-level GC below re-read the FRESH log again, so every
    // irreversible leg is guarded as a single-op TOCTOU rather than a whole-cascade window.
    if account_erasure_superseded_by_cancel(&account_log) {
        return Ok(ErasureOutcome::Cancelled);
    }

    // Tombstone every PENDING repo durably FIRST — the claim is gated on all of these.
    // MF2: each `tombstone_repo` is an IRREVERSIBLE leg, so re-check the FRESH standing state
    // immediately before EACH one — this makes every destructive leg a single-op TOCTOU, not a
    // whole-cascade window. `halted_by_cancel` records that a withdrawal landed mid-cascade so
    // we STOP the remaining destruction (never undo what is already gone) and take the truthful
    // outcome for what WAS destroyed (MF1).
    let sink: &dyn LogSink = state;
    let mut tombstoned = 0usize;
    let mut halted_by_cancel = false;
    for leg in &plan.repos {
        if leg.already_erased {
            continue; // idempotent — already terminal
        }
        // MF2: fresh per-leg re-check on a NEW account-log read, immediately before this
        // irreversible tombstone. A cancel seen with NOTHING yet destroyed → clean abort; a
        // cancel seen after destruction began → halt the REMAINING legs (the truthful path).
        if account_erasure_superseded_by_cancel(&state.load_account(account)?.0) {
            if tombstoned == 0 {
                return Ok(ErasureOutcome::Cancelled); // nothing destroyed → clean no-op
            }
            halted_by_cancel = true;
            break;
        }
        tombstone_repo(sink, &leg.repo, &principal_chain, at)?;
        // Task #74 (W-METENANT scaling follow-up): `repo.erased` is TERMINAL and
        // authz-relevant (`authorize_read` denies an erased repo to EVERYONE,
        // including the operator) — refresh the repo-meta cache immediately, before
        // this loop's TOCTOU re-assert (`plan_account_erasure` below) or any
        // concurrent `/v1/me/*` request can observe a stale "not erased" cached
        // answer. A stale-cache leak here would be a private/erased-content
        // exposure, not merely a perf regression.
        state.refresh_repo_meta_cache(&leg.repo);
        // WP-1: DECREMENT the tenant's durable repo registry — the cap is a HOLD-count
        // (erasable-down), matching the account model. Best-effort + idempotent: an
        // erased repo no longer counts, so a decrement fault leaves the SAFE direction (a
        // transient OVER-count → the cap is momentarily conservative, never bypassed) and
        // self-heals on a re-run; it must NEVER fail the irreversible erase cascade.
        if let Err(e) = state.unregister_repo_from_tenant(account, &leg.repo, &principal_chain, at)
        {
            eprintln!(
                "[hugit-serve] erase: repo {:?} tombstoned, but the tenant-registry \
                 decrement failed ({}); the cap over-counts until a re-run heals it",
                leg.repo, e.code
            );
        }
        tombstoned += 1;
    }

    // The account-exclusive CAS physical GC — the BYTE-LEVEL, most irreversible leg. `gc_complete`
    // gates `executed`. MF2: `drive_cas_gc` re-checks the FRESH standing state before EACH
    // physical `erase.erase` (a single-op TOCTOU) and HALTS the remaining byte-deletes on a
    // withdrawal. Skip the GC entirely if we already halted mid-tombstone.
    let (gc_complete, digests_erased, gc_halted) = if halted_by_cancel {
        (false, 0, false)
    } else {
        let mut cancel_supersedes = || -> Result<bool, EngineErr> {
            Ok(account_erasure_superseded_by_cancel(
                &state.load_account(account)?.0,
            ))
        };
        drive_cas_gc(
            &plan,
            erase,
            cas_tenant,
            exclusive_digests,
            dsr_id,
            &mut cancel_supersedes,
        )?
    };
    if gc_halted {
        // The byte-level GC halted on a withdrawal. If NOTHING was destroyed anywhere (no
        // tombstone, no byte erased), it is a clean no-op → `Cancelled`. Otherwise irreversible
        // destruction already occurred → the truthful `partial` path (MF1) below.
        if tombstoned == 0 && digests_erased == 0 {
            return Ok(ErasureOutcome::Cancelled);
        }
        halted_by_cancel = true;
    }

    // ENUMERATE-CLAIM TOCTOU guard (clw must-fix #3c): a repo the subject provisioned
    // AFTER the plan enumeration but BEFORE this claim would be missed by the tombstone
    // loop — so `executed` could be claimed while a fresh repo survives. RE-ASSERT the
    // durable owned set here, immediately before the claim: if ANY owned repo is still
    // pending (a new one appeared mid-cascade), we do NOT claim `executed` — we downgrade
    // to `partial` (re-runnable; the next run tombstones it). Fail-closed against a false
    // `executed`; `partial`-and-reconverge is the correct fail-safe (never over-claim).
    let toctou_clean = plan_account_erasure(state, account)?.pending_repo_count() == 0;

    // Whether ANY irreversible destruction actually happened — the pivot of the honesty
    // contract (MF1): the executor may NEVER report `Cancelled`/`erased:false` once a repo is
    // tombstoned or a CAS byte is gone; it must surface the TRUTHFUL outcome instead.
    let destroyed = tombstoned > 0 || digests_erased > 0;

    // The terminal claim, LAST.
    let acct_sink: &dyn AccountLogSink = state;
    if gc_complete && toctou_clean && !halted_by_cancel {
        // The cascade COMPLETED. If destruction occurred, `executed` is the truthful record
        // and MUST be written even over a late cancel (coherent with the repo logs, MF1); a
        // late-arriving withdrawal that lost the race is noted for the audit trail. If NOTHING
        // was destroyed (empty plan / all-shared digests) and a cancel now stands, the CAS
        // append refuses (`allow_over_cancel=false`) → honest `Cancelled` (nothing erased).
        let late_cancel_overrun =
            destroyed && account_erasure_superseded_by_cancel(&state.load_account(account)?.0);
        let payload = serde_json::json!({
            "account": account,
            "state": "executed",
            "repos_tombstoned": tombstoned,
            "digests_erased": digests_erased,
            "late_cancel_overrun": late_cancel_overrun,
        });
        let recorded = append_account_claim(
            acct_sink,
            account,
            ERASURE_EXECUTED_KIND,
            &principal_chain,
            at,
            &payload,
            destroyed, // allow_over_cancel: only when the erasure truly happened
        )?;
        if !recorded {
            // Refused: the completion was CLEAN (nothing destroyed) and a cancel superseded at
            // the append → honest non-destructive `Cancelled` (never a false "executed").
            return Ok(ErasureOutcome::Cancelled);
        }
        Ok(ErasureOutcome::Executed {
            repos_tombstoned: tombstoned,
        })
    } else if halted_by_cancel {
        // A withdrawal HALTED the cascade after irreversible destruction began. We can never
        // resurrect what is already gone, so record the TRUTHFUL `partial` (MF1) — the
        // already-destroyed legs are stated (`repos_tombstoned`/`digests_erased`) and the
        // standing cancel keeps governing the REMAINING legs (MF3: partial never clears it), so
        // no later sweep finishes the erasure the subject withdrew.
        let outstanding = "halted-by-cancel-mid-cascade (already-destroyed legs are irreversible; \
                           remaining destruction withdrawn per the standing cancellation)";
        let payload = serde_json::json!({
            "account": account,
            "state": "partial",
            "repos_tombstoned": tombstoned,
            "digests_erased": digests_erased,
            "outstanding": outstanding,
        });
        // Not cancel-guarded — a partial over a standing cancel is the honest record here, and
        // it provably cannot clear the cancel (MF3 predicate).
        append_account_claim(
            acct_sink,
            account,
            ERASURE_PARTIAL_KIND,
            &principal_chain,
            at,
            &payload,
            true, // allow_over_cancel is irrelevant for a partial (only executed is guarded)
        )?;
        Ok(ErasureOutcome::Partial {
            repos_tombstoned: tombstoned,
            blocked_reason: outstanding.to_string(),
        })
    } else {
        // Incomplete for a NON-cancel reason: an unmet CAS-GC obligation vs a repo that
        // appeared mid-cascade (the TOCTOU re-assert fired) — both are honestly re-runnable.
        let outstanding = if !toctou_clean {
            "owned-repo-appeared-mid-cascade"
        } else {
            "cas-exclusive-physical-gc"
        };
        let payload = serde_json::json!({
            "account": account,
            "state": "partial",
            "repos_tombstoned": tombstoned,
            "digests_erased": digests_erased,
            "outstanding": outstanding,
        });
        append_account_claim(
            acct_sink,
            account,
            ERASURE_PARTIAL_KIND,
            &principal_chain,
            at,
            &payload,
            true, // allow_over_cancel is irrelevant for a partial (only executed is guarded)
        )?;
        Ok(ErasureOutcome::Partial {
            repos_tombstoned: tombstoned,
            blocked_reason: plan.cas_gc.note.clone(),
        })
    }
}

/// Drive the account-exclusive CAS physical GC. Returns `(gc_complete, digests_erased,
/// halted_by_cancel)`.
///
/// FAIL-CLOSED: `gc_complete` is true ONLY when the obligation is not required, or every
/// exclusive digest is erased AND 410-verified. An erase fault propagates (`Err` → 503,
/// idempotent retry converges); a post-erase not-gone yields `gc_complete=false` (→
/// `partial`, never a silent over-claim).
///
/// MF2 — the byte-level phase is the MOST irreversible leg, so `cancel_supersedes` is re-run
/// on the FRESH account log IMMEDIATELY before EACH physical `erase.erase` (a single-op
/// TOCTOU, not a whole-phase window). A cancellation seen before a given digest HALTS the
/// remaining byte-deletes: it returns `halted_by_cancel=true` with the count already erased,
/// and the caller takes the TRUTHFUL outcome for what was destroyed (never `erased:false`).
fn drive_cas_gc(
    plan: &ErasurePlan,
    erase: Option<&dyn CasEraseTransport>,
    cas_tenant: &str,
    exclusive_digests: &[String],
    dsr_id: &str,
    cancel_supersedes: &mut dyn FnMut() -> Result<bool, EngineErr>,
) -> Result<(bool, usize, bool), EngineErr> {
    if !plan.cas_gc.required {
        return Ok((true, 0, false)); // no owned objects → nothing to physically GC
    }
    let Some(erase) = erase else {
        return Ok((false, 0, false)); // obligation required but no transport → unmet → partial
    };
    let mut erased = 0usize;
    let mut all_gone = true;
    for digest in exclusive_digests {
        // MF2: fresh re-check immediately before this irreversible byte-delete. A withdrawal
        // now standing → HALT the remaining deletes (never undo what is already gone).
        if cancel_supersedes()? {
            return Ok((false, erased, true));
        }
        erase.erase(cas_tenant, digest, dsr_id, "erasure")?; // hard fault → 503, retry
        if erase.is_gone(cas_tenant, digest)? {
            erased += 1;
        } else {
            all_gone = false; // erased but not 410-verified → do NOT over-claim
        }
    }
    Ok((all_gone, erased, false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::{Endpoint, PrincipalClass};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch_dir() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let seq = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "hugit-erasure-plan-{}-{nanos}-{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    // ── the standing-request reader (operator-execute route) ─────────────────────

    /// Build an account log by appending `(kind, payload)` records in order; the record's
    /// `recorded_at` (the grace-window anchor) = its 1-based position, in ms.
    fn account_log(records: &[(&str, serde_json::Value)]) -> EventLog {
        let mut log = EventLog::new();
        for (i, (kind, payload)) in records.iter().enumerate() {
            let p = payload.to_string();
            log.append_authorized(
                PrincipalClass::Orchestrator,
                Endpoint::Land,
                *kind,
                vec!["o".into()],
                hugit_refstore::canonical_json(&p).unwrap_or(p),
                (i as u64) + 1,
            )
            .expect("append");
        }
        log
    }

    const REQ: &str = crate::writes::verbs::write_account_erase::ERASURE_REQUESTED_KIND;

    #[test]
    fn standing_request_extracts_subject_dsr_and_time() {
        let log = account_log(&[(
            REQ,
            serde_json::json!({"account":"org-a","subject":"org-a","state":"requested","dsr_id":"dsr-9"}),
        )]);
        let s = read_standing_erasure_request(&log).expect("a standing request");
        assert_eq!(s.subject, "org-a");
        assert_eq!(s.dsr_id.as_deref(), Some("dsr-9"));
        assert_eq!(
            s.requested_at, 1,
            "the grace anchor is the request's recorded_at"
        );
    }

    #[test]
    fn standing_request_dsr_absent_is_none_dsr() {
        // A request staged before the anchor wiring carries no dsr_id → None (the route
        // then refuses the physical erase: no legitimacy id).
        let log = account_log(&[(
            REQ,
            serde_json::json!({"account":"org-a","subject":"org-a","state":"requested"}),
        )]);
        let s = read_standing_erasure_request(&log).expect("standing");
        assert_eq!(s.subject, "org-a");
        assert!(s.dsr_id.is_none());
    }

    #[test]
    fn standing_request_superseded_by_executed_is_none() {
        let log = account_log(&[
            (REQ, serde_json::json!({"subject":"org-a","dsr_id":"d"})),
            (
                ERASURE_EXECUTED_KIND,
                serde_json::json!({"state":"executed"}),
            ),
        ]);
        assert!(
            read_standing_erasure_request(&log).is_none(),
            "an executed account has no standing request to re-execute"
        );
    }

    #[test]
    fn standing_request_superseded_by_cancelled_is_none() {
        let log = account_log(&[
            (REQ, serde_json::json!({"subject":"org-a","dsr_id":"d"})),
            (
                ERASURE_CANCELLED_KIND,
                serde_json::json!({"state":"cancelled"}),
            ),
        ]);
        assert!(read_standing_erasure_request(&log).is_none());
    }

    #[test]
    fn standing_request_none_when_never_requested() {
        let log = account_log(&[("account.created", serde_json::json!({"x":1}))]);
        assert!(read_standing_erasure_request(&log).is_none());
    }

    #[test]
    fn standing_request_latest_governs_and_a_re_request_after_cancel_stands() {
        // requested → cancelled → requested again: the LAST governing request stands.
        let log = account_log(&[
            (REQ, serde_json::json!({"subject":"org-a","dsr_id":"old"})),
            (
                ERASURE_CANCELLED_KIND,
                serde_json::json!({"state":"cancelled"}),
            ),
            (REQ, serde_json::json!({"subject":"org-a","dsr_id":"new"})),
        ]);
        let s = read_standing_erasure_request(&log).expect("the re-request stands");
        assert_eq!(s.dsr_id.as_deref(), Some("new"));
        assert_eq!(s.requested_at, 3);
    }

    // ── the cancel-lifecycle classifier (self-serve withdrawal, G12a) ─────────────

    const GRACE: u64 = 1_000; // 1s grace for the pure classifier tests

    #[test]
    fn classify_in_grace_when_a_fresh_request_stands() {
        let log = account_log(&[(REQ, serde_json::json!({"subject":"org-a"}))]); // recorded_at = 1
        // now = 1 (== requested_at) is strictly < 1 + GRACE → in grace.
        assert_eq!(
            classify_erase_cancel(&log, 1, GRACE),
            EraseCancelState::InGrace { requested_at: 1 }
        );
    }

    #[test]
    fn classify_past_grace_at_the_boundary() {
        let log = account_log(&[(REQ, serde_json::json!({"subject":"org-a"}))]); // recorded_at = 1
        // now == requested_at + grace is NOT < the bound → past grace (boundary is exclusive).
        assert_eq!(
            classify_erase_cancel(&log, 1 + GRACE, GRACE),
            EraseCancelState::PastGrace
        );
    }

    #[test]
    fn classify_already_executed_wins_over_a_prior_request() {
        let log = account_log(&[
            (REQ, serde_json::json!({"subject":"org-a"})),
            (
                ERASURE_EXECUTED_KIND,
                serde_json::json!({"state":"executed"}),
            ),
        ]);
        assert_eq!(
            classify_erase_cancel(&log, 1, GRACE),
            EraseCancelState::AlreadyExecuted
        );
        // A `partial` cascade is likewise terminal (repos were physically tombstoned).
        let plog = account_log(&[
            (REQ, serde_json::json!({"subject":"org-a"})),
            (ERASURE_PARTIAL_KIND, serde_json::json!({"state":"partial"})),
        ]);
        assert_eq!(
            classify_erase_cancel(&plog, 1, GRACE),
            EraseCancelState::AlreadyExecuted
        );
    }

    #[test]
    fn classify_already_cancelled_is_idempotent_terminal() {
        let log = account_log(&[
            (REQ, serde_json::json!({"subject":"org-a"})),
            (
                ERASURE_CANCELLED_KIND,
                serde_json::json!({"state":"canceled"}),
            ),
        ]);
        assert_eq!(
            classify_erase_cancel(&log, 1, GRACE),
            EraseCancelState::AlreadyCancelled
        );
    }

    #[test]
    fn classify_none_pending_when_no_erasure_history() {
        assert_eq!(
            classify_erase_cancel(&EventLog::new(), 1, GRACE),
            EraseCancelState::NonePending
        );
    }

    #[test]
    fn classify_a_re_request_after_cancel_is_cancelable_again() {
        // requested → cancelled → requested again: the latest request re-opens the window.
        let log = account_log(&[
            (REQ, serde_json::json!({"subject":"org-a"})),
            (
                ERASURE_CANCELLED_KIND,
                serde_json::json!({"state":"canceled"}),
            ),
            (REQ, serde_json::json!({"subject":"org-a"})), // recorded_at = 3
        ]);
        assert_eq!(
            classify_erase_cancel(&log, 3, GRACE),
            EraseCancelState::InGrace { requested_at: 3 }
        );
    }

    // ── the real HTTP erase transport (config validation + mock-seam wire) ────────

    #[test]
    fn erase_config_absent_url_is_none_no_transport() {
        assert!(
            EraseConfig::validate(None, Some("k".into()))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn erase_config_url_without_key_fails_closed() {
        // A half-configured seam (URL present, key absent) must NEVER boot — an erase
        // with a missing internal-auth key would 401 silently.
        let err = EraseConfig::validate(Some("https://corelink-api.humangr.com".into()), None)
            .expect_err("url-without-key is fail-closed");
        assert!(err.contains("AUTH_KEY missing"), "{err}");
    }

    #[test]
    fn erase_config_untrusted_host_rejected_ssrf() {
        let err = EraseConfig::validate(Some("https://evil.example.com".into()), Some("k".into()))
            .expect_err("untrusted host is SSRF-rejected");
        assert!(err.contains("not on the trusted allowlist"), "{err}");
    }

    #[test]
    fn erase_config_trusted_host_ok() {
        let cfg = EraseConfig::validate(
            Some("https://corelink-api.humangr.com/".into()),
            Some("secret-key".into()),
        )
        .expect("trusted")
        .expect("some");
        assert_eq!(cfg.base_url, "https://corelink-api.humangr.com"); // trailing slash trimmed
        assert_eq!(cfg.auth_key, "secret-key");
    }

    /// A mock erase seam: the erase POST `.../erase` → `post_status`. Serves `n` requests,
    /// reporting each `(method, path, auth_header)`. (`is_gone` opens no socket, so the
    /// seam only ever sees the POST.)
    fn mock_erase_seam(
        post_status: u16,
        n: usize,
    ) -> (
        String,
        std::sync::mpsc::Receiver<(String, String, Option<String>)>,
    ) {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("bind mock");
        let port = server.server_addr().to_ip().expect("ip").port();
        let base = format!("http://127.0.0.1:{port}");
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for req in server.incoming_requests().take(n) {
                let method = req.method().as_str().to_string();
                let path = req.url().to_string();
                let auth = req
                    .headers()
                    .iter()
                    .find(|h| h.field.equiv("x-corelink-internal-auth"))
                    .map(|h| h.value.as_str().to_string());
                let _ = tx.send((method.clone(), path, auth));
                let _ =
                    req.respond(tiny_http::Response::from_string("").with_status_code(post_status));
            }
        });
        (base, rx)
    }

    #[test]
    fn http_erase_410_records_gone_and_sends_raw_key() {
        // 410 Gone is the SUCCESS signal; the key rides RAW in x-corelink-internal-auth
        // (never a Bearer). is_gone is then satisfied by the recorded gone-truth (no GET).
        let (base, rx) = mock_erase_seam(410, 1);
        let client = HttpCasErase::new(base, "the-erase-key".into());
        client
            .erase("d863fafb", "deadbeef", "dsr-1", "erasure")
            .expect("410 → Ok");
        assert!(client.is_gone("d863fafb", "deadbeef").expect("gone"));
        let (method, path, auth) = rx.recv().expect("server saw a request");
        assert_eq!(method, "POST");
        assert_eq!(path, "/_internal/cas/d863fafb/deadbeef/erase");
        assert_eq!(
            auth.as_deref(),
            Some("the-erase-key"),
            "RAW key, not a Bearer"
        );
    }

    #[test]
    fn http_erase_200_already_erased_is_gone() {
        let (base, _rx) = mock_erase_seam(200, 1);
        let client = HttpCasErase::new(base, "k".into());
        client
            .erase("t", "d1", "dsr", "erasure")
            .expect("200 AlreadyErased → Ok");
        assert!(client.is_gone("t", "d1").expect("gone"));
    }

    #[test]
    fn http_erase_401_fails_closed_not_recorded_gone() {
        // The RAW-vs-Bearer trap / wrong key → 401 → fail-closed Err, NEVER recorded gone.
        let (base, _rx) = mock_erase_seam(401, 1);
        let client = HttpCasErase::new(base, "wrong".into());
        assert_eq!(
            client
                .erase("t", "d1", "dsr", "erasure")
                .expect_err("401 aborts")
                .status,
            503
        );
    }

    #[test]
    fn http_erase_500_fails_closed() {
        let (base, _rx) = mock_erase_seam(500, 1);
        let client = HttpCasErase::new(base, "k".into());
        assert_eq!(
            client
                .erase("t", "d1", "dsr", "erasure")
                .expect_err("5xx aborts")
                .status,
            503
        );
    }

    #[test]
    fn http_is_gone_reflects_only_the_erase_no_network() {
        // is_gone opens NO socket (the erase 410 is durably read-consistent — corelink-TL
        // confirmed): a recorded-gone digest is gone; an un-erased one is honestly not-gone.
        // The mock serves ONLY the single erase POST (n=1) — if is_gone tried a network
        // read it would hang/observe a second request; it does neither.
        let (base, _rx) = mock_erase_seam(410, 1);
        let client = HttpCasErase::new(base, "k".into());
        assert!(
            !client
                .is_gone("t", "never-erased")
                .expect("un-erased → not gone"),
            "a digest this client never erased is honestly not-gone (no false positive)"
        );
        client.erase("t", "d1", "dsr", "erasure").expect("410 → Ok");
        assert!(
            client.is_gone("t", "d1").expect("erased → gone"),
            "the recorded gone-truth suffices with no independent read"
        );
        assert!(
            !client
                .is_gone("t", "still-never")
                .expect("other → not gone"),
            "recording one digest gone does not mark others gone"
        );
    }

    /// A serialized genesis `repo.meta{owner_tenant}` log, optionally with a terminal
    /// `repo.erased` record appended (a chain-valid tombstoned repo).
    fn repo_log_json(owner: &str, erased: bool) -> String {
        let mut log = EventLog::new();
        let meta = serde_json::json!({"visibility":"private","owner_tenant":owner}).to_string();
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            "repo.meta",
            vec!["o".into()],
            hugit_refstore::canonical_json(&meta).unwrap_or(meta),
            1,
        )
        .expect("append repo.meta");
        if erased {
            let payload = serde_json::json!({"account":owner,"reason":"erasure"}).to_string();
            log.append_authorized(
                PrincipalClass::Orchestrator,
                Endpoint::Land,
                REPO_ERASED_KIND,
                vec!["o".into()],
                hugit_refstore::canonical_json(&payload).unwrap_or(payload),
                2,
            )
            .expect("append repo.erased");
        }
        serde_json::to_string(log.records()).unwrap()
    }

    /// An `AppState` (Local) seeding `(slug, owner, erased)` repos as durable logs +
    /// countable git seams (a fully loaded repo — durable log AND in-memory seam).
    fn state_with(repos: &[(&str, &str, bool)]) -> AppState {
        let dir = scratch_dir();
        let mut st = AppState::new(dir.clone(), "dev-token".to_string());
        for (slug, owner, erased) in repos {
            std::fs::write(
                dir.join(format!("{slug}.json")),
                repo_log_json(owner, *erased),
            )
            .unwrap();
            st.set_repo_git(
                *slug,
                std::sync::Arc::new(hugit_proto::CasObjectSource::new()),
                gix_hash::ObjectId::empty_tree(gix_hash::Kind::Sha1),
                std::collections::BTreeMap::new(),
            );
        }
        st
    }

    /// Write a DURABLE repo log to the store dir WITHOUT wiring an in-memory git seam —
    /// the exact shape of a repo provisioned then dropped from the boot env: its log
    /// survives in the store, but it is absent from the loaded set. The B1 fixture.
    fn seed_durable_only(st: &AppState, slug: &str, owner: &str) {
        let dir = match &st.source {
            crate::state::LogSource::Local { dir } => dir.clone(),
            crate::state::LogSource::R2(_) => unreachable!("test is Local-mode"),
        };
        std::fs::write(
            dir.join(format!("{slug}.json")),
            repo_log_json(owner, false),
        )
        .unwrap();
    }

    #[test]
    fn plan_enumerates_only_the_subjects_repos() {
        let st = state_with(&[
            ("alpha", "org-a", false),
            ("beta", "org-a", false),
            ("gamma", "org-b", false), // a DIFFERENT tenant — must NOT be in org-a's plan
        ]);
        let plan = plan_account_erasure(&st, "org-a").expect("plan");
        let mut slugs: Vec<&str> = plan.repos.iter().map(|r| r.repo.as_str()).collect();
        slugs.sort();
        assert_eq!(
            slugs,
            vec!["alpha", "beta"],
            "only org-a's repos, never org-b's"
        );
        assert_eq!(plan.pending_repo_count(), 2);
    }

    #[test]
    fn plan_is_idempotency_aware_already_erased_repo_is_not_pending() {
        let st = state_with(&[("alpha", "org-a", false), ("beta", "org-a", true)]);
        let plan = plan_account_erasure(&st, "org-a").expect("plan");
        assert_eq!(plan.repos.len(), 2);
        assert_eq!(
            plan.pending_repo_count(),
            1,
            "the already-erased repo is not pending"
        );
        let beta = plan.repos.iter().find(|r| r.repo == "beta").unwrap();
        assert!(beta.already_erased, "beta carries a terminal repo.erased");
    }

    #[test]
    fn plan_is_read_only_the_logs_are_untouched() {
        let st = state_with(&[("alpha", "org-a", false)]);
        let before = st.load_verified("alpha").unwrap().records().len();
        let _ = plan_account_erasure(&st, "org-a").expect("plan");
        let after = st.load_verified("alpha").unwrap().records().len();
        assert_eq!(before, after, "planning mutates NOTHING");
    }

    #[test]
    fn plan_carries_honest_nonempty_residual_disclosures() {
        let st = state_with(&[("alpha", "org-a", false)]);
        let plan = plan_account_erasure(&st, "org-a").expect("plan");
        assert!(
            plan.is_honest(),
            "every residual-risk disclosure is non-empty"
        );
        let legs: Vec<&str> = plan.disclosures.iter().map(|d| d.leg.as_str()).collect();
        // The CAS leg here is the SHARED objects ONLY (the account-exclusive objects
        // are a physical-GC obligation, not a disclosure).
        assert!(legs.contains(&"cas-shared"));
        assert!(legs.contains(&"github-mirror"));
        assert!(legs.contains(&"context-store"));
    }

    #[test]
    fn plan_for_an_account_with_no_repos_is_empty_but_honest() {
        let st = state_with(&[("gamma", "org-b", false)]);
        let plan = plan_account_erasure(&st, "org-a").expect("plan");
        assert!(plan.repos.is_empty(), "org-a owns no repos here");
        assert_eq!(plan.pending_repo_count(), 0);
        assert!(
            plan.is_honest(),
            "the non-repo legs are still honestly disclosed"
        );
        // No content-addressed objects owned → no exclusive-GC obligation, not blocked.
        assert!(!plan.cas_gc.required);
        assert!(!plan.is_launch_blocked());
    }

    #[test]
    fn plan_enumerates_a_durable_but_unloaded_repo_the_b1_completeness_fix() {
        // AUDIT B1 (the worst class — under-erasure): a repo whose log is DURABLE in the
        // store but whose git seam is GONE (provisioned then dropped from the boot env)
        // MUST still be in the subject's plan — else the subject is told "erased" while
        // it survives. The old in-memory-only enumeration missed it; the durable listing
        // catches it.
        let st = state_with(&[("loaded", "org-a", false)]);
        seed_durable_only(&st, "stranded", "org-a"); // durable log, NO in-memory seam
        let plan = plan_account_erasure(&st, "org-a").expect("plan");
        let mut slugs: Vec<&str> = plan.repos.iter().map(|r| r.repo.as_str()).collect();
        slugs.sort();
        assert_eq!(
            slugs,
            vec!["loaded", "stranded"],
            "the durable-but-unloaded repo is NOT missed (no silent under-erasure)"
        );
    }

    fn operator() -> Vec<String> {
        vec!["orchestrator:hugit".to_string()]
    }

    #[test]
    fn toctou_reassert_predicate_detects_a_new_pending_repo() {
        // The enumerate-claim TOCTOU guard (clw #3c) is
        // `plan_account_erasure(...).pending_repo_count() == 0`, re-asserted immediately
        // before the `erasure.executed` claim. A NON-tombstoned owned repo (one that
        // appeared mid-cascade) makes it > 0 → the executor downgrades to `partial` rather
        // than over-claim `executed`. Once every owned repo is tombstoned it is 0 → clean.
        let pending = state_with(&[("alpha", "org-a", false)]); // NOT erased → pending
        assert!(
            plan_account_erasure(&pending, "org-a")
                .unwrap()
                .pending_repo_count()
                > 0,
            "a non-tombstoned owned repo trips the re-assert → partial, never a false executed"
        );
        let clean = state_with(&[("alpha", "org-a", true)]); // already erased
        assert_eq!(
            plan_account_erasure(&clean, "org-a")
                .unwrap()
                .pending_repo_count(),
            0,
            "all owned repos tombstoned → the re-assert is clean → executed is allowed"
        );
    }

    // ── the exclusive-digest partition ────────────────────────────────────────────

    /// A hermetic [`RepoDigestSource`]: slug → its digest set; `fault_on` forces an `Err`
    /// for one slug (the fail-closed branch).
    struct MockDigests {
        map: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
        fault_on: Option<String>,
    }
    impl MockDigests {
        fn new(entries: &[(&str, &[&str])]) -> Self {
            let mut map = std::collections::BTreeMap::new();
            for (slug, digs) in entries {
                map.insert(
                    (*slug).to_string(),
                    digs.iter().map(|d| (*d).to_string()).collect(),
                );
            }
            Self {
                map,
                fault_on: None,
            }
        }
    }
    impl RepoDigestSource for MockDigests {
        fn repo_digests(
            &self,
            _t: &str,
            slug: &str,
        ) -> Result<std::collections::BTreeSet<String>, EngineErr> {
            if self.fault_on.as_deref() == Some(slug) {
                return Err(EngineErr::unavailable("mock oid-index read fault"));
            }
            Ok(self.map.get(slug).cloned().unwrap_or_default())
        }
    }

    #[test]
    fn partition_keeps_only_subject_exclusive_digests() {
        // subject repo references {d1,d2,shared}; a surviving repo references {shared,dv}.
        // Exclusive = {d1,d2} (shared is a legitimate retention, NEVER erased).
        let src = MockDigests::new(&[
            ("subj", &["d1", "d2", "shared"]),
            ("surv", &["shared", "dv"]),
        ]);
        let ex = partition_exclusive_digests(&src, "d863fafb", &["subj".into()], &["surv".into()])
            .expect("partition");
        assert_eq!(ex, ["d1", "d2"].iter().map(|s| s.to_string()).collect());
        assert!(
            !ex.contains("shared"),
            "a surviving-referenced digest is NEVER exclusive"
        );
    }

    #[test]
    fn partition_no_surviving_repo_makes_all_subject_digests_exclusive() {
        let src = MockDigests::new(&[("subj", &["d1", "d2"])]);
        let ex = partition_exclusive_digests(&src, "t", &["subj".into()], &[]).expect("partition");
        assert_eq!(ex, ["d1", "d2"].iter().map(|s| s.to_string()).collect());
    }

    #[test]
    fn partition_fault_on_a_surviving_repo_aborts_never_over_erases() {
        // THE critical fail-closed: a fault reading a SURVIVING repo must abort — else a
        // shared digest would be mis-classified exclusive → a retained object deleted.
        let mut src = MockDigests::new(&[("subj", &["d1", "shared"]), ("surv", &["shared"])]);
        src.fault_on = Some("surv".into());
        let err = partition_exclusive_digests(&src, "t", &["subj".into()], &["surv".into()])
            .expect_err("a surviving-repo read fault aborts");
        assert_eq!(
            err.status, 503,
            "fail-closed: nothing is classified exclusive on a partial surviving set"
        );
    }

    #[test]
    fn partition_fault_on_a_subject_repo_also_aborts() {
        let mut src = MockDigests::new(&[("subj", &["d1"])]);
        src.fault_on = Some("subj".into());
        let err =
            partition_exclusive_digests(&src, "t", &["subj".into()], &[]).expect_err("aborts");
        assert_eq!(err.status, 503);
    }

    // ── the real R2 oid-index reader (clw bar #2) ─────────────────────────────────

    /// A hermetic `R2Get`: key → bytes; `fault` forces the read-fault (fail-closed) branch.
    struct MockR2 {
        objects: std::collections::BTreeMap<String, Vec<u8>>,
        fault: bool,
    }
    impl crate::cas::R2Get for MockR2 {
        fn get_object(&self, key: &str) -> Result<Option<Vec<u8>>, String> {
            if self.fault {
                return Err("mock R2 fault".to_string());
            }
            Ok(self.objects.get(key).cloned())
        }
    }
    fn mock_r2(entries: &[(&str, &[u8])]) -> MockR2 {
        MockR2 {
            objects: entries
                .iter()
                .map(|(k, v)| ((*k).to_string(), v.to_vec()))
                .collect(),
            fault: false,
        }
    }

    #[test]
    fn r2_oid_index_source_collects_the_blake3_values() {
        // oid-index.json = {git_oid: blake3}; the digests the repo references are the VALUES.
        let r2 = mock_r2(&[(
            "d863fafb/alpha/oid-index.json",
            br#"{"oid1":"blakeaaa","oid2":"blakebbb"}"#,
        )]);
        let src = R2OidIndexDigests { r2: &r2 };
        let digs = src.repo_digests("d863fafb", "alpha").expect("read");
        assert_eq!(
            digs,
            ["blakeaaa", "blakebbb"]
                .iter()
                .map(|s| s.to_string())
                .collect()
        );
    }

    #[test]
    fn r2_oid_index_absent_index_is_empty_not_a_fault() {
        // A repo with no pushed objects → no oid-index.json → 404→None → empty set (it
        // references nothing; safe on both sides).
        let r2 = mock_r2(&[]);
        let src = R2OidIndexDigests { r2: &r2 };
        assert!(src.repo_digests("t", "none").expect("read").is_empty());
    }

    #[test]
    fn r2_oid_index_malformed_is_fail_closed_503() {
        let r2 = mock_r2(&[("t/bad/oid-index.json", b"not json at all")]);
        let src = R2OidIndexDigests { r2: &r2 };
        assert_eq!(
            src.repo_digests("t", "bad")
                .expect_err("malformed aborts")
                .status,
            503,
            "a malformed index is never a partial set"
        );
    }

    #[test]
    fn r2_oid_index_read_fault_is_fail_closed_503() {
        let r2 = MockR2 {
            objects: std::collections::BTreeMap::new(),
            fault: true,
        };
        let src = R2OidIndexDigests { r2: &r2 };
        assert_eq!(
            src.repo_digests("t", "x")
                .expect_err("read fault aborts")
                .status,
            503
        );
    }

    #[test]
    fn execute_tombstones_repos_records_partial_and_repos_become_404() {
        // v0 is ALWAYS launch-blocked (the CAS-GC seam is not wired), so an account with
        // repos gets `erasure.partial` — the repos ARE tombstoned (404 to everyone) but
        // full erasure is NOT over-claimed.
        let st = state_with(&[("alpha", "org-a", false), ("beta", "org-a", false)]);
        let outcome = execute_account_erasure(&st, "org-a", operator(), 10).expect("execute");
        assert_eq!(
            outcome,
            ErasureOutcome::Partial {
                repos_tombstoned: 2,
                blocked_reason: st_plan_note(&st),
            }
        );
        // Each repo is now tombstoned → the read/authz gate is 404 for EVERYONE.
        for repo in ["alpha", "beta"] {
            let meta = crate::authz::project_repo_meta(&st.load_verified(repo).unwrap());
            assert!(meta.erased, "{repo} carries the terminal repo.erased");
            assert!(
                !crate::authz::authorize_read(&operator(), &meta),
                "{repo} reads 404 even to operator"
            );
            assert!(
                !crate::authz::authorize_read(&[], &meta),
                "{repo} reads 404 to anon"
            );
        }
        // The account log recorded `erasure.partial`, NOT `erasure.executed`.
        let (alog, _) = st.load_account_log("org-a").unwrap();
        assert!(
            alog.records()
                .iter()
                .any(|r| r.kind == ERASURE_PARTIAL_KIND)
        );
        assert!(
            !alog
                .records()
                .iter()
                .any(|r| r.kind == ERASURE_EXECUTED_KIND),
            "NEVER an over-claim of full erasure while launch-blocked"
        );
    }

    /// The blocked-reason a v0 plan carries (the note), for the outcome assertion.
    fn st_plan_note(st: &AppState) -> String {
        plan_account_erasure(st, "org-a").unwrap().cas_gc.note
    }

    #[test]
    fn execute_is_idempotent_second_run_tombstones_nothing_new() {
        let st = state_with(&[("alpha", "org-a", false)]);
        let first = execute_account_erasure(&st, "org-a", operator(), 10).expect("first");
        assert!(matches!(
            first,
            ErasureOutcome::Partial {
                repos_tombstoned: 1,
                ..
            }
        ));
        // The repo is already terminal; a replay tombstones NOTHING new (idempotent).
        let second = execute_account_erasure(&st, "org-a", operator(), 11).expect("replay");
        assert!(
            matches!(
                second,
                ErasureOutcome::Partial {
                    repos_tombstoned: 0,
                    ..
                }
            ),
            "a replay re-tombstones nothing (idempotent): {second:?}"
        );
        // Exactly ONE repo.erased on the repo log (no double-tombstone).
        let n = st
            .load_verified("alpha")
            .unwrap()
            .records()
            .iter()
            .filter(|r| r.kind == REPO_ERASED_KIND)
            .count();
        assert_eq!(n, 1, "exactly one terminal repo.erased despite the replay");
    }

    #[test]
    fn execute_account_with_no_repos_records_executed() {
        // No owned repos → no content-addressed objects → NOT launch-blocked → the
        // cascade completes and records `erasure.executed`.
        let st = state_with(&[("gamma", "org-b", false)]); // org-a owns nothing
        let outcome = execute_account_erasure(&st, "org-a", operator(), 10).expect("execute");
        assert_eq!(
            outcome,
            ErasureOutcome::Executed {
                repos_tombstoned: 0
            }
        );
        let (alog, _) = st.load_account_log("org-a").unwrap();
        assert!(
            alog.records()
                .iter()
                .any(|r| r.kind == ERASURE_EXECUTED_KIND)
        );
        // And a replay of a completed account is a NO-OP (irreversible + idempotent).
        let replay = execute_account_erasure(&st, "org-a", operator(), 11).expect("replay");
        assert_eq!(replay, ErasureOutcome::AlreadyExecuted);
    }

    // ── the CAS-erase transport path (premise-corrected: exclusive digests ARE
    //    physically deletable via the live seam) ───────────────────────────────────

    /// A hermetic [`CasEraseTransport`]: records erased digests; `is_gone` reflects the
    /// record. `verify_gone=false` forces the anomalous post-erase not-gone branch;
    /// `fail_erase=true` forces the 503 hard-fault branch.
    struct MockErase {
        erased: std::cell::RefCell<std::collections::BTreeSet<String>>,
        verify_gone: bool,
        fail_erase: bool,
    }
    impl MockErase {
        fn new() -> Self {
            Self {
                erased: std::cell::RefCell::new(std::collections::BTreeSet::new()),
                verify_gone: true,
                fail_erase: false,
            }
        }
    }
    impl CasEraseTransport for MockErase {
        fn erase(&self, _t: &str, digest: &str, _d: &str, _r: &str) -> Result<(), EngineErr> {
            if self.fail_erase {
                return Err(EngineErr::unavailable("mock erase fault"));
            }
            self.erased.borrow_mut().insert(digest.to_string());
            Ok(())
        }
        fn is_gone(&self, _t: &str, digest: &str) -> Result<bool, EngineErr> {
            Ok(self.verify_gone && self.erased.borrow().contains(digest))
        }
    }

    #[test]
    fn execute_with_erase_erases_exclusive_digests_and_records_executed() {
        // A transport present + every exclusive digest erased + 410-verified → the
        // cascade COMPLETES → `erasure.executed` (the premise-corrected happy path).
        let st = state_with(&[("alpha", "org-a", false)]);
        let mock = MockErase::new();
        let digests = vec!["deadbeef01".to_string(), "deadbeef02".to_string()];
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            10,
            &mock,
            "d863fafb",
            &digests,
            "dsr-1",
        )
        .expect("execute");
        assert_eq!(
            outcome,
            ErasureOutcome::Executed {
                repos_tombstoned: 1
            }
        );
        assert_eq!(
            mock.erased.borrow().len(),
            2,
            "both exclusive digests erased"
        );
        let (alog, _) = st.load_account_log("org-a").unwrap();
        assert!(
            alog.records()
                .iter()
                .any(|r| r.kind == ERASURE_EXECUTED_KIND)
        );
    }

    #[test]
    fn execute_with_erase_no_exclusive_digests_is_executed_legitimate_retention() {
        // The account owns a repo but ALL its objects are shared with a SURVIVING user
        // (exclusive set empty) → nothing to physically delete → complete (legitimate
        // retention, NOT a residual-risk). `erasure.executed`.
        let st = state_with(&[("alpha", "org-a", false)]);
        let mock = MockErase::new();
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            10,
            &mock,
            "d863fafb",
            &[],
            "dsr-1",
        )
        .expect("execute");
        assert_eq!(
            outcome,
            ErasureOutcome::Executed {
                repos_tombstoned: 1
            }
        );
        assert!(
            mock.erased.borrow().is_empty(),
            "no exclusive digest to erase"
        );
    }

    #[test]
    fn execute_with_erase_is_partial_when_a_digest_is_not_410_verified() {
        // FAIL-CLOSED: a digest erased but NOT 410-verified → the cascade does NOT
        // over-claim `executed` → `erasure.partial`.
        let st = state_with(&[("alpha", "org-a", false)]);
        let mut mock = MockErase::new();
        mock.verify_gone = false;
        let digests = vec!["deadbeef01".to_string()];
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            10,
            &mock,
            "d863fafb",
            &digests,
            "dsr-1",
        )
        .expect("execute");
        assert!(
            matches!(outcome, ErasureOutcome::Partial { .. }),
            "not 410-verified → partial, never over-claim"
        );
        let (alog, _) = st.load_account_log("org-a").unwrap();
        assert!(
            alog.records()
                .iter()
                .any(|r| r.kind == ERASURE_PARTIAL_KIND)
        );
        assert!(
            !alog
                .records()
                .iter()
                .any(|r| r.kind == ERASURE_EXECUTED_KIND)
        );
    }

    #[test]
    fn execute_with_erase_503_on_erase_fault_never_claims() {
        // A hard erase fault propagates (503) BEFORE any claim — fail-closed, idempotent
        // retry converges (nothing over-claimed).
        let st = state_with(&[("alpha", "org-a", false)]);
        let mut mock = MockErase::new();
        mock.fail_erase = true;
        let digests = vec!["deadbeef01".to_string()];
        let err = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            10,
            &mock,
            "d863fafb",
            &digests,
            "dsr-1",
        )
        .expect_err("a hard erase fault is a 503");
        assert_eq!(err.status, 503);
        let (alog, _) = st.load_account_log("org-a").unwrap();
        assert!(
            !alog
                .records()
                .iter()
                .any(|r| r.kind == ERASURE_EXECUTED_KIND || r.kind == ERASURE_PARTIAL_KIND),
            "no terminal claim on a mid-erase fault (repos tombstoned, retry converges)"
        );
    }

    // ── the DATA-LOSS RACE FIX: the executor re-checks cancellation at the point of no
    //    return (an in-grace cancel can never be silently overrun) ────────────────────

    /// Seed a chain-valid account log at `_accounts/{account}.json` (Local mode) from an
    /// ordered `(kind, payload)` erasure-lifecycle history — the standing state the executor
    /// re-reads immediately before the first irreversible action.
    fn seed_account_log(st: &AppState, account: &str, records: &[(&str, serde_json::Value)]) {
        let dir = match &st.source {
            crate::state::LogSource::Local { dir } => dir.clone(),
            crate::state::LogSource::R2(_) => unreachable!("test is Local-mode"),
        };
        let adir = dir.join("_accounts");
        std::fs::create_dir_all(&adir).unwrap();
        let log = account_log(records);
        std::fs::write(
            adir.join(format!("{account}.json")),
            serde_json::to_string(log.records()).unwrap(),
        )
        .unwrap();
    }

    #[test]
    fn superseded_by_cancel_predicate_is_last_governing_and_reopenable() {
        // The pure predicate: only a cancellation that is the STANDING terminal (not
        // re-opened by a later request, not overtaken by executed/partial) supersedes.
        let cases: &[(&[(&str, serde_json::Value)], bool)] = &[
            (
                &[(REQ, serde_json::json!({"subject":"a","dsr_id":"d"}))],
                false,
            ),
            (
                &[
                    (REQ, serde_json::json!({"subject":"a","dsr_id":"d"})),
                    (
                        ERASURE_CANCELLED_KIND,
                        serde_json::json!({"state":"cancelled"}),
                    ),
                ],
                true,
            ),
            // a fresh request AFTER the cancel re-opens the lifecycle → NOT superseded.
            (
                &[
                    (REQ, serde_json::json!({"subject":"a"})),
                    (
                        ERASURE_CANCELLED_KIND,
                        serde_json::json!({"state":"cancelled"}),
                    ),
                    (REQ, serde_json::json!({"subject":"a","dsr_id":"d2"})),
                ],
                false,
            ),
            // executed is a terminal, not a cancel.
            (
                &[
                    (REQ, serde_json::json!({"subject":"a"})),
                    (
                        ERASURE_EXECUTED_KIND,
                        serde_json::json!({"state":"executed"}),
                    ),
                ],
                false,
            ),
            // cancel is the LAST governing terminal after a re-request → superseded again.
            (
                &[
                    (REQ, serde_json::json!({"subject":"a"})),
                    (
                        ERASURE_CANCELLED_KIND,
                        serde_json::json!({"state":"cancelled"}),
                    ),
                    (REQ, serde_json::json!({"subject":"a","dsr_id":"d2"})),
                    (
                        ERASURE_CANCELLED_KIND,
                        serde_json::json!({"state":"cancelled"}),
                    ),
                ],
                true,
            ),
            // MF3: a `partial` after a cancel does NOT clear the cancellation — the withdrawal
            // still governs the remaining legs (so no later sweep finishes the erasure).
            (
                &[
                    (REQ, serde_json::json!({"subject":"a","dsr_id":"d"})),
                    (
                        ERASURE_CANCELLED_KIND,
                        serde_json::json!({"state":"cancelled"}),
                    ),
                    (ERASURE_PARTIAL_KIND, serde_json::json!({"state":"partial"})),
                ],
                true,
            ),
            // a LEGIT partial with no cancel is not superseded (a re-run may still complete).
            (
                &[
                    (REQ, serde_json::json!({"subject":"a","dsr_id":"d"})),
                    (ERASURE_PARTIAL_KIND, serde_json::json!({"state":"partial"})),
                ],
                false,
            ),
        ];
        for (records, expected) in cases {
            let log = account_log(records);
            assert_eq!(
                account_erasure_superseded_by_cancel(&log),
                *expected,
                "records {records:?}"
            );
        }
        assert!(
            !account_erasure_superseded_by_cancel(&EventLog::new()),
            "an empty lifecycle is never a false cancel"
        );
    }

    #[test]
    fn executor_aborts_when_a_cancel_superseded_before_the_irreversible_erase() {
        // THE RACE (test 1): a governing request, then an `erasure.cancelled` persists
        // BEFORE the executor's first physical action. The point-of-no-return re-read finds
        // the cancel → the executor ABORTS: NO tombstone, NO CAS GC, NO `erasure.executed`,
        // the account (repo) intact. Uses the FULL transport path to prove drive_cas_gc is
        // never reached (no digest erased).
        let st = state_with(&[("alpha", "org-a", false)]);
        seed_account_log(
            &st,
            "org-a",
            &[
                (REQ, serde_json::json!({"subject":"org-a","dsr_id":"dsr-1"})),
                (
                    ERASURE_CANCELLED_KIND,
                    serde_json::json!({"state":"cancelled"}),
                ),
            ],
        );
        let mock = MockErase::new();
        let digests = vec!["deadbeef01".to_string()];
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            50,
            &mock,
            "d863fafb",
            &digests,
            "dsr-1",
        )
        .expect("a superseding cancel is a non-destructive no-op, not an error");
        assert_eq!(
            outcome,
            ErasureOutcome::Cancelled,
            "the executor abandons the cascade on a superseding cancel"
        );
        assert!(
            mock.erased.borrow().is_empty(),
            "drive_cas_gc is NEVER reached — no CAS byte physically erased"
        );
        // The repo is NOT tombstoned — it is fully intact.
        assert!(
            !repo_is_erased(&st.load_verified("alpha").unwrap()),
            "the repo is intact — no terminal repo.erased was appended"
        );
        // The account log is UNCHANGED: no executed, no partial — the cancel still governs.
        let (alog, _) = st.load_account_log("org-a").unwrap();
        assert!(
            !alog
                .records()
                .iter()
                .any(|r| r.kind == ERASURE_EXECUTED_KIND || r.kind == ERASURE_PARTIAL_KIND),
            "no terminal erasure claim is ever stamped over a standing cancellation"
        );
    }

    #[test]
    fn append_claim_cas_refuses_executed_over_a_superseding_cancel() {
        // THE CAS HARDENING (test 2): even if two racers slipped the pre-destruct re-read,
        // the terminal-claim CAS append REFUSES to record `executed` while a superseding
        // `erasure.cancelled` stands on the fresh head → Ok(false), nothing written.
        let st = state_with(&[("alpha", "org-a", false)]);
        seed_account_log(
            &st,
            "org-a",
            &[
                (REQ, serde_json::json!({"subject":"org-a","dsr_id":"dsr-1"})),
                (
                    ERASURE_CANCELLED_KIND,
                    serde_json::json!({"state":"cancelled"}),
                ),
            ],
        );
        let acct_sink: &dyn AccountLogSink = &st;
        let payload = serde_json::json!({"account":"org-a","state":"executed"});
        // allow_over_cancel=false → the CLEAN-completion case (nothing destroyed) refuses.
        let recorded = append_account_claim(
            acct_sink,
            "org-a",
            ERASURE_EXECUTED_KIND,
            &operator(),
            60,
            &payload,
            false,
        )
        .expect("refusal is not a fault");
        assert!(
            !recorded,
            "the CAS append REFUSES executed-over-cancelled on a clean completion (Ok(false))"
        );
        let (alog, _) = st.load_account_log("org-a").unwrap();
        assert!(
            !alog
                .records()
                .iter()
                .any(|r| r.kind == ERASURE_EXECUTED_KIND),
            "no erasure.executed is ever recorded over a superseding cancellation"
        );

        // MF1 mirror: with allow_over_cancel=TRUE (destruction already occurred) the SAME
        // append RECORDS `executed` truthfully over the cancel — the honest, coherent record.
        let recorded_truthful = append_account_claim(
            acct_sink,
            "org-a",
            ERASURE_EXECUTED_KIND,
            &operator(),
            61,
            &payload,
            true,
        )
        .expect("truthful executed is not a fault");
        assert!(
            recorded_truthful,
            "when destruction already happened, executed is the truthful record (not refused)"
        );
        let (alog2, _) = st.load_account_log("org-a").unwrap();
        assert!(
            alog2
                .records()
                .iter()
                .any(|r| r.kind == ERASURE_EXECUTED_KIND),
            "the truthful executed IS recorded, coherent with the (already-erased) repo logs"
        );
    }

    #[test]
    fn happy_path_matured_uncancelled_request_still_fully_executes() {
        // TEST 3: the fix does NOT touch the happy path — a standing, un-cancelled request
        // with every exclusive digest erased + 410-verified still completes to
        // `erasure.executed` (the repo tombstoned, the digests erased).
        let st = state_with(&[("alpha", "org-a", false)]);
        seed_account_log(
            &st,
            "org-a",
            &[(REQ, serde_json::json!({"subject":"org-a","dsr_id":"dsr-1"}))],
        );
        let mock = MockErase::new();
        let digests = vec!["deadbeef01".to_string(), "deadbeef02".to_string()];
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            50,
            &mock,
            "d863fafb",
            &digests,
            "dsr-1",
        )
        .expect("execute");
        assert_eq!(
            outcome,
            ErasureOutcome::Executed {
                repos_tombstoned: 1
            },
            "an un-cancelled matured request still fully executes"
        );
        assert_eq!(
            mock.erased.borrow().len(),
            2,
            "both exclusive digests erased"
        );
        assert!(repo_is_erased(&st.load_verified("alpha").unwrap()));
        let (alog, _) = st.load_account_log("org-a").unwrap();
        assert!(
            alog.records()
                .iter()
                .any(|r| r.kind == ERASURE_EXECUTED_KIND)
        );
    }

    #[test]
    fn idempotency_preserved_already_executed_is_still_a_noop_replay() {
        // TEST 4: the cancel re-check did not disturb the existing already-executed
        // idempotency — an account carrying a terminal `erasure.executed` is a no-op replay
        // (never re-tombstones, never resurrects), and it is NEVER mistaken for cancelled.
        let st = state_with(&[("alpha", "org-a", true)]); // repo already tombstoned
        seed_account_log(
            &st,
            "org-a",
            &[
                (REQ, serde_json::json!({"subject":"org-a","dsr_id":"dsr-1"})),
                (
                    ERASURE_EXECUTED_KIND,
                    serde_json::json!({"state":"executed"}),
                ),
            ],
        );
        let mock = MockErase::new();
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            70,
            &mock,
            "d863fafb",
            &["deadbeef01".to_string()],
            "dsr-1",
        )
        .expect("replay");
        assert_eq!(
            outcome,
            ErasureOutcome::AlreadyExecuted,
            "a completed account replays as a no-op, never re-runs, never Cancelled"
        );
        assert!(
            mock.erased.borrow().is_empty(),
            "an already-executed replay erases nothing new"
        );
    }

    // ── MF1/MF2: a cancel landing MID-cascade (after destruction began) is HONEST, not
    //    silently overrun with a dishonest erased:false ────────────────────────────────

    /// Append a chain-valid `erasure.cancelled` to an account log (Local mode) — the
    /// grace-boundary withdrawal, injected at a controlled instant during a cascade.
    fn append_cancel(st: &AppState, account: &str, at: u64) {
        let (mut log, token) = st.load_account_log(account).unwrap();
        let payload = serde_json::json!({"state":"cancelled","subject":account}).to_string();
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            ERASURE_CANCELLED_KIND,
            vec!["o".into()],
            hugit_refstore::canonical_json(&payload).unwrap_or(payload),
            at,
        )
        .expect("append erasure.cancelled");
        st.persist_account_log(account, &log, &token).unwrap();
    }

    /// A [`CasEraseTransport`] that INJECTS an `erasure.cancelled` into the account log on its
    /// Nth `erase` call (0-based) — reproduces a grace-boundary withdrawal that lands DURING
    /// the irreversible byte-level GC, deterministically. Delegates the actual erase to an
    /// inner [`MockErase`] so `is_gone` still reflects reality.
    struct CancelInjectingErase<'a> {
        st: &'a AppState,
        account: &'a str,
        inject_on_call: usize,
        calls: std::cell::Cell<usize>,
        inner: MockErase,
    }
    impl CasEraseTransport for CancelInjectingErase<'_> {
        fn erase(&self, t: &str, digest: &str, d: &str, r: &str) -> Result<(), EngineErr> {
            let n = self.calls.get();
            if n == self.inject_on_call {
                // Inject the withdrawal at the START of this erase call — so the per-digest
                // re-check BEFORE the NEXT digest observes the standing cancel and halts.
                append_cancel(self.st, self.account, 1_000 + n as u64);
            }
            self.calls.set(n + 1);
            self.inner.erase(t, digest, d, r)
        }
        fn is_gone(&self, t: &str, digest: &str) -> Result<bool, EngineErr> {
            self.inner.is_gone(t, digest)
        }
    }

    #[test]
    fn mf1_full_completion_with_a_late_cancel_records_the_truthful_executed_not_erased_false() {
        // MF1: the cancel lands DURING the byte-level GC (on the only erase), AFTER the repo
        // was tombstoned and the digest physically deleted — irreversible destruction is
        // COMPLETE. The executor must surface the TRUTHFUL `Executed` (coherent with the erased
        // repo log), NEVER `Cancelled`/erased:false. The overrun is flagged for the audit trail.
        let st = state_with(&[("alpha", "org-a", false)]);
        seed_account_log(
            &st,
            "org-a",
            &[(REQ, serde_json::json!({"subject":"org-a","dsr_id":"dsr-1"}))],
        );
        let erase = CancelInjectingErase {
            st: &st,
            account: "org-a",
            inject_on_call: 0, // inject on the first (only) erase → cancel present at the claim
            calls: std::cell::Cell::new(0),
            inner: MockErase::new(),
        };
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            50,
            &erase,
            "d863fafb",
            &["deadbeef01".to_string()],
            "dsr-1",
        )
        .expect("execute");
        assert_eq!(
            outcome,
            ErasureOutcome::Executed {
                repos_tombstoned: 1
            },
            "destruction COMPLETED — the truthful outcome is Executed, never Cancelled/erased:false"
        );
        // Coherence: repo tombstoned, digest gone, account log records executed OVER the cancel.
        assert!(repo_is_erased(&st.load_verified("alpha").unwrap()));
        assert_eq!(
            erase.inner.erased.borrow().len(),
            1,
            "the digest was physically erased"
        );
        let (alog, _) = st.load_account_log("org-a").unwrap();
        let executed = alog
            .records()
            .iter()
            .find(|r| r.kind == ERASURE_EXECUTED_KIND)
            .expect("erasure.executed is recorded (coherent with the erased repo)");
        let v: serde_json::Value = serde_json::from_str(&executed.payload).unwrap();
        assert_eq!(
            v.get("late_cancel_overrun").and_then(|b| b.as_bool()),
            Some(true),
            "the late cancel that lost the race is flagged for the audit trail"
        );
        assert!(
            alog.records()
                .iter()
                .any(|r| r.kind == ERASURE_CANCELLED_KIND),
            "the cancel record is preserved (append-only) — the log states the full race"
        );
    }

    #[test]
    fn mf2_pre_erase_recheck_aborts_the_remaining_gc_and_reports_the_truthful_partial() {
        // MF2: two exclusive digests; the cancel lands during the GC, AFTER the first byte is
        // deleted but BEFORE the second. The per-digest re-check must ABORT the remaining
        // erase (the second digest is SPARED), and since irreversible destruction already began
        // the outcome is the TRUTHFUL `Partial` (repos_tombstoned>0), never Cancelled/erased:false.
        let st = state_with(&[("alpha", "org-a", false)]);
        seed_account_log(
            &st,
            "org-a",
            &[(REQ, serde_json::json!({"subject":"org-a","dsr_id":"dsr-1"}))],
        );
        let erase = CancelInjectingErase {
            st: &st,
            account: "org-a",
            inject_on_call: 0, // cancel injected during erase(d1); the check before d2 then halts
            calls: std::cell::Cell::new(0),
            inner: MockErase::new(),
        };
        let digests = vec!["d1aaaaaaaa".to_string(), "d2bbbbbbbb".to_string()];
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            50,
            &erase,
            "d863fafb",
            &digests,
            "dsr-1",
        )
        .expect("execute");
        assert!(
            matches!(
                outcome,
                ErasureOutcome::Partial {
                    repos_tombstoned: 1,
                    ..
                }
            ),
            "destruction began then a cancel halted the rest → truthful Partial, got {outcome:?}"
        );
        // The SECOND digest was spared (the pre-erase re-check aborted the remaining GC).
        let erased = erase.inner.erased.borrow();
        assert!(
            erased.contains("d1aaaaaaaa"),
            "the first digest was already deleted (irreversible)"
        );
        assert!(
            !erased.contains("d2bbbbbbbb"),
            "the pre-erase re-check ABORTED the remaining physical erase — the second byte is spared"
        );
        // Coherence + honesty: repo tombstoned, account log records `partial` (not executed,
        // not a false erased:false), and the standing cancel still governs (MF3).
        assert!(repo_is_erased(&st.load_verified("alpha").unwrap()));
        let (alog, _) = st.load_account_log("org-a").unwrap();
        assert!(
            alog.records()
                .iter()
                .any(|r| r.kind == ERASURE_PARTIAL_KIND)
        );
        assert!(
            !alog
                .records()
                .iter()
                .any(|r| r.kind == ERASURE_EXECUTED_KIND)
        );
        assert!(
            account_erasure_superseded_by_cancel(&alog),
            "MF3: the partial did NOT clear the standing cancel — the withdrawal still governs"
        );
    }

    #[test]
    fn mf3_partial_cannot_clear_a_standing_cancel_remaining_repos_are_spared() {
        // MF3 end-to-end: a prior halted cascade left `[requested, cancelled, partial]` and a
        // still-pending repo `beta`. A later auto-sweep run MUST honor the standing cancel
        // (the partial did not clear it) → `Cancelled`, and `beta` is NEVER tombstoned.
        let st = state_with(&[("beta", "org-a", false)]);
        seed_account_log(
            &st,
            "org-a",
            &[
                (REQ, serde_json::json!({"subject":"org-a","dsr_id":"dsr-1"})),
                (
                    ERASURE_CANCELLED_KIND,
                    serde_json::json!({"state":"cancelled"}),
                ),
                (ERASURE_PARTIAL_KIND, serde_json::json!({"state":"partial"})),
            ],
        );
        let mock = MockErase::new();
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            90,
            &mock,
            "d863fafb",
            &["deadbeef01".to_string()],
            "dsr-1",
        )
        .expect("execute");
        assert_eq!(
            outcome,
            ErasureOutcome::Cancelled,
            "a partial can NEVER clear a standing cancel — the withdrawal governs the remaining legs"
        );
        assert!(
            !repo_is_erased(&st.load_verified("beta").unwrap()),
            "the still-pending repo is SPARED — no later sweep finishes the withdrawn erasure"
        );
        assert!(
            mock.erased.borrow().is_empty(),
            "no CAS byte is erased on the withdrawn re-run"
        );
    }

    // ── RECONCILIATION: the OLD #102 `AbortedSuperseded → 409` call sites, re-proved under
    //    the unified honest model that supersedes them (Cancelled/200 when nothing was
    //    destroyed; truthful Executed/Partial once destruction began) ─────────────────────

    /// Append a governing standing `erasure.requested` onto the account log (the #102 test
    /// fixture — the state the operator route validates BEFORE driving the cascade).
    fn seed_account_request(st: &AppState, account: &str, dsr: &str, at: u64) {
        let (mut log, tok) = st.load_account_log(account).expect("load acct");
        let payload = serde_json::json!({
            "account": account, "subject": account, "state": "requested", "dsr_id": dsr,
        })
        .to_string();
        log.append_authorized(
            PrincipalClass::Orchestrator,
            Endpoint::Land,
            REQ,
            vec!["o".into()],
            hugit_refstore::canonical_json(&payload).unwrap_or(payload),
            at,
        )
        .expect("append requested");
        st.persist_account_log(account, &log, &tok)
            .expect("persist requested");
    }

    #[test]
    fn reconcile_old102_already_cancelled_now_returns_clean_cancelled_zero_erase() {
        // OLD #102: this returned `AbortedSuperseded { repos_tombstoned: 0 } → 409`. Under the
        // superseding model it returns the honest `Cancelled` (nothing destroyed): the repo is
        // intact, the CAS transport is never invoked, and no claim is recorded. Same physical
        // guarantee (zero erase), a cleaner/coherent contract (200 clean vs a bare 409).
        let st = state_with(&[("alpha", "org-a", false)]);
        seed_account_request(&st, "org-a", "dsr-1", 1);
        append_cancel(&st, "org-a", 2); // user withdraws within grace
        let mock = MockErase::new();
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            10,
            &mock,
            "d863fafb",
            &["deadbeef01".to_string()],
            "dsr-1",
        )
        .expect("drive");
        assert_eq!(
            outcome,
            ErasureOutcome::Cancelled,
            "an already-cancelled account aborts BEFORE any tombstone → clean Cancelled"
        );
        assert!(
            mock.erased.borrow().is_empty(),
            "the CAS erase transport was NEVER invoked — no digest erased (zero physical erase)"
        );
        assert!(
            !repo_is_erased(&st.load_verified("alpha").unwrap()),
            "the repo is intact — the cancel was honored before any erase"
        );
        let (alog, _) = st.load_account_log("org-a").unwrap();
        assert!(
            !alog
                .records()
                .iter()
                .any(|r| r.kind == ERASURE_EXECUTED_KIND || r.kind == ERASURE_PARTIAL_KIND),
            "no executed/partial claim — coherent with the intact repo"
        );
    }

    #[test]
    fn reconcile_old102_cancel_mid_drive_now_reports_truthful_not_a_bare_abort() {
        // OLD #102: a cancel landing DURING the drive (injected at the CAS-GC step, after the
        // repo was tombstoned + the digest deleted) returned `AbortedSuperseded { repos_tombstoned:
        // 1 } → 409` while LEAVING the repo tombstoned and the byte gone — a dishonest
        // `erased:false`/incoherent account-vs-repo state. Under the superseding model the
        // destruction is COMPLETE, so the TRUTHFUL outcome is `Executed` (coherent with the
        // erased repo log), with `late_cancel_overrun` flagged for the audit trail.
        let st = state_with(&[("alpha", "org-a", false)]);
        seed_account_request(&st, "org-a", "dsr-1", 1);
        let erase = CancelInjectingErase {
            st: &st,
            account: "org-a",
            inject_on_call: 0, // cancel lands during the (only) physical erase
            calls: std::cell::Cell::new(0),
            inner: MockErase::new(),
        };
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            10,
            &erase,
            "d863fafb",
            &["deadbeef01".to_string()],
            "dsr-1",
        )
        .expect("drive");
        assert_eq!(
            outcome,
            ErasureOutcome::Executed {
                repos_tombstoned: 1
            },
            "destruction completed → truthful Executed (NOT a bare abort over gone data)"
        );
        assert!(repo_is_erased(&st.load_verified("alpha").unwrap()));
        let (alog, _) = st.load_account_log("org-a").unwrap();
        let executed = alog
            .records()
            .iter()
            .find(|r| r.kind == ERASURE_EXECUTED_KIND)
            .expect("erasure.executed recorded, coherent with the erased repo");
        let v: serde_json::Value = serde_json::from_str(&executed.payload).unwrap();
        assert_eq!(
            v.get("late_cancel_overrun").and_then(|b| b.as_bool()),
            Some(true),
            "the late cancel that lost the race is flagged for audit"
        );
    }

    #[test]
    fn reconcile_old102_stable_request_still_drives_to_executed() {
        // OLD #102 kept this invariant and so does the unified model: a stable governing
        // request (no cancel) drives to `executed` — the guard never gates a legitimate drive.
        let st = state_with(&[("alpha", "org-a", false)]);
        seed_account_request(&st, "org-a", "dsr-1", 1);
        let mock = MockErase::new();
        let outcome = execute_account_erasure_with_erase(
            &st,
            "org-a",
            operator(),
            10,
            &mock,
            "d863fafb",
            &["deadbeef01".to_string()],
            "dsr-1",
        )
        .expect("drive");
        assert_eq!(
            outcome,
            ErasureOutcome::Executed {
                repos_tombstoned: 1
            },
            "a stable standing request drives to executed (guard inert on the happy path)"
        );
    }

    #[test]
    fn plan_splits_the_cas_leg_and_flags_the_exclusive_gc_go_live_blocker() {
        // AUDIT #4: an account that owns repos owns content-addressed objects → the
        // account-EXCLUSIVE physical-GC obligation is REQUIRED; with the CAS-GC seam not
        // wired in v0 the plan LOUDLY flags a go-live blocker (never folded into the soft
        // shared-object disclosure).
        let st = state_with(&[("alpha", "org-a", false)]);
        let plan = plan_account_erasure(&st, "org-a").expect("plan");
        assert!(
            plan.cas_gc.required,
            "owning a repo requires the exclusive-GC partition"
        );
        assert!(
            !plan.cas_gc.seam_wired,
            "the CAS-GC seam is not wired in v0"
        );
        assert!(
            plan.is_launch_blocked(),
            "account-exclusive physical GC is an unmet obligation → a loud go-live blocker"
        );
        // The shared-object CAS leg is a distinct, defensible disclosure (not the blocker).
        assert!(
            plan.disclosures
                .iter()
                .any(|d| d.leg == DisclosureLeg::CasShared),
            "the SHARED-object CAS leg stays a disclosure, split from the exclusive-GC obligation"
        );
    }

    // ── the COMPOSITION (clw bar #3): partition (bar #1) → digests (bar #2) → drive ──

    #[test]
    fn composed_erases_exactly_the_subject_exclusive_set_the_exact_superset_guarantee() {
        // subject `alpha` (org-a) refs {d1,d2,shared}; surviving `beta` (org-b) refs
        // {shared,dv}. The composition must erase EXACTLY {d1,d2} — never `shared` (a
        // surviving user's retained object) and never `dv` (not the subject's at all).
        let st = state_with(&[("alpha", "org-a", false), ("beta", "org-b", false)]);
        let src = MockDigests::new(&[
            ("alpha", &["d1", "d2", "shared"]),
            ("beta", &["shared", "dv"]),
        ]);
        let mock = MockErase::new();
        let outcome = execute_account_erasure_composed(
            &st,
            &src,
            "org-a",
            operator(),
            10,
            &mock,
            "d863fafb",
            "dsr-1",
        )
        .expect("composed execute");
        assert_eq!(
            outcome,
            ErasureOutcome::Executed {
                repos_tombstoned: 1
            }
        );
        // EXACT-SUPERSET: what got physically erased == the computed exclusive set == {d1,d2}.
        let erased: std::collections::BTreeSet<String> = mock.erased.borrow().clone();
        assert_eq!(
            erased,
            ["d1", "d2"].iter().map(|s| s.to_string()).collect(),
            "erased exactly subject − surviving; a shared/foreign digest is never touched"
        );
    }

    #[test]
    fn composed_empty_exclusive_is_executed_legitimate_retention() {
        // The subject owns a repo but shares EVERY object with a surviving user → the
        // exclusive set is empty → tombstone the repo, delete nothing physical, claim
        // `executed` honestly (legitimate retention, not an over-claim).
        let st = state_with(&[("alpha", "org-a", false), ("beta", "org-b", false)]);
        let src = MockDigests::new(&[("alpha", &["shared"]), ("beta", &["shared"])]);
        let mock = MockErase::new();
        let outcome = execute_account_erasure_composed(
            &st,
            &src,
            "org-a",
            operator(),
            10,
            &mock,
            "d863fafb",
            "dsr-1",
        )
        .expect("composed execute");
        assert_eq!(
            outcome,
            ErasureOutcome::Executed {
                repos_tombstoned: 1
            }
        );
        assert!(
            mock.erased.borrow().is_empty(),
            "every object is shared with a surviving user → nothing physical to delete"
        );
    }

    #[test]
    fn composed_fault_reading_a_surviving_repo_aborts_503_nothing_erased() {
        // THE fail-closed contract at the composition boundary: a fault reading a SURVIVING
        // repo's digests must abort (503) BEFORE the drive — else a shared digest could be
        // mis-classified exclusive and a retained object deleted. Nothing is erased, no claim.
        let st = state_with(&[("alpha", "org-a", false), ("beta", "org-b", false)]);
        let mut src = MockDigests::new(&[("alpha", &["d1", "shared"]), ("beta", &["shared"])]);
        src.fault_on = Some("beta".into()); // the surviving repo faults
        let mock = MockErase::new();
        let err = execute_account_erasure_composed(
            &st,
            &src,
            "org-a",
            operator(),
            10,
            &mock,
            "d863fafb",
            "dsr-1",
        )
        .expect_err("a surviving-repo read fault aborts the composition");
        assert_eq!(
            err.status, 503,
            "fail-closed: nothing proceeds on a partial surviving set"
        );
        assert!(
            mock.erased.borrow().is_empty(),
            "NOT a single digest erased when the partition is indeterminate"
        );
    }
    // ── the auto-executor pure predicates (maturity + selection, no I/O) ──────────

    #[test]
    fn matured_governing_request_is_ripe() {
        // requested at 100, 50ms grace → matured at 150; superseded=false.
        assert!(
            is_matured_governing_request(100, 50, 150, false),
            "now == requested_at + grace is matured"
        );
        assert!(
            is_matured_governing_request(100, 50, 10_000, false),
            "well past grace is matured"
        );
    }

    #[test]
    fn before_grace_is_not_matured() {
        assert!(
            !is_matured_governing_request(100, 50, 149, false),
            "one ms before the grace threshold is NOT matured"
        );
    }

    #[test]
    fn superseded_request_is_never_matured() {
        assert!(
            !is_matured_governing_request(100, 50, 10_000, true),
            "a superseded (executed/cancelled) request is never matured, however old"
        );
    }

    #[test]
    fn maturity_threshold_saturates_never_wraps_into_matured() {
        // A pathological grace can NEVER wrap the threshold down into a false "matured".
        assert!(
            !is_matured_governing_request(u64::MAX, u64::MAX, 0, false),
            "saturating add keeps an impossible threshold un-matured"
        );
    }

    #[test]
    fn should_auto_execute_drives_matured_with_dsr() {
        let s = StandingErasureRequest {
            subject: "org-a".into(),
            dsr_id: Some("dsr-1".into()),
            requested_at: 100,
        };
        assert!(
            should_auto_execute(Some(&s), 50, 200),
            "matured governing request WITH a DSR id → drive"
        );
    }

    #[test]
    fn should_auto_execute_skips_not_yet_matured() {
        let s = StandingErasureRequest {
            subject: "org-a".into(),
            dsr_id: Some("dsr-1".into()),
            requested_at: 100,
        };
        assert!(
            !should_auto_execute(Some(&s), 50, 120),
            "grace not elapsed → skip"
        );
    }

    #[test]
    fn should_auto_execute_skips_superseded_or_absent() {
        // `read_standing_erasure_request` returns None for a superseded (executed/cancelled)
        // OR never-requested account → the sweep must skip it (idempotent no-op).
        assert!(
            !should_auto_execute(None, 50, 10_000),
            "no governing request (superseded/absent) → never drive"
        );
    }

    #[test]
    fn should_auto_execute_skips_matured_without_dsr_id() {
        // Fail-closed: a physical erase is refused without an anchored DSR legitimacy id
        // (mirrors the operator route's 403), even when the grace has fully matured.
        let no_id = StandingErasureRequest {
            subject: "org-a".into(),
            dsr_id: None,
            requested_at: 100,
        };
        assert!(
            !should_auto_execute(Some(&no_id), 50, 10_000),
            "matured but NO DSR id → skip (no legitimacy id, no physical erase)"
        );
        let empty_id = StandingErasureRequest {
            subject: "org-a".into(),
            dsr_id: Some(String::new()),
            requested_at: 100,
        };
        assert!(
            !should_auto_execute(Some(&empty_id), 50, 10_000),
            "an empty DSR id is treated as absent → skip"
        );
    }
}
