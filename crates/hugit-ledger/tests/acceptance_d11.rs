//! WP-D11 acceptance oracle — journals + `ctx resume`.
//!
//! Owned items (VERBATIM from decomposition v2.0 — D11):
//!   ① journal persisted as tenant-private object bound to ws/intent
//!   ② post-crash `ctx resume` reconstructs session within supported horizon
//!   ③ beyond-horizon resume refused/degraded as documented
//!
//! Driven by `tests/acceptance/wp-d11/run.sh`. One `#[test] item_<n>_<slug>`
//! per owned item.
//!
//! Fixtures used:
//!   - `within_horizon_fixture` — crash 1 hour ago; resume within 7-day window.
//!   - `beyond_horizon_fixture` — last event 8 days ago; beyond 7-day horizon.

use hugit_ledger::journal::fixtures::{beyond_horizon_fixture, within_horizon_fixture};
use hugit_ledger::journal::horizon::DEFAULT_HORIZON_MS;
use hugit_ledger::journal::persist::{Journal, JournalKey, JournalStore};
use hugit_ledger::journal::resume::{ResumeError, ctx_resume, ctx_resume_from_journal};

/// ① journal persisted as tenant-private object bound to ws/intent.
///
/// Asserts:
/// - A journal carries its tenant/workspace/intent binding in its key.
/// - Storing it under tenant-A and then requesting it under tenant-B is
///   refused (tenant-private scope, fail-closed).
/// - Two different tenants with the same ws/intent produce separate,
///   inaccessible journals (no cross-tenant read).
#[test]
fn item_1_journal_tenant_private_bound() {
    // 1a. Journal is structurally bound to (tenant, workspace, intent).
    let key = JournalKey::new("tenant-alice", "ws-1", "intent-x");
    let mut journal = Journal::new(key.clone());
    journal.append(1_000_000, "agent:alice", "started the session");
    journal.append(1_001_000, "agent:alice", "read 2 files");

    assert_eq!(journal.key.tenant_id, "tenant-alice");
    assert_eq!(journal.key.workspace_id, "ws-1");
    assert_eq!(journal.key.intent_id, "intent-x");
    assert_eq!(journal.entries.len(), 2, "both entries appended");
    assert_eq!(journal.entries[0].seq, 1);
    assert_eq!(journal.entries[1].seq, 2);

    // 1b. Store it and retrieve it under the correct tenant — succeeds.
    let mut store = JournalStore::new();
    store.put(journal.clone());

    let retrieved = store
        .open("tenant-alice", "ws-1", "intent-x")
        .expect("correct tenant can retrieve the journal");
    assert_eq!(retrieved.key, key, "binding round-trips through the store");
    assert_eq!(
        retrieved.entries.len(),
        2,
        "entries round-trip through the store"
    );

    // 1c. Requesting the same ws/intent under a DIFFERENT tenant is refused.
    let cross_tenant_err = store.open("tenant-bob", "ws-1", "intent-x");
    assert!(
        cross_tenant_err.is_err(),
        "cross-tenant access must be refused (tenant-private scope)"
    );

    // 1d. Two different tenants with the SAME ws/intent are separate objects.
    let key_bob = JournalKey::new("tenant-bob", "ws-1", "intent-x");
    let mut journal_bob = Journal::new(key_bob.clone());
    journal_bob.append(2_000_000, "agent:bob", "bob's session");
    store.put(journal_bob);

    // Alice's journal is unaffected by Bob's.
    let alice_again = store
        .open("tenant-alice", "ws-1", "intent-x")
        .expect("alice's journal still accessible");
    assert_eq!(
        alice_again.entries.len(),
        2,
        "alice's journal is untouched by bob's write"
    );

    // Bob can read his own.
    let bob_retrieved = store
        .open("tenant-bob", "ws-1", "intent-x")
        .expect("bob can read his own journal");
    assert_eq!(bob_retrieved.key.tenant_id, "tenant-bob");
    assert_eq!(bob_retrieved.entries.len(), 1);
}

