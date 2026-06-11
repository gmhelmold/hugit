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
    // Lock BEFORE the load and hold it across the whole load→mutate→persist
    // (WF-CLI2 bug 2). `close` is a read-must-exist mutation: a missing `--log`
    // is `log_not_found`/exit-2, never a bootstrapped empty world that would let
    // a `campaign.closed` ghost-record be created from nothing for a campaign
    // that never had a `campaign.opened` (WF-CLI2 bug 1) — so it loads with
    // `bootstrap = false`.
    let (lock, world) = World::lock_and_load(&args.log, false)?;
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
                "rejected": world.ledger.rejected(key),
            },
            "rollup": rollup_json,
        })
        .to_string());
    }

    // Must be opened before it can be closed (WF-CLI2 bug 1, the ghost-record
    // guard — the same guard `abandon` already carries). A campaign with no
    // `campaign.opened` record on the log is unknown: closing it would seal a
    // campaign that never existed (a `campaign.closed` from nothing). Refuse
    // with a structured `not_opened`/exit-2 rather than fabricating the seal.
    // (An already-closed campaign was necessarily opened, so this guard sits
    // after the idempotent-close fast path above.)
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
        // Uniformity: fold the in-flight list FLAT under `error` (never `detail`)
        // — one parser (`error.<key>`) reads context across every porcelain verb.
        .with_context("in_flight", json!(in_flight)));
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
    // the principal chain. The `not_opened` guard above guarantees an opened
    // record exists, so `campaign_charter_owner` resolves; the `key` fallback is
    // a defence-in-depth no-op (a malformed opened record with no owner field).
    let owner = world
        .campaign_charter_owner(key)
        .map(|(_, o)| o)
        .unwrap_or_else(|| key.to_string());
    let payload = json!({
        "campaign": key,
        "envelope_ref": campaign_envelope_ref,
    })
    .to_string();
    append_authorized_and_persist(
        &lock,
        &world,
        &args.log,
        KIND_CAMPAIGN_CLOSED,
        &owner,
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
            "rejected": world.ledger.rejected(key),
        },
        "rollup": rollup_json,
    })
    .to_string())
}
