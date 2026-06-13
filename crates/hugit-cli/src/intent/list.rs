//! `hugit intent list` — the agent discovery verb (WP-WB-INT).
//!
//! An agent that has lost an intent id can recover it here.  `list` reads the
//! local store and resolves each intent's `landed` state against ITS OWN owning
//! log (PS-9 — the log it was authored against, recorded at `intent new --log`),
//! returning a stable JSON object:
//!
//! ```json
//! {"intents":[
//!   {"id":"intent-abc","charter":"Add the JSON …","campaign":"cli-porcelain",
//!    "agent":"main","landed":true,"log":"/abs/path/to/agent-a.log.json"},
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
//! - `landed` — resolved against the intent's OWN owning `log` (below):
//!   `true`/`false` when that log is recorded and present, `null` when no source
//!   log was recorded (authored without `--log`) or it no longer exists. A
//!   *tampered* source log fails the whole call closed (`chain_broken`/exit-2).
//! - `log` — the canonical owning log the intent was authored against, or `null`.
//!   A fleet orchestrator sees each intent's TRUE landed state AND which log owns
//!   it in ONE global `intent list` call — no more `landed:null` for every
//!   cross-log intent.
//!
//! Items are sorted by `id` (lexicographic) — stable order, so an agent can
//! diff two consecutive list calls without noise.
//!
//! ## Filters
//!
//! - `--campaign <key>` restricts the output to intents bound to that campaign
//!   (matched against the principal chain `campaign:` entry).
//! - `--log <path>` is a SCOPE FILTER (PS-9): restrict the listing to intents
//!   authored against that exact log (canonical-path match). Omit it for the
//!   global, all-logs fleet view. `landed` is resolved per-intent regardless.

use std::path::{Path, PathBuf};

use hugit_refstore::intent::intents_from_log;

use super::error::PorcelainError;
use super::store::IntentStore;

/// Inputs for `intent list`.
pub struct ListIntents {
    /// Optional **source-log scope filter** (PS-9): when given, restrict the
    /// listing to intents that were authored against this exact log (matched by
    /// canonical path). Omit it for the global, all-logs fleet view. (Note: this
    /// is a FILTER — `landed` is always resolved per-intent against each intent's
    /// OWN owning log, regardless of this flag.)
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
    /// `landed` resolved against this intent's OWN owning log (PS-9): `true`/`false`
    /// when the intent has a recorded source log that exists and verifies; `null`
    /// when no source log was recorded (authored without `--log`) or that log no
    /// longer exists. (A *tampered* source log fails the whole call closed —
    /// `chain_broken`/exit-2 — never silently `null`.)
    pub landed: Option<bool>,
    /// The canonical log this intent was authored against (PS-9), or `null` when
    /// authored without `--log`. Lets a fleet orchestrator see WHICH log owns each
    /// intent in a single global `intent list` call.
    pub log: Option<String>,
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

    // Gather all intents from the store, sorted by id below for stable output.
    let all_intents = store.intent_for_all().map_err(PorcelainError::from_store)?;

    // Canonical key for the optional `--log` SCOPE FILTER — the same canonical
    // form `intent new` recorded as the source log. Canonicalize fails on a
    // non-existent path → fall back to the lexical path so a filter still compares
    // sensibly against a not-yet-created log.
    let filter_key: Option<String> = input.log.as_deref().map(canonical_key);

    // Landed-id sets memoized per DISTINCT source log actually consulted, so N
    // intents over M logs do M reads, not N. A tampered/corrupt source log fails
    // CLOSED here (`chain_broken`/exit-2) — never projected as authoritative
    // landed state (the read-path invariant the whole CLI shares).
    let mut landed_cache: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();

    let mut items: Vec<IntentListItem> = Vec::new();
    for intent in all_intents {
        // Extract campaign + agent from the principal chain.
        let campaign = extract_principal(&intent.principal_chain, "campaign");
        let agent = extract_principal(&intent.principal_chain, "agent");

        // Campaign filter: skip when --campaign given and this one doesn't match.
        if input
            .campaign
            .as_deref()
            .is_some_and(|filter| campaign.as_deref() != Some(filter))
        {
            continue;
        }

        // The intent's OWN owning log (recorded at `intent new --log`), if any.
        let source_log = store.source_logs.get(&intent.intent_id).cloned();

        // --log SCOPE FILTER: keep only intents authored against the named log.
        if let Some(ref want) = filter_key
            && source_log.as_deref() != Some(want.as_str())
        {
            continue;
        }

        // Resolve `landed` against the intent's OWN source log (PS-9 — truthful
        // per-intent, log-aware). No recorded source log, or a source log that no
        // longer exists → `null` (genuinely unknown, never a misleading `false`).
        let landed = match source_log.as_deref() {
            Some(log_key) if Path::new(log_key).exists() => {
                if !landed_cache.contains_key(log_key) {
                    let ids = resolve_landed(Path::new(log_key))?;
                    landed_cache.insert(log_key.to_string(), ids);
                }
                Some(landed_cache[log_key].contains(&intent.intent_id))
            }
            _ => None,
        };

        // Redaction parity (Wave E, P-REDACT-SURFACE): scrub the echoed free-text
        // fields through the hardened engine on the way OUT (defence-in-depth over
        // the write-path redaction; also covers a pre-Wave-E log). Scrub the full
        // charter THEN excerpt, so a redacted charter shows the sentinel, never an
        // 80-char secret prefix. The `log` is a local filesystem path (structural
        // metadata, like `id`), not free text — surfaced as recorded.
        items.push(IntentListItem {
            id: intent.intent_id.clone(),
            charter: charter_excerpt(&crate::redaction::scrub(&intent.charter)),
            campaign: crate::redaction::scrub(&campaign.unwrap_or_default()),
            agent: agent.map(|a| crate::redaction::scrub(&a)),
            landed,
            log: source_log,
        });
    }

    // Stable sort by id — deterministic across runs.
    items.sort_by(|a, b| a.id.cmp(&b.id));

    Ok(ListResult { intents: items })
}

/// Canonical (absolute, cwd-stable) string key for a log path — the same form
/// `intent new` records as an intent's source log. A non-existent path cannot be
/// canonicalized, so it falls back to its lexical form (still a stable key for a
/// filter comparison against a not-yet-created log).
fn canonical_key(p: &Path) -> String {
    std::fs::canonicalize(p)
        .unwrap_or_else(|_| p.to_path_buf())
        .to_string_lossy()
        .into_owned()
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
pub(crate) fn resolve_landed(
    log_path: &Path,
) -> Result<std::collections::HashSet<String>, PorcelainError> {
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
