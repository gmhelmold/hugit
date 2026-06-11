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
    // Lock BEFORE the load and hold it across the whole load→mutate→persist
    // (WF-CLI2 bug 2: the load→lock inversion). `open` bootstraps a missing
    // `--log` (an absent log is a legitimate fresh empty world), so it loads
    // with `bootstrap = true`.
    let (lock, world) = World::lock_and_load(&args.log, true)?;

    // Redaction parity (Wave E, P-REDACT-SURFACE): scrub the user-supplied
    // free-text fields through the hardened engine BEFORE they reach the
    // hash-chained `campaign.opened` payload (the log is append-only and
    // forever; redact-before-append is the only fix) and the success echo. The
    // campaign key is the stable identifier callers join on, so it is NOT
    // rewritten — but it is echoed scrubbed for read-surface parity.
    let charter = crate::redaction::scrub(&args.charter);
    let owner = crate::redaction::scrub(&args.owner);

    // Idempotent open: the key already has a `campaign.opened` record → exit 0,
    // no duplicate. The charter/owner of the existing record are authoritative;
    // we do not overwrite them.
    //
    // Stable key-set (P8): the re-run shape carries charter/owner (null when the
    // projection is unavailable, never absent) — the same keys as the first-run
    // shape.
    if world.campaign_opened(&args.campaign) {
        // Scrub the projected charter/owner on the way out (defence-in-depth:
        // a pre-Wave-E log may carry an unredacted opened record).
        let (charter, owner) = world
            .campaign_charter_owner(&args.campaign)
            .map(|(c, o)| {
                (
                    serde_json::Value::String(crate::redaction::scrub(&c)),
                    serde_json::Value::String(crate::redaction::scrub(&o)),
                )
            })
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
        "charter": charter,
        "owner": owner,
    })
    .to_string();

    // D14 guarded append: campaign.opened is a human-owned mutation; route
    // through append_authorized so the matrix gates it and denials are audited.
    append_authorized_and_persist(
        &lock,
        &world,
        &args.log,
        KIND_CAMPAIGN_OPENED,
        &owner,
        payload,
        0,
    )?;

    Ok(json!({
        "campaign": args.campaign,
        "opened": true,
        "already_exists": false,
        "owner": owner,
        "charter": charter,
    })
    .to_string())
}
