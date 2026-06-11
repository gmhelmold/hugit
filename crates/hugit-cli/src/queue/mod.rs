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
//! - **NULL (disclosed)** — the per-entry / per-batch **verdict** (did the union
//!   test pass, and which PR is implicated on a failure). No porcelain seam
//!   records a union-batch verdict onto the log yet, so `verdict` is `null` with
//!   a `verdict_note` documenting exactly why — never a faked pass/fail.
//!
//! Every output is stable JSON on stdout under the WB0 one-error/one-exit law
//! ([`crate::porcelain`]): `log_not_found` / `parse_log` are the canonical
//! `{"error":{…}}` envelopes, exit `2`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Subcommand;
use serde_json::{Value, json};

use crate::checks::load_event_log;
use crate::porcelain::PorcelainError;

/// Disclosure attached to every `null` verdict: there is no union-batch verdict
/// seam on the porcelain log yet, so pass/fail + blame are honestly absent.
const VERDICT_NOTE: &str = "no union-batch verdict seam records pass/fail onto the \
                            log yet; verdict is null (never faked) — it goes live \
                            the moment the landing path records the batch outcome";

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
/// entries, `pr::find_pr_opened` supplies each entry's campaign. Entries are
/// emitted in queue (`order_index`) order; batches GROUP the entries by campaign
/// (the union-testing unit). Each entry's `verdict` is `null` (disclosed) — no
/// verdict seam exists yet.
fn show(args: &ShowArgs) -> Result<Value, PorcelainError> {
    let log = load_event_log(&args.log)?;

    // The queue's own ordered projection — `pr.queued` in queue order. Sorting
    // by order_index makes the displayed order the queue's authority even if the
    // log records arrived out of order.
    let mut queued = crate::pr::all_pr_queued(&log);
    queued.sort_by_key(|q| q.order_index);

    // Per-entry rows + campaign grouping, in one fold over the ordered queue.
    let mut entries: Vec<Value> = Vec::new();
    let mut by_campaign: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for q in &queued {
        // The owning pr.opened supplies the campaign; absent ⇒ honest null
        // (a queued PR with no pr.opened is a corrupt log, surfaced as null,
        // never guessed).
        let campaign = crate::pr::find_pr_opened(&log, &q.pr_id).map(|o| o.campaign);

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

        entries.push(json!({
            "pr_id": q.pr_id,
            "position": q.order_index,
            "mode": crate::pr::LANDING_MODE,
            "item_id": q.item_id,
            "campaign": campaign,
            // Disclosed gap: no per-entry union-test verdict seam yet.
            "verdict": Value::Null,
        }));
    }

    // Batch composition: one union batch per campaign, the PRs it tests together.
    let batches: Vec<Value> = by_campaign
        .into_iter()
        .map(|(campaign, members)| {
            json!({
                "campaign": campaign,
                "mode": crate::pr::LANDING_MODE,
                "members": members,
                "member_count": members.len(),
                // Disclosed gap: no batch-level pass/fail + blame seam yet.
                "verdict": Value::Null,
                "implicated_pr": Value::Null,
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
