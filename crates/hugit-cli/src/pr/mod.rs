//! `hugit pr open | land | show` — the PR-altitude porcelain (WP-PC3).
//!
//! A PR is a **bundle of intents** proposed to land together. This module is
//! the verb surface for that lifecycle; it owns no new primitive. Every
//! operation runs through the REAL engine seams:
//!
//! - the append-only, hash-chained event log
//!   ([`hugit_refstore::EventLog`]) — the source of truth a PR's lifecycle
//!   events are appended to and projected back out of (the same store
//!   `intent.landed` lives on; this module adds the `pr.opened` / `pr.queued`
//!   kinds **additively**, never a parallel store);
//! - the union-testing landing queue ([`hugit_queue::core`]) — `land` enters
//!   it through [`hugit_contracts::LandableEntry`] + [`hugit_queue::core::Batch`],
//!   so the reported position is the queue's own ordering, never invented;
//! - the F3 three-altitude rollup ([`hugit_ledger::pr_record`]) — `show`
//!   surfaces the PR's cost block when (and only when) the envelope/metrics
//!   data is present in the log.
//!
//! # Design law (the porcelain wave, 2026-06-10)
//!
//! - **Stable JSON on stdout always.** Every command returns a single JSON
//!   value (success or structured error). Structured errors carry the
//!   suggested fix.
//! - **Idempotent.** Re-running `open` with the same PR id + campaign returns
//!   the existing record (`"already_exists":true`, exit 0). Re-running `land`
//!   on an already-queued PR returns `"already_queued":true`, exit 0. Agents
//!   retry safely.
//! - **D14 at the door.** `open` takes an explicit `--author-kind`
//!   (`orchestrator | human`); `subagent` is NOT an accepted value — it is
//!   rejected with a structured error explaining the rule (a PR author is an
//!   orchestrator or a human, never a subagent — ADR-0001 §2.3/§5). The
//!   forge enforces the same rule again at the projection level (the F3
//!   rollup's `SubagentAuthor`); this is the porcelain's first line.
//! - **Hermetic-first.** Operates on the local event log through the real
//!   append/projection paths; live DO/CAS binding stays the P2 disclosed
//!   seam.
//! - **Honest gaps.** `show`'s cost block is the F3 `pr_record` rollup; its
//!   fields are honestly absent/null when the envelope/metrics data was never
//!   captured (sub-`full` capture, or no PR-altitude envelope on the log).

pub mod capture;
mod cli;
pub mod dispatch;
pub mod filelock;

pub use capture::{EnvelopeMetricsArgs, capture_on_land};
pub use cli::{PrArgs, PrCommand, run};

use std::collections::BTreeSet;

use hugit_contracts::context_envelope::{Altitude, CiCost, ContextEnvelope, PrRecord};
use hugit_contracts::event_record::EventRecord;
use hugit_contracts::verdict_object::VerdictObject;
use hugit_ledger::rollup::{PrQueueInput, pr_record};
use hugit_queue::core::affected::AffectedSet;
use hugit_queue::core::batch::Batch;
use hugit_refstore::EventLog;
use serde::Serialize;
use serde_json::{Value, json};

use crate::porcelain::PorcelainError;

/// Event kind: a PR was proposed (PROPOSED state). Additive over the D1 log,
/// same store as `intent.landed`. Payload (canonical JSON):
/// `{"pr_id","campaign","author_kind","run_id","principal","intent_ids":[...],
/// "commit_ids":[...]}`. `commit_ids` is the additive W2 field: captured
/// raw-commit members (see [`OpenArgs::commit_ids`]).
pub const PR_OPENED_KIND: &str = "pr.opened";

/// Event kind: a PR entered the landing queue. Payload (canonical JSON):
/// `{"pr_id","item_id","order_index","mode":"union"}`.
pub const PR_QUEUED_KIND: &str = "pr.queued";

/// Event kind: a PR was abandoned (terminal — leaves the queue projection).
/// Additive over the D1 log, same store as `pr.opened`. Payload (canonical
/// JSON): `{"pr_id","reason"}`. Routed through the D14-guarded append.
pub const PR_ABANDONED_KIND: &str = "pr.abandoned";

/// Event kind: a PR settled as LANDED (terminal). Emitted by `pr land --settle`
/// — the explicit land-confirm settlement step (W-PRLANDED). `abandon` refuses
/// to abandon a PR the log already records as landed (abandoning a landed PR is
/// a refusal), and the campaign world projects a `pr.landed` PR as `landed`
/// (out of the in-flight set, so `campaign close` reaches `closed:true`).
///
/// # Settlement honesty (P2 disclosed seam — not a faked union-test verdict)
///
/// `pr land --settle` requires the PR to be queued first (it is the
/// land-confirm over a PR that has entered the queue) and then records the
/// settlement. It does NOT run the union-test / queue disjointness verdict that
/// the B3 engine arbitrates over real tree hashes — that arbitration is the P2
/// live-infra seam (see [`land`]). So `--settle` is the explicit operator/
/// orchestrator confirmation that the PR has landed; AUTOMATIC settlement
/// (emit `pr.landed` the moment the queue verdict says the batch is disjoint
/// and green) awaits that P2 queue-verdict seam. The porcelain never fabricates
/// a union-test result. The payload carries `{"campaign","pr_id"}` (the
/// campaign is recovered from the PR's `pr.opened`) so the campaign world
/// scopes the settlement to the right campaign.
pub const PR_LANDED_KIND: &str = "pr.landed";

/// Event kind a `hugit campaign open` writes (the campaign module's own
/// constant, restated here as the read-only vocabulary `pr open --campaign`
/// validates against — a `campaign.opened` payload carries `{"campaign":…}`).
/// The campaign module owns the WRITE; this is the read-side mirror of the
/// stable wire string, so the symmetry check needs no cross-module dependency.
pub const CAMPAIGN_OPENED_KIND: &str = "campaign.opened";

/// The landing mode a PR enters the queue under — always `union` in v1 (the
/// union-testing landing queue is the only landing path; the wedge property).
pub const LANDING_MODE: &str = "union";

/// The author kind the PR-altitude D14 authz rule accepts at the door.
///
/// **`subagent` is deliberately not a variant.** A PR author is an
/// orchestrator or a human, never a subagent (ADR-0001 §2.3/§5); the absence
/// of the variant encodes the rule at the type level. The string `"subagent"`
/// (or anything else) is rejected by [`AuthorKind::parse`] with a structured
/// error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorKind {
    /// The orchestrator session that planned/dispatched/landed the bundle.
    Orchestrator,
    /// A human principal.
    Human,
}

impl AuthorKind {
    /// Parse an `--author-kind` token, enforcing D14 at the door.
    ///
    /// `orchestrator` / `human` accepted; `subagent` and any other value
    /// rejected (the caller turns the [`None`] into the structured
    /// `subagent_author` error so the message can name the offending value).
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "orchestrator" => Some(AuthorKind::Orchestrator),
            "human" => Some(AuthorKind::Human),
            _ => None,
        }
    }

    /// The wire token for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            AuthorKind::Orchestrator => "orchestrator",
            AuthorKind::Human => "human",
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Errors — structured, JSON, fix-carrying (design law).
// ─────────────────────────────────────────────────────────────────────────────

/// A structured PR-command error. Every variant serialises to a single JSON
/// object `{"error":<code>,"message":<text>,"fix":<suggested fix>, …}` so an
/// agent can branch on `error` and a human can read `message` + `fix`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrError {
    /// `--author-kind subagent` (or any non-`orchestrator`/`human` value) was
    /// supplied: D14 refusal at the door.
    SubagentAuthor {
        /// The rejected value the caller passed.
        got: String,
    },
    /// `open` was asked for a PR id that already exists with a *different*
    /// campaign — not the idempotent re-open case (same campaign), a genuine
    /// conflict.
    CampaignMismatch {
        /// The PR id.
        pr_id: String,
        /// The campaign already recorded for this PR.
        existing: String,
        /// The campaign the re-open attempt supplied.
        attempted: String,
    },
    /// `land` was asked for a PR id with no `pr.opened` on the log.
    UnknownPr {
        /// The PR id.
        pr_id: String,
    },
    /// `land` was asked to queue a PR that bundles zero intents — nothing to
    /// land.
    EmptyPr {
        /// The PR id.
        pr_id: String,
    },
    /// A queue position could not be derived because the queue rejected the
    /// batch (e.g. a duplicate order_index across already-queued PRs — a
    /// corrupt log). Carries the queue engine's message.
    QueueRefused {
        /// The PR id being landed.
        pr_id: String,
        /// The queue engine's reason.
        reason: String,
    },
    /// `show` was asked for a PR id with no `pr.opened` on the log.
    NotFound {
        /// The PR id.
        pr_id: String,
    },
    /// `open` referenced `--intent` ids the shared canonical log does NOT carry.
    ///
    /// Validation fires only when the log already carries at least one landed
    /// intent (the seam is in use); an intent-less log opts out, so an
    /// intent-less fixture stays valid. Names the missing ids so the agent can
    /// `hugit intent new --log` them first.
    MissingIntents {
        /// The PR id being opened.
        pr_id: String,
        /// The `--intent` ids not present on the log.
        missing: Vec<String>,
    },
    /// Referential asymmetry (audit P5): `--author-kind orchestrator` was
    /// supplied without the `--run-id` it requires (or `human` without
    /// `--principal`). The author class is named so the agent knows which flag
    /// to add.
    MissingAuthorBinding {
        /// The author kind whose binding flag is missing.
        author_kind: AuthorKind,
    },
    /// `open` referenced `--commit` oids (or `--commit-ref` targets) the log
    /// does NOT carry as captured `ref.update` targets. This is the honest
    /// `commit_not_found` — distinct from [`PrError::MissingIntents`]: a raw
    /// commit bundled into a PR must be PROVABLY captured by the hooks, never
    /// assumed. Naming the missing ids so the agent can retry after the capture
    /// lands.
    MissingCommits {
        /// The PR id being opened.
        pr_id: String,
        /// The commit oids (or unresolved `--commit-ref` refs) not on the log.
        missing: Vec<String>,
    },
    /// `open --campaign X` named a campaign the log does NOT carry, on a log that
    /// DOES carry campaign vocabulary (`campaign.opened` records). A log with no
    /// campaign records at all keeps the old permissive behavior (no error).
    UnknownCampaign {
        /// The PR id being opened.
        pr_id: String,
        /// The campaign key not found among the log's `campaign.opened` records.
        campaign: String,
    },
    /// `abandon` was asked for a PR id with no `pr.opened` on the log.
    AbandonUnknownPr {
        /// The PR id.
        pr_id: String,
    },
    /// `abandon` was asked to abandon a PR the log already records as LANDED —
    /// a terminal state; abandoning it is a refusal.
    AbandonLanded {
        /// The PR id.
        pr_id: String,
    },
    /// `land --settle` was asked to settle a PR that has not entered the queue
    /// (`pr land` first). Settlement is the land-confirm over a *queued* PR.
    SettleNotQueued {
        /// The PR id.
        pr_id: String,
    },
    /// An internal fault: a post-append projection failed to find the record that
    /// was just appended. This is a bug (infallible by construction), not a user
    /// error. Exits with the reserved internal-fault code (`1`), still as
    /// structured JSON (`{"error":{"kind":"internal",…}}`).
    Internal {
        /// A short description of what went wrong (never user-supplied input).
        message: String,
    },
}