/// ② post-crash `ctx resume` reconstructs session within supported horizon.
///
/// Uses the `within_horizon_fixture`: a journal whose last event was 1 hour
/// ago, well within the 7-day window.
///
/// Asserts:
/// - `ctx_resume` succeeds and returns a `ReconstructedContext`.
/// - The reconstruction carries the same entries as the journal.
/// - The binding (tenant/workspace/intent) is preserved in the reconstruction.
/// - The `last_recorded_at` and `horizon_ms` fields are correct.
#[test]
fn item_2_resume_within_horizon() {
    let (journal, now_ms) = within_horizon_fixture();

    // 2a. Resume via store.
    let mut store = JournalStore::new();
    store.put(journal.clone());

    let ctx = ctx_resume(
        &store,
        &journal.key.tenant_id,
        &journal.key.workspace_id,
        &journal.key.intent_id,
        now_ms,
    )
    .expect("within-horizon resume must succeed");

    // Binding preserved.
    assert_eq!(ctx.tenant_id, journal.key.tenant_id);
    assert_eq!(ctx.workspace_id, journal.key.workspace_id);
    assert_eq!(ctx.intent_id, journal.key.intent_id);

    // All entries reconstructed.
    assert_eq!(
        ctx.entries.len(),
        journal.entries.len(),
        "reconstruction carries all journal entries"
    );
    assert_eq!(
        ctx.entries, journal.entries,
        "entries are identical to the original journal"
    );

    // Horizon metadata.
    assert_eq!(ctx.horizon_ms, DEFAULT_HORIZON_MS);
    assert_eq!(
        ctx.last_recorded_at,
        journal.last_recorded_at().unwrap(),
        "last_recorded_at matches the final journal entry"
    );

    // 2b. Age is strictly within the horizon.
    let age_ms = now_ms.saturating_sub(ctx.last_recorded_at);
    assert!(
        age_ms <= DEFAULT_HORIZON_MS,
        "fixture age {age_ms}ms must be ≤ horizon {DEFAULT_HORIZON_MS}ms"
    );

    // 2c. Direct variant (no store) gives identical result.
    let ctx_direct = ctx_resume_from_journal(&journal, now_ms)
        .expect("direct within-horizon resume must succeed");
    assert_eq!(
        ctx, ctx_direct,
        "store and direct paths give identical results"
    );
}

/// ③ beyond-horizon resume refused/degraded as documented.
///
/// Uses the `beyond_horizon_fixture`: a journal whose last event was 8 days
/// ago, exceeding the 7-day DEFAULT_HORIZON_MS.
///
/// Asserts:
/// - `ctx_resume` returns `Err(ResumeError::BeyondHorizon { age_ms, horizon_ms })`.
/// - `age_ms` > `horizon_ms` (the refusal is correctly attributed).
/// - The error message cites the documented refusal (catalog §D + whitepaper
///   §13 risk 2) — NOT a silent stale reconstruction.
/// - Resuming a journal that IS within horizon is NOT refused (control case).
#[test]
fn item_3_beyond_horizon_refused_or_degraded() {
    let (journal, now_ms) = beyond_horizon_fixture();

    // 3a. Beyond-horizon resume is refused.
    let mut store = JournalStore::new();
    store.put(journal.clone());

    let err = ctx_resume(
        &store,
        &journal.key.tenant_id,
        &journal.key.workspace_id,
        &journal.key.intent_id,
        now_ms,
    )
    .expect_err("beyond-horizon resume MUST be refused");

    match err {
        ResumeError::BeyondHorizon { age_ms, horizon_ms } => {
            assert!(
                age_ms > horizon_ms,
                "age {age_ms}ms must exceed horizon {horizon_ms}ms"
            );
            assert_eq!(
                horizon_ms, DEFAULT_HORIZON_MS,
                "refused with the standard horizon"
            );
        }
        other => panic!("expected BeyondHorizon, got {other:?}"),
    }

    // 3b. The error Display cites the documented refusal strings.
    let err_display = ctx_resume_from_journal(&journal, now_ms)
        .expect_err("direct beyond-horizon must also be refused")
        .to_string();
    assert!(
        err_display.contains("documented refusal"),
        "error message must cite 'documented refusal'; got: {err_display}"
    );
    assert!(
        err_display.contains("catalog") || err_display.contains("whitepaper"),
        "error message must reference catalog or whitepaper; got: {err_display}"
    );

    // 3c. Control: a within-horizon journal is NOT refused.
    let (journal_within, now_within) = within_horizon_fixture();
    let within_result = ctx_resume_from_journal(&journal_within, now_within);
    assert!(
        within_result.is_ok(),
        "within-horizon control case must not be refused"
    );

    // 3d. Edge: exactly at the horizon boundary (age == horizon_ms) is allowed.
    let (journal_edge, _) = within_horizon_fixture();
    let last = journal_edge.last_recorded_at().unwrap();
    let now_edge = last + DEFAULT_HORIZON_MS; // exactly at boundary
    let edge_result = ctx_resume_from_journal(&journal_edge, now_edge);
    assert!(
        edge_result.is_ok(),
        "age exactly equal to horizon must be within-horizon (≤ is the boundary)"
    );

    // 3e. One millisecond beyond the boundary is refused.
    let now_just_over = last + DEFAULT_HORIZON_MS + 1;
    let just_over_result = ctx_resume_from_journal(&journal_edge, now_just_over);
    assert!(just_over_result.is_err(), "age horizon+1ms must be refused");
}
