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
    // Lock BEFORE the load and hold it across the whole load→mutate→persist
    // (WF-CLI2 bug 2). `abandon` is a read-must-exist mutation: a missing `--log`
    // is `log_not_found`/exit-2, never a bootstrapped empty world that would let
    // a `campaign.abandoned` ghost-record be created from nothing (WF-CLI2 bug 1)
    // — so it loads with `bootstrap = false`.
    let (lock, world) = World::lock_and_load(&args.log, false)?;
    let key = &args.campaign;

    // Redaction parity (Wave E, P-REDACT-SURFACE): scrub the free-text reason
    // through the hardened engine BEFORE it reaches the hash-chained
    // `campaign.abandoned` payload and the echo.
    let reason = crate::redaction::scrub(&args.reason);

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
            // Scrub the projected reason on the way out (defence-in-depth).
            "reason": world
                .campaign_abandon_reason(key)
                .map(|r| crate::redaction::scrub(&r)),
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

    // Derive the owner from the opened record for the D14 principal chain
    // (scrubbed — it feeds the principal chain, which `intent show` echoes).
    let owner = crate::redaction::scrub(
        &world
            .campaign_charter_owner(key)
            .map(|(_, o)| o)
            .unwrap_or_else(|| key.to_string()),
    );

    let payload = json!({
        "campaign": key,
        "reason": reason,
    })
    .to_string();

    // D14 guarded append: abandon is a human-owned mutation.
    append_authorized_and_persist(
        &lock,
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
        "reason": reason,
    })
    .to_string())
}