impl PrError {
    /// The stable machine-readable error code (the `kind` field of the canonical
    /// `{"error":{"kind":…}}` envelope).
    pub fn code(&self) -> &'static str {
        match self {
            PrError::SubagentAuthor { .. } => "subagent_author",
            PrError::CampaignMismatch { .. } => "campaign_mismatch",
            PrError::UnknownPr { .. } => "unknown_pr",
            PrError::EmptyPr { .. } => "empty_pr",
            PrError::QueueRefused { .. } => "queue_refused",
            PrError::NotFound { .. } => "pr_not_found",
            PrError::MissingIntents { .. } => "missing_intents",
            PrError::MissingAuthorBinding { .. } => "missing_author_binding",
            PrError::MissingCommits { .. } => "commit_not_found",
            PrError::UnknownCampaign { .. } => "unknown_campaign",
            PrError::AbandonUnknownPr { .. } => "unknown_pr",
            PrError::AbandonLanded { .. } => "pr_already_landed",
            PrError::SettleNotQueued { .. } => "pr_not_queued",
            PrError::Internal { .. } => "internal",
        }
    }

    /// Build the canonical [`PorcelainError`] for this PR error (the one error
    /// law: nested `{"error":{"kind","message","fix", …context}}`, `fix` key —
    /// never a flat object, never `suggested_fix`). All flat domain fields are
    /// folded in as structured context.
    pub fn to_porcelain(&self) -> PorcelainError {
        match self {
            PrError::SubagentAuthor { got } => PorcelainError::new(
                self.code(),
                format!(
                    "author-kind '{got}' is not accepted: a PR author is an \
                     orchestrator or a human, never a subagent (D14)"
                ),
                "pass --author-kind orchestrator (with --run-id) or \
                 --author-kind human (with --principal)",
            )
            .with_context("got", json!(got)),
            PrError::CampaignMismatch {
                pr_id,
                existing,
                attempted,
            } => PorcelainError::new(
                self.code(),
                format!(
                    "pr '{pr_id}' already exists under campaign '{existing}', \
                     cannot re-open under '{attempted}'"
                ),
                format!("re-open with --campaign {existing}, or open a new PR id"),
            )
            .with_context("pr_id", json!(pr_id))
            .with_context("existing_campaign", json!(existing))
            .with_context("attempted_campaign", json!(attempted)),
            PrError::UnknownPr { pr_id } => PorcelainError::new(
                self.code(),
                format!("pr '{pr_id}' has no pr.opened on the log — open it first"),
                format!("hugit pr open --pr {pr_id} --campaign <key> --intent <id>…"),
            )
            .with_context("pr_id", json!(pr_id)),
            PrError::EmptyPr { pr_id } => PorcelainError::new(
                self.code(),
                format!("pr '{pr_id}' bundles zero intents — nothing to land"),
                "re-open the PR with one or more --intent <id> before landing",
            )
            .with_context("pr_id", json!(pr_id)),
            PrError::QueueRefused { pr_id, reason } => PorcelainError::new(
                self.code(),
                format!("landing queue refused pr '{pr_id}': {reason}"),
                "the event log's queue order is corrupt; inspect with hugit pr show",
            )
            .with_context("pr_id", json!(pr_id)),
            PrError::NotFound { pr_id } => PorcelainError::new(
                self.code(),
                format!("pr '{pr_id}' not found on the log"),
                format!("hugit pr open --pr {pr_id} … to create it"),
            )
            .with_context("pr_id", json!(pr_id)),
            PrError::MissingIntents { pr_id, missing } => PorcelainError::new(
                self.code(),
                format!(
                    "pr '{pr_id}' references intent(s) not on the log: {}",
                    missing.join(", ")
                ),
                format!(
                    "land them first: hugit intent new --log <log> --campaign <key> \
                     --charter <c> --id {} …",
                    missing.first().map(String::as_str).unwrap_or("<id>")
                ),
            )
            .with_context("pr_id", json!(pr_id))
            .with_context("missing_intents", json!(missing)),
            PrError::MissingCommits { pr_id, missing } => PorcelainError::new(
                self.code(),
                format!(
                    "pr '{pr_id}' references commit(s) not captured on the log: {}",
                    missing.join(", ")
                ),
                "the commit must be captured by the silent hooks first \
                 (a real git commit fires ref.update); re-run once the capture \
                 is on the log",
            )
            .with_context("pr_id", json!(pr_id))
            .with_context("missing_commits", json!(missing)),
            PrError::MissingAuthorBinding { author_kind } => {
                let (got_flag, needs) = match author_kind {
                    AuthorKind::Orchestrator => ("--author-kind orchestrator", "--run-id <id>"),
                    AuthorKind::Human => ("--author-kind human", "--principal <id>"),
                };
                PorcelainError::new(
                    self.code(),
                    format!(
                        "{got_flag} requires {needs}: an author kind must bind to \
                         its principal (referential symmetry — D14)"
                    ),
                    format!("re-run with {needs}"),
                )
                .with_context("author_kind", json!(author_kind.as_str()))
            }
            PrError::UnknownCampaign { pr_id, campaign } => PorcelainError::new(
                self.code(),
                format!(
                    "pr '{pr_id}' names campaign '{campaign}', which has no \
                     campaign.opened on the log"
                ),
                format!("hugit campaign open --campaign {campaign} … first"),
            )
            .with_context("pr_id", json!(pr_id))
            .with_context("campaign", json!(campaign)),
            PrError::AbandonUnknownPr { pr_id } => PorcelainError::new(
                self.code(),
                format!("pr '{pr_id}' has no pr.opened on the log — nothing to abandon"),
                format!("hugit pr open --pr {pr_id} --campaign <key> --intent <id>… first"),
            )
            .with_context("pr_id", json!(pr_id)),
            PrError::AbandonLanded { pr_id } => PorcelainError::new(
                self.code(),
                format!("pr '{pr_id}' has already landed — a landed PR cannot be abandoned"),
                "a landed PR is terminal; nothing to abandon",
            )
            .with_context("pr_id", json!(pr_id)),
            PrError::SettleNotQueued { pr_id } => PorcelainError::new(
                self.code(),
                format!(
                    "pr '{pr_id}' has not entered the landing queue — settle is the \
                     land-confirm over a queued PR"
                ),
                format!("hugit pr land --pr {pr_id} first, then --settle"),
            )
            .with_context("pr_id", json!(pr_id)),
            PrError::Internal { message } => PorcelainError::internal(message.clone()),
        }
    }

    /// Serialise to the canonical `{"error":{…}}` JSON object (nested, `fix`).
    pub fn to_json(&self) -> Value {
        // `PorcelainError::to_json` renders the canonical string; parse it back
        // to a `Value` so the library API stays `Value`-typed.
        serde_json::from_str(&self.to_porcelain().to_json())
            .expect("PorcelainError renders valid JSON")
    }
}

impl std::fmt::Display for PrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.to_porcelain().to_json())
    }
}

impl std::error::Error for PrError {}

// ─────────────────────────────────────────────────────────────────────────────
// open — bundle intents into a PROPOSED PR.
// ─────────────────────────────────────────────────────────────────────────────

/// Inputs to `hugit pr open`.
#[derive(Debug, Clone)]
pub struct OpenArgs {
    /// PR id (`--pr <n|auto>`). The caller resolves `auto` to a concrete id
    /// before calling (the porcelain owns id allocation); this is the chosen id.
    pub pr_id: String,
    /// Campaign key the PR belongs to (`--campaign <key>`).
    pub campaign: String,
    /// Author kind (D14 at the door) — already validated to be
    /// orchestrator/human by [`AuthorKind::parse`].
    pub author_kind: AuthorKind,
    /// Orchestrator run id (`--run-id`), when `author_kind == Orchestrator`.
    pub run_id: Option<String>,
    /// Human principal (`--principal`), when `author_kind == Human`.
    pub principal: Option<String>,
    /// Bundled intent ids (`--intent <id>…`, repeatable).
    pub intent_ids: Vec<String>,
    /// Captured commit oids (`--commit <oid>…`, repeatable) — raw-commit
    /// members of the PR bundle, recorded **externally** (the `pr.opened`
    /// payload carries them under `commit_ids`, NEVER forged into an intent;
    /// the no-fake-intent invariant). Every oid MUST be present on the log as a
    /// `ref.update` payload `target` ([`PrError::MissingCommits`] otherwise).
    /// The `--commit-ref <ref>` shape is resolved to its captured target oid by
    /// the CLI layer before this struct is built, so this field is oids only.
    pub commit_ids: Vec<String>,
    /// Unix ms to stamp the appended event with.
    pub recorded_at: u64,
}

