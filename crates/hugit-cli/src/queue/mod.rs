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

/// Disclosure attached to every `queue show`: the entry `state` projects the
/// idempotent landing state machine ([`hugit_queue::core::EntryState`]) AS FAR AS
/// the canonical log records it. A `pr.landed` event settles an entry to the
/// terminal `Landed` (it then leaves the active queue — observed by its absence);
/// but the OTHER terminal — `UnionFail` (the bisected failing-pair exclusion) — and
/// the bisection itself are computed IN-CORE by the queue engine and are **not yet
/// emitted as log events**. So a reject-verdict entry surfaces `state:"union_fail"`
/// projected from the SAME ledger reject the verdict uses (real), while the
/// minimal failing PAIR is `null`/`n/a` until a `queue.union_fail` recorder lands.
const STATE_MACHINE_NOTE: &str = "entry `state` projects hugit_queue's landing state machine \
                                  (queued/landable/blocked/union_fail) from the log + ledger; a \
                                  terminal `landed` entry leaves the active queue (absent here); \
                                  the bisected minimal failing-pair is recorded by `hugit land queue` \
                                  as a queue.union_fail event (failing_pair is null until a batch \
                                  land has localised a failure for the campaign) — never faked";

/// Project the most recent bisected minimal failing pair from the log's
/// `queue.union_fail` records (the recorder is the `hugit land queue` batch
/// land). Scoped to `campaign` when given. Returns `{item_a,item_b}` for a
/// genuine pair locus, `null` otherwise (no batch land yet, a single-item or
/// unlocalised locus — never a fabricated pair).
fn failing_pair_for(log: &hugit_refstore::EventLog, campaign: Option<&str>) -> Value {
    log.records()
        .iter()
        .filter(|r| r.kind == crate::land::QUEUE_UNION_FAIL_KIND)
        .filter_map(|r| serde_json::from_str::<Value>(&r.payload).ok())
        .filter(|v| campaign.is_none_or(|c| v.get("campaign").and_then(Value::as_str) == Some(c)))
        .rfind(|v| v.get("locus").and_then(Value::as_str) == Some("pair"))
        .map(|v| {
            json!({
                "item_a": v.get("item_a").cloned().unwrap_or(Value::Null),
                "item_b": v.get("item_b").cloned().unwrap_or(Value::Null),
            })
        })
        .unwrap_or(Value::Null)
}

