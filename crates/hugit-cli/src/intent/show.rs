//! `hugit intent show` — project an intent's record (WP-PC2).
//!
//! Reads the local store and prints ONE stable JSON object describing the
//! intent: its native projection off the event log (the real
//! [`intents_from_log`](hugit_refstore::intent::intents_from_log) path), the
//! non-authoritative [`IntentSidecar`](hugit_contracts::IntentSidecar) corpus
//! (charter / acceptance / campaign / authored-by), the context-envelope ref if
//! one was captured, and any recorded verdicts.
//!
//! ## Honesty (decided)
//!
//! Nothing is invented. A field that was never captured is `null` / `[]` — the
//! `context_ref` is `null` until an envelope is wired, `verdicts` is `[]` until
//! an adversarial panel runs. `show` NEVER fabricates a value to look complete.

use std::path::Path;

use serde_json::{Value, json};

use super::error::PorcelainError;
use super::store::IntentStore;

/// The inputs to `intent show`.
pub struct ShowIntent {
    /// The intent id to project.
    pub intent_id: String,
}

/// Project an intent into its stable JSON object, or a structured `not_found`
/// error when the id is absent from the store.
pub fn run(input: ShowIntent, store_path: &Path) -> Result<Value, PorcelainError> {
    let store = IntentStore::load(store_path).map_err(PorcelainError::from_store)?;

    let intent = store
        .intent_for(&input.intent_id)
        .map_err(PorcelainError::from_store)?
        .ok_or_else(|| {
            PorcelainError::new(
                "not_found",
                format!("no intent {} in the store", input.intent_id),
                "create it first with `hugit intent new`, or check the --store path",
            )
        })?;

    let sidecar = store.sidecars.get(&input.intent_id);

    // The context-envelope ref: prefer the captured envelopes index, fall back
    // to the sidecar's own context_ref — honestly null when neither is set.
    let context_ref: Option<&str> = store
        .envelopes
        .get(&input.intent_id)
        .map(String::as_str)
        .or_else(|| {
            sidecar
                .map(|s| s.context_ref.as_str())
                .filter(|r| !r.is_empty())
        });

    // Verdicts: present only when a panel actually recorded one.
    let verdict = store.verdicts.get(&input.intent_id);

    // Campaign: extracted from the principal chain (`campaign:<key>` entry).
    // Explicit null when not recorded — never invented.
    let campaign: Option<String> = intent
        .principal_chain
        .iter()
        .find_map(|p| p.strip_prefix("campaign:").map(str::to_string));

    // Agent: extracted from the principal chain (`agent:<id>` or `orch:<id>`).
    // Explicit null when not recorded.
    let agent: Option<String> = intent.principal_chain.iter().find_map(|p| {
        p.strip_prefix("agent:")
            .or_else(|| p.strip_prefix("orch:"))
            .or_else(|| p.strip_prefix("orchestrator:"))
            .map(str::to_string)
    });

    // Stable key-set: ALL fields are present on every show call, whether the
    // data was captured or not.  Absent optional data is explicit `null` —
    // never missing keys, never invented values.
    Ok(json!({
        "intent_id": intent.intent_id,
        // The native projection off the real event log (the authoritative spine).
        "charter": intent.charter,
        "ref": intent.ref_name,
        "target": intent.target,
        "seq": intent.seq,
        "recorded_at": intent.recorded_at,
        "principal_chain": intent.principal_chain,
        // Derived convenience fields (always present, null when not extractable).
        "campaign": campaign,
        "agent": agent,
        // The non-authoritative sidecar corpus — null when no corpus is stored.
        "acceptance": sidecar.map(|s| s.acceptance.clone()).unwrap_or_default(),
        "authoritative": sidecar.map(|s| s.authoritative).unwrap_or(false),
        // Envelope ref + verdicts: honestly null/empty until captured (never faked).
        "context_ref": context_ref,
        "verdicts": match verdict {
            Some(v) => vec![v.clone()],
            None => vec![],
        },
    }))
}
