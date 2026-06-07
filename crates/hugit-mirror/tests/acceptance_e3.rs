//! WP-E3 acceptance oracle — status / badge compat emitter.
//!
//! Items:
//!   ① `item_1_checks_appear_as_github_statuses`
//!   ② `item_2_badge_staleness_bound`
//!   ② `item_2_badge_api_down_last_known`
//!   ③ `item_3_api_429_retry_backoff`
//!   ③ `item_3_no_stuck_pending_observable`
//!
//! Live-GitHub items: env HUGIT_GH_TEST_REPO=humangr-labs/hugit-fleet-syn-1 is
//! always set by run.sh. Items that require live App-JWT are run when the env
//! is set; they produce PARTIAL/blocked output (not fake) when the installation
//! is unreachable.
//!
//! # Contract deps (consumed, never modified)
//! - `hugit_contracts::{CheckResult}` (frozen by WP-00)
//! - `hugit_app::ChecksClient` (frozen by WP-B1)
//! - `hugit_mirror::status::{StatusEmitter, BadgeState, BadgeStatus,
//!    STALENESS_BOUND_MS, StatusBackoff, BackoffConfig, MockHttpResponse,
//!    RetryOutcome}`

use hugit_contracts::CheckResult;
use hugit_mirror::status::{
    BackoffConfig, BadgeState, BadgeStatus, MockHttpResponse, RetryOutcome, STALENESS_BOUND_MS,
    StatusBackoff, StatusEmitter,
};

// ── helpers ──────────────────────────────────────────────────────────────────

fn dummy_result(memo_key: &str, exit: i32) -> CheckResult {
    CheckResult {
        memo_key: memo_key.to_string(),
        tree_hash: "a".repeat(64),
        def_digest: "b".repeat(64),
        toolchain_digest: "c".repeat(64),
        exit,
        artifacts: vec![],
        stdout_ref: "ref:stdout".to_string(),
        stderr_ref: "ref:stderr".to_string(),
        duration_ms: 42,
        runner_ref: "runner-1".to_string(),
        produced_at: 0,
    }
}

fn dummy_write_request() -> hugit_contracts::ChecksWriteRequest {
    hugit_contracts::ChecksWriteRequest {
        repo: "humangr-labs/hugit-fleet-syn-1".to_string(),
        head_sha: "a".repeat(40),
        check_name: "hugit/test-check".to_string(),
        status: "completed".to_string(),
        conclusion: Some("success".to_string()),
        summary: "test".to_string(),
        output_ref: "ref:out".to_string(),
    }
}

// ── ① checks appear as GitHub statuses ───────────────────────────────────────
#[test]
fn item_1_checks_appear_as_github_statuses() {
    // A CheckResult with exit=0 must produce a "success" completed check run.
    let emitter = StatusEmitter::new_local("humangr-labs/hugit-fleet-syn-1");
    let result_ok = dummy_result("ci/build", 0);

    let payload = emitter
        .project(&result_ok, Some(&"a".repeat(40)))
        .expect("project must succeed for valid CheckResult");

    assert_eq!(payload.request.repo, "humangr-labs/hugit-fleet-syn-1");
    assert_eq!(payload.request.check_name, "ci/build");
    assert_eq!(payload.request.status, "completed");
    assert_eq!(payload.request.conclusion.as_deref(), Some("success"));

    // exit=1 → failure conclusion
    let result_fail = dummy_result("ci/lint", 1);
    let payload_fail = emitter
        .project(&result_fail, Some(&"b".repeat(40)))
        .expect("project must succeed for failing CheckResult");

    assert_eq!(payload_fail.request.conclusion.as_deref(), Some("failure"));

    // emit() must succeed in local mode (no real HTTP). An installation token is
    // required even in local mode: the checks client fail-closes on an absent
    // token (None → TokenRevoked) per the app-uninstall security fix, so a valid
    // token must be supplied — passing None here previously relied on the old
    // silently-accept-None behavior that the fix correctly removed.
    let emitted = emitter
        .emit(
            &result_ok,
            Some(&"a".repeat(40)),
            Some("ghs_local_test_installation_token"),
        )
        .expect("emit must succeed in local mode");
    assert_eq!(emitted.request.status, "completed");
}

// ── ② badge staleness bound ───────────────────────────────────────────────────
#[test]
fn item_2_badge_staleness_bound() {
    // Fresh: age < STALENESS_BOUND_MS → fresh=true, stale=false
    let now_ms: u64 = 100_000;
    let data_at: u64 = now_ms - (STALENESS_BOUND_MS / 2); // half the bound
    let fresh_badge = BadgeState::from_live("ci/build", BadgeStatus::Passing, data_at, now_ms);

    assert!(
        fresh_badge.fresh,
        "badge within staleness bound must be fresh"
    );
    assert!(!fresh_badge.stale, "fresh badge must not be stale");
    assert!(!fresh_badge.api_down, "live badge must not have api_down");
    assert!(!fresh_badge.last_known, "live badge must not be last_known");

    // Stale: age > STALENESS_BOUND_MS → fresh=false, stale=true
    let old_data_at: u64 = now_ms.saturating_sub(STALENESS_BOUND_MS + 1_000);
    let stale_badge = BadgeState::from_live("ci/build", BadgeStatus::Passing, old_data_at, now_ms);

    assert!(
        stale_badge.stale,
        "badge older than staleness bound must be stale"
    );
    assert!(
        !stale_badge.fresh,
        "stale badge must not be fresh (never silent-wrong)"
    );

    // The staleness bound is a published constant — must be > 0.
    const { assert!(STALENESS_BOUND_MS > 0) };

    // Text render includes "stale" annotation when stale.
    let text = stale_badge.render_text();
    assert!(
        text.contains("stale"),
        "stale badge render must contain 'stale': {text}"
    );
}

