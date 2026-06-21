//! Shared presentation + safety helpers for the Wave-1 handlers.
//!
//! Consolidated here (not copied per-handler) after the adversarial audit found
//! three divergent `humanize_age` copies and an unredacted free-text leak. The
//! ONE redaction seam ([`scrub`]) is the security spine of this adapter: EVERY
//! free-text field echoed from the log into a view-model MUST pass through it,
//! exactly as every CLI read-path does (`crate::redaction::scrub` →
//! `hugit_ledger::redact::apply`). Structural fields (sha, intent_id, memo_key,
//! numbers) are NOT scrubbed (they are content-addresses, not free text).

use serde_json::Value;

// ── Redaction (the security spine — defence-in-depth on the read path) ───────

/// Scrub a single echoed free-text field through the engine's redaction
/// detector — the SAME `hugit_ledger::redact::apply` every CLI read-surface
/// uses. A secret-shaped value becomes the `[REDACTED]` sentinel, so a secret in
/// the log (or one a write-path forgot) never reaches the browser.
#[must_use]
pub fn scrub(s: &str) -> String {
    hugit_ledger::redact::apply(s)
}

/// Scrub each item of a free-text list (e.g. principal chains, acceptance lines).
#[must_use]
pub fn scrub_all(items: &[String]) -> Vec<String> {
    items.iter().map(|s| scrub(s)).collect()
}

// ── List caps (fail-honest bound — the VM IS the page; backend cuts it) ──────

/// Max commit rows projected into a `CommitsVm` (backend-API-v1 §1 list caps).
pub const COMMITS_CAP: usize = 100;
/// Max check rows projected into a `ChecksVm`/`PrDetailVm`.
pub const CHECKS_CAP: usize = 200;
/// Max PR cards projected into a `LandingVm` column set.
pub const PR_CARDS_CAP: usize = 200;
/// Max contributors listed on a `RepoHomeVm`.
pub const CONTRIBUTORS_CAP: usize = 100;

// ── Presentation helpers ──────────────────────────────────────────────────────

/// Humanize a Unix-epoch-millisecond timestamp into a pt-BR relative age, e.g.
/// "há menos de 1 min" / "há 5 min" / "há 2h" / "há 3d" / "há 2 meses" / "há 1 ano".
/// The single canonical implementation (was forked 3 ways with divergent output).
/// Presentation-only (never hashed), so the wall-clock read is acceptable.
#[must_use]
pub fn humanize_age(unix_ms: u64) -> String {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    let secs = now_ms.saturating_sub(unix_ms) / 1_000;
    match secs {
        s if s < 60 => "há menos de 1 min".to_string(),
        s if s < 3_600 => format!("há {} min", s / 60),
        s if s < 86_400 => format!("há {}h", s / 3_600),
        s if s < 2_592_000 => format!("há {}d", s / 86_400),
        s if s < 31_536_000 => {
            let m = s / 2_592_000;
            if m == 1 {
                "há 1 mês".to_string()
            } else {
                format!("há {m} meses")
            }
        }
        s => {
            let y = s / 31_536_000;
            if y == 1 {
                "há 1 ano".to_string()
            } else {
                format!("há {y} anos")
            }
        }
    }
}

/// Map an author string to the mock's avatar CSS class: "opus" / "sonnet" /
/// the first whitespace token (a human handle). Presentation-only.
///
/// The fallback token (first whitespace-delimited word from a raw author string)
/// is derived BEFORE the caller scrubs `author`, so it is scrubbed HERE before
/// being serialised — `ghp_… name` must not leak the token into `avatar_class`.
#[must_use]
pub fn classify_avatar(author: &str) -> String {
    let a = author.to_ascii_lowercase();
    if a.contains("opus") {
        "opus".to_string()
    } else if a.contains("sonnet") {
        "sonnet".to_string()
    } else {
        // Scrub the first whitespace token so a secret-shaped author prefix
        // (e.g. `ghp_… Name`) does not survive into the avatar_class field.
        let token = author.split_whitespace().next().unwrap_or("");
        scrub(token)
    }
}

/// The first `n` characters of a sha (char-boundary safe — never panics on a
/// multibyte byte; structural, not scrubbed).
#[must_use]
pub fn sha_prefix(sha: &str, n: usize) -> String {
    sha.chars().take(n).collect()
}

/// Read a string field from a JSON payload (`None` when absent/non-string).
#[must_use]
pub fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

/// Clamp a `[0.0, 1.0]`-ish ratio (or already-percent) to an honest percentage
/// `u8` in `0..=100` — NOT 255 (the field is a percentage by contract).
#[must_use]
pub fn pct_u8(ratio_times_100: f64) -> u8 {
    ratio_times_100.round().clamp(0.0, 100.0) as u8
}

#[cfg(test)]
mod tests {
    use hugit_ledger::redact::REDACTED;

    use super::*;

    // ── classify_avatar: secret-shaped author must not leak via avatar_class ──

    #[test]
    fn avatar_class_scrubs_secret_shaped_first_token() {
        // An author whose first whitespace-delimited token is a GitHub PAT must
        // NOT appear verbatim in avatar_class — it must be redacted.
        let author = "ghp_16C7e42F292c6912E7710c838347Ae178B4a Some Name";
        let class = classify_avatar(author);
        assert_ne!(
            class, "ghp_16C7e42F292c6912E7710c838347Ae178B4a",
            "a secret-shaped first token must be scrubbed in avatar_class"
        );
        assert_eq!(
            class, REDACTED,
            "the REDACTED sentinel must replace a secret-shaped avatar_class token"
        );
    }

    #[test]
    fn avatar_class_opus_is_unchanged() {
        // The "opus" / "sonnet" fast-paths do not go through the scrub; confirm
        // they still work correctly (no regression on the happy path).
        assert_eq!(classify_avatar("Claude Opus 4.8"), "opus");
        assert_eq!(classify_avatar("claude-sonnet-4-6"), "sonnet");
    }

    #[test]
    fn avatar_class_normal_handle_survives() {
        // A plain human handle (no secret shape) passes through unchanged.
        assert_eq!(classify_avatar("alice"), "alice");
        assert_eq!(classify_avatar("alice bob"), "alice");
    }
}
