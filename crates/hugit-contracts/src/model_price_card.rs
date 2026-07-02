//! model_price_card — a frozen, versioned model→per-token price card for the
//! cost-killer (Option A: real tokens × EXACT published price = the invoice).
//!
//! WP-COST-1. HONESTY-critical: every rate here is the provider's EXACT
//! published rate — never approximated, never a nearest/default fallback. The
//! cost identity is `Σ(real provider-/usage tokens × exact published per-token
//! rate)`. [`cost_micros`] returns [`None`] for an unknown model (honest-zero)
//! rather than guessing a price.
//!
//! # Source of the rates (verified 2026-07-02)
//!
//! Transcribed from Anthropic's published price list:
//! <https://platform.claude.com/docs/en/about-claude/pricing> (model-pricing
//! table) and cross-checked against the models overview
//! <https://platform.claude.com/docs/en/about-claude/models/overview>.
//!
//! Published per-million-token (`/MTok`) USD rates, verbatim:
//!
//! | Model (API id)                  | Input | Output | Cache read (hit) | Cache write (5m) |
//! |---------------------------------|-------|--------|------------------|------------------|
//! | `claude-opus-4-8`               | $5    | $25    | $0.50            | $6.25            |
//! | `claude-sonnet-4-6`             | $3    | $15    | $0.30            | $3.75            |
//! | `claude-haiku-4-5-20251001`     | $1    | $5     | $0.10            | $1.25            |
//! | `claude-fable-5`                | $10   | $50    | $1.00            | $12.50           |
//!
//! These four are exactly the model ids hugit's authoring fleet uses
//! (`claude-opus-4-8`, `claude-sonnet-4-6`, `claude-haiku-4-5-20251001`,
//! `claude-fable-5`). The Haiku dated-id alias `claude-haiku-4-5` resolves to
//! the same verified rate (Anthropic publishes both as ids for the identical
//! model/price) and is included as a second exact key so the runner may emit
//! either form. Any model NOT in this table returns [`None`] from
//! [`cost_micros`] — honest-zero, never a guessed rate.
//!
//! **Cache write = the 5-minute-TTL write** (Anthropic's default prompt-cache
//! TTL, the `1.25×` base tier). The separate `1h` cache-write tier ($10 / $6 /
//! $2 / $20 per MTok respectively) is a distinct TTL not modelled in v1 — add a
//! `cache_write_1h` field when a caller needs it, rather than overloading this
//! one.
//!
//! # Unit — pico-USD per token (`10⁻¹² USD`), and WHY it is not micro-USD
//!
//! The WP names the struct [`PerTokenMicros`], but the fields are stored in
//! **pico-USD per token** (`1 token's cost × 10¹²`), NOT micro-USD per token.
//! This is forced by the honesty law: several published rates are *fractional*
//! micro-USD per single token — e.g. `claude-opus-4-8` cache-read is
//! `$0.50 / MTok = 0.5` micro-USD per token, and cache-write is `6.25` micro-USD
//! per token. Storing those as an integer `u64` micro-USD-per-token would force
//! rounding (`0.5 → 0` or `→ 1`), i.e. an *approximated* rate — a honesty-law
//! violation. Pico-USD per token represents every published rate as an EXACT
//! integer with zero approximation:
//!
//! ```text
//! pico_per_token = published_USD_per_MTok × 1_000_000
//!   $5.00 / MTok → 5_000_000        $0.50 / MTok →   500_000
//!   $6.25 / MTok → 6_250_000        $0.10 / MTok →   100_000
//! ```
//!
//! [`cost_micros`] accumulates `Σ(tokens × pico_rate)` in pico-USD, then divides
//! by `1_000_000` (`= PICO_PER_MICRO`) to yield integer **micro-USD** — the unit
//! of the downstream `cost_usd_micros: u64` contract
//! ([`crate::IntentMetrics::cost_usd_micros`] / the runner-lease close DTO). Any
//! sub-micro-USD remainder floors; at realistic invoice scale (thousands+ of
//! tokens) the result is exact.

