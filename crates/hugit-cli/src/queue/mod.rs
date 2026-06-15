//! `hugit queue` — the union-batch landing-queue wedge made visible (WP-WB2).
//!
//! The audit's Tier-2 P1 finding: `pr land` returns a bare position with no
//! batch / blame, and the union-batch landing queue is otherwise invisible. WB0
//! landed the VERB as an honest stub; WB2 fills the projection over the real
//! engine seams.
//!
//! # What is real vs. honestly-null
//!
//! `queue show --log <path>` projects the landing queue from the SAME canonical
//! event log every porcelain verb shares, reusing the `pr` module's own
//! projections (`pr::all_pr_queued`, `pr::find_pr_opened`) so there is no second
//! source of truth:
//!
//! - **REAL** — every `pr.queued` event, in queue order (the `order_index` the
//!   queue engine assigned through `Batch`), each with `pr_id`, `position`,
//!   `mode` (always `union` in v1 — the wedge property). The owning `pr.opened`
//!   supplies the `campaign`, so entries are GROUPED by campaign and the union
//!   batch each campaign tests together is composed from its members.
//! - **REAL (PS-6)** — the per-entry / per-batch **verdict** + **implicated_pr**,
//!   projected from the SAME `verdict.recorded` events `campaign show` reads
//!   (via the shared [`hugit_ledger::Ledger`] reject-sticky fold), so the queue
//!   and the campaign view AGREE by construction. A union batch's verdict is
//!   `"reject"` if any member intent has an outstanding reject (and
//!   `implicated_pr` names the first such PR in queue order), `"approve"` once
//!   every member intent is proven, and `null` (with the disclosing
//!   `verdict_note`) while no `verdict.recorded` event covers the batch yet —
//!   honest unknown, never a faked pass/fail.
//!
//! Every output is stable JSON on stdout under the WB0 one-error/one-exit law
//! ([`crate::porcelain`]): `log_not_found` / `parse_log` are the canonical
//! `{"error":{…}}` envelopes, exit `2`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use serde_json::{Value, json};

use hugit_ledger::Ledger;

use crate::checks::load_event_log;
use crate::porcelain::PorcelainError;

/// Disclosure attached to a `null` verdict: a null is the honest "no
/// `verdict.recorded` event covers this batch/entry yet" state (incomplete
/// coverage or no verdicts), never a faked pass/fail. A non-null verdict is
/// the union of the recorded per-intent verdicts (PS-6).
const VERDICT_NOTE: &str = "verdict is the union of the recorded per-intent verdicts \
                            (reject is decisive and names implicated_pr; approve \
                            requires every member intent proven); null means no \
                            verdict.recorded event covers the batch yet — never faked";

/// The resolved union verdict over a set of intents (PS-6). `None` is the honest
/// "undecided" state (no covering verdict, or coverage incomplete).
#[derive(Clone, Copy, PartialEq, Eq)]
enum UnionVerdict {
    Approve,
    Reject,
}

/// Aggregate a union verdict over `intent_ids` from the ledger's resolved
/// per-intent `(proven, rejected)` state.
///
/// Reject is decisive — any member intent carrying an outstanding (sticky)
/// reject fails the whole union. `Approve` requires EVERY member intent proven.
/// Anything else (an intent with no recorded verdict, or an empty set) is `None`
/// — the honest "not decided yet" state, surfaced as a `null` verdict.
fn aggregate_union_verdict(
    intent_ids: &[String],
    state: &std::collections::BTreeMap<&str, (bool, bool)>,
) -> Option<UnionVerdict> {
    if intent_ids.is_empty() {
        return None;
    }
    let mut all_proven = true;
    for id in intent_ids {
        match state.get(id.as_str()) {
            // A sticky reject on any member is decisive for the whole union.
            Some((_, true)) => return Some(UnionVerdict::Reject),
            // Proven (approve recorded, no reject).
            Some((true, false)) => {}
            // No verdict (or landed-but-unproven) for this intent → undecided.
            _ => all_proven = false,
        }
    }
    if all_proven {
        Some(UnionVerdict::Approve)
    } else {
        None
    }
}

/// Render a [`UnionVerdict`] as the wire JSON value (`"approve"` / `"reject"` /
/// `null`).
fn verdict_json(v: Option<UnionVerdict>) -> Value {
    match v {
        Some(UnionVerdict::Approve) => Value::String("approve".to_string()),
        Some(UnionVerdict::Reject) => Value::String("reject".to_string()),
        None => Value::Null,
    }
}

/// `hugit queue <subcommand>` — landing-queue visibility (WP-WB2).
#[derive(clap::Args, Debug)]
pub struct QueueArgs {
    #[command(subcommand)]
    pub command: QueueCommand,
}

/// The queue subcommand surface — `show`.
#[derive(Subcommand, Debug)]
pub enum QueueCommand {
    /// Show the landing-queue state: entries in order, batch composition, campaign.
    Show(ShowArgs),
}

/// `hugit queue show` flags.
#[derive(clap::Args, Debug)]
pub struct ShowArgs {
    /// Path to the canonical JSON event log (`[EventRecord, …]`) — the one
    /// `--log` seam every porcelain verb shares.
    #[arg(long)]
    pub log: PathBuf,
    /// Optional campaign key: scope the projection to one campaign's batch.
    #[arg(long)]
    pub campaign: Option<String>,
}

