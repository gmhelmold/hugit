//! `hugit intent list` — the agent discovery verb (WP-WB-INT).
//!
//! An agent that has lost an intent id can recover it here.  `list` reads the
//! local store, resolves `landed` state against the shared canonical log when
//! `--log` is given, and returns a stable JSON object:
//!
//! ```json
//! {"intents":[
//!   {"id":"intent-abc","charter":"Add the JSON …","campaign":"cli-porcelain",
//!    "agent":"main","landed":true},
//!   …
//! ]}
//! ```
//!
//! ## Field spec (stable wire contract)
//!
//! - `id` — the intent id (`intent_id` string, stable key)
//! - `charter` — first 80 chars of the charter (excerpt; enough to identify)
//! - `campaign` — the campaign key from the principal chain (`campaign:<key>` entry)
//! - `agent` — the agent token from the principal chain (`agent:<id>` entry),
//!   or `null` when not recorded
//! - `landed` — `true` when the `--log` is given AND the id appears on it;
//!   `null` when `--log` is omitted (i.e. the log was not consulted)
//!
//! Items are sorted by `id` (lexicographic) — stable order, so an agent can
//! diff two consecutive list calls without noise.
//!
//! ## Campaign filter
//!
//! `--campaign <key>` restricts the output to intents bound to that campaign
//! (matched against the principal chain `campaign:` entry).

use std::path::{Path, PathBuf};

use hugit_refstore::intent::intents_from_log;

use super::error::PorcelainError;
use super::store::IntentStore;

/// Inputs for `intent list`.
pub struct ListIntents {
    /// Optional canonical event log (to resolve `landed` state for each intent).
    pub log: Option<PathBuf>,
    /// Optional campaign key filter (none = all campaigns).
    pub campaign: Option<String>,
}

/// One item in the `{"intents":[…]}` list.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct IntentListItem {
    /// The stable intent id.
    pub id: String,
    /// First 80 chars of the charter (excerpt for identification).
    pub charter: String,
    /// The campaign key from the principal chain.
    pub campaign: String,
    /// The agent token from the principal chain, or null when not recorded.
    pub agent: Option<String>,
    /// `true`/`false` when a `--log` was consulted; `null` when `--log` omitted.
    pub landed: Option<bool>,
}

/// The stable result of `intent list`.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ListResult {
    /// The enumerated intents, sorted by id.
    pub intents: Vec<IntentListItem>,
}

/// Enumerate all intents in the store, optionally resolving landed state.
///
/// A MISSING `--store` is an explicit `store_not_found`/exit-2 (via
/// [`IntentStore::load_existing`]) — matching the sibling read-only contract
/// (`intent show`, and the `--log` reads' `log_not_found`), never a silent
/// empty list/exit-0 (the divergence WF flagged).
pub fn run(input: ListIntents, store_path: &Path) -> Result<ListResult, PorcelainError> {
    let store = IntentStore::load_existing(store_path).map_err(PorcelainError::from_store)?;

    // Projection of the real event log for landed-state resolution.
    let landed_ids: Option<std::collections::HashSet<String>> =
        input.log.as_deref().map(resolve_landed).transpose()?;

    // Gather all intents from the log, sorted by id for stable output.
    let all_intents = store.intent_for_all().map_err(PorcelainError::from_store)?;

    let mut items: Vec<IntentListItem> = all_intents
        .into_iter()
        .filter_map(|intent| {
            // Extract campaign + agent from the principal chain.
            let campaign = extract_principal(&intent.principal_chain, "campaign");
            let agent = extract_principal(&intent.principal_chain, "agent");

            // Campaign filter: skip when --campaign given and this one doesn't match.
            if input
                .campaign
                .as_deref()
                .is_some_and(|filter| campaign.as_deref() != Some(filter))
            {
                return None;
            }

            let landed = landed_ids
                .as_ref()
                .map(|ids| ids.contains(&intent.intent_id));

            // Redaction parity (Wave E, P-REDACT-SURFACE): scrub the echoed
            // free-text fields through the hardened engine on the way OUT
            // (defence-in-depth over the write-path redaction; also covers a
            // pre-Wave-E log). Scrub the full charter THEN excerpt, so a redacted
            // charter shows the sentinel, never an 80-char secret prefix.
            Some(IntentListItem {
                id: intent.intent_id.clone(),
                charter: charter_excerpt(&crate::redaction::scrub(&intent.charter)),
                campaign: crate::redaction::scrub(&campaign.unwrap_or_default()),
                agent: agent.map(|a| crate::redaction::scrub(&a)),
                landed,
            })
        })
        .collect();

    // Stable sort by id — deterministic across runs.
    items.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(ListResult { intents: items })
}

