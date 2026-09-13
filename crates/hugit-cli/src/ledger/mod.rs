//! `hugit ledger` — the default forge history view (Phase D, graduated from
//! RESERVED with REAL wiring — not a stub).
//!
//! Projects the canonical event log into the asked→done→proven ledger — the
//! human/agent-facing read surface over the forge event stream. REUSES the
//! engine's own projection [`hugit_ledger::Ledger`] (the SAME reject-sticky
//! per-intent fold `campaign show` and `queue show` read), so the ledger view
//! AGREES with every other surface by construction — one projection, never a
//! second source of truth.
//!
//! `hugit ledger --log <path> [--campaign <name>]` emits stable JSON: a
//! per-campaign rollup (`asked` / `done` / `proven` / `rejected`) + the ledger
//! entries (each redacted at the view boundary by the projection). With
//! `--campaign`, the projection is scoped to one campaign. On a log with no
//! `intent.landed` records the rollup is empty and `entries: []` with a
//! disclosing `note` — honest-null, never a fabricated history.
//!
//! `--live` (the SSE tail against the running engine) is the serve surface, not
//! the static `--log` read — deferred to the `hugit-serve` ledger stream; the
//! static projection is the complete, source-of-truth read.
//!
//! Every output is stable JSON on stdout under the WB0 one-error/one-exit law
//! ([`crate::porcelain`]): `log_not_found` / `parse_log` are the canonical
//! `{"error":{…}}` envelopes, exit `2`.

use std::path::PathBuf;
use std::process::ExitCode;

use serde_json::{Value, json};

use hugit_ledger::Ledger;

use crate::checks::load_event_log;
use crate::porcelain::PorcelainError;

/// Disclosure attached to an empty ledger: no `intent.landed` event covers the
/// (scoped) view yet, so the history is honestly empty — never a fabricated row.
const EMPTY_NOTE: &str = "no intent.landed event covers this view yet — the history is \
                          honestly empty, never fabricated; entries appear as intents \
                          land through the canonical append path";

/// `hugit ledger` — the default forge history view (Phase D).
#[derive(clap::Args, Debug)]
pub struct LedgerArgs {
    /// Path to the canonical JSON event log. Defaults to $HUGIT_LOG, else
    /// .hugit/log.json.
    #[arg(long, help = crate::log_resolve::LOG_FLAG_HELP)]
    pub log: Option<PathBuf>,
    /// Optional campaign key: scope the projection to one campaign's history.
    #[arg(long)]
    pub campaign: Option<String>,
}

/// Dispatch `hugit ledger`, emitting stable JSON on stdout and returning the
/// process exit code under the WB0 one-exit-code law.
pub fn run(args: LedgerArgs) -> ExitCode {
    match project(&args) {
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

/// Project the ledger from the canonical log.
///
/// The chain is verified on read (`load_event_log`), the projection redacts
/// every surfaced string at the view boundary, and the per-campaign counts come
/// straight from [`Ledger`]'s own folds — so this verb is a pure presentation of
/// the engine's authority, with no second source of truth.
fn project(args: &LedgerArgs) -> Result<Value, PorcelainError> {
    let log_path = crate::log_resolve::resolve_log(args.log.clone())?;
    let log = load_event_log(&log_path)?;
    let ledger = Ledger::from_records(log.records());

    // Entries, optionally scoped to one campaign. The projection already
    // redacted every surfaced string at the view boundary.
    let entries: Vec<&hugit_ledger::LedgerEntry> = match &args.campaign {
        Some(c) => ledger.by_campaign(c).collect(),
        None => ledger.entries().iter().collect(),
    };

    // Per-campaign rollup, in a stable (sorted, deduped) order so the output is
    // deterministic regardless of log arrival order.
    let mut campaigns: Vec<String> = entries.iter().map(|e| e.campaign.clone()).collect();
    campaigns.sort();
    campaigns.dedup();

    let rollup: Vec<Value> = campaigns
        .iter()
        .map(|c| {
            json!({
                "campaign": c,
                "asked": ledger.asked(c),
                "done": ledger.done(c),
                "proven": ledger.proven(c),
                "rejected": ledger.rejected(c),
            })
        })
        .collect();

    let entries_json: Vec<Value> = entries
        .iter()
        .map(|e| {
            serde_json::to_value(e)
                .map_err(|err| PorcelainError::internal(format!("serialise ledger entry: {err}")))
        })
        .collect::<Result<_, _>>()?;

    let mut out = json!({
        "campaigns": rollup,
        "entries": entries_json,
    });
    if entries.is_empty() {
        out.as_object_mut()
            .expect("json! built an object")
            .insert("note".to_string(), json!(EMPTY_NOTE));
    }
    Ok(out)
}
