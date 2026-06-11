//! `hugit campaign list` (WP-WB-CAMP) — enumerate all campaigns on the log.
//!
//! Projects every campaign from the local event log:
//! - key, charter, owner (from `campaign.opened`)
//! - opened/closed/abandoned state
//! - progress counts (landed / in-flight / blocked PR counts)
//!
//! **Stable order**: campaigns are emitted sorted by key (BTreeSet insertion
//! order in [`World::all_campaign_keys`]) — identical across runs regardless of
//! event log insertion order.
//!
//! Read-only: `list` never writes.

use serde_json::json;

use super::ListArgs;
use super::output::CampaignError;
use super::show::progress_counts;
use super::world::World;

pub fn run(args: ListArgs) -> Result<String, CampaignError> {
    let world = World::load(&args.log)?;
    let keys = world.all_campaign_keys();

    let campaigns: Vec<_> = keys
        .iter()
        .map(|key| {
            let (charter, owner) = world
                .campaign_charter_owner(key)
                .map(|(c, o)| (serde_json::Value::String(c), serde_json::Value::String(o)))
                .unwrap_or((serde_json::Value::Null, serde_json::Value::Null));

            let phases = world.pr_phases(key);
            let progress = progress_counts(&phases);
            let abandoned = world.campaign_abandoned(key);
            let reason = world
                .campaign_abandon_reason(key)
                .map(serde_json::Value::String)
                .unwrap_or(serde_json::Value::Null);

            json!({
                "campaign": key,
                "charter": charter,
                "owner": owner,
                "opened": world.campaign_opened(key),
                "closed": world.campaign_closed(key),
                "abandoned": abandoned,
                "abandon_reason": reason,
                "progress": progress,
            })
        })
        .collect();

    Ok(json!({
        "campaigns": campaigns,
        "count": campaigns.len(),
    })
    .to_string())
}