use crate::context_envelope::TokenCounts;

/// Frozen snapshot version of this price card. Stable + serializable (a plain
/// `&'static str`); bump it (`pc-YYYY-MM`) whenever any rate changes so a
/// rendered cost can cite exactly which price snapshot produced it.
pub const PRICE_CARD_VERSION: &str = "pc-2026-07";

/// `1 micro-USD = 1_000_000 pico-USD`. The divisor turning the pico-USD
/// accumulation of [`cost_micros`] into integer micro-USD.
const PICO_PER_MICRO: u64 = 1_000_000;

/// Scale factor from a published `USD / MTok` rate to pico-USD per token:
/// `pico_per_token = usd_per_mtok × TOKENS_PER_MTOK_TIMES_PICO_PER_USD`.
/// (`USD/MTok ÷ 1_000_000 tokens/MTok × 10¹² pico/USD = usd_per_mtok × 10⁶`.)
const USD_PER_MTOK_TO_PICO_PER_TOKEN: u64 = 1_000_000;

/// Per-token price for one model, in **pico-USD (`10⁻¹² USD`) per token**.
///
/// See the module docs for why the unit is pico-USD (not micro-USD): it is the
/// coarsest integer unit that represents every EXACT published Anthropic rate —
/// including the fractional-micro-USD cache rates — without any approximation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PerTokenMicros {
    /// Base (non-cached) input tokens — pico-USD per token.
    pub input: u64,
    /// Output tokens — pico-USD per token.
    pub output: u64,
    /// Prompt-cache read (hit) tokens — pico-USD per token.
    pub cache_read: u64,
    /// Prompt-cache write (5-minute TTL) tokens — pico-USD per token.
    pub cache_write: u64,
}

impl PerTokenMicros {
    /// Build a per-token rate from a model's four published `USD / MTok`
    /// figures, expressed as integer hundredths of a dollar (cents) per MTok so
    /// fractional-dollar rates (`$6.25`, `$0.50`) stay EXACT with no float.
    ///
    /// `cents/MTok → pico/token`: `pico = cents × 10⁴`
    /// (`cents ÷ 100 = USD/MTok`, then `× 10⁶` per [`USD_PER_MTOK_TO_PICO_PER_TOKEN`]).
    const fn from_cents_per_mtok(
        input_cents: u64,
        output_cents: u64,
        cache_read_cents: u64,
        cache_write_cents: u64,
    ) -> Self {
        // cents × 10⁴ == (cents / 100 dollars) × 1_000_000 pico-per-dollar-token.
        const CENTS_TO_PICO: u64 = USD_PER_MTOK_TO_PICO_PER_TOKEN / 100;
        Self {
            input: input_cents * CENTS_TO_PICO,
            output: output_cents * CENTS_TO_PICO,
            cache_read: cache_read_cents * CENTS_TO_PICO,
            cache_write: cache_write_cents * CENTS_TO_PICO,
        }
    }
}

/// A frozen, versioned model→per-token price card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelPriceCard {
    /// The snapshot version ([`PRICE_CARD_VERSION`] for [`CURRENT`]).
    pub version: &'static str,
    /// Exact-match table of `(model_id, per-token rate)`. Lookup is by EXACT
    /// `model_id` equality only — never prefix/nearest/default.
    pub rates: &'static [(&'static str, PerTokenMicros)],
}

impl ModelPriceCard {
    /// The exact rate for `model_id`, or [`None`] if this card does not list it
    /// (honest-zero — never a fallback rate).
    #[must_use]
    pub fn rate(&self, model_id: &str) -> Option<&PerTokenMicros> {
        self.rates
            .iter()
            .find(|(id, _)| *id == model_id)
            .map(|(_, r)| r)
    }
}

