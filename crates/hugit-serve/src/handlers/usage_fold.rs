//! Read-time `ctx.usage` cost fold (WP-COST last-mile, serve side).
//!
//! Today every serve cost surface (`/insights`, `/intents/{id}`, `/pr/{id}`)
//! renders the ALREADY-priced envelope metrics; none reads the raw
//! `ctx.usage` records that a `hugit ctx usage` capture appends. This module is
//! the ONE shared fold that prices those records at READ time, so a posted
//! `ctx.usage` record lights the cost rows with **no re-ingest and no re-land**.
//!
//! It is the serve-side mirror of the CLI's `pr::capture::priced_usage`
//! (`crates/hugit-cli/src/pr/capture.rs`), keyed on a single
//! `(target_kind, target_id)` instead of an `OpenedPr` bundle. The honesty rule
//! is transcribed VERBATIM — see [`priced_usage_for`].

use hugit_cli::ctx::CTX_USAGE_KIND;
use hugit_contracts::context_envelope::{ContextEnvelope, TokenCounts};
use hugit_contracts::model_price_card;
use hugit_refstore::EventLog;
use serde_json::Value;

/// The complete-or-nothing priced authoring usage for a single target
/// `(target_kind, target_id)` — the serve mirror of
/// [`hugit_cli::pr::capture::PricedUsage`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PricedUsage {
    /// The Σ of the contributing records' token cache-splits (the summed
    /// [`TokenCounts`] the figure was priced from — recorded for reproducibility).
    pub tokens: TokenCounts,
    /// Σ over ALL contributing records of `cost_micros(card, record.model,
    /// record.tokens)`, in integer micro-USD.
    pub cost_usd_micros: u64,
}

