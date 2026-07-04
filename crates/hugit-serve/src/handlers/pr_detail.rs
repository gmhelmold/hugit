//! `GET /v1/repos/{repo}/prs/{n}` → [`PrDetailVm`] (None = 404). FROZEN signature;
//! body filled by the fleet per master-plan §5 (REAL: PR projection + cost split
//! via ledger + intents + check_rows + envelope + the REAL diffstat; STUB:
//! impact, reviewers, labels, mirror).
//!
//! Data-readiness (master-plan §0, the prs/{n} row): cost-real, diff-real. REAL =
//! number/title/state/author/session, campaign, cost split (ledger `pr_record`),
//! intents, check_rows, envelope/CAS, why/acceptance (from envelope), and the
//! diff/added/removed/file_count (the tip intent's commit-vs-first-parent numstat,
//! or — for a branch PR opened via `POST …/prs` — the pinned head-vs-base compare;
//! `source_branch`/`target_branch` are then the pinned branch names). STUB (honest
//! defaults, never faked) = impact (blast-radius not wired), reviewers/labels/
//! conversation, mirror (P2); `source/target_branch` stay empty for a
//! dispatch/edit PR (no pinned branch names).

use std::sync::Arc;
use std::time::Instant;

use crate::fmt::{CHECKS_CAP, pct_u8, scrub, scrub_all, str_field};
use crate::handlers::diff::{diff_totals, empty_diff};
use gix_hash::ObjectId;
use gix_object::CommitRefIter;
use hugit_cli::checks::CHECK_RECORDED_KIND;
use hugit_cli::pr::{
    INTENT_ENVELOPE_KIND, OpenedPr, PR_ABANDONED_KIND, PR_ENVELOPE_KIND, PR_LANDED_KIND,
    PR_OPENED_KIND, PR_QUEUED_KIND, find_pr_opened,
};
use hugit_contracts::context_envelope::{Altitude, CiCost, ContextEnvelope, PrRecord};
use hugit_http_contracts::common::{
    CampaignChipVm, CheckRowVm, DiffVm, EnvelopeVm, FileRowVm, IntentSummaryVm, MirrorVm, UnionVm,
};
use hugit_http_contracts::{CostSplitVm, ImpactVm, PrDetailVm};
use hugit_ledger::{Ledger, PrQueueInput, pr_record};
use hugit_refstore::EventLog;
use hugit_refstore::intent::projection::{ProjectionRow, project_machine};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Build the PR-detail view-model for PR `pr_number`.
///
/// The `log` is ALREADY chain-verified by the caller — do NOT re-load or
/// re-verify. Returns `None` when no `pr.opened` for `pr_number` is on the log
/// (→ HTTP 404; the read is `get_opt`: not-found and no-access are
/// indistinguishable — no existence leak).
///
/// Field provenance is tagged inline: REAL (a real engine fn), PRESENTATION
/// (adapter humanizes a real datum), or STUB (no local source → honest default,
/// never faked) per master-plan §0/§5.
pub fn build_pr_detail(
    log: &EventLog,
    repo: &str,
    pr_number: u32,
    git_source: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
) -> Option<PrDetailVm> {
    let pr_id = pr_number.to_string();

    // ── REAL: the PR must exist on the log (else 404, no existence leak) ─────
    let opened = find_pr_opened(log, &pr_id)?;

    // ── REAL: a branch PR (opened from head vs base via `POST …/prs`) pins both
    //          tip SHAs + the branch names + title/body on its `pr.opened`. When
    //          present, the PR-level diff is the head-vs-base compare (below) and
    //          the source/target branches + title come from these pinned fields —
    //          not the intent-bundle projection. `None` for a dispatch/edit PR
    //          (keeps the existing intent-commit diff path). ───────────────────
    let branch_pr = find_branch_pr(log, &pr_id);

    // ── REAL: map each intent_id → its landed commit oid (machine projection) ─
    // Used to compute the per-intent and PR-level numstats (commit vs first
    // parent). Only well-formed 40-hex targets are kept; a short/synthetic target
    // simply has no entry → an honest-empty per-intent diff.
    let intent_commits = intent_commit_oids(log);

    // ── REAL: state_label — PR lifecycle state projected off the log ────────
    // Mirrors the documented `pr_state` precedence (landed ≻ abandoned ≻ queued
    // ≻ proposed); the engine's fn is private, so the projection is replicated.
    let state_label = pr_state_label(log, &pr_id).to_string();

    // ── REAL: campaign chip from the OpenedPr ───────────────────────────────
    let campaign = if opened.campaign.is_empty() {
        None
    } else {
        Some(CampaignChipVm {
            id: scrub(&opened.campaign),
            label: scrub(&opened.campaign),
            color_class: String::new(),   // STUB — no color seam
            display_label: String::new(), // serde(default)
        })
    };
    let provenance_campaign = scrub(&opened.campaign);

    // ── REAL: PR-altitude ContextEnvelope (charter→why, acceptance→note,
    //          authorship→author/session, context_ref→envelope_cas) ──────────
    let pr_env = pr_altitude_envelope(log, &pr_id);

    let (author, session, why, acceptance_note, envelope, envelope_cas) = match &pr_env {
        Some(env) => {
            let author = scrub(&env.authorship.operator); // REAL (scrubbed free text)
            let session = env.intent_id.clone(); // REAL — structural session/run id
            let why = scrub(&env.charter); // REAL (scrubbed free text)
            let acceptance_note = scrub_all(&env.acceptance).join("; "); // REAL (scrubbed)
            let envelope = envelope_vm(env); // REAL (free text scrubbed inside)
            let envelope_cas = env.snapshot.prompt_ref.clone().unwrap_or_default(); // structural ref
            (
                author,
                session,
                why,
                acceptance_note,
                envelope,
                envelope_cas,
            )
        }
        None => (
            String::new(),         // STUB — no envelope authorship on log
            String::new(),         // STUB
            String::new(),         // STUB — no charter
            String::new(),         // STUB — no acceptance
            EnvelopeVm::default(), // STUB — honest empty envelope
            String::new(),         // STUB
        ),
    };

    // ── DoS (single-thread latency): ONE shared wall-clock deadline for the
    //          WHOLE PR render's diff work (the N per-intent diffs + the PR-level
    //          tip diff), NOT a fresh per-diff budget. On the single-threaded
    //          lazy-git-from-CAS engine each tree/blob is a synchronous R2 fetch;
    //          without a SHARED bound a bundle with N intents would cost
    //          N × DIFF_BUDGET wall-clock and wedge the accept loop for minutes. A
    //          shared deadline caps the entire render at ~one DIFF_BUDGET: once it
    //          passes, remaining intents get an honest-empty/partial diff (never a
    //          fabricated full diff, never a 500) — the same honest-partial the
    //          per-diff `tree_diff` already applies at its own budget. ────────────
    let diff_deadline = Instant::now() + hugit_proto::DIFF_BUDGET;

    // ── REAL: intents in this PR's bundle (Ledger projection) — each carries
    //          its own commit-vs-first-parent numstat when a git source is wired ─
    let intents = build_intents(log, &opened, git_source, &intent_commits, diff_deadline);

    // ── REAL or honest-EMPTY: the PR-level numstat ───────────────────────────
    // The PR's representative diff is its tip intent's commit vs first parent —
    // the LAST bundle intent with a resolvable commit (bundle order = landing
    // order). No git source / no resolvable commit → an honest-empty diff. The
    // scalar diffstat (file_count/added/removed) is the rollup of that diff.
    // Shares the render's ONE `diff_deadline` (see above).
    // A branch PR's representative diff is its pinned head-vs-base compare (ONE
    // bounded `diff_commits`, the same primitive the `compare` handler serves);
    // otherwise the tip intent's commit vs first parent (the bundle path). Both are
    // honest-empty without a git seam / resolvable tips — never fabricated.
    let pr_diff = match (&branch_pr, git_source) {
        (Some(bp), Some(src)) => {
            crate::handlers::diff::diff_commits(Some(src), Some(&bp.base_oid), Some(&bp.head_oid))
        }
        _ => pr_level_diff(git_source, &opened, &intent_commits, diff_deadline),
    };
    let (pr_file_count, pr_added, pr_removed) = diff_totals(&pr_diff);

    // ── REAL: check_rows from check.recorded events (capped — the VM is the page) ─
    let mut check_rows = build_check_rows(log);
    check_rows.truncate(CHECKS_CAP);
    let checks_count = check_rows.len() as u32;

    // ── REAL or honest-ZERO: the F3 cost split (ledger pr_record) ───────────
    // Present only when a PR-altitude envelope is captured on the log; honest
    // ZERO (all *_usd 0.0, notes "") otherwise — NEVER faked.
    let cost = build_cost(log, &opened, pr_env.as_ref());

    // ── REAL: title — a branch PR carries the author-supplied title (scrubbed at
    //          the read boundary); a dispatch/edit PR composes it from the bundle. ─
    let title = match &branch_pr {
        Some(bp) if !bp.title.is_empty() => scrub(&bp.title),
        _ => pr_title(&opened),
    };

    // ── REAL: a branch PR's description (`body`) surfaces as `why` when there is
    //          no PR-altitude envelope (the branch PR has no authored envelope). ──
    let why = match (&branch_pr, &pr_env) {
        (Some(bp), None) if !bp.body.is_empty() => scrub(&bp.body),
        _ => why,
    };

    // ── REAL: source/target branches — the pinned head/base names for a branch PR
    //          (scrubbed at the read boundary), honest-empty otherwise. ──────────
    let (source_branch, target_branch) = match &branch_pr {
        Some(bp) => (scrub(&bp.head_branch), scrub(&bp.base_branch)),
        None => (String::new(), String::new()),
    };

    // ── Assemble ────────────────────────────────────────────────────────────
    Some(PrDetailVm {
        repo: repo.to_string(),
        number: pr_number,
        title,
        state_label,
        author,
        session,
        // REAL for a branch PR (pinned head/base names); honest-empty otherwise.
        source_branch,
        target_branch,
        models_note: String::new(), // STUB — no models seam at this altitude
        // REAL — rollup of the PR-level numstat (0/0/0 honestly when no git seam)
        file_count: pr_file_count,
        added: pr_added,
        removed: pr_removed,
        better_chips: vec![], // STUB
        regen_note: None,     // STUB
        why,
        acceptance_note,
        stats: vec![], // STUB — no stat strip source
        envelope,
        campaign,
        // STUB — blast-radius not wired; checks_selected/total are REAL (row count)
        impact: ImpactVm {
            crates_touched: vec![],
            direct_dependents: vec![],
            transitive_dependents: vec![],
            critical_paths_note: String::new(),
            critical_paths_target: String::new(),
            critical_paths_anchor: String::new(),
            critical_paths_safe: true,
            checks_selected: checks_count, // REAL — count of executed/checked rows
            checks_total: checks_count,    // REAL
            source_note: String::new(),
        },
        cost,
        conversation: vec![], // STUB — no Q&A seam
        intents,
        // STUB — live union-oracle is P2; honest empty/false
        union: UnionVm {
            batch: vec![],
            verdict: String::new(),
            green: false,
        },
        check_rows,
        checks_cost_note: String::new(),  // STUB — not in log payload
        checks_cache_note: String::new(), // STUB
        diff: pr_diff, // REAL — PR tip intent vs first parent (honest-empty w/o git seam)
        landing_status: vec![], // STUB — no landing-status (k,v) seam
        reviewers: vec![], // STUB — no reviewer rail seam
        panel_note: String::new(),
        assignee: String::new(),  // STUB
        labels: vec![],           // STUB
        milestone: None,          // STUB
        milestone_progress: None, // STUB
        provenance_campaign,
        stack: None,                     // STUB
        change_id: opened.pr_id.clone(), // REAL — the PR's stable id
        attested_label: String::new(),   // STUB
        envelope_cas,
        transcripts_label: String::new(), // STUB
        mirror: MirrorVm {
            synced: false,
            detail: String::new(),
        }, // STUB — P2 mirror
    })
}

