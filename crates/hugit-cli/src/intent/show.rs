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

/// Project an intent into its stable JSON object, or a structured error: a
/// MISSING `--store` is `store_not_found`/exit-2 (the read-only sibling
/// contract — never a silent empty store reported as the intent merely being
/// absent); an existing store missing the queried id is `not_found`.
pub fn run(input: ShowIntent, store_path: &Path) -> Result<Value, PorcelainError> {
    let store = IntentStore::load_existing(store_path).map_err(PorcelainError::from_store)?;

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

    // PS-9: the intent's OWN owning log (recorded at `intent new --log`) and its
    // `landed` state resolved against THAT log — truthful per-intent, not against
    // some unrelated `--log`. No recorded log, or a log that no longer exists →
    // `null` (genuinely unknown). A tampered source log fails closed
    // (`chain_broken`/exit-2) via the shared verified loader — never silent `null`.
    let source_log = store.source_logs.get(&input.intent_id).cloned();
    let landed: Option<bool> = match source_log.as_deref() {
        Some(log_key) if Path::new(log_key).exists() => {
            let ids = super::list::resolve_landed(Path::new(log_key))?;
            Some(ids.contains(&input.intent_id))
        }
        _ => None,
    };

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

    // Redaction parity (Wave E, P-REDACT-SURFACE): scrub every echoed free-text
    // field through the hardened engine on the way OUT, as defence-in-depth. The
    // write path (`intent new`) already redacts before persisting, so a fresh
    // store carries nothing verbatim; this guard also covers a pre-Wave-E log
    // that was written before redact-on-write existed. Structural fields
    // (`ref`/`target`/`seq`/`recorded_at`/`authoritative`/`verdicts`) are not
    // free text and are not scrubbed.
    let charter = crate::redaction::scrub(&intent.charter);
    let context_ref = context_ref.map(crate::redaction::scrub);
    let campaign = campaign.map(|c| crate::redaction::scrub(&c));
    let agent = agent.map(|a| crate::redaction::scrub(&a));
    let acceptance: Vec<String> = sidecar
        .map(|s| crate::redaction::scrub_all(&s.acceptance))
        .unwrap_or_default();
    let principal_chain = crate::redaction::scrub_all(&intent.principal_chain);

    // Stable key-set: ALL fields are present on every show call, whether the
    // data was captured or not.  Absent optional data is explicit `null` —
    // never missing keys, never invented values.
    Ok(json!({
        "intent_id": intent.intent_id,
        // The native projection off the real event log (the authoritative spine).
        "charter": charter,
        "ref": intent.ref_name,
        "target": intent.target,
        "seq": intent.seq,
        "recorded_at": intent.recorded_at,
        "principal_chain": principal_chain,
        // Derived convenience fields (always present, null when not extractable).
        "campaign": campaign,
        "agent": agent,
        // The non-authoritative sidecar corpus — null when no corpus is stored.
        "acceptance": acceptance,
        "authoritative": sidecar.map(|s| s.authoritative).unwrap_or(false),
        // Envelope ref + verdicts: honestly null/empty until captured (never faked).
        "context_ref": context_ref,
        "verdicts": match verdict {
            Some(v) => vec![v.clone()],
            None => vec![],
        },
        // PS-9: the owning log + per-log landed state (both null when not recorded).
        "log": source_log,
        "landed": landed,
    }))
}
