//! `hugit ledger` — the asked→done→proven projection per campaign.
//!
//! Reads the EventRecord stream and emits a structured ledger: for each
//! campaign, which intents were asked, which landed (done), and which have been
//! verdict-proven.  Pure projection, zero writes back to the stream.
//!
//! # Redaction (④)
//! Secrets are redacted at the view boundary.  Any payload field or string that
//! matches the planted-secret marker is replaced with the sentinel "[REDACTED]"
//! before the record is surfaced in a `LedgerEntry` or `VerdictView`.

use crate::redact;
use hugit_contracts::event_record::EventRecord;
use hugit_contracts::verdict_object::{Verdict, VerdictObject};
use serde::{Deserialize, Serialize};

/// The canonical redaction sentinel (view-boundary, ④).
///
/// Re-exported from [`crate::redact::REDACTED`] — single source of truth.
pub use crate::redact::REDACTED;

/// One entry in the ledger — an intent at its current lifecycle stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LedgerEntry {
    /// The campaign this intent belongs to (e.g. "wave-D2").
    pub campaign: String,
    /// The stable intent id.
    pub intent_id: String,
    /// Short human charter (redacted if it contained a secret).
    pub charter: String,
    /// Log sequence when the intent landed (done).
    pub seq: u64,
    /// Unix epoch ms when the intent was recorded.
    pub recorded_at: u64,
    /// The deep-link target object identifier (content address or intent_id).
    pub deep_link_target: String,
    /// Whether a verdict with an APPROVE outcome has been recorded for this
    /// intent.  A REJECT (or fix_first) verdict does NOT set `proven`.
    pub proven: bool,
    /// Whether a verdict with a non-approve (REJECT or fix_first) outcome has
    /// been recorded for this intent.  Distinct from `proven` — a rejected
    /// intent is landed but NOT proven.
    pub rejected: bool,
    /// The verdict view (redacted), if a verdict has been recorded.
    pub verdict: Option<VerdictView>,
}

/// A redacted view of a VerdictObject suitable for ledger display.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerdictView {
    /// Intent this verdict covers.
    pub intent: String,
    /// Review lens.
    pub lens: String,
    /// The verdict outcome as a string.
    pub outcome: String,
    /// Claims checked (each entry redacted if needed).
    pub claims_checked: Vec<String>,
}

impl VerdictView {
    /// Build a VerdictView from a VerdictObject, applying view-boundary redaction.
    pub fn from_verdict_object(v: &VerdictObject) -> Self {
        let claims = v.claims_checked.iter().map(|c| redact::apply(c)).collect();
        VerdictView {
            intent: redact::apply(&v.intent),
            lens: redact::apply(&v.lens),
            outcome: format!("{:?}", v.verdict),
            claims_checked: claims,
        }
    }
}

/// The full ledger for a set of event records.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    entries: Vec<LedgerEntry>,
}

impl Ledger {
    /// Build a ledger from a slice of EventRecords.
    ///
    /// Recognises `intent.landed` events as "asked+done" and
    /// `verdict.recorded` events as "proven".
    pub fn from_records(records: &[EventRecord]) -> Self {
        let mut entries: Vec<LedgerEntry> = Vec::new();
        // Internal index: raw (unredacted) intent_id → entry index.
        // Used in the verdict pass so linking survives view-boundary redaction.
        let mut raw_id_to_idx: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        // First pass: collect all intent.landed events.
        for r in records {
            if r.kind == "intent.landed"
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.payload)
            {
                let raw_intent_id = string_field(&v, "intent_id").unwrap_or_default();
                let raw_campaign =
                    string_field(&v, "campaign").unwrap_or_else(|| "default".to_string());
                let raw_charter = string_field(&v, "charter").unwrap_or_default();
                let raw_deep_link =
                    string_field(&v, "deep_link_target").unwrap_or_else(|| raw_intent_id.clone());

                // Route every surfaced string through the view-boundary redaction
                // filter (④). intent_id/campaign/deep_link_target were previously
                // surfaced raw — this closes that gap.
                let intent_id = redact::apply(&raw_intent_id);
                let campaign = redact::apply(&raw_campaign);
                let charter = redact::apply(&raw_charter);
                let deep_link_target = redact::apply(&raw_deep_link);

                let idx = entries.len();
                // Register the raw id so the verdict pass can link correctly even
                // when the surfaced intent_id is "[REDACTED]".
                raw_id_to_idx.insert(raw_intent_id, idx);

                entries.push(LedgerEntry {
                    campaign,
                    intent_id,
                    charter,
                    seq: r.seq,
                    recorded_at: r.recorded_at,
                    deep_link_target,
                    proven: false,
                    rejected: false,
                    verdict: None,
                });
            }
        }

        // Second pass: attach verdicts.
        // Match by raw intent id (via the internal index) so that redaction
        // of the surfaced field does not break the verdict linkage.
        //
        // WI-PROVEN2 / latest-verdict-wins: records are in log (seq) order;
        // iterating forward means each subsequent `verdict.recorded` for the
        // same intent overwrites the previous one.  The LAST verdict on the
        // log is authoritative — `proven` and `rejected` are MUTUALLY
        // EXCLUSIVE: only the latest verdict outcome governs.
        //
        // WH-PROVEN: `proven` is true ONLY when the LATEST outcome is
        // `Approve`.  A `Reject` or `FixFirst` revision clears `proven` and
        // sets `rejected=true`.  An `Approve` revision clears `rejected` and
        // sets `proven=true`.  The two flags are never simultaneously true.
        for r in records {
            if r.kind == "verdict.recorded"
                && let Ok(vo) = serde_json::from_str::<VerdictObject>(&r.payload)
                && let Some(&idx) = raw_id_to_idx.get(&vo.intent)
            {
                match vo.verdict {
                    Verdict::Approve => {
                        // Latest verdict is approve: proven=true, rejected cleared.
                        entries[idx].proven = true;
                        entries[idx].rejected = false;
                    }
                    Verdict::Reject | Verdict::FixFirst => {
                        // Latest verdict is non-approve: rejected=true, proven cleared.
                        entries[idx].rejected = true;
                        entries[idx].proven = false;
                    }
                }
                entries[idx].verdict = Some(VerdictView::from_verdict_object(&vo));
            }
        }

        Ledger { entries }
    }

    /// All ledger entries in log order.
    pub fn entries(&self) -> &[LedgerEntry] {
        &self.entries
    }

    /// Entries for a specific campaign, in log order.
    pub fn by_campaign<'a>(&'a self, campaign: &str) -> impl Iterator<Item = &'a LedgerEntry> {
        self.entries.iter().filter(move |e| e.campaign == campaign)
    }

    /// Count of asked (landed) intents for a campaign.
    pub fn asked(&self, campaign: &str) -> usize {
        self.by_campaign(campaign).count()
    }

    /// Count of done (landed) intents for a campaign (same as asked — landing = done).
    pub fn done(&self, campaign: &str) -> usize {
        self.asked(campaign)
    }

    /// Count of proven (approve-verdict recorded) intents for a campaign.
    pub fn proven(&self, campaign: &str) -> usize {
        self.by_campaign(campaign).filter(|e| e.proven).count()
    }

    /// Count of rejected (non-approve verdict recorded) intents for a campaign.
    pub fn rejected(&self, campaign: &str) -> usize {
        self.by_campaign(campaign).filter(|e| e.rejected).count()
    }
}

fn string_field(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}