/// The current price card (`pc-2026-07`). Rates verified 2026-07-02 against the
/// Anthropic published price list — see the module docs for the source URL and
/// the verbatim `/MTok` figures.
pub const CURRENT: ModelPriceCard = ModelPriceCard {
    version: PRICE_CARD_VERSION,
    rates: &[
        // (input, output, cache_read, cache_write-5m) in cents per MTok.
        // claude-opus-4-8:   $5   / $25  / $0.50 / $6.25
        (
            "claude-opus-4-8",
            PerTokenMicros::from_cents_per_mtok(500, 2500, 50, 625),
        ),
        // claude-sonnet-4-6: $3   / $15  / $0.30 / $3.75
        (
            "claude-sonnet-4-6",
            PerTokenMicros::from_cents_per_mtok(300, 1500, 30, 375),
        ),
        // claude-haiku-4-5-20251001: $1 / $5 / $0.10 / $1.25
        (
            "claude-haiku-4-5-20251001",
            PerTokenMicros::from_cents_per_mtok(100, 500, 10, 125),
        ),
        // claude-haiku-4-5 (alias id for the identical model/price above).
        (
            "claude-haiku-4-5",
            PerTokenMicros::from_cents_per_mtok(100, 500, 10, 125),
        ),
        // claude-fable-5:    $10  / $50  / $1.00 / $12.50
        (
            "claude-fable-5",
            PerTokenMicros::from_cents_per_mtok(1000, 5000, 100, 1250),
        ),
    ],
};

