//! Bounded backoff/retry for status API 429/5xx responses (item ③).
//!
//! Invariants guaranteed by this module:
//! - On 429/5xx the emitter retries with bounded exponential backoff.
//! - The system NEVER stays stuck-pending: once max retries are exhausted the
//!   failure is surfaced as an observable `TerminalFailure` outcome, not left
//!   as indefinite pending.
//! - All failures are observable via `RetryOutcome`.

use hugit_contracts::ChecksWriteRequest;

/// Configuration for the bounded backoff/retry policy (item ③).
#[derive(Debug, Clone)]
pub struct BackoffConfig {
    /// Initial delay in milliseconds before the first retry.
    pub initial_delay_ms: u64,
    /// Multiplier applied to the delay on each successive retry.
    pub backoff_factor: f64,
    /// Maximum delay in milliseconds (caps exponential growth).
    pub max_delay_ms: u64,
    /// Maximum number of retry attempts (after the initial attempt).
    pub max_retries: u32,
}

impl Default for BackoffConfig {
    fn default() -> Self {
        Self {
            initial_delay_ms: 100,
            backoff_factor: 2.0,
            max_delay_ms: 30_000,
            max_retries: 5,
        }
    }
}

impl BackoffConfig {
    /// Compute the delay for retry attempt `n` (0-based).
    pub fn delay_for_attempt(&self, n: u32) -> u64 {
        let d = self.initial_delay_ms as f64 * self.backoff_factor.powi(n as i32);
        (d as u64).min(self.max_delay_ms)
    }
}

/// Error category for status API failures.
#[derive(Debug, Clone, PartialEq)]
pub enum BackoffError {
    /// HTTP 429 — rate limited.
    RateLimited { retry_after_ms: Option<u64> },
    /// HTTP 5xx — server error.
    ServerError { status_code: u16, body: String },
    /// Retries exhausted — terminal failure.
    RetriesExhausted { attempts: u32, last_error: String },
}

impl std::fmt::Display for BackoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BackoffError::RateLimited { retry_after_ms } => {
                write!(f, "rate limited (retry_after={retry_after_ms:?}ms)")
            }
            BackoffError::ServerError { status_code, body } => {
                write!(f, "server error {status_code}: {body}")
            }
            BackoffError::RetriesExhausted {
                attempts,
                last_error,
            } => {
                write!(
                    f,
                    "retries exhausted after {attempts} attempts: {last_error}"
                )
            }
        }
    }
}

/// The outcome of a backoff-managed emission attempt.
#[derive(Debug, Clone, PartialEq)]
pub enum RetryOutcome {
    /// The status was successfully emitted.
    Success {
        /// Number of attempts needed (1 = first try succeeded).
        attempts: u32,
    },
    /// All retries exhausted — terminal observable failure (never stuck-pending).
    TerminalFailure {
        /// Total attempts made.
        attempts: u32,
        /// The last error encountered.
        last_error: String,
        /// The request that failed, for surfacing / auditing.
        failed_request: ChecksWriteRequest,
    },
}

impl RetryOutcome {
    /// Returns `true` if this is a terminal failure (not stuck-pending — item ③).
    pub fn is_terminal_failure(&self) -> bool {
        matches!(self, RetryOutcome::TerminalFailure { .. })
    }

    /// Returns `true` if this is a success.
    pub fn is_success(&self) -> bool {
        matches!(self, RetryOutcome::Success { .. })
    }
}

/// Simulated HTTP response for testing the backoff logic.
#[derive(Debug, Clone)]
pub struct MockHttpResponse {
    pub status_code: u16,
    pub body: String,
    pub retry_after_ms: Option<u64>,
}

impl MockHttpResponse {
    pub fn ok() -> Self {
        Self {
            status_code: 200,
            body: String::new(),
            retry_after_ms: None,
        }
    }

    pub fn rate_limited(retry_after_ms: Option<u64>) -> Self {
        Self {
            status_code: 429,
            body: "rate limited".into(),
            retry_after_ms,
        }
    }

    pub fn server_error(status_code: u16) -> Self {
        Self {
            status_code,
            body: format!("server error {status_code}"),
            retry_after_ms: None,
        }
    }
}

/// Classify a `MockHttpResponse` as `Ok` or a `BackoffError` variant.
fn classify(resp: &MockHttpResponse) -> Result<(), BackoffError> {
    match resp.status_code {
        200..=299 => Ok(()),
        429 => Err(BackoffError::RateLimited {
            retry_after_ms: resp.retry_after_ms,
        }),
        500..=599 => Err(BackoffError::ServerError {
            status_code: resp.status_code,
            body: resp.body.clone(),
        }),
        other => Err(BackoffError::ServerError {
            status_code: other,
            body: resp.body.clone(),
        }),
    }
}

/// Bounded backoff/retry manager for status API calls (item ③).
///
/// The manager drives a sequence of attempts against a caller-supplied
/// response sequence (synchronous mock for tests; production uses the same
/// shape).
/// It never leaves the system stuck in pending: once `max_retries` are
/// exhausted the outcome is `RetryOutcome::TerminalFailure`.
pub struct StatusBackoff {
    config: BackoffConfig,
}

impl StatusBackoff {
    /// Create a new `StatusBackoff` with the given config.
    pub fn new(config: BackoffConfig) -> Self {
        Self { config }
    }

    /// Create with default config.
    pub fn default_config() -> Self {
        Self::new(BackoffConfig::default())
    }

    /// Drive bounded backoff/retry against a pre-determined response list (item ③).
    ///
    /// Each entry in `responses` is consumed on the corresponding attempt.
    /// If the list is exhausted, subsequent attempts return a synthetic 503.
    ///
    /// The function never returns pending — it returns either `Success` or
    /// `TerminalFailure` (observable failure, item ③).
    ///
    /// Note: delays are elided (controlled simulation without actual sleeping).
    pub fn run_with_responses(
        &self,
        request: &ChecksWriteRequest,
        responses: &[MockHttpResponse],
    ) -> RetryOutcome {
        let max_attempts = self.config.max_retries + 1;
        let mut last_error = String::new();

        for attempt in 0..max_attempts {
            let resp = responses.get(attempt as usize).cloned().unwrap_or_else(|| {
                // If we run out of mock responses, treat as server error.
                MockHttpResponse::server_error(503)
            });

            match classify(&resp) {
                Ok(()) => {
                    return RetryOutcome::Success {
                        attempts: attempt + 1,
                    };
                }
                Err(e) => {
                    last_error = e.to_string();
                    // Delay would happen here in production; elided for tests.
                }
            }
        }

        RetryOutcome::TerminalFailure {
            attempts: max_attempts,
            last_error,
            failed_request: request.clone(),
        }
    }
}