// ---------------------------------------------------------------------------
// Projections (replicate the documented engine folds; the engine fns are
// private, so the logic is transcribed, never re-designed).
// ---------------------------------------------------------------------------

/// The branch-PR fields pinned on a `pr.opened` opened via `POST …/prs`
/// (`write_pr_create`): both tip SHAs (resolved at open time) + the branch names +
/// the author-supplied title/body. A dispatch/edit PR carries none of these.
struct BranchPr {
    /// The base (target) branch tip commit — the "before" side of the compare.
    base_oid: ObjectId,
    /// The head (source) branch tip commit — the "after" side of the compare.
    head_oid: ObjectId,
    /// The head (source) branch name (free text — scrubbed at the read boundary).
    head_branch: String,
    /// The base (target) branch name (free text — scrubbed at the read boundary).
    base_branch: String,
    /// The author-supplied PR title (already scrubbed at write; re-scrubbed on read).
    title: String,
    /// The author-supplied PR description/body (surfaced as `why`; scrubbed on read).
    body: String,
}

/// Project the branch-PR fields for `pr_id`, if the latest `pr.opened` for it pins
/// BOTH a valid 40-hex `head_sha` and `base_sha` (the `POST …/prs` shape). Returns
/// `None` for a dispatch/edit PR (no pinned SHAs) → the caller keeps the
/// intent-bundle diff path. Never fabricates: a malformed/absent SHA → `None`.
fn find_branch_pr(log: &EventLog, pr_id: &str) -> Option<BranchPr> {
    // The LATEST pr.opened for this id wins (robust to a re-open), mirroring
    // `find_pr_opened`'s rfind-latest semantics.
    let rec = log.records().iter().rfind(|r| {
        r.kind == PR_OPENED_KIND
            && serde_json::from_str::<Value>(&r.payload)
                .ok()
                .and_then(|v| v.get("pr_id").and_then(Value::as_str).map(|s| s == pr_id))
                .unwrap_or(false)
    })?;
    let v: Value = serde_json::from_str(&rec.payload).ok()?;
    let head_oid =
        ObjectId::from_hex(v.get("head_sha").and_then(Value::as_str)?.as_bytes()).ok()?;
    let base_oid =
        ObjectId::from_hex(v.get("base_sha").and_then(Value::as_str)?.as_bytes()).ok()?;
    let field = |k: &str| {
        v.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    Some(BranchPr {
        base_oid,
        head_oid,
        head_branch: field("head"),
        base_branch: field("base"),
        title: field("title"),
        body: field("body"),
    })
}

/// PR lifecycle state, projected off the log (mirrors `pr::pr_state`):
/// `landed` ≻ `abandoned` ≻ `queued` ≻ `proposed` (most-settled wins).
fn pr_state_label(log: &EventLog, pr_id: &str) -> &'static str {
    if has_pr_event(log, PR_LANDED_KIND, pr_id) {
        "landed"
    } else if has_pr_event(log, PR_ABANDONED_KIND, pr_id) {
        "abandoned"
    } else if has_pr_event(log, PR_QUEUED_KIND, pr_id) {
        "queued"
    } else {
        "proposed"
    }
}

