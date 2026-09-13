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
    let log_path = crate::log_resolve::resolve_log(args.log.clone())
        .map_err(|error| CampaignError::new(error.kind(), error.message(), error.fix()))?;
    // Read-only query: a MISSING --log is an explicit `log_not_found` (exit-2),
    // never a silent empty world (P-CAMPAIGN-EMPTY).
    let world = World::load_existing(&log_path)?;
    let keys = world.all_campaign_keys();

    let campaigns: Vec<_> = keys
        .iter()
        .map(|key| {
            // Redaction parity (Wave E, P-REDACT-SURFACE): scrub the echoed
            // free-text fields through the hardened engine (defence-in-depth
            // over the write-path redaction; also covers a pre-Wave-E log).
            let (charter, owner) = world
                .campaign_charter_owner(key)
                .map(|(c, o)| {
                    (
                        serde_json::Value::String(crate::redaction::scrub(&c)),
                        serde_json::Value::String(crate::redaction::scrub(&o)),
                    )
                })
                .unwrap_or((serde_json::Value::Null, serde_json::Value::Null));

            let phases = world.pr_phases(key);
            let progress = progress_counts(&phases);
            let abandoned = world.campaign_abandoned(key);
            let reason = world
                .campaign_abandon_reason(key)
                .map(|r| serde_json::Value::String(crate::redaction::scrub(&r)))
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