/// `hugit pr open` — bundle intents into a PROPOSED PR (appends `pr.opened`).
///
/// Idempotent: if a `pr.opened` already exists for this PR id **with the same
/// campaign**, no event is appended and the existing record is returned with
/// `"already_exists":true` (exit 0). A re-open with a *different* campaign is a
/// genuine conflict ([`PrError::CampaignMismatch`]).
///
/// The D14 author-kind is validated by the caller via [`AuthorKind::parse`]
/// (a `subagent` token never reaches here as a valid [`AuthorKind`]).
///
/// **Intent validation (PC4).** When the shared canonical log already carries at
/// least one landed intent (the `intent new --log` seam is in use), every
/// `--intent` id MUST be present on the log — a missing id is a
/// [`PrError::MissingIntents`] refusal naming the gaps. An intent-less log opts
/// out (an intent-less fixture stays valid), so the check never breaks a log
/// that does not yet use the intent seam.
///
/// **Commit validation (W2).** Every `--commit` oid MUST be present on the log
/// as the `target` of a captured `ref.update` record — the silent hooks
/// (post-commit) are the ONLY prover a raw commit is real. A missing oid is a
/// [`PrError::MissingCommits`] refusal (`commit_not_found`), deliberately
/// distinct from `missing_intents`: a raw commit is an EXTERNAL PR member, never
/// forged into an intent. Unlike the intent check there is NO opt-out — an
/// uncaptured commit is unprovable and must fail closed.
///
/// **Referential symmetry (audit P5).** `--author-kind orchestrator` REQUIRES
/// `--run-id` and `--author-kind human` REQUIRES `--principal` — a missing
/// binding is a [`PrError::MissingAuthorBinding`] refusal naming the flag. And
/// `--campaign X` is validated against the log: when the log already carries
/// campaign vocabulary (`campaign.opened` records) and `X` is not among them,
/// it is a [`PrError::UnknownCampaign`] refusal. A log with NO campaign records
/// at all keeps the old permissive behavior (an early fixture log without the
/// campaign seam in use opts out of the check).
pub fn open(log: &mut EventLog, args: &OpenArgs) -> Result<Value, PrError> {
    // WJ-UNIFY: normalise the args at entry through the ONE structural identifier
    // scrub (not the full free-text engine). Identifier ADDRESSES
    // (`pr_id`/`campaign`/`run_id`/`principal`) get `structural_secret_scrub`: a
    // bare 40/64-hex/slug address SURVIVES verbatim (no collapse → no wrong-PR
    // landing), a prefixed secret REDACTS. The lookup key and the stored payload
    // value are then identical, so the join stays symmetric and the central
    // boundary (`scrub_to_canonical` on the payload) re-applies the same scrub.
    let args = &scrub_open_args(args);
    // Idempotency: look for an existing pr.opened for this id.
    if let Some(existing) = find_pr_opened(log, &args.pr_id) {
        if existing.campaign != args.campaign {
            return Err(PrError::CampaignMismatch {
                pr_id: args.pr_id.clone(),
                existing: existing.campaign.clone(),
                attempted: args.campaign.clone(),
            });
        }
        // Same campaign → idempotent no-op: return the existing record.
        return Ok(open_json(&existing, true));
    }

    // Referential symmetry: the author kind must carry its binding flag.
    validate_author_binding(args.author_kind, &args.run_id, &args.principal)?;
    // Campaign symmetry: a campaign named on a campaign-aware log must exist.
    validate_campaign(log, &args.pr_id, &args.campaign)?;

    validate_intents(log, args)?;
    validate_commits(log, args)?;

    // `args` is already WG-SCRUB-scrubbed at entry, so the principal chain and
    // payload are both built from redacted user strings.
    let principal_chain = author_principal_chain(args.author_kind, &args.run_id, &args.principal);
    let payload = canonical_open_payload(args);
    // D14 on the mutation primitive: route the pr.opened append through the
    // guarded entry point. The author class is the caller-asserted --author-kind
    // (orchestrator|human — never a subagent; the binding of that assertion to an
    // authenticated principal is the disclosed P2/identity seam). A worker/model
    // class asserted here is denied by the matrix and audited, never appended.
    let (class, endpoint) = author_authz(args.author_kind);
    log.append_authorized(
        class,
        endpoint,
        PR_OPENED_KIND,
        principal_chain,
        payload,
        args.recorded_at,
    )
    .map_err(|denied| PrError::SubagentAuthor {
        got: denied.reason.code().to_string(),
    })?;

    let opened = find_pr_opened(log, &args.pr_id).ok_or_else(|| PrError::Internal {
        message: format!(
            "pr.opened for '{}' not found after a successful append — \
             the payload round-tripped through canonical_json into a form that \
             parse_opened could not project back; this is a hugit internal bug",
            args.pr_id
        ),
    })?;
    Ok(open_json(&opened, false))
}

/// The shared canonical event kind a landed intent is recorded under — the
/// read-side mirror of [`hugit_refstore::intent::INTENT_LANDED_KIND`], restated
/// so the intent-existence check keys on the EXACT same raw record vocabulary
/// the `verdict`/`tournament` verbs do (no projection in between).
const INTENT_LANDED_KIND: &str = "intent.landed";

/// Whether the log carries intent vocabulary at all — at least one raw
/// `intent.landed` record. Keying the opt-out on raw record presence (not on a
/// successful PROJECTION) matches `verdict`/`tournament` EXACTLY: a log with a
/// malformed `intent.landed` payload still HAS the vocabulary, so the check
/// must fire rather than be silently skipped (WI-PR defect 2 — the silent-skip
/// gap that let a ghost `--intent` through and produced an un-provable PR).
fn log_has_intent_vocabulary(log: &EventLog) -> bool {
    log.records().iter().any(|r| r.kind == INTENT_LANDED_KIND)
}

/// Whether a raw `intent.landed` record names `intent_id` — the SAME raw-payload
/// existence rule `verdict`'s `intent_is_on_log` uses (read straight off the
/// record's `intent_id`, never through the fail-closed projection).
fn intent_landed_on_log(log: &EventLog, intent_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == INTENT_LANDED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("intent_id").and_then(Value::as_str) == Some(intent_id))
}

/// Validate that the PR's `--intent` ids exist on the shared canonical log.
///
/// **Symmetry with `verdict`/`tournament` (WI-PR defect 2).** A PR whose
/// `--intent` references an id NOT on the log is permanently un-provable — a
/// later `hugit verdict --intent <id>` refuses it (`intent_not_found`). So
/// `pr open` validates intent existence by the EXACT rule the verdict verb uses:
/// keyed on raw `intent.landed` record vocabulary ([`log_has_intent_vocabulary`])
/// and raw-payload existence ([`intent_landed_on_log`]) — NOT on the fail-closed
/// projection (which previously skipped validation whole on a malformed payload,
/// the silent-skip gap that let a ghost intent through).
///
/// Opt-out by design: a log carrying NO `intent.landed` vocabulary is not using
/// the intent seam, so any `--intent` ids are accepted (an intent-less fixture
/// log stays valid — the documented permissive rule, mirroring verdict). Once
/// the log carries the vocabulary, every referenced id MUST be present — a gap
/// is a structured [`PrError::MissingIntents`] (exit-2, naming the missing ids).
fn validate_intents(log: &EventLog, args: &OpenArgs) -> Result<(), PrError> {
    if !log_has_intent_vocabulary(log) {
        // The intent seam is not in use on this log — opt out (matches verdict).
        return Ok(());
    }
    let missing: Vec<String> = args
        .intent_ids
        .iter()
        .filter(|id| !intent_landed_on_log(log, id))
        .cloned()
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(PrError::MissingIntents {
            pr_id: args.pr_id.clone(),
            missing,
        })
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// W2 — captured raw commits as external PR members (`--commit` / `--commit-ref`).
// ─────────────────────────────────────────────────────────────────────────────

/// The shared canonical event kind a silent-hook / receive-pack capture is
/// recorded under — the read-side mirror of the raw external-change vocabulary
/// ([`hugit_refstore::intent::model::ExternalChangeKind::RefUpdate`], appended
/// by `hugit capture commit` and `hugit-proto`'s `record_ref_update`), restated
/// so the commit lookup keys on the EXACT same raw record vocabulary the capture
/// path writes. No projection in between — the same raw-vocabulary discipline
/// [`validate_intents`] uses.
const REF_UPDATE_KIND: &str = "ref.update";

/// Whether the log carries a captured `ref.update` whose payload `target` is
/// exactly `oid` — the W2 existence rule for a `--commit` member. Reads the raw
/// `ref.update` payloads straight off the records (never through a fail-closed
/// projection), matching the raw-vocabulary discipline of [`intent_landed_on_log`].
fn commit_ref_target_on_log(log: &EventLog, oid: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == REF_UPDATE_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("target").and_then(Value::as_str) == Some(oid))
}

/// Resolve the `target` of the LATEST `ref.update` record whose payload `ref` is
/// exactly `ref_name` — the `--commit-ref` lookup. `None` when the ref was never
/// captured (or its payload carries no `target`), so an unresolvable
/// `--commit-ref` is honestly a missing capture, surfaced as
/// [`PrError::MissingCommits`] by the CLI layer.
pub fn commit_ref_target_for_ref(log: &EventLog, ref_name: &str) -> Option<String> {
    log.records()
        .iter()
        .filter(|r| r.kind == REF_UPDATE_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter(|v| v.get("ref").and_then(Value::as_str) == Some(ref_name))
        .filter_map(|v| v.get("target").and_then(Value::as_str).map(str::to_string))
        .next_back()
}

/// Validate that every `--commit` oid is PROVABLY captured on the log — present
/// as the `target` of a raw `ref.update` record.
///
/// **W2 fail-closed rule.** Unlike [`validate_intents`] there is NO vocabulary
/// opt-out: the silent hooks are the only prover a raw commit exists, so an
/// uncaptured oid is unprovable and MUST refuse ([`PrError::MissingCommits`],
/// exit-2, `kind:"commit_not_found"`). A raw commit is never forged into an
/// intent — the no-fake-intent invariant is structural (the `pr.opened` payload
/// carries commits under `commit_ids`, separate from `intent_ids`).
fn validate_commits(log: &EventLog, args: &OpenArgs) -> Result<(), PrError> {
    let missing: Vec<String> = args
        .commit_ids
        .iter()
        .filter(|oid| !commit_ref_target_on_log(log, oid))
        .cloned()
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(PrError::MissingCommits {
            pr_id: args.pr_id.clone(),
            missing,
        })
    }
}

/// Referential symmetry (audit P5): the author kind must carry its binding flag
/// — `orchestrator` REQUIRES `--run-id`, `human` REQUIRES `--principal`. A
/// missing/blank binding is a [`PrError::MissingAuthorBinding`] refusal naming
/// the flag (the `--author-kind subagent` door check is upstream of this).
fn validate_author_binding(
    author_kind: AuthorKind,
    run_id: &Option<String>,
    principal: &Option<String>,
) -> Result<(), PrError> {
    let bound = match author_kind {
        AuthorKind::Orchestrator => run_id.as_deref().is_some_and(|s| !s.is_empty()),
        AuthorKind::Human => principal.as_deref().is_some_and(|s| !s.is_empty()),
    };
    if bound {
        Ok(())
    } else {
        Err(PrError::MissingAuthorBinding { author_kind })
    }
}