/// Extract the value of the FIRST `<prefix>:<value>` entry in the chain,
/// returning `None` when not found.
fn extract_principal(chain: &[String], prefix: &str) -> Option<String> {
    chain
        .iter()
        .find_map(|p| p.strip_prefix(&format!("{prefix}:")).map(str::to_string))
}

/// First 80 chars of a charter string (UTF-8 character boundary safe).
fn charter_excerpt(charter: &str) -> String {
    let mut end = charter.len().min(80);
    while end > 0 && !charter.is_char_boundary(end) {
        end -= 1;
    }
    charter[..end].to_string()
}

/// Load the canonical log at `path` and return the set of landed intent ids.
///
/// Routes through the engine's [`hugit_refstore::verify_chain`] — the same
/// primitive every sibling read-path (checks/pr/campaign/verdict/queue) calls
/// — so the hash chain is ALWAYS verified before projecting landed state.
/// A tampered chain propagates as `chain_broken`/exit-2, never silently read.
///
/// A *missing* file is treated as zero landed intents (the log has not been
/// created yet; `--log` is optional for `intent list`).  Every other fault
/// (tampered chain, malformed JSON, I/O error) is re-raised as exit-2.
fn resolve_landed(log_path: &Path) -> Result<std::collections::HashSet<String>, PorcelainError> {
    let bytes = match std::fs::read(log_path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // A missing log means no intents are landed; not an error for list.
            return Ok(std::collections::HashSet::new());
        }
        Err(e) => {
            return Err(PorcelainError::new(
                "io",
                format!("read log {}: {e}", log_path.display()),
                "check the --log path exists and is readable",
            ));
        }
    };

    let records: Vec<hugit_contracts::event_record::EventRecord> = serde_json::from_slice(&bytes)
        .map_err(|e| {
        PorcelainError::new(
            "parse_log",
            format!("parse log {}: {e}", log_path.display()),
            "the --log file must be a canonical JSON [EventRecord, …] array",
        )
    })?;

    // PS-13 (was the L-B fix): rehydrate + verify the chain through the SINGLE
    // chokepoint (`checks::rehydrate_and_verify`) every read-path shares — never
    // re-implementing the `EventLog::new() + push_record + verify_chain` loop
    // here, which is how this verb shipped the R8 hole in the first place. A
    // tampered chain is `chain_broken`/exit-2, never projected as authoritative
    // landed state.
    let log = crate::checks::rehydrate_and_verify(records).map_err(|fault| match fault {
        crate::checks::ChainLoadFault::Rehydrate(e) => PorcelainError::new(
            "rehydrate",
            format!("rehydrate log {}: {e}", log_path.display()),
            "the --log file's records must form a gap-free, monotonic chain",
        ),
        crate::checks::ChainLoadFault::ChainBroken(e) => PorcelainError::new(
            "chain_broken",
            format!(
                "log {} failed integrity verification: {e}",
                log_path.display()
            ),
            "the --log file's hash chain is tampered or corrupt",
        ),
    })?;

    let ids = intents_from_log(&log)
        .map(|il| il.intents().iter().map(|i| i.intent_id.clone()).collect())
        .unwrap_or_default();

    Ok(ids)
}
