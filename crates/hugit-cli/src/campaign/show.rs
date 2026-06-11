//! `hugit campaign show` (WP-PC1) — progress projection (read-only).
//!
//! Projects the campaign's PR phases (landed / in-flight / blocked) off the
//! event log, the asked→done→proven Ledger figures, and — when the campaign
//! envelope is captured — the F3 rollup summary. Never writes.

use serde_json::{Value, json};

use super::ShowArgs;
use super::output::CampaignError;
use super::world::World;
use hugit_ledger::rollup::PrPhase;

pub fn run(args: ShowArgs) -> Result<String, CampaignError> {
    // Read-only query: a MISSING --log is an explicit `log_not_found` (exit-2),
    // never a silent empty world (P-CAMPAIGN-EMPTY).
    let world = World::load_existing(&args.log)?;
    let key = &args.campaign;

    let phases = world.pr_phases(key);
    let progress = progress_counts(&phases);
    let prs = pr_list(&phases);

    // Asked→done→proven from the Ledger projection (the same view `hugit ledger`
    // surfaces). `asked == done` (landing IS done); `proven` = verdict-recorded.
    let ledger = json!({
        "done": world.ledger.done(key),
        "proven": world.ledger.proven(key),
    });

    let rollup_summary = match world.build_rollup(key, &phases)? {
        Some(r) => rollup_summary(&r),
        None => Value::Null,
    };

    Ok(json!({
        "campaign": key,
        "opened": world.campaign_opened(key),
        "closed": world.campaign_closed(key),
        "progress": progress,
        "prs": prs,
        "ledger": ledger,
        "rollup": rollup_summary,
    })
    .to_string())
}

/// Count PRs per phase.
pub fn progress_counts(phases: &[(String, PrPhase)]) -> Value {
    let mut landed = 0;
    let mut in_flight = 0;
    let mut blocked = 0;
    for (_, p) in phases {
        match p {
            PrPhase::Landed => landed += 1,
            PrPhase::InFlight => in_flight += 1,
            PrPhase::Blocked => blocked += 1,
        }
    }
    json!({ "landed": landed, "in_flight": in_flight, "blocked": blocked })
}

/// The PR list with each PR's phase (sorted by pr_id — `pr_phases` is sorted).
pub fn pr_list(phases: &[(String, PrPhase)]) -> Value {
    Value::Array(
        phases
            .iter()
            .map(|(id, p)| json!({ "pr_id": id, "phase": phase_str(*p) }))
            .collect(),
    )
}

fn phase_str(p: PrPhase) -> &'static str {
    match p {
        PrPhase::Landed => "landed",
        PrPhase::InFlight => "in_flight",
        PrPhase::Blocked => "blocked",
    }
}

/// A compact summary of the F3 campaign rollup (the full rollup is the close
/// seal; `show` surfaces the headline figures).
pub fn rollup_summary(r: &hugit_contracts::context_envelope::CampaignRollup) -> Value {
    json!({
        "pr_count": r.pr_count,
        "intent_count": r.intent_count,
        "agent_count": r.agent_count,
        "cost_usd_micros": r.cost.total.cost_usd_micros,
        "tokens": r.cost.total.tokens,
        "waste_cost_usd_micros": r.cost.waste.cost_usd_micros,
        "first_pass_yield": r.efficiency.first_pass_yield,
    })
}
