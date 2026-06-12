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

        // Second pass: attach verdicts — REJECT-STICKY resolution (K-VERDICT).
        //
        // Ownership rule (decided, do not relitigate):
        //
        //   A reject is STICKY: once any lens has rejected an intent, that
        //   intent stays `rejected` (and NOT `proven`) regardless of later
        //   approvals under a DIFFERENT lens name.  A reject is cleared ONLY
        //   by a later approval of the **same lens** that issued the reject
        //   (i.e. the same reviewer dimension re-ran and now approves).
        //
        // Implementation: accumulate a per-lens map (lens_name → latest
        // Verdict for that lens) across ALL `verdict.recorded` events for an
        // intent, processing them in log (seq) order.  Each event's
        // `claims_checked` vector carries "lens:result" entries; each entry
        // updates the map entry for that lens.  The final per-lens map is the
        // canonical state:
        //
        //   - If ANY entry is Reject or FixFirst → rejected=true, proven=false
        //   - Else if ANY entry is Approve       → proven=true,  rejected=false
        //   - Else (no entries at all)           → unchanged (no verdict yet)
        //
        // This closes the laundering bypass: an `approve` under a novel lens
        // name cannot clear an outstanding `reject` from a different lens.
        //
        // The last `VerdictView` for the intent (the most-recent
        // `verdict.recorded` record) is surfaced for display; the resolution
        // flags are governed by the per-lens fold above, not by that view.
        //
        // Match by raw intent id (via the internal index) so that redaction
        // of the surfaced field does not break the verdict linkage.

        // Per-intent per-lens accumulator: raw_intent_id → BTreeMap<lens, Verdict>.
        let mut lens_state: std::collections::HashMap<
            usize,
            std::collections::BTreeMap<String, Verdict>,
        > = std::collections::HashMap::new();

        for r in records {
            if r.kind == "verdict.recorded"
                && let Ok(vo) = serde_json::from_str::<VerdictObject>(&r.payload)
                && let Some(&idx) = raw_id_to_idx.get(&vo.intent)
            {
                // Update the per-lens map for this intent from claims_checked.
                // Each entry has the form "lens:result"; parse it and update.
                //
                // Backward-compat fallback: if no `claims_checked` entries are
                // parseable as "lens:result" pairs (either the list is empty or
                // all entries lack the colon separator — e.g. legacy or fixture
                // records), fall back to the stored aggregate verdict on the
                // "panel" pseudo-lens so the record still contributes to the
                // resolution.  This preserves the existing latest-wins semantics
                // for callers that do not populate `claims_checked` with lens
                // details (legacy records, test fixtures, pre-K-VERDICT records).
                let per_lens = lens_state.entry(idx).or_default();
                let mut parsed_any = false;
                for claim in &vo.claims_checked {
                    if let Some((lens, result_str)) = claim.split_once(':') {
                        let outcome = match result_str {
                            "approve" => Verdict::Approve,
                            "fix_first" => Verdict::FixFirst,
                            _ => Verdict::Reject,
                        };
                        per_lens.insert(lens.to_string(), outcome);
                        parsed_any = true;
                    }
                }
                if !parsed_any {
                    // Backward-compat fallback: treat the whole record as a
                    // single "panel" lens using the stored aggregate outcome.
                    per_lens.insert("panel".to_string(), vo.verdict.clone());
                }
                // Update the display view to the most-recent record (the last
                // one wins for the VerdictView field — display only).
                entries[idx].verdict = Some(VerdictView::from_verdict_object(&vo));
            }
        }

        // Third pass: resolve per-lens maps into proven/rejected flags.
        for (idx, per_lens) in &lens_state {
            let any_reject = per_lens
                .values()
                .any(|v| matches!(v, Verdict::Reject | Verdict::FixFirst));
            let any_approve = per_lens.values().any(|v| matches!(v, Verdict::Approve));
            if any_reject {
                // Any outstanding reject wins — reject is sticky.
                entries[*idx].rejected = true;
                entries[*idx].proven = false;
            } else if any_approve {
                // All rejects have been cleared by same-lens re-approvals.
                entries[*idx].proven = true;
                entries[*idx].rejected = false;
            }
            // Else: no verdicts at all (shouldn't happen here, but safe to skip).
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
