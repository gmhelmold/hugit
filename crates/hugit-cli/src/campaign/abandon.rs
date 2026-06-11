//! `hugit campaign abandon` (WP-WB-CAMP) — mark a campaign abandoned.
//!
//! Appends a `campaign.abandoned` record to the local event log and writes it
//! back. Semantics:
//!
//! - **Idempotent**: re-abandoning the same campaign is a no-op (exit 0,
//!   `"already_abandoned":true`). An agent may retry safely.
//! - **Releases in-flight PRs from blocking close semantics**: once abandoned,
//!   `campaign close` no longer refuses because of in-flight PRs. The in-flight
//!   PRs are not automatically settled — they remain in-flight on the log — but
//!   the seal no longer waits for them (the campaign is done for a different
//!   reason than landing everything).
//! - **Abandoning a closed campaign is a structured error** (`already_closed`):
//!   the SEAL is final; you cannot abandon after the proof is issued.
//! - **D14 guarded**: appended through [`append_authorized_and_persist`] so the
//!   mutation path carries the human principal.

use serde_json::json;

use super::AbandonArgs;
use super::output::CampaignError;
use super::world::{KIND_CAMPAIGN_ABANDONED, World, append_authorized_and_persist};

pub fn run(args: AbandonArgs) -> Result<String, CampaignError> {
    let world = World::load(&args.log)?;
    let key = &args.campaign;

    // Abandoning a closed campaign is an error: the SEAL is final.
    if world.campaign_closed(key) {
        return Err(CampaignError::new(
            "already_closed",
            format!(
                "campaign '{key}' is already closed (sealed); \
                 a sealed campaign cannot be abandoned"
            ),
            "the SEAL is final — no further lifecycle mutations are accepted",
        ));
    }

    // Idempotent abandon: already abandoned → exit 0, no duplicate record.
    if world.campaign_abandoned(key) {
        return Ok(json!({
            "campaign": key,
            "abandoned": true,
            "already_abandoned": true,
            "reason": world.campaign_abandon_reason(key),
        })
        .to_string());
    }

    // Must be opened before it can be abandoned (a campaign with no
    // `campaign.opened` record is unknown — guard against stale key typos).
    if !world.campaign_opened(key) {
        return Err(CampaignError::new(
            "not_opened",
            format!(
                "campaign '{key}' has no campaign.opened record on the log; \
                 open it first"
            ),
            "run `hugit campaign open --campaign <key> …` to open the campaign",
        ));
    }

    // Derive the owner from the opened record for the D14 principal chain.
    let owner = world
        .campaign_charter_owner(key)
        .map(|(_, o)| o)
        .unwrap_or_else(|| key.to_string());

    let payload = json!({
        "campaign": key,
        "reason": args.reason,
    })
    .to_string();

    // D14 guarded append: abandon is a human-owned mutation.
    append_authorized_and_persist(
        &world,
        &args.log,
        KIND_CAMPAIGN_ABANDONED,
        &owner,
        payload,
        0,
    )?;

    Ok(json!({
        "campaign": key,
        "abandoned": true,
        "already_abandoned": false,
        "reason": args.reason,
    })
    .to_string())
}
