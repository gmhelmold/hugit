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
use hugit_contracts::verdict_object::VerdictObject;
use serde::{Deserialize, Serialize};

/// The canonical redaction sentinel (view-boundary, ④).
pub const REDACTED: &str = "[REDACTED]";

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
    /// Whether a verdict has been recorded for this intent (proven).
    pub proven: bool,
    /// The verdict view (redacted), if proven.
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

        // First pass: collect all intent.landed events.
        for r in records {
            if r.kind == "intent.landed"
                && let Ok(v) = serde_json::from_str::<serde_json::Value>(&r.payload)
            {
                let intent_id = string_field(&v, "intent_id").unwrap_or_default();
                let campaign =
                    string_field(&v, "campaign").unwrap_or_else(|| "default".to_string());
                let charter = string_field(&v, "charter").unwrap_or_default();
                let deep_link_target =
                    string_field(&v, "deep_link_target").unwrap_or_else(|| intent_id.clone());

                entries.push(LedgerEntry {
                    campaign,
                    intent_id: intent_id.clone(),
                    charter: redact::apply(&charter),
                    seq: r.seq,
                    recorded_at: r.recorded_at,
                    deep_link_target,
                    proven: false,
                    verdict: None,
                });
            }
        }

        // Second pass: attach verdicts.
        for r in records {
            if r.kind == "verdict.recorded"
                && let Ok(vo) = serde_json::from_str::<VerdictObject>(&r.payload)
                && let Some(entry) = entries.iter_mut().find(|e| e.intent_id == vo.intent)
            {
                entry.proven = true;
                entry.verdict = Some(VerdictView::from_verdict_object(&vo));
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

    /// Count of proven (verdict recorded) intents for a campaign.
    pub fn proven(&self, campaign: &str) -> usize {
        self.by_campaign(campaign).filter(|e| e.proven).count()
    }
}

fn string_field(v: &serde_json::Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}