/// Cost of `tokens` for `model_id`, in integer **micro-USD**, using `card`.
///
/// Returns:
/// - `Some(Σ(usage × exact published rate))` in micro-USD when `model_id` is an
///   EXACT match in `card` (sub-micro-USD remainders floor);
/// - `None` when `model_id` is unknown to `card` — honest-zero, NEVER a
///   default/nearest/fallback rate (the honesty law);
/// - `None` on `u64` overflow (checked arithmetic — never panics, never a wrong
///   number).
#[must_use]
pub fn cost_micros(card: &ModelPriceCard, model_id: &str, tokens: &TokenCounts) -> Option<u64> {
    let rate = card.rate(model_id)?; // unknown model → None (honest-zero)

    // Accumulate Σ(tokens × pico_rate) in pico-USD, fully checked.
    let pico = tokens
        .input
        .checked_mul(rate.input)?
        .checked_add(tokens.output.checked_mul(rate.output)?)?
        .checked_add(tokens.cache_read.checked_mul(rate.cache_read)?)?
        .checked_add(tokens.cache_write.checked_mul(rate.cache_write)?)?;

    // pico-USD → integer micro-USD (the downstream cost_usd_micros unit).
    Some(pico / PICO_PER_MICRO)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tc(input: u64, output: u64, cache_read: u64, cache_write: u64) -> TokenCounts {
        TokenCounts {
            input,
            output,
            cache_read,
            cache_write,
            total: input
                .saturating_add(output)
                .saturating_add(cache_read)
                .saturating_add(cache_write),
        }
    }

    #[test]
    fn unknown_model_is_none_never_a_fallback() {
        // Not in the card → honest-zero (None), even for close-but-not-exact ids.
        assert_eq!(cost_micros(&CURRENT, "gpt-4", &tc(1, 1, 1, 1)), None);
        assert_eq!(cost_micros(&CURRENT, "", &tc(1, 1, 1, 1)), None);
        // Exact match only — no prefix / trailing-space tolerance.
        assert_eq!(cost_micros(&CURRENT, "claude-opus", &tc(1, 1, 1, 1)), None);
        assert_eq!(
            cost_micros(&CURRENT, "claude-opus-4-8 ", &tc(1, 1, 1, 1)),
            None
        );
    }

    #[test]
    fn opus_golden_hand_computed() {
        // 1_500_000 input, 200_000 output, 4_000_000 cache_read, 1_000_000 cache_write.
        // Opus 4.8 published: $5 / $25 / $0.50 / $6.25 per MTok.
        //   input:       1.5M × $5.00 = $7.50
        //   output:      0.2M × $25.0 = $5.00
        //   cache_read:  4.0M × $0.50 = $2.00
        //   cache_write: 1.0M × $6.25 = $6.25
        //   total = $20.75 = 20_750_000 micro-USD
        let got = cost_micros(
            &CURRENT,
            "claude-opus-4-8",
            &tc(1_500_000, 200_000, 4_000_000, 1_000_000),
        );
        assert_eq!(got, Some(20_750_000));
    }

    #[test]
    fn sonnet_golden_hand_computed() {
        // Sonnet 4.6 published: $3 / $15 / $0.30 / $3.75 per MTok.
        //   2M input × $3      = $6.00
        //   1M output × $15    = $15.00
        //   10M cache_read×$0.30 = $3.00
        //   4M cache_write×$3.75 = $15.00
        //   total = $39.00 = 39_000_000 micro-USD
        let got = cost_micros(
            &CURRENT,
            "claude-sonnet-4-6",
            &tc(2_000_000, 1_000_000, 10_000_000, 4_000_000),
        );
        assert_eq!(got, Some(39_000_000));
    }

    #[test]
    fn haiku_dated_id_and_alias_agree() {
        let tokens = tc(3_000_000, 500_000, 1_000_000, 800_000);
        // Haiku 4.5: $1 / $5 / $0.10 / $1.25 per MTok.
        //   3M×$1 = $3.00; 0.5M×$5 = $2.50; 1M×$0.10 = $0.10; 0.8M×$1.25 = $1.00
        //   total = $6.60 = 6_600_000 micro-USD
        let dated = cost_micros(&CURRENT, "claude-haiku-4-5-20251001", &tokens);
        let alias = cost_micros(&CURRENT, "claude-haiku-4-5", &tokens);
        assert_eq!(dated, Some(6_600_000));
        assert_eq!(alias, Some(6_600_000));
    }

    #[test]
    fn fable_golden_hand_computed() {
        // Fable 5: $10 / $50 / $1.00 / $12.50 per MTok.
        //   1M×$10 = $10; 1M×$50 = $50; 1M×$1 = $1; 1M×$12.50 = $12.50
        //   total = $73.50 = 73_500_000 micro-USD
        let got = cost_micros(
            &CURRENT,
            "claude-fable-5",
            &tc(1_000_000, 1_000_000, 1_000_000, 1_000_000),
        );
        assert_eq!(got, Some(73_500_000));
    }

    #[test]
    fn single_token_floors_sub_micro_remainder() {
        // One cache_read token on Opus = 0.5 micro-USD → floors to 0 micro-USD.
        // (Documented: sub-micro-USD remainders floor; exact at invoice scale.)
        let got = cost_micros(&CURRENT, "claude-opus-4-8", &tc(0, 0, 1, 0));
        assert_eq!(got, Some(0));
    }

    #[test]
    fn overflow_is_none_not_panic() {
        // u64::MAX tokens × a positive rate overflows the checked multiply → None.
        let got = cost_micros(&CURRENT, "claude-opus-4-8", &tc(u64::MAX, 0, 0, 0));
        assert_eq!(got, None);
        // Overflow in the accumulation (add) path is also None, not a panic.
        let got2 = cost_micros(
            &CURRENT,
            "claude-fable-5",
            &tc(u64::MAX / 2, u64::MAX / 2, 0, 0),
        );
        assert_eq!(got2, None);
    }

    #[test]
    fn version_is_stable_and_pinned() {
        assert_eq!(PRICE_CARD_VERSION, "pc-2026-07");
        assert_eq!(CURRENT.version, PRICE_CARD_VERSION);
    }

    #[test]
    fn rate_lookup_is_exact_pico_per_token() {
        // Spot-check the raw stored pico-USD/token values are the EXACT published
        // rates (no rounding): Opus cache_read $0.50/MTok = 500_000 pico/token.
        let opus = CURRENT.rate("claude-opus-4-8").unwrap();
        assert_eq!(opus.input, 5_000_000);
        assert_eq!(opus.output, 25_000_000);
        assert_eq!(opus.cache_read, 500_000);
        assert_eq!(opus.cache_write, 6_250_000);
        assert!(CURRENT.rate("nope").is_none());
    }
}
