//! Test fixtures for D11 acceptance oracle.
//!
//! Two fixtures required by the contract:
//!
//! 1. `within_horizon_fixture` — a journal that crashed at T0 and is being
//!    resumed at T0 + 1 hour (well within the 7-day horizon).
//! 2. `beyond_horizon_fixture` — a journal whose last event is 8 days old
//!    (beyond the 7-day horizon).

use crate::journal::horizon::DEFAULT_HORIZON_MS;
use crate::journal::persist::{Journal, JournalKey};

/// Unix epoch milliseconds for a stable anchor (2026-06-05 00:00:00 UTC).
pub const BASE_MS: u64 = 1_749_081_600_000;

/// One day in milliseconds.
pub const ONE_DAY_MS: u64 = 24 * 60 * 60 * 1_000;

/// One hour in milliseconds.
pub const ONE_HOUR_MS: u64 = 60 * 60 * 1_000;

/// A journal that was last written 1 hour ago — well within the 7-day horizon.
///
/// Session crashed at `BASE_MS`; resume attempted at `BASE_MS + 1 hour`.
/// The fixture's `now_ms` companion returns `BASE_MS + ONE_HOUR_MS`.
pub fn within_horizon_fixture() -> (Journal, u64) {
    let key = JournalKey::new("tenant-a", "ws-alpha", "intent-i1");
    let mut journal = Journal::new(key);

    // Session started and ran for a while before the crash.
    journal.append(
        BASE_MS - ONE_HOUR_MS,
        "agent:worker",
        "session started; reading plan",
    );
    journal.append(
        BASE_MS - ONE_HOUR_MS / 2,
        "agent:worker",
        "tool calls: read 3 files; applied 2 edits",
    );
    journal.append(
        BASE_MS,
        "agent:worker",
        "crash detected; final state recorded",
    );

    // Resume is attempted 1 hour after the crash — within horizon.
    let now_ms = BASE_MS + ONE_HOUR_MS;
    (journal, now_ms)
}

/// A journal whose last event is 8 days old — beyond the 7-day horizon.
///
/// The horizon constant is `DEFAULT_HORIZON_MS` (604_800_000 ms = 7 days).
/// The fixture's `now_ms` companion is `BASE_MS + 8 * ONE_DAY_MS`, so
/// `age_ms = 8 * ONE_DAY_MS > DEFAULT_HORIZON_MS`.
pub fn beyond_horizon_fixture() -> (Journal, u64) {
    let key = JournalKey::new("tenant-b", "ws-beta", "intent-i2");
    let mut journal = Journal::new(key);

    // Old session from 8 days ago.
    let old_ms = BASE_MS - 8 * ONE_DAY_MS;
    journal.append(old_ms - ONE_HOUR_MS, "agent:worker", "old session started");
    journal.append(old_ms, "agent:worker", "old session ended (8 days ago)");

    // Resume attempted now — 8 days later, beyond the 7-day horizon.
    let now_ms = BASE_MS;
    debug_assert!(
        now_ms.saturating_sub(old_ms) > DEFAULT_HORIZON_MS,
        "beyond-horizon fixture: age must exceed DEFAULT_HORIZON_MS"
    );
    (journal, now_ms)
}
