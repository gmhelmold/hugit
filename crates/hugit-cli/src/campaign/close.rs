//! `hugit campaign close` (WP-PC1) — the SEAL.
//!
//! NOT a CI trigger (the union-test fires per-PR at landing, continuously).
//! `close` is the seal: the final whole-bundle proof + Ledger "provado" + the
//! F3 cost rollup + envelope sealing.
//!
//! - **Refuses** if any PR of the campaign is still in-flight in the queue — a
//!   structured error listing them, fix "land or abandon first". The seal must
//!   not close over unsettled work.
//! - On success appends `campaign.closed` and prints the F3 `campaign_rollup`
//!   JSON (cost decomposition + progress).
//! - **Envelope seal**: prints `"envelope":"not_captured"` honestly unless a
//!   campaign envelope ref is available in the world (F2b emits it in dogfood
//!   waves — if present, the ref is included).
//! - **Idempotent**: closing an already-closed campaign returns exit 0,
//!   `"already_closed":true`, and appends no duplicate.

use serde_json::{Value, json};

use super::CloseArgs;
use super::output::CampaignError;
use super::show::{pr_list, progress_counts};
use super::world::{KIND_CAMPAIGN_CLOSED, World, append_authorized_and_persist};

pub fn run(args: CloseArgs) -> Result<String, CampaignError> {
    let world = World::load(&args.log)?;
    let key = &args.campaign;

    let phases = world.pr_phases(key);

    // Idempotent close: already sealed → exit 0, no duplicate record.
    // Stable key-set (P8): the re-run shape carries all the same keys as the
    // first-run shape — null over absent where projection is not available.
    if world.campaign_closed(key) {
        let rollup = world.build_rollup(key, &phases)?;
        let rollup_json = match &rollup {
            Some(r) => serde_json::to_value(r).map_err(|e| {
                CampaignError::new(
                    "serialize",
                    format!("rollup did not serialise: {e}"),
                    "internal error — report it",
                )
            })?,
            None => Value::Null,
        };
        let envelope = world
            .campaign_envelope_ref(key)
            .unwrap_or_else(|| "not_captured".to_string());
        return Ok(json!({
            "campaign": key,
            "closed": true,
            "already_closed": true,
            "envelope": envelope,
            "progress": progress_counts(&phases),
            "prs": pr_list(&phases),
            "ledger": {
                "done": world.ledger.done(key),
                "proven": world.ledger.proven(key),
            },
            "rollup": rollup_json,
        })
        .to_string());
    }

    // Refuse to seal over unsettled work: any in-flight PR blocks the close
    // (unless the campaign is abandoned — abandon releases the blocking
    // constraint, as its own close-semantic).
    let in_flight = world.in_flight_prs(key);
    if !in_flight.is_empty() && !world.campaign_abandoned(key) {
        return Err(CampaignError::new(
            "in_flight_prs",
            format!(
                "campaign '{key}' has {} PR(s) still in-flight in the landing queue",
                in_flight.len()
            ),
            "land or abandon first",
        )
        .with_detail(json!({ "in_flight": in_flight })));
    }

    // The F3 rollup — the seal's cost report (real projection; None when the
    // campaign envelope is not captured, sealed progress-only then).
    let rollup = world.build_rollup(key, &phases)?;
    let rollup_json = match &rollup {
        Some(r) => serde_json::to_value(r).map_err(|e| {
            CampaignError::new(
                "serialize",
                format!("rollup did not serialise: {e}"),
                "internal error — report it",
            )
        })?,
        None => Value::Null,
    };

    // Envelope seal: honest unless a campaign envelope ref is captured on the log.
    let campaign_envelope_ref = world.campaign_envelope_ref(key);
    let envelope = campaign_envelope_ref
        .clone()
        .unwrap_or_else(|| "not_captured".to_string());

    // Append the seal record before printing (the seal must be on the log).
    // D14 guarded: close is a human-owned mutation; use the campaign owner for
    // the principal chain. Fall back to the campaign key when no opened record
    // is present (log-less campaigns are honest about missing context).
    let owner = world
        .campaign_charter_owner(key)
        .map(|(_, o)| o)
        .unwrap_or_else(|| key.to_string());
    let payload = json!({
        "campaign": key,
        "envelope_ref": campaign_envelope_ref,
    })
    .to_string();
    append_authorized_and_persist(&world, &args.log, KIND_CAMPAIGN_CLOSED, &owner, payload, 0)?;

    Ok(json!({
        "campaign": key,
        "closed": true,
        "already_closed": false,
        "envelope": envelope,
        "progress": progress_counts(&phases),
        "prs": pr_list(&phases),
        "ledger": {
            "done": world.ledger.done(key),
            "proven": world.ledger.proven(key),
        },
        "rollup": rollup_json,
    })
    .to_string())
}
