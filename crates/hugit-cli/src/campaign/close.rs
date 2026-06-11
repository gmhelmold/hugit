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
use super::world::{KIND_CAMPAIGN_CLOSED, World, append_and_persist};

pub fn run(args: CloseArgs) -> Result<String, CampaignError> {
    let world = World::load(&args.log)?;
    let key = &args.campaign;

    // Idempotent close: already sealed → exit 0, no duplicate record.
    if world.campaign_closed(key) {
        return Ok(json!({
            "campaign": key,
            "closed": true,
            "already_closed": true,
        })
        .to_string());
    }

    let phases = world.pr_phases(key);

    // Refuse to seal over unsettled work: any in-flight PR blocks the close.
    let in_flight = world.in_flight_prs(key);
    if !in_flight.is_empty() {
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
    let payload = json!({
        "campaign": key,
        "envelope_ref": campaign_envelope_ref,
    })
    .to_string();
    append_and_persist(
        &world,
        &args.log,
        KIND_CAMPAIGN_CLOSED,
        vec![format!("campaign:{key}")],
        payload,
        0,
    )?;

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