/// Whether the log carries an event of `kind` whose payload names `pr_id`.
fn has_pr_event(log: &EventLog, kind: &str, pr_id: &str) -> bool {
    log.records()
        .iter()
        .filter(|r| r.kind == kind)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .any(|v| v.get("pr_id").and_then(Value::as_str) == Some(pr_id))
}

/// The latest PR-altitude [`ContextEnvelope`] captured for `pr_id`, if any
/// (mirrors `pr::envelopes_for_pr`'s PR-envelope selection).
fn pr_altitude_envelope(log: &EventLog, pr_id: &str) -> Option<ContextEnvelope> {
    log.records()
        .iter()
        .filter(|r| r.kind == PR_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .rfind(|e| e.altitude == Altitude::Pr && e.intent_id == pr_id)
}

/// The intent-altitude envelopes attributed to this PR's bundle (mirrors
/// `pr::envelopes_for_pr`'s intent selection).
fn intent_altitude_envelopes(log: &EventLog, opened: &OpenedPr) -> Vec<ContextEnvelope> {
    let bundle: BTreeSet<&str> = opened.intent_ids.iter().map(String::as_str).collect();
    log.records()
        .iter()
        .filter(|r| r.kind == INTENT_ENVELOPE_KIND)
        .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
        .filter(|e| e.altitude == Altitude::Intent && bundle.contains(e.intent_id.as_str()))
        .collect()
}

/// Map a PR-altitude [`ContextEnvelope`] into the wire [`EnvelopeVm`] — REAL.
fn envelope_vm(env: &ContextEnvelope) -> EnvelopeVm {
    let summary = env.trajectory.summary.clone().unwrap_or_default();
    let headline = scrub(&summary);
    EnvelopeVm {
        session: env.intent_id.clone(),      // structural
        model: scrub(&env.authorship.model), // free text — scrubbed
        window: String::new(),               // STUB — no window seam
        headline: headline.clone(),          // free text — scrubbed
        transcript_complete: env.trajectory.raw_transcript_ref.is_some()
            && env.trajectory.task_transcript_ref.is_some(),
        session_summary: headline, // same scrubbed summary text
        compact_transcript_ref: env
            .trajectory
            .task_transcript_ref
            .clone()
            .unwrap_or_default(),
        raw_transcript_ref: env
            .trajectory
            .raw_transcript_ref
            .clone()
            .unwrap_or_default(),
        snapshot_files: env
            .snapshot
            .files_read
            .iter()
            .map(|f| scrub(&f.path)) // free text — scrubbed
            .collect(),
        context_json: String::new(), // STUB — no pretty-printed context.json here
        context_cas: env.snapshot.prompt_ref.clone().unwrap_or_default(),
        compact_context_ref: String::new(), // STUB
        compact_json_note: String::new(),   // STUB
        bundle_note: String::new(),         // STUB
    }
}

/// The PR's intents (Ledger projection), filtered to the PR's bundle — REAL.
/// Each intent's `diff` is its landed commit vs first parent (REAL when a git
/// source is wired AND the intent has a resolvable 40-hex commit; honest-empty
/// otherwise — never faked).
///
/// DoS: every per-intent diff shares the ONE `deadline` established by the caller
/// (`build_pr_detail`) — so a bundle with N intents is bounded by a SINGLE
/// wall-clock budget, not N × DIFF_BUDGET. Past the deadline each remaining
/// intent gets the honest-empty/partial diff (`diff_first_parent_until` → an
/// already-expired `tree_diff_until` returns immediately) — never a fabricated
/// full diff, never an unbounded loop.
fn build_intents(
    log: &EventLog,
    opened: &OpenedPr,
    git_source: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    intent_commits: &BTreeMap<String, ObjectId>,
    deadline: Instant,
) -> Vec<IntentSummaryVm> {
    let bundle: BTreeSet<&str> = opened.intent_ids.iter().map(String::as_str).collect();
    let ledger = Ledger::from_records(log.records());
    ledger
        .entries()
        .iter()
        .filter(|e| bundle.contains(e.intent_id.as_str()))
        .map(|e| {
            // REAL: this intent's commit-vs-first-parent numstat (honest-empty
            // when no git seam or no resolvable commit for the intent). Bounded by
            // the render's SHARED `deadline` (not a fresh per-intent budget).
            let diff = match (git_source, intent_commits.get(e.intent_id.as_str())) {
                (Some(src), Some(commit)) => diff_first_parent_until(src, commit, deadline),
                _ => empty_diff(),
            };
            IntentSummaryVm {
                id: e.intent_id.clone(),
                title: e.charter.clone(),
                status: if e.rejected {
                    "REJECTED".to_string()
                } else if e.proven {
                    "PROVEN".to_string()
                } else {
                    "LANDED".to_string()
                },
                charter: e.charter.clone(),
                context_json: String::new(), // STUB — no context.json snapshot here
                diff,                        // REAL — intent commit vs first parent
                verdicts: vec![],            // STUB — verdict detail not projected here
                model: None,                 // STUB
                envelope: None,              // STUB — intent envelope refs not surfaced here
                blame_quote: None,           // serde(default)
                blame_proof: None,           // serde(default)
            }
        })
        .collect()
}

/// Map `intent_id → landed commit oid` from the machine projection. Only
/// well-formed 40-hex targets are kept (a short/synthetic target has no entry →
/// an honest-empty per-intent diff downstream). The LAST landed commit wins for a
/// re-landed intent (the current tip).
fn intent_commit_oids(log: &EventLog) -> BTreeMap<String, ObjectId> {
    let mut out = BTreeMap::new();
    let Ok(machine) = project_machine(log) else {
        return out;
    };
    for row in machine.rows() {
        if let ProjectionRow::Intent(c) = row
            && let Ok(oid) = ObjectId::from_hex(c.target.as_bytes())
        {
            out.insert(c.intent_id.clone(), oid);
        }
    }
    out
}

/// The PR's representative numstat: its TIP intent's commit vs first parent —
/// the last bundle intent (landing order) with a resolvable commit. No git source
/// / no resolvable commit → the honest-empty diff. Shares the render's ONE
/// `deadline` (see [`build_pr_detail`]) so it cannot add another fresh budget on
/// top of the per-intent diffs.
fn pr_level_diff(
    git_source: Option<&Arc<dyn hugit_proto::ObjectSource + Send + Sync>>,
    opened: &OpenedPr,
    intent_commits: &BTreeMap<String, ObjectId>,
    deadline: Instant,
) -> DiffVm {
    let tip = opened
        .intent_ids
        .iter()
        .rev()
        .find_map(|id| intent_commits.get(id.as_str()));
    match (git_source, tip) {
        (Some(src), Some(commit)) => diff_first_parent_until(src, commit, deadline),
        _ => empty_diff(),
    }
}

/// A single COMMIT's numstat against its FIRST parent, bounded by a CALLER-SUPPLIED
/// wall-clock `deadline` (SHARED across a whole PR render's diffs).
///
/// This is the deadline-threading twin of [`crate::handlers::diff::diff_against_first_parent`]:
/// same honest-default contract (a root commit / an absent-or-non-commit object /
/// an unresolvable parent tree → the honest-empty diff — never a fabricated
/// all-added wall against a non-existent before-state), but the tree walk runs via
/// [`hugit_proto::tree_diff_until`] with the SHARED `deadline` instead of a fresh
/// per-call `DIFF_BUDGET`. So N intents cost ONE budget total, not N × budget. Past
/// the deadline `tree_diff_until` returns immediately with its partial (here empty)
/// diff — the honest-partial, indistinguishable from a real empty change (no oracle).
///
/// Paths are SCRUBBED at this read boundary ([`scrub`]) — a repo file literally
/// named `ghp_….key` must not echo verbatim into the view-model (parity with
/// `diff::diff_vm`, which scrubs identically).
fn diff_first_parent_until(
    src: &Arc<dyn hugit_proto::ObjectSource + Send + Sync>,
    commit: &ObjectId,
    deadline: Instant,
) -> DiffVm {
    // The commit's own root tree (the "new" side). Absent / not-a-commit → empty.
    let new_tree = match hugit_proto::commit_root_tree(src.as_ref(), commit) {
        Ok(Some(t)) => t,
        _ => return empty_diff(),
    };
    // The FIRST parent (`^1`). A ROOT commit (no parent) → honest-empty (no
    // before-state to diff). Decoded with the canonical `CommitRefIter`.
    let parent = src
        .get(commit)
        .ok()
        .flatten()
        .filter(|o| o.kind == hugit_proto::ObjectKind::Commit)
        .and_then(|o| CommitRefIter::from_bytes(&o.data).parent_ids().next());
    let Some(parent) = parent else {
        return empty_diff();
    };
    let parent_tree = match hugit_proto::commit_root_tree(src.as_ref(), &parent) {
        Ok(Some(t)) => t,
        _ => return empty_diff(),
    };
    // The SHARED-deadline tree walk. Fail-closed: a broken walk → honest-empty.
    match hugit_proto::tree_diff_until(src.as_ref(), &parent_tree, &new_tree, deadline) {
        Ok(files) => DiffVm {
            files: files
                .into_iter()
                .map(|f| FileRowVm {
                    path: scrub(&f.path),
                    added: f.added,
                    removed: f.removed,
                })
                .collect(),
            hunks: vec![],
        },
        Err(_) => empty_diff(),
    }
}

/// The check rows from `check.recorded` events — REAL (same projection as the
/// checks handler).
fn build_check_rows(log: &EventLog) -> Vec<CheckRowVm> {
    log.records()
        .iter()
        .filter(|r| r.kind == CHECK_RECORDED_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .map(|v| {
            let ok = v
                .get("exit")
                .and_then(Value::as_i64)
                .map(|e| e == 0)
                .unwrap_or(false);
            CheckRowVm {
                name: scrub(&str_field(&v, "name").unwrap_or_default()), // REAL (scrubbed read-boundary)
                ok,                                                      // REAL (exit==0)
                duration_ms: v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0), // REAL
                cache_hit: v.get("cache_hit").and_then(Value::as_bool).unwrap_or(false), // REAL
                log: String::new(),                                      // STUB — not in payload
                memo_key: str_field(&v, "memo_key").unwrap_or_default(), // REAL
                reason: String::new(),                                   // STUB
                cost: String::new(),                                     // STUB
            }
        })
        .collect()
}

/// The cost split — REAL from `pr_record` when a PR-altitude envelope is
/// captured, honest ZERO otherwise (all `*_usd` 0.0, notes ""). NEVER faked.
fn build_cost(log: &EventLog, opened: &OpenedPr, pr_env: Option<&ContextEnvelope>) -> CostSplitVm {
    let record = pr_env.and_then(|env| {
        // WP-COST read-time fold: augment each envelope's honest-zero cost from
        // its raw `ctx.usage` records BEFORE the rollup, so a posted usage record
        // lights the PR cost split with no re-land. Shares the helper with the
        // `/insights` cost X-ray (site C). Each ctx.usage targets an intent XOR a
        // pr, so work (intents) + orchestration (pr) never fold the same record.
        // Double-count guard + complete-or-nothing live in `envelope_with_usage`.
        let env = super::usage_fold::envelope_with_usage(env.clone(), log, "pr", &opened.pr_id);
        let intents: Vec<ContextEnvelope> = intent_altitude_envelopes(log, opened)
            .into_iter()
            .map(|e| {
                let id = e.intent_id.clone();
                super::usage_fold::envelope_with_usage(e, log, "intent", &id)
            })
            .collect();
        // No CI / queue / verdict seam at this altitude (P2 disclosed) — passed
        // zero/default exactly as the engine's `pr_record_for` does; the rollup
        // computes work/orchestration/waste/first-pass-yield from the envelopes.
        pr_record(
            &env,
            "", // no CAS binding at this altitude — threaded empty, not faked
            &intents,
            &opened.intent_ids,
            &[],
            CiCost {
                cache_hit: 0,
                exec: 0,
                cost_usd_micros: 0,
                saved_usd_micros: 0,
            },
            PrQueueInput::default(),
        )
        .ok() // a subagent-authored PR envelope is rejected → honest ZERO
    });

    match record {
        Some(r) => cost_split_from_record(&r),
        None => zero_cost(),
    }
}

/// Micro-USD → USD (`1 USD = 1_000_000`).
fn usd(micros: u64) -> f64 {
    micros as f64 / 1_000_000.0
}

/// Map a computed [`PrRecord`] into the wire [`CostSplitVm`]. The `*_usd` figures
/// are REAL; the notes/labels are PRESENTATION (short honest strings).
fn cost_split_from_record(r: &PrRecord) -> CostSplitVm {
    let c = &r.cost;
    CostSplitVm {
        author_line: String::new(), // PRESENTATION — no author line composed here
        author_badge: None,
        author_suffix: None,
        work_usd: usd(c.work.cost_usd_micros), // REAL
        work_note: format!("{} intents", r.intent_count),
        orchestration_usd: usd(c.orchestration.cost_usd_micros), // REAL
        orchestration_note: String::new(),
        verification_usd: usd(c.verification.cost_usd_micros), // REAL
        verification_note: format!("{} painéis", c.verification.verdict_panels),
        ci_usd: usd(c.ci.cost_usd_micros), // REAL
        ci_note: String::new(),
        total_usd: usd(c.total.cost_usd_micros), // REAL
        waste_usd: usd(c.waste.cost_usd_micros), // REAL
        waste_note: String::new(),
        // The efficiency ratios are fractions in `[0,1]` per ADR-0001 — scale to
        // percent at the call site; `pct_u8` clamps to 0..=100 (NOT 255).
        overhead_pct: pct_u8(r.efficiency.overhead_pct * 100.0), // REAL
        cache_savings_pct: pct_u8(r.efficiency.cache_savings_pct * 100.0), // REAL
        time_note: String::new(), // PRESENTATION — no time line composed here
    }
}

/// The honest-ZERO cost block (no PR-altitude envelope on the log).
fn zero_cost() -> CostSplitVm {
    CostSplitVm {
        author_line: String::new(),
        author_badge: None,
        author_suffix: None,
        work_usd: 0.0,
        work_note: String::new(),
        orchestration_usd: 0.0,
        orchestration_note: String::new(),
        verification_usd: 0.0,
        verification_note: String::new(),
        ci_usd: 0.0,
        ci_note: String::new(),
        total_usd: 0.0,
        waste_usd: 0.0,
        waste_note: String::new(),
        overhead_pct: 0,
        cache_savings_pct: 0,
        time_note: String::new(),
    }
}

/// A presentation title for the PR (no stored title field — composed honestly
/// from the campaign + bundle size).
fn pr_title(opened: &OpenedPr) -> String {
    let n = opened.intent_ids.len();
    // `pr_id` is payload-derived free text (not a router-validated u32), so the
    // WHOLE composed title is scrubbed at the read boundary — a secret-shaped
    // pr_id embedded here would otherwise echo verbatim.
    let raw = if opened.campaign.is_empty() {
        format!("PR #{} — {} intents", opened.pr_id, n)
    } else {
        format!("PR #{} ({}) — {} intents", opened.pr_id, opened.campaign, n)
    };
    scrub(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_cli::pr::PR_OPENED_KIND;
    use hugit_proto::{CasObjectSource, GitObject, ObjectKind, ObjectSource};
    use hugit_refstore::intent::INTENT_LANDED_KIND;
    use std::time::Duration;

    fn push(log: &mut EventLog, kind: &str, payload: serde_json::Value, seq: u64) {
        log.append_for_test(kind, vec!["t".to_string()], payload.to_string(), seq);
    }
    fn blob(src: &mut CasObjectSource, body: &str) -> ObjectId {
        src.insert(GitObject::new(ObjectKind::Blob, body.as_bytes().to_vec()))
    }
    fn tree(src: &mut CasObjectSource, mut entries: Vec<(&str, &str, ObjectId)>) -> ObjectId {
        entries.sort_by(|a, b| a.1.as_bytes().cmp(b.1.as_bytes()));
        let mut out = Vec::new();
        for (mode, name, oid) in &entries {
            out.extend_from_slice(mode.as_bytes());
            out.push(b' ');
            out.extend_from_slice(name.as_bytes());
            out.push(0);
            out.extend_from_slice(oid.as_bytes());
        }
        src.insert(GitObject::new(ObjectKind::Tree, out))
    }
    fn commit(src: &mut CasObjectSource, tree_oid: ObjectId, parent: Option<ObjectId>) -> ObjectId {
        let parent_line = parent.map(|p| format!("parent {p}\n")).unwrap_or_default();
        let body = format!(
            "tree {tree_oid}\n{parent_line}author a <a@a> 0 +0000\ncommitter a <a@a> 0 +0000\n\nm\n"
        );
        src.insert(GitObject::new(ObjectKind::Commit, body.into_bytes()))
    }

    /// Build a commit that modifies `f.txt` (root parent → child), returning the
    /// child oid. Distinct per `i` (varied content), so every intent has a real,
    /// non-empty first-parent diff.
    fn landed_commit(src: &mut CasObjectSource, i: usize) -> ObjectId {
        let old = blob(src, "a\n");
        let new = blob(src, &format!("a\nb{i}\n"));
        let pt = tree(src, vec![("100644", "f.txt", old)]);
        let ct = tree(src, vec![("100644", "f.txt", new)]);
        let pc = commit(src, pt, None);
        commit(src, ct, Some(pc))
    }

    /// The shared-deadline diff primitive: a LIVE deadline yields the REAL
    /// first-parent numstat; a PAST deadline yields the honest-empty diff (the
    /// tree walk short-circuits at the shared deadline) — never a fabrication.
    #[test]
    fn diff_first_parent_until_is_deadline_bounded() {
        let mut src = CasObjectSource::new();
        let cc = landed_commit(&mut src, 0);
        let arc: Arc<dyn ObjectSource + Send + Sync> = Arc::new(src);

        let live = Instant::now() + Duration::from_secs(60);
        let d = diff_first_parent_until(&arc, &cc, live);
        assert_eq!(d.files.len(), 1, "live budget → the real diff");
        assert_eq!((d.files[0].added, d.files[0].removed), (1, 0));

        let past = Instant::now() - Duration::from_secs(1);
        assert!(
            diff_first_parent_until(&arc, &cc, past).files.is_empty(),
            "a spent deadline → honest-empty, never a fabricated diff"
        );
    }

    /// The DoS fix: `build_intents` over MANY intents is bounded by ONE shared
    /// wall-clock deadline, NOT N × DIFF_BUDGET. Deterministic proof (no flaky
    /// timing): with a PAST shared deadline EVERY per-intent tree walk
    /// short-circuits → every intent's diff is honest-empty; with a LIVE deadline
    /// the same N intents each carry their real diff (the bound is transparent on
    /// the happy path). Never a panic / 500 — the honest-partial answer.
    #[test]
    fn build_intents_many_bounded_by_one_shared_deadline() {
        let n = 40usize;
        let mut src = CasObjectSource::new();
        let ids: Vec<String> = (0..n).map(|i| format!("i-{i}")).collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let mut log = EventLog::new();
        push(
            &mut log,
            PR_OPENED_KIND,
            serde_json::json!({"author_kind":"orchestrator","campaign":"c","intent_ids":id_refs,"pr_id":"1"}),
            1,
        );
        let mut map: BTreeMap<String, ObjectId> = BTreeMap::new();
        for (i, id) in ids.iter().enumerate() {
            let cc = landed_commit(&mut src, i);
            push(
                &mut log,
                INTENT_LANDED_KIND,
                serde_json::json!({"intent_id":id,"charter":"x","ref":"r","target":cc.to_string()}),
                (i as u64) + 2,
            );
            map.insert(id.clone(), cc);
        }
        let arc: Arc<dyn ObjectSource + Send + Sync> = Arc::new(src);
        let opened = find_pr_opened(&log, "1").expect("pr.opened present");

        // LIVE shared deadline: each intent carries its real diff.
        let live = Instant::now() + Duration::from_secs(60);
        let intents = build_intents(&log, &opened, Some(&arc), &map, live);
        assert_eq!(intents.len(), n);
        assert!(
            intents.iter().all(|i| i.diff.files.len() == 1),
            "a live shared budget projects each intent's real diff"
        );

        // PAST shared deadline: ALL N diffs short-circuit at once → honest-empty,
        // proving the render is bounded by ONE budget, not N × budget.
        let past = Instant::now() - Duration::from_secs(1);
        let intents_past = build_intents(&log, &opened, Some(&arc), &map, past);
        assert_eq!(intents_past.len(), n);
        assert!(
            intents_past.iter().all(|i| i.diff.files.is_empty()),
            "a spent shared deadline bounds ALL {n} diffs at once (not N×)"
        );
    }
}