/// Project an active queue entry's landing-state-machine state from its resolved
/// union verdict (the ledger's reject-sticky / proven fold — the SAME source the
/// `verdict` field uses, so the two never disagree). Honest vocabulary mapped to
/// [`hugit_queue::core::EntryState`]:
///
/// - `"union_fail"` — the entry's union verdict is `reject` (a member intent has an
///   outstanding sticky reject): it is destined for the `UnionFail` terminal
///   (excluded from the batch). REAL — projected from the recorded verdict.
/// - `"landable"` — the entry's union verdict is `approve` (every member intent
///   proven): it can transition to `Landed` once its predecessors settle. The
///   predecessor-settled gate + the actual `Landed` settlement are the queue
///   engine's job (a `pr.landed` event), so this is "proven, eligible", not "landed".
/// - `"queued"` — the entry has no covering verdict yet (the verdict is `null`): it
///   sits in the queue awaiting its batch's union test. Honest unknown.
///
/// The terminal `landed` state is NEVER produced here — a landed PR has left the
/// active queue (`all_pr_queued` filters it), so it is observed by absence, not by
/// a state string. This keeps the projection honest: we never claim a settlement
/// the log did not record.
fn entry_state(verdict: Option<UnionVerdict>) -> Value {
    match verdict {
        Some(UnionVerdict::Reject) => Value::String("union_fail".to_string()),
        Some(UnionVerdict::Approve) => Value::String("landable".to_string()),
        None => Value::String("queued".to_string()),
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
    /// Path to the canonical JSON event log. Defaults to $HUGIT_LOG, else
    /// .hugit/log.json.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,
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
    let log_path = crate::log_resolve::resolve_log(args.log.clone());
    let log = load_event_log(&log_path)?;

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
            // The entry's position in the idempotent landing STATE MACHINE
            // (hugit_queue::core::EntryState), projected from what the log records.
            // Every entry here is in the ACTIVE queue (`all_pr_queued` excludes a
            // `pr.landed`-settled PR), so the terminal `landed` state is observed by
            // its ABSENCE from this list — see the top-level `state_machine_note`.
            "state": entry_state(entry_verdict),
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
                // The batch's landing-state-machine state, from its union verdict
                // (the same ledger fold). A union batch with an outstanding member
                // reject is heading for `union_fail`.
                "state": entry_state(verdict),
                "implicated_pr": implicated_pr,
                // The MINIMAL FAILING PAIR from the bisect (hugit_queue's reason to
                // exist: "exclude the failing pair, the rest proceeds"). REAL the
                // moment a `hugit land queue` batch land records a `queue.union_fail`
                // event carrying the bisected pair (the `land` module is the
                // recorder); `null` until a batch land has localised a failure for
                // this campaign — honest unknown, never a fabricated pair.
                "failing_pair": failing_pair_for(&log, Some(&campaign)),
            })
        })
        .collect();

    Ok(json!({
        "log": log_path.display().to_string(),
        "campaign": args.campaign,
        "queue_depth": entries.len(),
        "entries": entries,
        "batches": batches,
        "verdict_note": VERDICT_NOTE,
        "state_machine_note": STATE_MACHINE_NOTE,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::EventLog;

    /// `entry_state` maps the resolved union verdict to the landing state-machine
    /// vocabulary — the ONE place the state names are projected. Pure + total.
    #[test]
    fn entry_state_maps_verdict_to_landing_state_machine() {
        assert_eq!(entry_state(Some(UnionVerdict::Reject)), json!("union_fail"));
        assert_eq!(entry_state(Some(UnionVerdict::Approve)), json!("landable"));
        // No covering verdict ⇒ honest "queued" (awaiting the union test), never
        // a guessed landable/fail.
        assert_eq!(entry_state(None), json!("queued"));
    }

    /// Append a record onto a synthetic log via the `test-support` raw shim — the
    /// canonical fixture path (the production raw door is `pub(crate)`).
    fn push(log: &mut EventLog, kind: &str, payload: Value) {
        log.append_for_test(
            kind,
            vec!["orchestrator:hugit".to_string()],
            payload.to_string(),
            0,
        );
    }

    /// Persist a seeded log to a unique temp path and run `queue show` over it.
    fn show_over(log: &EventLog, tag: &str, campaign: Option<&str>) -> Value {
        let dir =
            std::env::temp_dir().join(format!("hugit-queue-show-{}-{tag}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("queue-seed.json");
        std::fs::write(&path, serde_json::to_vec_pretty(log.records()).unwrap()).unwrap();
        show(&ShowArgs {
            log: Some(path),
            campaign: campaign.map(str::to_string),
        })
        .unwrap()
    }

    /// A clean queue (PRs opened + queued, no verdict) projects each entry in
    /// `order_index` order, grouped into one union batch per campaign, with the
    /// entry/batch state `queued` and the failing-pair honestly null + disclosed.
    #[test]
    fn show_projects_union_batch_composition_and_queued_state() {
        let mut log = EventLog::new();
        // Two PRs in the same campaign — the union-batch composition under test.
        push(
            &mut log,
            "pr.opened",
            json!({"pr_id": "PR-1", "campaign": "camp-a", "intent_ids": ["I-1"],
                   "author_kind": "orchestrator", "principal": null, "run_id": null}),
        );
        push(
            &mut log,
            "pr.opened",
            json!({"pr_id": "PR-2", "campaign": "camp-a", "intent_ids": ["I-2"],
                   "author_kind": "orchestrator", "principal": null, "run_id": null}),
        );
        push(
            &mut log,
            "pr.queued",
            json!({"pr_id": "PR-1", "item_id": "PR-1#0", "order_index": 0, "mode": "union"}),
        );
        push(
            &mut log,
            "pr.queued",
            json!({"pr_id": "PR-2", "item_id": "PR-2#1", "order_index": 1, "mode": "union"}),
        );

        let out = show_over(&log, "clean", None);

        assert_eq!(out["queue_depth"], 2);
        let entries = out["entries"].as_array().unwrap();
        assert_eq!(entries[0]["pr_id"], "PR-1");
        assert_eq!(entries[0]["position"], 0);
        assert_eq!(entries[0]["state"], "queued");
        assert_eq!(entries[1]["pr_id"], "PR-2");

        // One union batch per campaign, composed of both member PRs.
        let batches = out["batches"].as_array().unwrap();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0]["campaign"], "camp-a");
        assert_eq!(batches[0]["mode"], "union");
        assert_eq!(batches[0]["members"], json!(["PR-1", "PR-2"]));
        assert_eq!(batches[0]["member_count"], 2);
        // The bisected failing pair is not log-recorded → honest null + disclosed.
        assert!(batches[0]["failing_pair"].is_null());
        assert!(
            out["state_machine_note"]
                .as_str()
                .unwrap()
                .contains("failing_pair")
        );
    }

    /// A union batch with an outstanding member REJECT surfaces `state:"union_fail"`
    /// and blames the implicated PR — projected from the SAME ledger reject the
    /// `verdict` uses, so the queue view and the verdict agree by construction.
    #[test]
    fn show_surfaces_union_fail_state_and_implicated_pr_on_a_reject() {
        let mut log = EventLog::new();
        // The ledger builds an entry from `intent.landed`, then attaches verdicts.
        push(
            &mut log,
            "intent.landed",
            json!({"intent_id": "I-9", "campaign": "camp-b"}),
        );
        push(
            &mut log,
            "pr.opened",
            json!({"pr_id": "PR-9", "campaign": "camp-b", "intent_ids": ["I-9"],
                   "author_kind": "orchestrator", "principal": null, "run_id": null}),
        );
        push(
            &mut log,
            "pr.queued",
            json!({"pr_id": "PR-9", "item_id": "PR-9#0", "order_index": 0, "mode": "union"}),
        );
        // A recorded reject on I-9's review lens — the ledger resolves it sticky.
        push(
            &mut log,
            "verdict.recorded",
            json!({
                "intent": "I-9", "tree_hash": "th", "lens": "sec", "model": "m",
                "prompt_digest": "pd", "verdict": "reject",
                "claims_checked": ["sec:reject"], "evidence_refs": [],
            }),
        );

        let out = show_over(&log, "reject", None);

        let entries = out["entries"].as_array().unwrap();
        assert_eq!(entries[0]["verdict"], "reject");
        assert_eq!(entries[0]["state"], "union_fail");

        let batches = out["batches"].as_array().unwrap();
        assert_eq!(batches[0]["verdict"], "reject");
        assert_eq!(batches[0]["state"], "union_fail");
        // Blame names the implicated member PR (real); the bisected PAIR is null.
        assert_eq!(batches[0]["implicated_pr"], "PR-9");
        assert!(batches[0]["failing_pair"].is_null());
    }
}
