//! Badge renderer — reflects true state within a stated staleness bound (item ②).
//!
//! The badge always carries an explicit staleness timestamp. When the status
//! API is down, the badge shows the last-known state annotated with observable
//! staleness — never silently serves a wrong or stale value as fresh.
//!
//! Published constant: `STALENESS_BOUND_MS` — the maximum age (in milliseconds)
//! at which the badge is considered "fresh". Callers must publish this bound
//! to users.

/// The maximum badge freshness window in milliseconds (item ②, staleness bound).
///
/// Badges older than this are stale and must be annotated with their staleness.
pub const STALENESS_BOUND_MS: u64 = 60_000; // 60 seconds

/// The state of a check from the badge's perspective.
#[derive(Debug, Clone, PartialEq)]
pub enum BadgeStatus {
    /// Check passed (exit 0).
    Passing,
    /// Check failed (exit != 0).
    Failing,
    /// Check is in progress / queued.
    Pending,
    /// The status API is unavailable and no last-known state exists.
    Unknown,
}

impl BadgeStatus {
    /// Human-readable label for badge rendering.
    pub fn label(&self) -> &'static str {
        match self {
            BadgeStatus::Passing => "passing",
            BadgeStatus::Failing => "failing",
            BadgeStatus::Pending => "pending",
            BadgeStatus::Unknown => "unknown",
        }
    }
}

/// A rendered badge state with an explicit staleness annotation.
///
/// Guarantees (item ②):
/// - `fresh` is `true` iff `age_ms <= STALENESS_BOUND_MS`.
/// - When `api_down` is `true` and a last-known state exists, `status` reflects
///   that last-known state and `stale` / `last_known` are both set.
/// - The badge NEVER silently serves a stale value as fresh.
#[derive(Debug, Clone, PartialEq)]
pub struct BadgeState {
    /// The check name / context this badge represents.
    pub check_name: String,
    /// The badge status value.
    pub status: BadgeStatus,
    /// Age of the underlying data in milliseconds.
    pub age_ms: u64,
    /// Whether the badge value is within the freshness window.
    pub fresh: bool,
    /// Observable staleness: `true` when `age_ms > STALENESS_BOUND_MS`.
    pub stale: bool,
    /// Whether the API was unavailable when this badge was rendered.
    pub api_down: bool,
    /// Whether this reflects a last-known (cached) state vs. live state.
    pub last_known: bool,
    /// Timestamp (Unix epoch ms) of the underlying data.
    pub data_at: u64,
}

impl BadgeState {
    /// Render a fresh badge from live API data.
    pub fn from_live(
        check_name: impl Into<String>,
        status: BadgeStatus,
        data_at: u64,
        now_ms: u64,
    ) -> Self {
        let age_ms = now_ms.saturating_sub(data_at);
        let stale = age_ms > STALENESS_BOUND_MS;
        let fresh = !stale;
        Self {
            check_name: check_name.into(),
            status,
            age_ms,
            fresh,
            stale,
            api_down: false,
            last_known: false,
            data_at,
        }
    }

    /// Render a badge from last-known state when the status API is down (item ②).
    ///
    /// The badge shows the last-known value annotated with observable staleness.
    /// It is NEVER served as fresh — `fresh` is always `false` in this path.
    pub fn from_last_known(
        check_name: impl Into<String>,
        last_status: BadgeStatus,
        data_at: u64,
        now_ms: u64,
    ) -> Self {
        let age_ms = now_ms.saturating_sub(data_at);
        Self {
            check_name: check_name.into(),
            status: last_status,
            age_ms,
            // Never fresh when API is down — staleness must be observable.
            fresh: false,
            stale: true,
            api_down: true,
            last_known: true,
            data_at,
        }
    }

    /// Render an "unknown" badge when the API is down and no last-known state
    /// exists. Observable staleness is infinite.
    pub fn unknown_api_down(check_name: impl Into<String>, _now_ms: u64) -> Self {
        Self {
            check_name: check_name.into(),
            status: BadgeStatus::Unknown,
            age_ms: u64::MAX,
            fresh: false,
            stale: true,
            api_down: true,
            last_known: false,
            data_at: 0,
        }
    }

    /// Render badge as SVG-compatible text (simplified).
    ///
    /// Format: `{check_name}: {status_label}[ (stale, {age_s}s old)]`
    pub fn render_text(&self) -> String {
        if self.stale {
            let age_s = self.age_ms / 1000;
            format!(
                "{}: {} (stale, {}s old)",
                self.check_name,
                self.status.label(),
                age_s
            )
        } else {
            format!("{}: {}", self.check_name, self.status.label())
        }
    }
}