// ── ② badge API-down: last-known + observable staleness, never silent wrong ───
#[test]
fn item_2_badge_api_down_last_known() {
    let now_ms: u64 = 200_000;
    let data_at: u64 = 100_000; // 100 s ago, well beyond bound

    // Case A: API down, last-known state exists → last_known=true, stale=true, fresh=false.
    let badge = BadgeState::from_last_known("ci/build", BadgeStatus::Passing, data_at, now_ms);

    assert!(
        badge.api_down,
        "badge from last-known must set api_down=true"
    );
    assert!(
        badge.last_known,
        "badge from last-known must set last_known=true"
    );
    assert!(
        badge.stale,
        "last-known badge must always be stale (observable)"
    );
    assert!(
        !badge.fresh,
        "last-known badge must never be fresh (no silent-wrong)"
    );
    assert_eq!(
        badge.status,
        BadgeStatus::Passing,
        "last-known status must be preserved"
    );

    // The render must surface the staleness visibly.
    let text = badge.render_text();
    assert!(
        text.contains("stale"),
        "api-down badge render must annotate staleness: {text}"
    );

    // Case B: API down, NO last-known state → Unknown status, still observable.
    let unknown_badge = BadgeState::unknown_api_down("ci/build", now_ms);

    assert_eq!(unknown_badge.status, BadgeStatus::Unknown);
    assert!(unknown_badge.api_down);
    assert!(!unknown_badge.fresh);
    assert!(unknown_badge.stale);
    // age is u64::MAX (infinite staleness)
    assert_eq!(unknown_badge.age_ms, u64::MAX);
}

// ── ③ 429/5xx retry w/ backoff → eventually true state ───────────────────────
#[test]
fn item_3_api_429_retry_backoff() {
    let config = BackoffConfig {
        initial_delay_ms: 10,
        backoff_factor: 2.0,
        max_delay_ms: 1_000,
        max_retries: 4,
    };
    let backoff = StatusBackoff::new(config.clone());
    let req = dummy_write_request();

    // Scenario: 2× 429 then success → outcome is Success with 3 attempts.
    let responses = vec![
        MockHttpResponse::rate_limited(Some(50)),
        MockHttpResponse::rate_limited(Some(50)),
        MockHttpResponse::ok(),
    ];
    let outcome = backoff.run_with_responses(&req, &responses);
    assert!(
        outcome.is_success(),
        "should succeed after 2 rate-limited retries: {outcome:?}"
    );
    if let RetryOutcome::Success { attempts } = outcome {
        assert_eq!(
            attempts, 3,
            "should take 3 attempts (2 failures + 1 success)"
        );
    }

    // Scenario: 1× 503 then success → Success in 2 attempts.
    let responses2 = vec![MockHttpResponse::server_error(503), MockHttpResponse::ok()];
    let outcome2 = backoff.run_with_responses(&req, &responses2);
    assert!(
        outcome2.is_success(),
        "should succeed after 5xx retry: {outcome2:?}"
    );

    // Delay computation is monotonic and capped.
    for i in 0..10u32 {
        let d = config.delay_for_attempt(i);
        assert!(
            d <= config.max_delay_ms,
            "delay must not exceed max_delay_ms"
        );
    }
    // Delays grow (until cap).
    let d0 = config.delay_for_attempt(0);
    let d1 = config.delay_for_attempt(1);
    assert!(d1 >= d0, "delay must be non-decreasing");
}

// ── ③ no stuck-pending; failures observable ───────────────────────────────────
#[test]
fn item_3_no_stuck_pending_observable() {
    let config = BackoffConfig {
        initial_delay_ms: 10,
        backoff_factor: 2.0,
        max_delay_ms: 1_000,
        max_retries: 2, // only 2 retries → 3 total attempts
    };
    let backoff = StatusBackoff::new(config);
    let req = dummy_write_request();

    // All attempts fail → must yield TerminalFailure (not pending forever).
    let all_fail: Vec<MockHttpResponse> = vec![
        MockHttpResponse::server_error(503),
        MockHttpResponse::server_error(503),
        MockHttpResponse::server_error(503),
    ];
    let outcome = backoff.run_with_responses(&req, &all_fail);

    assert!(
        outcome.is_terminal_failure(),
        "exhausted retries must yield TerminalFailure (no stuck-pending): {outcome:?}"
    );

    if let RetryOutcome::TerminalFailure {
        attempts,
        last_error,
        failed_request,
    } = &outcome
    {
        assert_eq!(*attempts, 3, "should report all 3 attempts");
        assert!(
            !last_error.is_empty(),
            "TerminalFailure must carry a non-empty error description"
        );
        assert_eq!(
            failed_request.repo, req.repo,
            "TerminalFailure must preserve the failed request for observability"
        );
    }

    // Verify that the outcome is NOT pending (invariant: no stuck-pending state).
    // RetryOutcome has no Pending variant — the type itself enforces this.
    assert!(
        !outcome.is_success(),
        "exhausted retries must not be Success"
    );
}