/// Dispatch a `queue` subcommand, emitting stable JSON on stdout and returning
/// the process exit code under the WB0 one-exit-code law.
pub fn run(args: QueueArgs) -> ExitCode {
    let result = match args.command {
        QueueCommand::Show(a) => show(&a),
    };
    match result {
        Ok(value) => {
            println!("{value}");
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("{}", e.to_json());
            e.exit_code()
        }
    }
}

/// `hugit queue show` — project the landing queue from the canonical log.
///
/// Reuses the `pr` module's own projections so the queue surface and `pr land`
/// agree by construction: `pr::all_pr_queued` gives the ordered `pr.queued`
/// entries, `pr::find_pr_opened` supplies each entry's campaign + intents.
/// Entries are emitted in queue (`order_index`) order; batches GROUP the entries
/// by campaign (the union-testing unit). The per-entry / per-batch `verdict` +
/// `implicated_pr` are projected from the `verdict.recorded` events via the
/// shared [`Ledger`] (PS-6) — `null` only until a verdict covers the batch.
fn show(args: &ShowArgs) -> Result<Value, PorcelainError> {
    let log = load_event_log(&args.log)?;

    // The queue's own ordered projection — `pr.queued` in queue order. Sorting
    // by order_index makes the displayed order the queue's authority even if the
    // log records arrived out of order.
    let mut queued = crate::pr::all_pr_queued(&log);
    queued.sort_by_key(|q| q.order_index);

    // Per-intent verdict resolution (PS-6). REUSE the SAME `Ledger` projection
    // `campaign show` reads, so the queue's batch verdict AGREES with the
    // campaign view by construction — the reject-sticky / per-lens fold
    // (K-VERDICT / C5-F1) lives in ONE place. We read only the resolved
    // per-intent `(proven, rejected)` flags.
    //
    // The join is by `intent_id`. A recorded intent_id is always a safe-address
    // shape — the identifier door rejects a secret-shaped `--id`/`--intent` at
    // input (`secret_in_identifier`, exit 2) before it ever reaches the log — so
    // the ledger's view-boundary redaction is a no-op on it and the surfaced
    // `intent_id` equals the raw id `pr.opened` carries. The lookup is exact.
    let ledger = Ledger::from_records(log.records());
    let verdict_state: BTreeMap<&str, (bool, bool)> = ledger
        .entries()
        .iter()
        .map(|e| (e.intent_id.as_str(), (e.proven, e.rejected)))
        .collect();

    // Per-entry rows + campaign grouping, in one fold over the ordered queue.
    let mut entries: Vec<Value> = Vec::new();
    let mut by_campaign: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // pr_id → its intent_ids, for the batch verdict + blame join below.
    let mut pr_intents: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for q in &queued {
        // The owning pr.opened supplies the campaign + intent ids; absent ⇒
        // honest null (a queued PR with no pr.opened is a corrupt log, surfaced
        // as null, never guessed).
        let opened = crate::pr::find_pr_opened(&log, &q.pr_id);
        let campaign = opened.as_ref().map(|o| o.campaign.clone());
        let intent_ids: Vec<String> = opened
            .as_ref()
            .map(|o| o.intent_ids.clone())
            .unwrap_or_default();

        // Apply the --campaign scope: skip entries outside the requested batch.
        if let Some(want) = &args.campaign
            && campaign.as_deref() != Some(want.as_str())
        {
            continue;
        }

        if let Some(c) = &campaign {
            by_campaign
                .entry(c.clone())
                .or_default()
                .push(q.pr_id.clone());
        }
        pr_intents.insert(q.pr_id.clone(), intent_ids.clone());

        // Per-entry verdict: the union over THIS PR's own intents.
        let entry_verdict = aggregate_union_verdict(&intent_ids, &verdict_state);

        entries.push(json!({
            "pr_id": q.pr_id,
            "position": q.order_index,
            "mode": crate::pr::LANDING_MODE,
            "item_id": q.item_id,
            "campaign": campaign,
            "verdict": verdict_json(entry_verdict),
        }));
    }

    // Batch composition: one union batch per campaign, the PRs it tests together.
    let batches: Vec<Value> = by_campaign
        .into_iter()
        .map(|(campaign, members)| {
            // The batch's verdict is the union over ALL member PRs' intents.
            let batch_intents: Vec<String> = members
                .iter()
                .filter_map(|pr_id| pr_intents.get(pr_id))
                .flat_map(|ids| ids.iter().cloned())
                .collect();
            let verdict = aggregate_union_verdict(&batch_intents, &verdict_state);
            // Blame: on a reject, the first member PR (queue order) owning an
            // intent with an outstanding reject.
            let implicated_pr = match verdict {
                Some(UnionVerdict::Reject) => members
                    .iter()
                    .find(|pr_id| {
                        pr_intents.get(*pr_id).is_some_and(|ids| {
                            ids.iter()
                                .any(|i| verdict_state.get(i.as_str()).is_some_and(|(_, rej)| *rej))
                        })
                    })
                    .cloned(),
                _ => None,
            };
            json!({
                "campaign": campaign,
                "mode": crate::pr::LANDING_MODE,
                "members": members,
                "member_count": members.len(),
                "verdict": verdict_json(verdict),
                "implicated_pr": implicated_pr,
            })
        })
        .collect();

    Ok(json!({
        "log": args.log.display().to_string(),
        "campaign": args.campaign,
        "queue_depth": entries.len(),
        "entries": entries,
        "batches": batches,
        "verdict_note": VERDICT_NOTE,
    }))
}