/// Fold + price the `ctx.usage` records whose payload target is exactly
/// `(target_kind, target_id)`.
///
/// # The honesty rule — COMPLETE-OR-NOTHING (the #113 per-PR honesty law)
///
/// Prices each contributing record by ITS OWN model against the FROZEN
/// [`model_price_card::CURRENT`] card, and returns:
///
/// - `Some(PricedUsage { tokens, cost_usd_micros })` — **only when there is at
///   least one contributing record AND every one of them priced to `Some`** (a
///   known model) AND no `u64` overflow occurred anywhere in the accumulation.
///   `cost_usd_micros` is the EXACT `Σ(usage × published rate)`; `tokens` is the
///   summed cache-split it was priced from.
/// - `None` — honest-zero — when there are **no** contributing records, OR
///   **any** contributing record's model is unknown to the card
///   (`cost_micros → None`), OR a matching record is malformed, OR the
///   accumulation overflows. NEVER a partial / under-counted sum, NEVER an
///   estimate.
///
/// A record contributes iff it is a `ctx.usage` record whose payload
/// `(target_kind, target_id)` matches the args EXACTLY. Records for other
/// targets are ignored (no misattribution). The append-only ACCUMULATE rule
/// holds: multiple records for one target each contribute (WP-COST-2 never
/// dedups; this read-time fold sums them).
#[must_use]
pub fn priced_usage_for(log: &EventLog, target_kind: &str, target_id: &str) -> Option<PricedUsage> {
    let mut contributing = 0u64;
    let mut cost: u64 = 0;
    let mut sum = TokenCounts {
        input: 0,
        output: 0,
        cache_read: 0,
        cache_write: 0,
        total: 0,
    };

    for r in log.records().iter().filter(|r| r.kind == CTX_USAGE_KIND) {
        let Ok(v) = serde_json::from_str::<Value>(&r.payload) else {
            // A ctx.usage payload we cannot even parse cannot carry the target
            // we're keyed on — skip it (an id we can't read cannot match).
            // Fail-closed below applies only to a record that DOES match but is
            // malformed.
            continue;
        };
        let rec_id = v
            .get("target_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let rec_kind = v
            .get("target_kind")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if rec_kind != target_kind || rec_id != target_id {
            continue;
        }

        // A CONTRIBUTING (matching) record that is malformed (no tokens block /
        // a missing component field) → fail-closed to None for the WHOLE total
        // (never a partial figure). `?` short-circuits the whole function.
        let t = v.get("tokens")?;
        let rec = TokenCounts {
            input: t.get("input").and_then(Value::as_u64)?,
            output: t.get("output").and_then(Value::as_u64)?,
            cache_read: t.get("cache_read").and_then(Value::as_u64)?,
            cache_write: t.get("cache_write").and_then(Value::as_u64)?,
            // total is the TokenCounts identity — recomputed (checked) below
            // rather than trusting a possibly-absent stored field.
            total: 0,
        };
        let model = v.get("model").and_then(Value::as_str).unwrap_or_default();

        // Price by THIS record's OWN model. Unknown model → None → whole None
        // (honest-zero; NEVER a fallback/nearest rate). Overflow inside
        // cost_micros is likewise None.
        let c = model_price_card::cost_micros(&model_price_card::CURRENT, model, &rec)?;
        cost = cost.checked_add(c)?;

        sum.input = sum.input.checked_add(rec.input)?;
        sum.output = sum.output.checked_add(rec.output)?;
        sum.cache_read = sum.cache_read.checked_add(rec.cache_read)?;
        sum.cache_write = sum.cache_write.checked_add(rec.cache_write)?;
        contributing += 1;
    }

    if contributing == 0 {
        // No contributing records → honest-zero (None), never a fabricated figure.
        return None;
    }

    // The summed cache-split total (checked — overflow ⇒ None, never wrong).
    sum.total = sum
        .input
        .checked_add(sum.output)?
        .checked_add(sum.cache_read)?
        .checked_add(sum.cache_write)?;

    Some(PricedUsage {
        tokens: sum,
        cost_usd_micros: cost,
    })
}

/// Augment a [`ContextEnvelope`]'s cost metrics from the `ctx.usage` records
/// targeting `(kind, id)` — **but ONLY when the envelope already carries an
/// honest-zero cost** (`metrics.cost_usd_micros == 0`). This is the double-count
/// guard: an envelope that already carries a non-zero cost (a fabric-attested
/// land) is returned untouched — the raw usage is NEVER stacked on top.
///
/// Complete-or-nothing rides through [`priced_usage_for`]: a `None` (unknown
/// model / malformed / no records) leaves the envelope at its honest-zero. When
/// it does fold, BOTH the priced `cost_usd_micros` and the summed `tokens`
/// cache-split are written so the rollup's token + cost columns move together.
///
/// Shared by the `/insights` cost-xray (C) and the `/pr/{id}` cost split (D):
/// each `ctx.usage` targets an intent XOR a pr, so work (intent envelopes) and
/// orchestration (the pr envelope) can never fold the same record twice.
#[must_use]
pub fn envelope_with_usage(
    mut env: ContextEnvelope,
    log: &EventLog,
    kind: &str,
    id: &str,
) -> ContextEnvelope {
    if env.metrics.cost_usd_micros == 0
        && let Some(priced) = priced_usage_for(log, kind, id)
    {
        env.metrics.cost_usd_micros = priced.cost_usd_micros;
        env.metrics.tokens = priced.tokens;
    }
    env
}

#[cfg(test)]
mod tests {
    use super::*;
    use hugit_refstore::EventLog;

    /// The frozen Opus rate id in the CURRENT card (see model_price_card).
    const OPUS: &str = "claude-opus-4-8";

    fn usage_payload(
        kind: &str,
        id: &str,
        model: &str,
        i: u64,
        o: u64,
        cr: u64,
        cw: u64,
    ) -> String {
        serde_json::json!({
            "target_id": id,
            "target_kind": kind,
            "source": "provider_usage",
            "model": model,
            "recorded_at": 0,
            "tokens": { "input": i, "output": o, "cache_read": cr, "cache_write": cw, "total": i + o + cr + cw },
        })
        .to_string()
    }

    fn log_with(records: &[(&str, String)]) -> EventLog {
        let mut log = EventLog::new();
        let mut seq = 0u64;
        for (kind, payload) in records {
            seq += 1;
            log.append_for_test(*kind, vec!["t".to_string()], payload.clone(), seq);
        }
        log
    }

    fn expected_micros(model: &str, i: u64, o: u64, cr: u64, cw: u64) -> u64 {
        let tc = TokenCounts {
            input: i,
            output: o,
            cache_read: cr,
            cache_write: cw,
            total: i + o + cr + cw,
        };
        model_price_card::cost_micros(&model_price_card::CURRENT, model, &tc).unwrap()
    }

    #[test]
    fn prices_matching_records_exactly() {
        let log = log_with(&[(
            CTX_USAGE_KIND,
            usage_payload("intent", "i-1", OPUS, 1000, 200, 50, 10),
        )]);
        let got = priced_usage_for(&log, "intent", "i-1").expect("priced");
        assert_eq!(
            got.cost_usd_micros,
            expected_micros(OPUS, 1000, 200, 50, 10)
        );
        assert_eq!(got.tokens.input, 1000);
        assert_eq!(got.tokens.output, 200);
        assert_eq!(got.tokens.cache_read, 50);
        assert_eq!(got.tokens.cache_write, 10);
        assert_eq!(got.tokens.total, 1260);
    }

    #[test]
    fn accumulates_multiple_records_for_same_target() {
        let log = log_with(&[
            (
                CTX_USAGE_KIND,
                usage_payload("intent", "i-1", OPUS, 1000, 200, 0, 0),
            ),
            (
                CTX_USAGE_KIND,
                usage_payload("intent", "i-1", OPUS, 500, 100, 0, 0),
            ),
        ]);
        let got = priced_usage_for(&log, "intent", "i-1").expect("priced");
        assert_eq!(got.tokens.input, 1500);
        assert_eq!(got.tokens.output, 300);
        assert_eq!(got.tokens.total, 1800);
        assert_eq!(
            got.cost_usd_micros,
            expected_micros(OPUS, 1000, 200, 0, 0) + expected_micros(OPUS, 500, 100, 0, 0)
        );
    }

    #[test]
    fn ignores_foreign_target_records() {
        let log = log_with(&[
            (
                CTX_USAGE_KIND,
                usage_payload("intent", "i-OTHER", OPUS, 9999, 9999, 0, 0),
            ),
            (
                CTX_USAGE_KIND,
                usage_payload("pr", "i-1", OPUS, 9999, 9999, 0, 0),
            ),
            (
                CTX_USAGE_KIND,
                usage_payload("intent", "i-1", OPUS, 1000, 200, 0, 0),
            ),
        ]);
        let got = priced_usage_for(&log, "intent", "i-1").expect("priced");
        // Only the ("intent","i-1") record contributes.
        assert_eq!(got.tokens.input, 1000);
        assert_eq!(got.cost_usd_micros, expected_micros(OPUS, 1000, 200, 0, 0));
    }

    #[test]
    fn none_on_unknown_model() {
        let log = log_with(&[(
            CTX_USAGE_KIND,
            usage_payload("intent", "i-1", "totally-unknown-model", 1000, 200, 0, 0),
        )]);
        assert!(priced_usage_for(&log, "intent", "i-1").is_none());
    }

    #[test]
    fn none_when_no_contributing_records() {
        let log = log_with(&[(
            CTX_USAGE_KIND,
            usage_payload("intent", "i-OTHER", OPUS, 1000, 200, 0, 0),
        )]);
        assert!(priced_usage_for(&log, "intent", "i-1").is_none());
    }

    #[test]
    fn none_when_a_matching_record_is_malformed_even_if_another_prices() {
        // Complete-or-nothing: a matching-but-malformed record (missing a tokens
        // component) fails the WHOLE fold to None — never a partial sum.
        let malformed = serde_json::json!({
            "target_id": "i-1",
            "target_kind": "intent",
            "model": OPUS,
            "tokens": { "input": 100, "output": 200 }, // missing cache_read/cache_write
        })
        .to_string();
        let log = log_with(&[
            (
                CTX_USAGE_KIND,
                usage_payload("intent", "i-1", OPUS, 1000, 200, 0, 0),
            ),
            (CTX_USAGE_KIND, malformed),
        ]);
        assert!(priced_usage_for(&log, "intent", "i-1").is_none());
    }
}
