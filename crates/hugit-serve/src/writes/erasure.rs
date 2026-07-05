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
//! terminal, so the read/authz gate serves 404 and the git wire serves nothing. The
//! CoreLink CAS is cross-tenant DEDUPLICATED, so a shared object cannot be physically
//! deleted unilaterally — the honest v0 posture is manifest/repo tombstone + a
//! non-empty RESIDUAL-RISK disclosure for the shared-object CAS leg (the X7/X12
//! mirror-leg discipline). The disclosure IS the deliverable where physical delete is
//! not unilaterally provable.

use hugit_refstore::EventLog;

use crate::error::EngineErr;
use crate::state::AppState;

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
}
