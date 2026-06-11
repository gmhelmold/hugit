//! `hugit campaign open` (WP-PC1) — record `campaign.opened`, idempotently.
//!
//! Appends a `campaign.opened` record (charter + human owner — D14) to the
//! local event log and writes it back. **Idempotent**: re-opening the same key
//! returns the existing record (`"already_exists":true`, exit 0) and appends no
//! duplicate, so an agent can retry `open` safely.

use serde_json::json;

use super::OpenArgs;
use super::output::CampaignError;
use super::world::{KIND_CAMPAIGN_OPENED, World, append_authorized_and_persist};

pub fn run(args: OpenArgs) -> Result<String, CampaignError> {
    let world = World::load(&args.log)?;

    // Idempotent open: the key already has a `campaign.opened` record → exit 0,
    // no duplicate. The charter/owner of the existing record are authoritative;
    // we do not overwrite them.
    //
    // Stable key-set (P8): the re-run shape carries charter/owner (null when the
    // projection is unavailable, never absent) — the same keys as the first-run
    // shape.
    if world.campaign_opened(&args.campaign) {
        let (charter, owner) = world
            .campaign_charter_owner(&args.campaign)
            .map(|(c, o)| (serde_json::Value::String(c), serde_json::Value::String(o)))
            .unwrap_or((serde_json::Value::Null, serde_json::Value::Null));
        return Ok(json!({
            "campaign": args.campaign,
            "opened": true,
            "already_exists": true,
            "owner": owner,
            "charter": charter,
        })
        .to_string());
    }

    // The opened payload — canonical JSON is computed by the append path.
    let payload = json!({
        "campaign": args.campaign,
        "charter": args.charter,
        "owner": args.owner,
    })
    .to_string();

    // D14 guarded append: campaign.opened is a human-owned mutation; route
    // through append_authorized so the matrix gates it and denials are audited.
    append_authorized_and_persist(
        &world,
        &args.log,
        KIND_CAMPAIGN_OPENED,
        &args.owner,
        payload,
        0,
    )?;

    Ok(json!({
        "campaign": args.campaign,
        "opened": true,
        "already_exists": false,
        "owner": args.owner,
        "charter": args.charter,
    })
    .to_string())
}