/// Whether the log carries campaign vocabulary at all — at least one
/// `campaign.opened` record. The campaign-existence check opts out entirely when
/// this is false (the documented permissive behavior for a log that does not yet
/// use the campaign seam).
fn log_has_campaign_vocabulary(log: &EventLog) -> bool {
    log.records().iter().any(|r| r.kind == CAMPAIGN_OPENED_KIND)
}

/// Whether `campaign` is named by a `campaign.opened` record on the log.
fn campaign_exists(log: &EventLog, campaign: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == CAMPAIGN_OPENED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("campaign").and_then(Value::as_str) == Some(campaign))
}

/// Campaign symmetry (audit P5): validate `--campaign` against the log.
///
/// When the log carries campaign vocabulary ([`log_has_campaign_vocabulary`])
/// and the named campaign has no `campaign.opened` record, refuse with
/// [`PrError::UnknownCampaign`] (naming it + a fix). A log with NO campaign
/// records at all keeps the old permissive behavior (no check), documented.
fn validate_campaign(log: &EventLog, pr_id: &str, campaign: &str) -> Result<(), PrError> {
    if log_has_campaign_vocabulary(log) && !campaign_exists(log, campaign) {
        Err(PrError::UnknownCampaign {
            pr_id: pr_id.to_string(),
            campaign: campaign.to_string(),
        })
    } else {
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// land — enter the union-testing landing queue.
// ─────────────────────────────────────────────────────────────────────────────

/// Inputs to `hugit pr land`.
#[derive(Debug, Clone)]
pub struct LandArgs {
    /// PR id to land (`--pr <n>`).
    pub pr_id: String,
    /// Unix ms to stamp the appended event with.
    pub recorded_at: u64,
}

/// `hugit pr land` — enter the union-testing landing queue.
///
/// Drives the REAL queue path: every already-queued PR plus this one is folded
/// into a [`hugit_queue::core::Batch`] of [`hugit_contracts::LandableEntry`]s
/// (ordered by `order_index`), so the reported `position` is the queue's own
/// ordering — never invented. Reports `{"queued":true,"position":N,"mode":"union"}`.
///
/// # Queue-authority honesty (audit F15 — disclosed seam, not an overclaim)
///
/// The reported `position` is the **porcelain's tail index** over the log's
/// `pr.queued` records: a PR enters at `order_index = count(pr.queued)` and the
/// `Batch` confirms that order (it enforces `order_index` uniqueness + sort).
/// It is an honest enqueue ordinal, NOT a landing ETA and NOT a disjointness
/// verdict.
///
/// **Affected-set arbitration is the engine queue's job, not the porcelain's.**
/// The real union-testing landing queue (B3) decides which queued PRs can land
/// together by intersecting their AffectedSets over **tree hashes** — but a tree
/// hash only exists once a PR's bundle is materialized into a real tree, which
/// is the P2 live-infra seam, not this hermetic file altitude. So every
/// [`hugit_contracts::LandableEntry`] here carries an **empty `tree_hash` and an
/// empty [`AffectedSet`]** (the honest placeholder): the porcelain cannot, and
/// does not claim to, compute disjointness or a batch landing decision. When
/// real tree hashes flow (P2), the same `Batch` path feeds them through and the
/// engine arbitrates the affected set; the porcelain's reported `position` stays
/// the enqueue ordinal it is today. The wedge property (union testing) lives in
/// the engine the position threads into — this verb is the honest enqueue seam.
///
/// Refusals (structured): the PR has no `pr.opened` ([`PrError::UnknownPr`]),
/// or it bundles zero intents ([`PrError::EmptyPr`]).
///
/// Idempotent: a PR already queued returns `"already_queued":true` (exit 0),
/// no second `pr.queued` is appended, and the original position is reported.
pub fn land(log: &mut EventLog, args: &LandArgs) -> Result<Value, PrError> {
    // WJ-UNIFY: the pr_id is an identifier ADDRESS — use it RAW so it joins the
    // raw id `pr open` stored, and let the ONE central structural boundary
    // (`scrub_to_canonical` on the payload below) redact a prefixed secret while
    // a 40/64-hex/slug address SURVIVES. Pre-scrubbing it through the full engine
    // (the old behaviour) collapsed a bare-hex id to `[REDACTED]`, so a DIFFERENT
    // 40-hex `--pr` resolved to the collapsed record and landed the WRONG PR.
    let args = &LandArgs {
        pr_id: crate::porcelain::structural_secret_scrub(&args.pr_id),
        recorded_at: args.recorded_at,
    };
    let opened = find_pr_opened(log, &args.pr_id).ok_or_else(|| PrError::UnknownPr {
        pr_id: args.pr_id.clone(),
    })?;

    // Terminal-landed idempotency (WI-PR defect 1): a PR the log already settles
    // as LANDED is terminal — `pr.landed` is the end of its lifecycle. Re-running
    // `pr land` on it (the canonical agent retry-on-land pattern) must be a NO-OP,
    // never a fresh enqueue. Without this guard, `find_pr_queued` below returns
    // None for a landed PR (a settled PR leaves the queue projection — see
    // `all_pr_queued`), so the idempotency check missed it and a SECOND `pr.queued`
    // was appended AFTER the terminal `pr.landed` — post-terminal log corruption,
    // reported as `already_queued:false` (a fresh-enqueue lie). Detect the
    // terminal state FIRST (before the empty/queued checks) and report it
    // idempotently with NO append. The reported `position` is the PR's original
    // recorded enqueue ordinal (the settled `pr.queued` still sits on the log; it
    // is only filtered OUT of the active queue projection, not deleted).
    if pr_is_landed(log, &args.pr_id) {
        return Ok(json!({
            "queued": true,
            "already_queued": true,
            "already_landed": true,
            "pr_id": args.pr_id,
            "position": landed_pr_order_index(log, &args.pr_id),
            "mode": LANDING_MODE,
        }));
    }

    // A PR is landable iff it bundles ANY content: intents OR captured-commit
    // members (W2: a commits-only PR — the LLM's raw git activity — must be
    // queueable; its commit_ids ARE the content, not feel-behind).
    if opened.intent_ids.is_empty() && opened.commit_ids.is_empty() {
        return Err(PrError::EmptyPr {
            pr_id: args.pr_id.clone(),
        });
    }

    // Idempotency: already queued → no-op, report the recorded position.
    if let Some(existing) = find_pr_queued(log, &args.pr_id) {
        return Ok(json!({
            "queued": true,
            "already_queued": true,
            "pr_id": args.pr_id,
            "position": existing.order_index,
            "mode": LANDING_MODE,
        }));
    }

    // The next free order_index: the count of PRs already queued (tail append).
    // We derive it THROUGH the real queue: fold the existing queued PRs plus
    // this new tail entry into a Batch, and read the position the queue assigns
    // (queue order is order_index order — Batch enforces uniqueness + sort).
    let queued = all_pr_queued(log);
    let order_index = queued.len() as u64;
    let item_id = format!("{}#{}", args.pr_id, order_index);

    let mut entries: Vec<(hugit_contracts::LandableEntry, AffectedSet)> = queued
        .iter()
        .map(|q| {
            (
                hugit_contracts::LandableEntry {
                    item_id: q.item_id.clone(),
                    intent_id: q.pr_id.clone(),
                    tree_hash: String::new(),
                    order_index: q.order_index,
                },
                // Affected-set is unknown at porcelain altitude (the queue
                // computes disjointness from B3's shape); an empty set is the
                // honest placeholder — it does not affect the position, which
                // is purely order_index ordering.
                AffectedSet::new(Vec::<String>::new()),
            )
        })
        .collect();
    entries.push((
        hugit_contracts::LandableEntry {
            item_id: item_id.clone(),
            intent_id: args.pr_id.clone(),
            tree_hash: String::new(),
            order_index,
        },
        AffectedSet::new(Vec::<String>::new()),
    ));

    // The queue is the authority on position: build the batch through the real
    // path and read where this PR landed in queue order.
    let batch = Batch::try_from_entries(format!("land-{}", args.pr_id), entries).map_err(|e| {
        PrError::QueueRefused {
            pr_id: args.pr_id.clone(),
            reason: e.to_string(),
        }
    })?;
    let position = batch
        .entries()
        .iter()
        .position(|e| e.item_id() == item_id)
        .ok_or_else(|| PrError::Internal {
            message: format!(
                "item_id '{}' not found in the Batch after being pushed — \
                 the queue engine reordered or dropped the entry; \
                 this is a hugit internal bug",
                item_id
            ),
        })? as u64;

    // Scrubbed-on-append (WG-SCRUB): `item_id`/`pr_id` derive from `--pr` (a user
    // string) and are hash-chained — redacted BEFORE the bytes reach the chain.
    let payload = crate::porcelain::scrub_to_canonical(json!({
        "item_id": item_id,
        "mode": LANDING_MODE,
        "order_index": order_index,
        "pr_id": args.pr_id,
    }));
    // D14 on the mutation primitive (WA2b, pr side): route the pr.queued append
    // through the guarded entry point under the opened PR's own author class —
    // the same guarded routing `pr open` uses. A non-author class would be denied
    // by the matrix and audited, never appended.
    let (class, endpoint) = author_authz(opened.author_kind);
    log.append_authorized(
        class,
        endpoint,
        PR_QUEUED_KIND,
        vec![],
        payload,
        args.recorded_at,
    )
    .map_err(|denied| PrError::QueueRefused {
        pr_id: args.pr_id.clone(),
        reason: denied.reason.code().to_string(),
    })?;

    Ok(json!({
        "queued": true,
        "already_queued": false,
        "pr_id": args.pr_id,
        "position": position,
        "mode": LANDING_MODE,
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// settle — terminal: a queued PR settles as LANDED (the `pr land --settle` step).
// ─────────────────────────────────────────────────────────────────────────────

/// Inputs to `hugit pr land --settle`.
#[derive(Debug, Clone, Default)]
pub struct SettleArgs {
    /// PR id to settle as landed (`--pr <n>`).
    pub pr_id: String,
    /// Unix ms to stamp the appended event with.
    pub recorded_at: u64,
    /// The real run metrics the orchestrator carries into the captured
    /// context envelope (WP-F2). Every field is optional — an omitted flag is
    /// honest-zero, never a fabricated figure (the dogfood / explicit-metrics
    /// path). Defaults to all-zero (a HUMAN-authored, honest-zero capture).
    pub envelope_metrics: EnvelopeMetricsArgs,
}

/// `hugit pr land --settle` — settle a queued PR as **landed** (appends
/// `pr.landed`, the terminal settlement event the campaign world projects to
/// the `landed` phase).
///
/// This is the explicit land-confirm settlement step (W-PRLANDED): it is the
/// producer the campaign progress projection needs to advance a PR past
/// in-flight to `landed`, so a campaign whose PRs all landed can `campaign
/// close` to `closed:true` driven by REAL `pr.landed` (not only by `abandon`).
///
/// # Settlement honesty (P2 disclosed seam — see [`PR_LANDED_KIND`])
///
/// `--settle` does NOT run the B3 union-test / disjointness verdict over real
/// tree hashes (that arbitration is the P2 live-infra seam — see [`land`]). It
/// is the operator/orchestrator's explicit confirmation that the PR (already in
/// the landing queue) has landed; automatic settlement off the queue verdict
/// awaits P2. It never fabricates a union-test result.
///
/// Pre-conditions:
/// - the PR must have a `pr.opened` ([`PrError::UnknownPr`] otherwise);
/// - the PR must already be queued ([`PrError::SettleNotQueued`] otherwise —
///   settle is the land-confirm over a queued PR; `pr land` enqueues first).
///
/// Idempotent: re-settling an already-landed PR appends no second event and
/// returns `"already_landed":true` (exit 0). The append is routed through the
/// SAME D14-guarded [`EventLog::append_authorized`] seam (under the opened PR's
/// author class — orchestrator/human) `pr land`/`pr open` use.
pub fn settle(log: &mut EventLog, args: &SettleArgs) -> Result<Value, PrError> {
    // WJ-UNIFY: the pr_id is an identifier ADDRESS — use it RAW (see `land`); the
    // central structural boundary on the payload below redacts a prefixed secret
    // while a bare-hex/slug address survives. No full-engine pre-scrub (collapse).
    let args = &SettleArgs {
        pr_id: crate::porcelain::structural_secret_scrub(&args.pr_id),
        recorded_at: args.recorded_at,
        envelope_metrics: args.envelope_metrics.clone(),
    };
    let opened = find_pr_opened(log, &args.pr_id).ok_or_else(|| PrError::UnknownPr {
        pr_id: args.pr_id.clone(),
    })?;

    // Idempotency: already landed → no-op, report the settled state. No second
    // envelope is captured (capture fires once, on the first settlement).
    if pr_is_landed(log, &args.pr_id) {
        return Ok(settle_json(&args.pr_id, &opened.campaign, true));
    }

    // Settle is the land-confirm over a QUEUED PR: it must have entered the
    // queue (via `pr land`) first.
    if find_pr_queued(log, &args.pr_id).is_none() {
        return Err(PrError::SettleNotQueued {
            pr_id: args.pr_id.clone(),
        });
    }

    // The `pr.landed` payload carries the campaign (recovered from the PR's
    // `pr.opened`) so the campaign world scopes the settlement to this campaign
    // — unlike `pr.abandoned`, the world matches `pr.landed` by campaign field.
    // Scrubbed-on-append (WG-SCRUB): `campaign` (from the opened PR) and `pr_id`
    // (`--pr`) are user strings, hash-chained — redacted before reaching the chain.
    let payload = crate::porcelain::scrub_to_canonical(json!({
        "campaign": opened.campaign,
        "pr_id": args.pr_id,
    }));
    let (class, endpoint) = author_authz(opened.author_kind);
    log.append_authorized(
        class,
        endpoint,
        PR_LANDED_KIND,
        vec![],
        payload,
        args.recorded_at,
    )
    .map_err(|denied| PrError::QueueRefused {
        pr_id: args.pr_id.clone(),
        reason: denied.reason.code().to_string(),
    })?;

    // WP-F2: capture the context envelope(s) on land. AFTER the terminal
    // `pr.landed` is on the log — the settlement is the trigger, the envelope
    // is the legibility record that rides alongside it. Appends one
    // `intent.envelope` per bundled intent + one top-level `pr.envelope`
    // (the F3 rollup / serve cost surfaces read these). Honest-zero when no
    // `--tokens`/`--cost-usd-micros`/… flags were supplied; real figures when
    // the orchestrator passes them (the dogfood path). Best-effort wrt the
    // already-completed land: a capture failure leaves the cost block honestly
    // absent, never unwinds the land.
    let captured = capture::capture_on_land(log, &opened, &args.envelope_metrics, args.recorded_at);

    Ok(settle_json_captured(
        &args.pr_id,
        &opened.campaign,
        false,
        captured,
    ))
}

/// The stable `settle` success shape — the SAME key-set on first-run and the
/// idempotent re-run (`already_landed` carries the difference). The idempotent
/// re-settle captures no new envelope, so `envelopes_captured` is 0.
fn settle_json(pr_id: &str, campaign: &str, already: bool) -> Value {
    settle_json_captured(pr_id, campaign, already, 0)
}

/// Like [`settle_json`], carrying the WP-F2 count of context-envelope records
/// captured on this settlement (the `intent.envelope` + `pr.envelope` records
/// appended alongside the terminal `pr.landed`). Zero on the idempotent
/// re-settle (capture fires once) and on a best-effort capture that appended
/// nothing.
fn settle_json_captured(pr_id: &str, campaign: &str, already: bool, captured: u64) -> Value {
    json!({
        "pr_id": pr_id,
        "campaign": campaign,
        "landed": true,
        "already_landed": already,
        "state": "landed",
        "envelopes_captured": captured,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// abandon — terminal: a PROPOSED/queued PR leaves the queue projection.
// ─────────────────────────────────────────────────────────────────────────────

/// Inputs to `hugit pr abandon`.
#[derive(Debug, Clone)]
pub struct AbandonArgs {
    /// PR id to abandon (`--pr <id>`).
    pub pr_id: String,
    /// The reason for abandoning (`--reason <text>`) — recorded on the event.
    pub reason: String,
    /// Unix ms to stamp the appended event with.
    pub recorded_at: u64,
}

/// `hugit pr abandon` — terminally abandon a PR (appends `pr.abandoned`).
///
/// An abandoned PR leaves the queue projection (the campaign world treats
/// `pr.abandoned` as a settled, non-in-flight PR; this verb is the porcelain
/// that writes it). The append is routed through the D14-guarded
/// [`EventLog::append_authorized`] under the opened PR's author class (WA2b).
///
/// Refusals (structured): no `pr.opened` for the id ([`PrError::AbandonUnknownPr`]),
/// or the PR has already landed ([`PrError::AbandonLanded`] — a landed PR is
/// terminal, not abandonable).
///
/// Idempotent: re-abandoning an already-abandoned PR appends no second event and
/// returns `"already_abandoned":true` (exit 0).
pub fn abandon(log: &mut EventLog, args: &AbandonArgs) -> Result<Value, PrError> {
    // WJ-UNIFY: the pr_id is an identifier ADDRESS — use it RAW (stable join, see
    // `land`); the central structural boundary on the payload below redacts a
    // prefixed secret while a bare-hex/slug address survives (no collapse). The
    // `--reason` is genuine FREE TEXT (the adversary's headline leak vector) — it
    // KEEPS the full free-text engine scrub at entry so no secret persists.
    let args = &AbandonArgs {
        pr_id: crate::porcelain::structural_secret_scrub(&args.pr_id),
        reason: crate::redaction::scrub(&args.reason),
        recorded_at: args.recorded_at,
    };
    let opened = find_pr_opened(log, &args.pr_id).ok_or_else(|| PrError::AbandonUnknownPr {
        pr_id: args.pr_id.clone(),
    })?;

    // A landed PR is terminal — refuse to abandon it.
    if pr_is_landed(log, &args.pr_id) {
        return Err(PrError::AbandonLanded {
            pr_id: args.pr_id.clone(),
        });
    }

    // Idempotency: already abandoned → no-op, report the recorded reason.
    if let Some(existing_reason) = find_pr_abandoned_reason(log, &args.pr_id) {
        return Ok(abandon_json(&args.pr_id, &existing_reason, true));
    }

    // Scrubbed-on-append (WG-SCRUB): `--reason` is THE adversary's exact leak
    // vector (a `ghp_…` in `pr abandon --reason`) — and `pr_id` (`--pr`). Both
    // are user strings, hash-chained; redacted BEFORE reaching the chain.
    let payload = crate::porcelain::scrub_to_canonical(json!({
        "pr_id": args.pr_id,
        "reason": args.reason,
    }));
    let (class, endpoint) = author_authz(opened.author_kind);
    log.append_authorized(
        class,
        endpoint,
        PR_ABANDONED_KIND,
        vec![],
        payload,
        args.recorded_at,
    )
    .map_err(|denied| PrError::QueueRefused {
        pr_id: args.pr_id.clone(),
        reason: denied.reason.code().to_string(),
    })?;

    Ok(abandon_json(&args.pr_id, &args.reason, false))
}

/// The stable `abandon` success shape — the SAME key-set on first-run and the
/// idempotent re-run (`already_abandoned` carries the difference).
fn abandon_json(pr_id: &str, reason: &str, already: bool) -> Value {
    json!({
        "pr_id": pr_id,
        "abandoned": true,
        "already_abandoned": already,
        "reason": reason,
    })
}

/// The recorded enqueue `order_index` of a (now-landed) PR's `pr.queued` event.
///
/// A settled `pr.landed` PR leaves the ACTIVE queue projection
/// ([`all_pr_queued`] filters it out), so [`find_pr_queued`] returns `None` for
/// it — but the original `pr.queued` record still sits on the append-only log.
/// The terminal-landed idempotency path reads it directly so it can echo the
/// PR's original queue position without resurrecting it into the active queue.
/// `null` only if the PR somehow has no `pr.queued` at all (settle requires a
/// queued PR, so a landed PR normally always has one).
fn landed_pr_order_index(log: &EventLog, pr_id: &str) -> Value {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_QUEUED_KIND)
        .filter_map(parse_queued)
        .rfind(|q| q.pr_id == pr_id)
        .map(|q| json!(q.order_index))
        .unwrap_or(Value::Null)
}

/// Whether the log records this PR as LANDED (a `pr.landed` record names it).
fn pr_is_landed(log: &EventLog, pr_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_LANDED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("pr_id").and_then(Value::as_str) == Some(pr_id))
}

/// The reason recorded on this PR's `pr.abandoned`, if it has been abandoned.
fn find_pr_abandoned_reason(log: &EventLog, pr_id: &str) -> Option<String> {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_ABANDONED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter(|v| v.get("pr_id").and_then(Value::as_str) == Some(pr_id))
        .filter_map(|v| v.get("reason").and_then(Value::as_str).map(str::to_string))
        .next_back()
}

// ─────────────────────────────────────────────────────────────────────────────
// show — the PR's intents, queue state, and the F3 cost rollup when present.
// ─────────────────────────────────────────────────────────────────────────────

/// Inputs to `hugit pr show`.
#[derive(Debug, Clone)]
pub struct ShowArgs {
    /// PR id to show (`--pr <n>`).
    pub pr_id: String,
}

/// `hugit pr show` — the PR's intents, queue state, and (when captured) the F3
/// `pr_record` rollup cost block.
///
/// The cost block is computed from the PR-altitude envelope + intent-altitude
/// envelopes carried (additively) on the log under the `pr.envelope` /
/// `intent.envelope` kinds — see [`envelopes_for_pr`]. When no PR-altitude
/// envelope is on the log, the `cost` field is honestly `null` (the rollup was
/// never captured), never faked.
pub fn show(log: &EventLog, args: &ShowArgs) -> Result<Value, PrError> {
    let opened = find_pr_opened(log, &args.pr_id).ok_or_else(|| PrError::NotFound {
        pr_id: args.pr_id.clone(),
    })?;

    // Queue state, if landed into the queue.
    let queue_state = match find_pr_queued(log, &args.pr_id) {
        Some(q) => json!({
            "queued": true,
            "position": q.order_index,
            "mode": LANDING_MODE,
        }),
        None => json!({ "queued": false }),
    };

    // The F3 pr_record rollup — present only when a PR-altitude envelope is
    // captured on the log; null otherwise (honest gap, never faked).
    let cost = pr_record_for(log, &opened);

    Ok(json!({
        "pr_id": opened.pr_id,
        "campaign": opened.campaign,
        "author_kind": opened.author_kind.as_str(),
        "intent_ids": opened.intent_ids,
        "intent_count": opened.intent_ids.len(),
        "commit_ids": opened.commit_ids,
        "commit_count": opened.commit_ids.len(),
        "queue": queue_state,
        "cost": cost,
    }))
}

// ─────────────────────────────────────────────────────────────────────────────
// list — every PR on the log, full info per row, stable order.
// ─────────────────────────────────────────────────────────────────────────────

/// Inputs to `hugit pr list`.
#[derive(Debug, Clone, Default)]
pub struct ListArgs {
    /// Optional campaign filter (`--campaign <key>`) — only PRs of this campaign.
    pub campaign: Option<String>,
    /// Optional state filter (`--state <s>`): `proposed | queued | abandoned |
    /// landed`. An unrecognized state matches nothing (the row set is empty).
    pub state: Option<String>,
}

/// The terminal/lifecycle state of a PR projected off the log.
///
/// Precedence (most-settled wins): `landed` ≻ `abandoned` ≻ `queued` ≻
/// `proposed`. A PR is `proposed` the moment it is opened; entering the queue
/// makes it `queued`; a `pr.abandoned` makes it `abandoned`; a `pr.landed`
/// (the disclosed landing seam) makes it `landed`.
fn pr_state(log: &EventLog, opened: &OpenedPr) -> &'static str {
    if pr_is_landed(log, &opened.pr_id) {
        "landed"
    } else if find_pr_abandoned_reason(log, &opened.pr_id).is_some() {
        "abandoned"
    } else if find_pr_queued(log, &opened.pr_id).is_some() {
        "queued"
    } else {
        "proposed"
    }
}

/// `hugit pr list` — every PR on the log, one full-info row each, stable order.
///
/// Rows are ordered by the `pr.opened` log seq (chain order — stable and
/// deterministic). Each row carries the SAME key-set: `pr_id`, `campaign`,
/// `author_kind`, `intent_ids`, `intent_count`, `state`, and `position` (the
/// queue position, or `null` when not queued). Optional `--campaign` / `--state`
/// filters narrow the set without changing the row shape.
pub fn list(log: &EventLog, args: &ListArgs) -> Value {
    // Project the latest pr.opened per id, in first-seen (open seq) order.
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut ordered: Vec<OpenedPr> = Vec::new();
    for r in log.records().iter().filter(|r| r.kind == PR_OPENED_KIND) {
        if let Some(o) = parse_opened(r)
            && seen.insert(o.pr_id.clone())
        {
            // Re-resolve to the LATEST opened for this id (robust to re-state).
            if let Some(latest) = find_pr_opened(log, &o.pr_id) {
                ordered.push(latest);
            }
        }
    }

    let rows: Vec<Value> = ordered
        .iter()
        .filter(|o| args.campaign.as_deref().is_none_or(|c| o.campaign == c))
        .filter_map(|o| {
            let state = pr_state(log, o);
            if let Some(want) = args.state.as_deref()
                && want != state
            {
                return None;
            }
            let position = find_pr_queued(log, &o.pr_id)
                .map(|q| json!(q.order_index))
                .unwrap_or(Value::Null);
            Some(json!({
                "pr_id": o.pr_id,
                "campaign": o.campaign,
                "author_kind": o.author_kind.as_str(),
                "intent_ids": o.intent_ids,
                "intent_count": o.intent_ids.len(),
                "commit_ids": o.commit_ids,
                "commit_count": o.commit_ids.len(),
                "state": state,
                "position": position,
            }))
        })
        .collect();

    let shown = rows.len();
    json!({
        "prs": rows,
        "count": ordered.len(),
        "shown": shown,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Projections over the event log — the read models the verbs share.
// ─────────────────────────────────────────────────────────────────────────────

/// A projected `pr.opened` event (the PROPOSED PR record).
#[derive(Debug, Clone)]
pub struct OpenedPr {
    /// The PR id.
    pub pr_id: String,
    /// Campaign key the PR belongs to.
    pub campaign: String,
    /// The author kind recorded at open time (D14: orchestrator | human).
    pub author_kind: AuthorKind,
    /// Bundled intent ids.
    pub intent_ids: Vec<String>,
    /// Captured raw-commit members (`commit_ids` — external, never intents).
    /// Empty for a PR opened before W2 (the field is additive-optional on read).
    pub commit_ids: Vec<String>,
    /// Log seq of the `pr.opened` event.
    pub seq: u64,
}

/// A projected `pr.queued` event (the PR's landing-queue position).
#[derive(Debug, Clone)]
pub struct QueuedPr {
    /// The PR id.
    pub pr_id: String,
    /// The queue item id.
    pub item_id: String,
    /// The queue position (`order_index`) — the porcelain's **tail enqueue
    /// ordinal** over the log's `pr.queued` records (the `position` reported by
    /// `land`/`show`/`list`). It is an honest enqueue order, NOT a landing ETA
    /// and NOT a disjointness/batch verdict: affected-set arbitration over real
    /// tree hashes is the engine queue's job (the P2 seam — see [`land`]).
    pub order_index: u64,
}

/// Project the latest `pr.opened` for a PR id out of the log, if any.
///
/// Folds the log in chain order; the last `pr.opened` for the id wins (a
/// re-open with the same campaign is an idempotent no-op so there is normally
/// at most one, but folding the latest is robust under any future re-state).
pub fn find_pr_opened(log: &EventLog, pr_id: &str) -> Option<OpenedPr> {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_OPENED_KIND)
        .filter_map(parse_opened)
        .rfind(|o| o.pr_id == pr_id)
}

/// Project the `pr.queued` for a PR id out of the log, if any.
///
/// Returns `None` when the PR has no `pr.queued` record OR when the PR has a
/// terminal `pr.landed` event — a landed PR leaves the queue projection.
pub fn find_pr_queued(log: &EventLog, pr_id: &str) -> Option<QueuedPr> {
    all_pr_queued(log).into_iter().find(|q| q.pr_id == pr_id)
}

/// Project every ACTIVE `pr.queued` on the log, in log (queue) order.
///
/// A PR whose `pr_id` appears in a terminal `pr.landed` event is excluded — a
/// settled PR leaves the queue projection. This is the single filter point so
/// both `pr show` and `queue show` agree by construction.
pub fn all_pr_queued(log: &EventLog) -> Vec<QueuedPr> {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_QUEUED_KIND)
        .filter_map(parse_queued)
        .filter(|q| !pr_is_landed(log, &q.pr_id))
        .collect()
}

fn parse_opened(r: &EventRecord) -> Option<OpenedPr> {
    let v: Value = serde_json::from_str(&r.payload).ok()?;
    let pr_id = v.get("pr_id")?.as_str()?.to_string();
    let campaign = v.get("campaign")?.as_str()?.to_string();
    let author_kind = AuthorKind::parse(v.get("author_kind")?.as_str()?)?;
    let intent_ids = v
        .get("intent_ids")?
        .as_array()?
        .iter()
        .filter_map(|x| x.as_str().map(str::to_string))
        .collect();
    // W2 additive read: a pre-W2 `pr.opened` payload has no `commit_ids` key —
    // default to empty (never fail a legacy PR over the new additive field).
    let commit_ids = v
        .get("commit_ids")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Some(OpenedPr {
        pr_id,
        campaign,
        author_kind,
        intent_ids,
        commit_ids,
        seq: r.seq,
    })
}

fn parse_queued(r: &EventRecord) -> Option<QueuedPr> {
    let v: Value = serde_json::from_str(&r.payload).ok()?;
    Some(QueuedPr {
        pr_id: v.get("pr_id")?.as_str()?.to_string(),
        item_id: v.get("item_id")?.as_str()?.to_string(),
        order_index: v.get("order_index")?.as_u64()?,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// The F3 cost rollup seam (show's cost block).
// ─────────────────────────────────────────────────────────────────────────────

/// Event kind carrying a captured PR-altitude [`ContextEnvelope`] for the
/// rollup. Additive; payload is the envelope as canonical JSON.
pub const PR_ENVELOPE_KIND: &str = "pr.envelope";

/// Event kind carrying a captured intent-altitude [`ContextEnvelope`].
pub const INTENT_ENVELOPE_KIND: &str = "intent.envelope";

/// The PR-altitude + intent-altitude envelopes captured on the log for a PR.
struct PrEnvelopes {
    pr: ContextEnvelope,
    intents: Vec<ContextEnvelope>,
}

/// Read the captured envelopes for a PR off the log, if a PR-altitude envelope
/// is present. Intent-altitude envelopes whose `intent_id` is in the PR's
/// bundle are attributed to it.
fn envelopes_for_pr(log: &EventLog, opened: &OpenedPr) -> Option<PrEnvelopes> {
    let bundle: BTreeSet<&str> = opened.intent_ids.iter().map(String::as_str).collect();

    let pr = log
        .records()
        .iter()
        .filter(|r| r.kind == PR_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .rfind(|e| e.altitude == Altitude::Pr && e.intent_id == opened.pr_id)?;

    let intents = log
        .records()
        .iter()
        .filter(|r| r.kind == INTENT_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .filter(|e| e.altitude == Altitude::Intent && bundle.contains(e.intent_id.as_str()))
        .collect();

    Some(PrEnvelopes { pr, intents })
}

/// Compute the F3 [`PrRecord`] for a PR when its envelope/metrics data is
/// present in the log; `None` (→ JSON `null`) when no PR-altitude envelope was
/// captured (honest gap, never faked).
///
/// `landed_intent_ids` is the PR's own bundle (the v1 landing unit is the
/// intent id; the bundle's intents are what the PR proposes to land). CI cost
/// and queue timing have no porcelain seam yet, so they are passed as zero /
/// default — the rollup tolerates that and computes the measured figures
/// (work/orchestration/waste/first-pass-yield) from the envelopes.
fn pr_record_for(log: &EventLog, opened: &OpenedPr) -> Option<Value> {
    let envs = envelopes_for_pr(log, opened)?;
    // The PR's bundled intents are its landing unit.
    let landed: Vec<String> = opened.intent_ids.clone();
    let verdicts: Vec<VerdictObject> = Vec::new();
    let record: PrRecord = pr_record(
        &envs.pr,
        // No CAS binding at porcelain altitude (P2 disclosed seam): the
        // envelope_ref is threaded through as empty rather than faked.
        "",
        &envs.intents,
        &landed,
        &verdicts,
        // No CI / pricing seam at porcelain altitude — zero, never faked.
        CiCost {
            cache_hit: 0,
            exec: 0,
            cost_usd_micros: 0,
            saved_usd_micros: 0,
        },
        PrQueueInput::default(),
    )
    .ok()?; // a subagent-authored PR envelope is rejected → no cost block.
    serde_json::to_value(record).ok()
}

// ─────────────────────────────────────────────────────────────────────────────
// JSON helpers.
// ─────────────────────────────────────────────────────────────────────────────

fn open_json(opened: &OpenedPr, already_exists: bool) -> Value {
    json!({
        "pr_id": opened.pr_id,
        "campaign": opened.campaign,
        "author_kind": opened.author_kind.as_str(),
        "intent_ids": opened.intent_ids,
        "intent_count": opened.intent_ids.len(),
        "commit_ids": opened.commit_ids,
        "commit_count": opened.commit_ids.len(),
        "state": "proposed",
        "already_exists": already_exists,
    })
}

/// Map an [`AuthorKind`] to the D14 `(class, endpoint)` the `pr.opened` append
/// is gated on by [`EventLog::append_authorized`].
///
/// `pr.opened` is authored by an orchestrator or a human, never a worker/model
/// (the [`AuthorKind`] type already encodes that — `subagent` is not a variant).
/// We gate each author class on a matrix verb it legitimately owns
/// (orchestrator→`land`, human→`undo`), so the only way the guard denies here is
/// if a non-author class were ever asserted — a worker/model — which the matrix
/// rejects and audits. This is defense-in-depth on the mutation primitive behind
/// the door-level [`AuthorKind::parse`] check.
pub(super) fn author_authz(
    kind: AuthorKind,
) -> (hugit_refstore::PrincipalClass, hugit_refstore::Endpoint) {
    use hugit_refstore::{Endpoint, PrincipalClass};
    match kind {
        AuthorKind::Orchestrator => (PrincipalClass::Orchestrator, Endpoint::Land),
        AuthorKind::Human => (PrincipalClass::Human, Endpoint::Undo),
    }
}

/// The principal chain to stamp a `pr.opened` event with: the human principal
/// for a human author, the orchestrator run id for an orchestrator author.
fn author_principal_chain(
    kind: AuthorKind,
    run_id: &Option<String>,
    principal: &Option<String>,
) -> Vec<String> {
    match kind {
        AuthorKind::Human => principal.clone().into_iter().collect(),
        AuthorKind::Orchestrator => run_id.clone().into_iter().collect(),
    }
}

/// Return a copy of `args` with the IDENTIFIER-ADDRESS fields routed through the
/// STRUCTURAL scrub (WJ-UNIFY) and the remaining user strings left for the ONE
/// central boundary to scrub on append.
///
/// **The WJ-UNIFY fix.** `pr_id`/`campaign`/`run_id` are identifier ADDRESSES
/// (the keys the rest of the flow looks up by). Pre-scrubbing them through the
/// FULL free-text engine — as this did before — collapsed a bare 40/64-hex
/// `pr_id`/`campaign` to a single `[REDACTED]`: two distinct content-address PRs
/// became one record, so `pr land --pr <B>` resolved to the collapsed `A`'s
/// record and landed the WRONG PR. The fix routes every identifier through the
/// ONE structural boundary instead:
///
/// - `pr_id`/`campaign` get the STRUCTURAL scrub here — the SAME scrub the
///   central boundary (`crate::porcelain::scrub_to_canonical`, keyed by
///   `crate::porcelain::is_identifier_key`) applies to these keys on append. A
///   bare 40/64-hex/slug address is a NO-OP (survives verbatim, no collapse → no
///   wrong-PR landing); a prefixed secret REDACTS. Applying it here too keeps the
///   LOOKUP key (`find_pr_opened`) symmetric with the STORED payload value, so a
///   re-open/land/abandon joins the right record and the post-append projection
///   finds what it just wrote (a prefixed-secret id round-trips as `[REDACTED]`
///   on BOTH sides; two distinct addresses never collapse to one).
/// - `run_id`/`principal` ALSO feed the event's `principal_chain`, which
///   `append_authorized` hashes VERBATIM (it is not a JSON payload, so the
///   central `scrub_payload` never sees it). Those are routed through the SAME
///   [`structural_secret_scrub`](crate::porcelain::structural_secret_scrub) here
///   so a prefixed secret in `--run-id`/`--principal` still REDACTS in the chain
///   while a bare-hex/slug address SURVIVES — one boundary, no per-verb bypass.
/// - `intent_ids` keep the existing free-text scrub (they are validated against
///   the log and are not the WJ-UNIFY collapse vector).
///
/// Called ONCE at the top of [`open`] so lookups + payload + echo agree.
fn scrub_open_args(args: &OpenArgs) -> OpenArgs {
    OpenArgs {
        // Identifier addresses: STRUCTURALLY scrubbed (NOT the full free-text
        // engine) — the SAME scrub the central boundary applies to these keys on
        // append. A bare 40/64-hex/slug address is a no-op (survives, no
        // collapse); a prefixed secret redacts to the sentinel. Doing it here too
        // keeps the LOOKUP key (`find_pr_opened`) symmetric with the STORED
        // payload value, so a re-open/land/abandon joins the right record and the
        // post-append projection finds what it just wrote.
        pr_id: crate::porcelain::structural_secret_scrub(&args.pr_id),
        campaign: crate::porcelain::structural_secret_scrub(&args.campaign),
        author_kind: args.author_kind,
        // Principal-chain identifiers: structurally scrubbed here (the chain is
        // hashed verbatim by append_authorized, outside the payload boundary).
        run_id: args
            .run_id
            .as_deref()
            .map(crate::porcelain::structural_secret_scrub),
        principal: args
            .principal
            .as_deref()
            .map(crate::porcelain::structural_secret_scrub),
        intent_ids: crate::redaction::scrub_all(&args.intent_ids),
        // Captured commit oids are identifier ADDRESSES (40-hex) — structural
        // scrub (bare hex survives verbatim; a prefixed secret redacts), so the
        // LOOKUP key (`commit_ref_target_on_log`) stays symmetric with the raw
        // `target` the capture path stored.
        commit_ids: args
            .commit_ids
            .iter()
            .map(|oid| crate::porcelain::structural_secret_scrub(oid))
            .collect(),
        recorded_at: args.recorded_at,
    }
}

/// Build the canonical-JSON `pr.opened` payload (sorted keys, no insignificant
/// whitespace — the hash chain covers these bytes verbatim), SCRUBBED-ON-APPEND
/// (WG-SCRUB): every user-supplied string value (`campaign`, `intent_ids`,
/// `commit_ids`, `pr_id`, `principal`, `run_id`) is routed through the
/// redaction engine BEFORE the bytes reach the chain, so a secret in any flag
/// never leaks to the forever log. None of these are digest fields, so all
/// scrub.
fn canonical_open_payload(args: &OpenArgs) -> String {
    // Optional fields are emitted as null when absent so the payload is
    // self-describing; the scrub + canonicalisation sorts keys deterministically.
    let intent_ids: Vec<Value> = args
        .intent_ids
        .iter()
        .map(|s| Value::String(s.clone()))
        .collect();
    // `commit_ids` are ADDRESSES (the captured commits' oids — 40-hex SHA-1
    // content refs), so they go through the identifier-level scrub that PRESERVES
    // a content-address shape (like pr_id/campaign), NOT the free-text engine
    // that would collapse a bare 40-hex run to [REDACTED].
    let commit_ids: Vec<Value> = args
        .commit_ids
        .iter()
        .map(|s| Value::String(crate::porcelain::structural_secret_scrub(s)))
        .collect();
    let payload = json!({
        "author_kind": args.author_kind.as_str(),
        "campaign": args.campaign,
        "commit_ids": commit_ids,
        "intent_ids": intent_ids,
        "pr_id": args.pr_id,
        "principal": args.principal.clone(),
        "run_id": args.run_id.clone(),
    });
    crate::porcelain::scrub_to_canonical(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_args(pr: &str, campaign: &str, intents: &[&str]) -> OpenArgs {
        OpenArgs {
            pr_id: pr.to_string(),
            campaign: campaign.to_string(),
            author_kind: AuthorKind::Orchestrator,
            run_id: Some("run-1".to_string()),
            principal: None,
            intent_ids: intents.iter().map(|s| s.to_string()).collect(),
            commit_ids: Vec::new(),
            recorded_at: 1000,
        }
    }

    /// Seed a captured `ref.update` (`{"ref","target"}`) + an `intent.landed`
    /// (`{"intent_id"}`) raw record onto a fresh log — the W2 precondition for
    /// `--commit`/`--intent` validation (both raw vocabularies in use).
    fn seeded_capture_log() -> EventLog {
        let mut log = EventLog::new();
        log.append_authorized(
            hugit_refstore::PrincipalClass::Orchestrator,
            hugit_refstore::Endpoint::Push,
            INTENT_LANDED_KIND.to_string(),
            vec!["orchestrator:test".to_string()],
            r#"{"intent_id":"i1"}"#.to_string(),
            1,
        )
        .expect("seed intent.landed");
        log.append_authorized(
            hugit_refstore::PrincipalClass::Orchestrator,
            hugit_refstore::Endpoint::Push,
            REF_UPDATE_KIND.to_string(),
            vec!["orchestrator:test".to_string()],
            r#"{"ref":"refs/heads/main","target":"aaaa1111"}"#.to_string(),
            2,
        )
        .expect("seed ref.update");
        log
    }

    #[test]
    fn author_kind_rejects_subagent() {
        assert_eq!(AuthorKind::parse("subagent"), None);
        assert_eq!(
            AuthorKind::parse("orchestrator"),
            Some(AuthorKind::Orchestrator)
        );
        assert_eq!(AuthorKind::parse("human"), Some(AuthorKind::Human));
    }

    #[test]
    fn open_payload_is_canonical_json() {
        let args = open_args("7", "camp-a", &["i1", "i2"]);
        let payload = canonical_open_payload(&args);
        // Round-trips and re-canonicalises to itself (sorted keys).
        let recanon = hugit_refstore::canonical_json(&payload).unwrap();
        assert_eq!(payload, recanon, "payload must already be canonical JSON");
    }

    #[test]
    fn open_then_show_roundtrips_over_real_log() {
        let mut log = EventLog::new();
        let out = open(&mut log, &open_args("7", "camp-a", &["i1", "i2"])).unwrap();
        assert_eq!(out["already_exists"], json!(false));
        assert_eq!(out["state"], json!("proposed"));

        let shown = show(
            &log,
            &ShowArgs {
                pr_id: "7".to_string(),
            },
        )
        .unwrap();
        assert_eq!(shown["intent_count"], json!(2));
        assert_eq!(shown["queue"]["queued"], json!(false));
        // No envelope captured → honest null cost block.
        assert_eq!(shown["cost"], Value::Null);
    }

    #[test]
    fn open_is_idempotent_same_campaign() {
        let mut log = EventLog::new();
        open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();
        let n_before = log.len();
        let again = open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();
        assert_eq!(again["already_exists"], json!(true));
        assert_eq!(log.len(), n_before, "re-open appends no second event");
    }

    /// WG-PR fix 1: the `Internal` error variant renders as structured JSON with
    /// `kind:"internal"` (the one-error-law reserved internal-fault code) rather
    /// than panicking. This ensures both `.expect()` → `?` sites produce
    /// machine-parseable output on a malformed-payload fault.
    #[test]
    fn internal_error_renders_structured_json_not_panic() {
        let e = PrError::Internal {
            message: "test internal fault".to_string(),
        };
        let v: Value = serde_json::from_str(&e.to_porcelain().to_json())
            .expect("PrError::Internal renders valid JSON");
        assert_eq!(
            v["error"]["kind"], "internal",
            "kind must be 'internal': {v}"
        );
        assert!(v["error"]["fix"].is_string(), "fix must be present: {v}");
        assert!(
            v["error"]["message"].is_string(),
            "message must be present: {v}"
        );
        // It is an internal fault (exit 1), not a domain error (exit 2).
        use crate::porcelain::INTERNAL_FAULT_EXIT;
        assert_eq!(
            e.to_porcelain().exit_code(),
            std::process::ExitCode::from(INTERNAL_FAULT_EXIT)
        );
    }

    /// WG-PR fix 2: `all_pr_queued` excludes PRs that have a terminal
    /// `pr.landed` event — a settled PR leaves the queue projection.
    #[test]
    fn all_pr_queued_excludes_landed_prs() {
        let mut log = EventLog::new();
        // open + land PR "7".
        open(&mut log, &open_args("7", "camp-a", &["i1"])).unwrap();
        land(
            &mut log,
            &LandArgs {
                pr_id: "7".to_string(),
                recorded_at: 2000,
            },
        )
        .unwrap();
        // Before settlement: queued.
        assert_eq!(all_pr_queued(&log).len(), 1, "one queued PR before settle");
        // Settle → pr.landed.
        settle(
            &mut log,
            &SettleArgs {
                pr_id: "7".to_string(),
                recorded_at: 3000,
                ..Default::default()
            },
        )
        .unwrap();
        // After settlement: must NOT appear in all_pr_queued.
        assert_eq!(
            all_pr_queued(&log).len(),
            0,
            "landed PR must be excluded from all_pr_queued"
        );
        // find_pr_queued also returns None for the landed PR.
        assert!(
            find_pr_queued(&log, "7").is_none(),
            "find_pr_queued returns None for a landed PR"
        );
        // pr show reflects queue.queued:false.
        let shown = show(
            &log,
            &ShowArgs {
                pr_id: "7".to_string(),
            },
        )
        .unwrap();
        assert_eq!(
            shown["queue"]["queued"],
            json!(false),
            "pr show queue.queued must be false after landing: {shown}"
        );
    }

    // ── W2 — captured raw commits as external PR members (--commit / --commit-ref).

    /// W2 happy path: a captured commit oid is accepted as an EXTERNAL member —
    /// recorded under `commit_ids`, never merged into `intent_ids`.
    #[test]
    fn open_records_captured_commit_as_external_member() {
        let mut log = seeded_capture_log();
        let out = open(
            &mut log,
            &OpenArgs {
                commit_ids: vec!["aaaa1111".to_string()],
                ..open_args("7", "camp-a", &["i1"])
            },
        )
        .expect("captured commit accepted");
        assert_eq!(out["intent_ids"], json!(["i1"]), "intent stays an intent");
        assert_eq!(
            out["commit_ids"],
            json!(["aaaa1111"]),
            "captured commit rides as an external member"
        );
        // The payload on the log is the source of truth.
        let opened = find_pr_opened(&log, "7").expect("opened");
        assert_eq!(opened.intent_ids, vec!["i1"]);
        assert_eq!(opened.commit_ids, vec!["aaaa1111"]);
        // Intent vocabulary unchanged: exactly the one seeded record, no forging.
        let landed: Vec<_> = log
            .records()
            .iter()
            .filter(|r| r.kind == INTENT_LANDED_KIND)
            .collect();
        assert_eq!(
            landed.len(),
            1,
            "no intent.landed forged for the raw commit"
        );
    }

    /// W2 fail-closed: an oid NOT captured on the log is `commit_not_found` —
    /// the honest distinction from `missing_intents`.
    #[test]
    fn open_uncaptured_commit_is_commit_not_found() {
        let mut log = seeded_capture_log();
        let e = open(
            &mut log,
            &OpenArgs {
                commit_ids: vec!["bbbb2222".to_string()],
                ..open_args("7", "camp-a", &["i1"])
            },
        )
        .expect_err("uncaptured commit must refuse");
        assert!(
            matches!(e, PrError::MissingCommits { ref missing, .. } if missing == &["bbbb2222"])
        );
        assert_eq!(e.code(), "commit_not_found");
        assert_eq!(
            e.to_porcelain().exit_code(),
            std::process::ExitCode::from(crate::porcelain::PORCELAIN_ERROR_EXIT)
        );
        // Nothing appended — a refusal, never a partial PR.
        assert_eq!(log.len(), seeded_capture_log().len());
    }

    /// W2 no-fake-intent invariant: a PR bundling ONLY commits carries an empty
    /// `intent_ids` and appends no `intent.landed` — a raw commit is never
    /// forged into an intent, even when no intent accompanies it.
    #[test]
    fn open_commits_only_never_forges_intent() {
        let mut log = seeded_capture_log();
        let out = open(
            &mut log,
            &OpenArgs {
                intent_ids: Vec::new(),
                commit_ids: vec!["aaaa1111".to_string()],
                ..open_args("7", "camp-a", &[])
            },
        )
        .expect("commits-only PR opens");
        assert_eq!(out["intent_ids"], json!([]), "no intent forged");
        assert_eq!(out["commit_ids"], json!(["aaaa1111"]));
        assert_eq!(
            log.records()
                .iter()
                .filter(|r| r.kind == INTENT_LANDED_KIND)
                .count(),
            1,
            "intent.landed set unchanged — no forged intent"
        );
    }

    /// W2 `--commit-ref` resolution reads the LATEST captured target of a ref.
    #[test]
    fn commit_ref_resolves_to_latest_captured_target() {
        let mut log = EventLog::new();
        for (seq, target) in ["aaaa1111", "cccc3333"].iter().enumerate() {
            log.append_authorized(
                hugit_refstore::PrincipalClass::Orchestrator,
                hugit_refstore::Endpoint::Push,
                REF_UPDATE_KIND.to_string(),
                vec!["orchestrator:test".to_string()],
                serde_json::json!({"ref": "refs/heads/main", "target": target}).to_string(),
                seq as u64 + 1,
            )
            .expect("seed ref.update");
        }
        assert_eq!(
            commit_ref_target_for_ref(&log, "refs/heads/main").as_deref(),
            Some("cccc3333"),
            "latest capture wins"
        );
        assert_eq!(
            commit_ref_target_for_ref(&log, "refs/heads/never"),
            None,
            "uncaptured ref resolves to None"
        );
    }

    /// W2 backward compat: a pre-W2 `pr.opened` payload without `commit_ids`
    /// still projects — the field defaults to empty.
    #[test]
    fn parse_opened_tolerates_missing_commit_ids() {
        let mut log = EventLog::new();
        log.append_authorized(
            hugit_refstore::PrincipalClass::Orchestrator,
            hugit_refstore::Endpoint::Land,
            PR_OPENED_KIND.to_string(),
            vec!["orchestrator:test".to_string()],
            r#"{"pr_id":"old","campaign":"c","author_kind":"orchestrator",
                "intent_ids":["i0"],"principal":null,"run_id":"r"}"#
                .to_string(),
            1,
        )
        .expect("seed pre-W2 pr.opened");
        let opened = find_pr_opened(&log, "old").expect("legacy PR projects");
        assert_eq!(opened.intent_ids, vec!["i0"]);
        assert!(
            opened.commit_ids.is_empty(),
            "legacy commit_ids default empty"
        );
    }
}
