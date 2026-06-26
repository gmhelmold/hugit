//! WP-Wave-E-PR5 — `hugit pr land --dispatch`: run the PR's check on the
//! CoreLink runner fabric and capture the REAL §13.1 per-job metrics into the
//! land envelope (the "real per-PR cost" killer).
//!
//! This is the FINAL wiring of the dispatch wave. It does not invent a seam — it
//! CONNECTS the two halves that already exist:
//!
//! - [`hugit_checks::runner::dispatch_check`] — acquire → exec → poll_meta →
//!   close (always releasing the lease), returning the frozen `CheckResult` plus
//!   the §13.1 [`IntentMetrics`] (the FULL token cache-split, never flattened);
//! - [`super::capture::capture_on_land`] (via [`super::settle`]) — captures the
//!   ADR-0001 context envelope on land, preserving a supplied
//!   [`IntentMetrics`] verbatim through [`EnvelopeMetricsArgs::full_metrics`].
//!
//! `--dispatch` runs the PR's check on the fabric and feeds the returned REAL
//! metrics into the captured envelope, so the land carries the measured per-job
//! cost + cache split instead of the manual / honest-zero figure.
//!
//! # Honesty law (the load-bearing rule)
//!
//! A captured cost figure must be TRUE for that specific land. Real metrics come
//! ONLY from a real dispatch — so under `--dispatch`:
//!
//! - the runner being UNCONFIGURED / UNWIRED is a **fatal error**, never a
//!   silent fall-back to honest-zero or a fabricated number (the user asked for
//!   real cost; absence is an error). The land does NOT settle and NO envelope
//!   is captured.
//! - `--dispatch` is **mutually exclusive** with the manual `--tokens` / … flags
//!   (handled at the CLI surface) — we never merge two metric sources.
//!
//! Without `--dispatch`, the land path is byte-for-byte the existing honest-zero
//! / manual-flag path ([`super::settle`]).
//!
//! # Secret discipline
//!
//! The runner PAT lives only inside the private [`hugit_checks::runner::RunnerConfig`]
//! held by the [`LeaseClient`]; it is never logged, never in `Debug`/`Display`,
//! and the [`RunnerError`] / [`RunnerExecError`] this module surfaces are
//! secret-free by construction. The porcelain error mappers below carry only the
//! error's own (secret-free) rendering.
//!
//! # The disclosed P2 seam (honest scope)
//!
//! There is no materialized tree, real `CheckDef`, or stored CI definition at the
//! porcelain land altitude — that flows from the union-test / queue engine at
//! P2 (the same seam [`super::land`] documents for the empty `tree_hash`). So the
//! [`CheckDef`] + memo axes built here are a deterministic land-time identity
//! whose ONLY consumed output is the §13.1 [`IntentMetrics`] (the real per-job
//! cost the runner reports); the stamped `CheckResult` is discarded. The wiring
//! is proven hermetically end-to-end here; live execution additionally waits on
//! the runner fabric's `/v1/leases/{id}/exec` spawn path (the runners TL's lane).

use hugit_checks::runner::{
    AcquireLeaseRequest, LeaseClient, RunnerError, RunnerExecError, RunnerTransport, dispatch_check,
};
use hugit_contracts::{CheckDef, IntentMetrics};
use hugit_refstore::EventLog;
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::capture::EnvelopeMetricsArgs;
use super::{OpenedPr, PrError, SettleArgs, find_pr_opened, settle};
use crate::porcelain::PorcelainError;

/// The egress policy a land-time check lease requests. A land-confirm check runs
/// no outbound network at the porcelain altitude → deny-all (fail-closed).
const LAND_DISPATCH_NET_POLICY: &str = "deny-all";

/// The TTL (ms) requested for a land-time check-dispatch lease.
const LAND_DISPATCH_TTL_MS: u64 = 300_000;

/// An error from the `--dispatch` land path.
///
/// Either a PR-domain refusal (the PR is not on the log, is empty, …) or a
/// runner-fabric failure (the lease lifecycle errored, or the runner is not
/// configured/wired). Under `--dispatch` a [`DispatchLandError::Runner`] is
/// FATAL — never a silent fall-back to a fabricated / honest-zero cost — and the
/// land does not settle (no envelope is captured). The wrapped runner error is
/// secret-free by construction (the PAT never appears in any `RunnerExecError`).
#[derive(Debug)]
pub enum DispatchLandError {
    /// A PR-domain refusal (`unknown_pr`, `empty_pr`, `pr_not_queued`, …).
    Pr(PrError),
    /// A runner-fabric failure (acquire / exec / poll / close, or unconfigured).
    Runner(RunnerExecError),
}

/// Land a PR with REAL per-job metrics measured on the runner fabric.
///
/// Resolves the PR (fail-closed: an unknown PR errors BEFORE any lease is
/// acquired), runs its check through [`dispatch_check`] to collect the §13.1
/// [`IntentMetrics`], then settles the PR through the SAME [`super::settle`] seam
/// the manual path uses — feeding the metrics through verbatim as
/// [`EnvelopeMetricsArgs::full_metrics`] so the captured envelope preserves the
/// full token cache split (`input / output / cache_read / cache_write / total`)
/// instead of flattening to the aggregate total.
///
/// Fail-closed honesty: if the dispatch errors (including an unwired runner), the
/// PR is NOT settled and NO `pr.landed` / `pr.envelope` record is appended — the
/// real cost is absent, which under `--dispatch` is an error, never a fabricated
/// or honest-zero stand-in.
///
/// Generic over the transport so the wiring is proven hermetically against a fake
/// transport (zero network); in production `T = UreqRunnerTransport`.
pub fn land_with_dispatch<T: RunnerTransport>(
    log: &mut EventLog,
    client: &LeaseClient<T>,
    pr_id: &str,
    recorded_at: u64,
) -> Result<Value, DispatchLandError> {
    // The pr_id is an identifier ADDRESS — scrub it STRUCTURALLY so it joins the
    // raw id `pr open` stored (the SAME rule `settle`/`land` apply: a bare
    // hex/slug address survives verbatim, a prefixed secret redacts). Resolve the
    // PR FIRST so a non-existent PR never burns a runner lease.
    let scrubbed = crate::porcelain::structural_secret_scrub(pr_id);
    let opened = find_pr_opened(log, &scrubbed).ok_or_else(|| {
        DispatchLandError::Pr(PrError::UnknownPr {
            pr_id: scrubbed.clone(),
        })
    })?;

    // Run the PR's check on the fabric — REAL §13.1 metrics, or a FATAL error
    // (never a fabricated / honest-zero fall-back under --dispatch).
    let metrics = dispatch_pr_metrics(client, &opened).map_err(DispatchLandError::Runner)?;

    // Settle with the measured metrics carried through as the full IntentMetrics
    // (cache split preserved, never flattened). Capture fires inside `settle`.
    settle(
        log,
        &SettleArgs {
            pr_id: scrubbed,
            recorded_at,
            envelope_metrics: EnvelopeMetricsArgs {
                full_metrics: Some(metrics),
                ..Default::default()
            },
        },
    )
    .map_err(DispatchLandError::Pr)
}

/// Dispatch the PR's land-time check on the runner fabric and return its §13.1
/// [`IntentMetrics`].
///
/// Drives the full lease lifecycle via [`dispatch_check`] (acquire → exec →
/// poll_meta → close, always releasing the lease). Only the metrics are
/// consumed; the stamped `CheckResult` is discarded (see the module-level
/// disclosed-seam note — there is no real tree / CI definition at the porcelain
/// land altitude).
fn dispatch_pr_metrics<T: RunnerTransport>(
    client: &LeaseClient<T>,
    opened: &OpenedPr,
) -> Result<IntentMetrics, RunnerExecError> {
    let acquire = AcquireLeaseRequest {
        principal_chain: vec![format!("pr:{}", opened.pr_id)],
        path_set: vec![],
        net_policy: LAND_DISPATCH_NET_POLICY.to_string(),
        ttl_ms: LAND_DISPATCH_TTL_MS,
    };
    let def = land_check_def(opened);
    let def_digest = def.def_digest.clone();
    let memo_key = land_memo_key(opened, &def_digest);
    let outcome = dispatch_check(
        client,
        &acquire,
        &def,
        &memo_key,
        // tree_root: honestly empty — no materialized tree at this altitude (the
        // P2 seam). The CheckResult these axes stamp onto is discarded; only the
        // §13.1 metrics are consumed.
        "",
        &def_digest,
        // toolchain_digest: unknown at the porcelain land altitude → empty.
        "",
    )?;
    Ok(outcome.metrics)
}

/// The deterministic land-time [`CheckDef`] for a PR (the disclosed-seam
/// identity — see the module note). Its `def_digest` is the SHA-256 of the
/// command; the def's only role is to give the dispatch a stable identity, the
/// consumed output is the runner's §13.1 metrics.
fn land_check_def(opened: &OpenedPr) -> CheckDef {
    let command = format!("hugit pr land --pr {}", opened.pr_id);
    CheckDef {
        def_digest: sha256_hex(command.as_bytes()),
        command,
        inputs: vec![],
        toolchain_ref: String::new(),
        env_manifest: String::new(),
        glob_set: vec![],
    }
}

/// The deterministic land-time memo key (the dispatch identity). Lowercase-hex
/// SHA-256 over a domain-tagged `(pr_id, def_digest)` so re-dispatching the same
/// PR keys identically.
fn land_memo_key(opened: &OpenedPr, def_digest: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"hugit:land-dispatch:v1\n");
    h.update(opened.pr_id.as_bytes());
    h.update(b"\n");
    h.update(def_digest.as_bytes());
    hex::encode(h.finalize())
}

/// Lowercase-hex SHA-256 of `bytes` (the canonical digest used across the repo).
fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

// ─────────────────────────────────────────────────────────────────────────────
// Porcelain error mappers — secret-free (the PAT never reaches an error string).
// ─────────────────────────────────────────────────────────────────────────────

/// The structured error for `--dispatch` combined with manual metric flags
/// (don't merge two metric sources). A usage error on stdout, exit `2`.
pub fn conflict_error() -> PorcelainError {
    PorcelainError::new(
        "dispatch_conflict",
        "--dispatch measures the real per-job cost by running the PR's check on \
         the runner fabric; it cannot be combined with the manual metric flags \
         (--tokens / --cost-usd-micros / --tool-calls / --active-ms / \
         --model-turns / --model / --context-cas / --compact-transcript-ref / \
         --verdicts-ref)",
        "either drop --dispatch and pass the measured figures yourself, or drop \
         the manual metric flags and let --dispatch measure them",
    )
}

/// Map an unconfigured-runner [`RunnerError`] (from `from_runtime`) to a clear
/// structured error. Fail-closed: the user asked for real cost and the runner is
/// not wired — that is an error, never a fabricated / honest-zero fall-back. The
/// `RunnerError` rendering is secret-free (the PAT never appears in it).
pub fn unconfigured_error(e: &RunnerError) -> PorcelainError {
    PorcelainError::new(
        "runner_not_configured",
        format!(
            "--dispatch needs a wired runner fabric to measure real per-job cost, \
             but it is not configured: {e}"
        ),
        "set HUGIT_RUNNER_HOST and provision the runner PAT (the secret file \
         ~/.hugit/secrets/runner/pat, or HUGIT_RUNNER_PAT), or land without \
         --dispatch (honest-zero / manual metrics)",
    )
}

/// Map a runner-fabric [`RunnerExecError`] (the lease lifecycle errored) to a
/// clear structured error. Fail-closed: the dispatch failed, so the land did NOT
/// settle and NO envelope was captured (never a fabricated cost). The
/// `RunnerExecError` rendering is secret-free (the PAT never appears in it).
pub fn runner_error(e: &RunnerExecError) -> PorcelainError {
    PorcelainError::new(
        "runner_dispatch_failed",
        format!(
            "--dispatch failed to run the PR's check on the runner fabric; the PR \
             was NOT landed and no cost was captured: {e}"
        ),
        "retry once the runner fabric is reachable, or land without --dispatch \
         (honest-zero / manual metrics)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pr::{AuthorKind, LandArgs, OpenArgs, land, open};
    use hugit_checks::runner::{ExecAck, RunnerConfig};
    use hugit_contracts::context_envelope::{ContextEnvelope, TokenCounts};
    use hugit_contracts::{Altitude, RunnerLease, RunnerState};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// A sentinel PAT — if it ever surfaces in an error rendering, the secret
    /// discipline is broken.
    const SENTINEL_PAT: &str = "pat-DISPATCH-SECRET-do-not-leak-9c2e";

    /// In-memory transport: FIFO `(status, body)` responses, no socket.
    #[derive(Debug, Default)]
    struct FakeTransport {
        responses: Mutex<VecDeque<(u16, Vec<u8>)>>,
    }
    impl FakeTransport {
        fn with(responses: Vec<(u16, Vec<u8>)>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
            }
        }
        fn next(&self) -> Result<(u16, Vec<u8>), RunnerError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| RunnerError::Transport("fake: no queued response".into()))
        }
    }
    impl RunnerTransport for FakeTransport {
        fn post(&self, _u: &str, _b: &str, _body: &[u8]) -> Result<(u16, Vec<u8>), RunnerError> {
            self.next()
        }
        fn get(&self, _u: &str, _b: &str) -> Result<(u16, Vec<u8>), RunnerError> {
            self.next()
        }
    }

    fn client_with(transport: FakeTransport) -> LeaseClient<FakeTransport> {
        let config = RunnerConfig::new("https://runner.example/", SENTINEL_PAT).unwrap();
        LeaseClient::with_transport(config, transport)
    }

    fn held_lease() -> RunnerLease {
        RunnerLease {
            lease_id: "lease-land-1".to_string(),
            principal_chain: vec!["pr:42".to_string()],
            path_set: vec![],
            expiry: 1_000_000,
            net_policy: LAND_DISPATCH_NET_POLICY.to_string(),
            tmp_root: "/tmp/runner".to_string(),
            state: RunnerState::Held,
        }
    }

    /// A result-envelope meta body carrying a DISTINCTIVE §13.1 cache split (with
    /// a runner-only extra to prove the permissive decode tolerates it).
    fn envelope_body() -> Vec<u8> {
        br#"{
            "exit": 0,
            "artifacts": [],
            "stdout_ref": "blob:o",
            "stderr_ref": "blob:e",
            "duration_ms": 4242,
            "runner_ref": "runner:box-land",
            "metrics": {
                "tokens": {"input": 111, "output": 222, "cache_read": 333, "cache_write": 444, "total": 1110},
                "wall_ms": 9100,
                "active_ms": 7600,
                "tool_calls": 6,
                "tool_breakdown": [{"tool": "Bash", "count": 6}],
                "model_turns": 2,
                "cost_usd_micros": 5555555,
                "cpu_ms": 4321
            }
        }"#
        .to_vec()
    }

    /// A full acquire→exec→poll→close FIFO that returns the distinctive metrics.
    fn happy_responses() -> Vec<(u16, Vec<u8>)> {
        let ack = ExecAck {
            lease_id: "lease-land-1".to_string(),
            accepted: true,
        };
        vec![
            (201, serde_json::to_vec(&held_lease()).unwrap()), // acquire
            (202, serde_json::to_vec(&ack).unwrap()),          // exec
            (200, envelope_body()),                            // poll_meta
            (204, Vec::new()),                                 // close
        ]
    }

    /// Open + queue PR "42" on a fresh log (the precondition for settle).
    fn open_and_queue() -> EventLog {
        let mut log = EventLog::new();
        open(
            &mut log,
            &OpenArgs {
                pr_id: "42".to_string(),
                campaign: "camp-dispatch".to_string(),
                author_kind: AuthorKind::Orchestrator,
                run_id: Some("run-orq".to_string()),
                principal: None,
                intent_ids: vec!["i-a".to_string(), "i-b".to_string()],
                recorded_at: 1000,
            },
        )
        .expect("open");
        land(
            &mut log,
            &LandArgs {
                pr_id: "42".to_string(),
                recorded_at: 2000,
            },
        )
        .expect("queue");
        log
    }

    /// Read the captured PR-altitude envelope off the log (the round-trip the
    /// serve read side performs), if present.
    fn captured_pr_envelope(log: &EventLog) -> Option<ContextEnvelope> {
        log.records()
            .iter()
            .filter(|r| r.kind == super::super::PR_ENVELOPE_KIND)
            .filter_map(|r| serde_json::from_str::<ContextEnvelope>(&r.payload).ok())
            .rfind(|e| e.altitude == Altitude::Pr && e.intent_id == "42")
    }

    /// (1) `land --dispatch` over the FAKE transport feeds the REAL §13.1 metrics
    /// into the captured envelope — the FULL cache split, not flattened to total.
    #[test]
    fn land_with_dispatch_captures_real_cache_split_metrics() {
        let mut log = open_and_queue();
        let client = client_with(FakeTransport::with(happy_responses()));

        let out = land_with_dispatch(&mut log, &client, "42", 3000).expect("dispatch land");
        // 2 intents + 1 pr envelope captured (the same shape the manual path uses).
        assert_eq!(out["envelopes_captured"], serde_json::json!(3), "{out}");
        assert_eq!(out["landed"], serde_json::json!(true));

        let pr_env = captured_pr_envelope(&log).expect("pr.envelope on log");
        // The FULL cache split survives verbatim — the whole point of --dispatch
        // (memo economics legibility), never flattened to `total`.
        let want = TokenCounts {
            input: 111,
            output: 222,
            cache_read: 333,
            cache_write: 444,
            total: 1110,
        };
        assert_eq!(pr_env.metrics.tokens, want, "exact cache split preserved");
        assert_eq!(pr_env.metrics.cost_usd_micros, 5_555_555, "real cost");
        assert_eq!(pr_env.metrics.tool_calls, 6);
        assert_eq!(pr_env.metrics.model_turns, 2);
        assert_eq!(pr_env.metrics.active_ms, 7_600);
        assert_eq!(pr_env.metrics.wall_ms, 9_100);
    }

    /// (2) A failed dispatch (the runner is unwired/unreachable) is FATAL: the PR
    /// is NOT settled and NO envelope is captured — never a fabricated cost.
    #[test]
    fn dispatch_failure_lands_nothing_no_fabricated_envelope() {
        let mut log = open_and_queue();
        let records_before = log.records().len();
        // acquire returns 500 → terminal Status(500); close is still attempted.
        let client = client_with(FakeTransport::with(vec![
            (500, Vec::new()),
            (204, Vec::new()),
        ]));

        let err = land_with_dispatch(&mut log, &client, "42", 3000)
            .expect_err("an unreachable runner is a fatal --dispatch error");
        assert!(
            matches!(err, DispatchLandError::Runner(_)),
            "must be a runner-fabric error, got {err:?}"
        );

        // Fail-closed: no pr.landed and no pr.envelope/intent.envelope were appended.
        let kinds: Vec<&str> = log.records().iter().map(|r| r.kind.as_str()).collect();
        assert!(
            !kinds.contains(&super::super::PR_LANDED_KIND),
            "the PR must NOT be settled on a dispatch failure: {kinds:?}"
        );
        assert!(
            !kinds.contains(&super::super::PR_ENVELOPE_KIND),
            "NO envelope may be captured on a dispatch failure: {kinds:?}"
        );
        assert!(
            captured_pr_envelope(&log).is_none(),
            "no fabricated envelope"
        );
        assert_eq!(
            log.records().len(),
            records_before,
            "a failed --dispatch appends nothing to the log"
        );
    }

    /// (3) An unknown PR errors BEFORE any lease is acquired (fail-closed) and
    /// captures nothing.
    #[test]
    fn dispatch_unknown_pr_errors_without_dispatching() {
        let mut log = EventLog::new();
        // No queued responses: if a lease were acquired, the fake would error
        // differently. The PR-domain refusal must fire first.
        let client = client_with(FakeTransport::with(vec![]));
        let err = land_with_dispatch(&mut log, &client, "does-not-exist", 3000)
            .expect_err("unknown PR is a refusal");
        assert!(
            matches!(err, DispatchLandError::Pr(PrError::UnknownPr { .. })),
            "got {err:?}"
        );
        assert_eq!(log.records().len(), 0, "nothing appended for an unknown PR");
    }

    /// (4) The PAT NEVER leaks through the dispatch error path — not in the
    /// `DispatchLandError`, not in the porcelain error it maps to.
    #[test]
    fn pat_never_leaks_in_dispatch_error() {
        let mut log = open_and_queue();
        // exec rejected with 401 (bad PAT) → terminal; close attempted.
        let client = client_with(FakeTransport::with(vec![
            (201, serde_json::to_vec(&held_lease()).unwrap()),
            (401, Vec::new()),
            (204, Vec::new()),
        ]));
        let err = land_with_dispatch(&mut log, &client, "42", 3000).unwrap_err();
        let DispatchLandError::Runner(runner) = err else {
            panic!("expected a runner error, got {err:?}");
        };
        assert!(!format!("{runner:?}").contains(SENTINEL_PAT));
        assert!(!format!("{runner}").contains(SENTINEL_PAT));
        // The porcelain rendering the CLI emits is likewise secret-free.
        let porcelain = runner_error(&runner);
        assert!(!porcelain.to_json().contains(SENTINEL_PAT));
    }

    /// (5) The unconfigured-runner mapper names the missing piece and never echoes
    /// a value — the clear, fail-closed message the CLI emits on an unwired box.
    #[test]
    fn unconfigured_error_is_clear_and_secret_free() {
        let e = RunnerError::NotConfigured("host: HUGIT_RUNNER_HOST unset/empty".into());
        let porcelain = unconfigured_error(&e);
        let json = porcelain.to_json();
        assert!(json.contains("runner_not_configured"), "{json}");
        assert!(
            json.contains("HUGIT_RUNNER_HOST"),
            "names the missing piece"
        );
        assert!(!json.contains(SENTINEL_PAT));
    }

    /// (6) Plain land (no --dispatch) is UNCHANGED: `settle` with the default
    /// (manual / honest-zero) metrics still captures an honest-zero envelope and
    /// never touches the runner fabric. Regression guard for the seam.
    #[test]
    fn plain_settle_without_dispatch_is_unchanged_honest_zero() {
        let mut log = open_and_queue();
        let out = settle(
            &mut log,
            &SettleArgs {
                pr_id: "42".to_string(),
                recorded_at: 3000,
                envelope_metrics: EnvelopeMetricsArgs::default(),
            },
        )
        .expect("plain settle");
        assert_eq!(out["envelopes_captured"], serde_json::json!(3));
        let pr_env = captured_pr_envelope(&log).expect("pr.envelope on log");
        // Honest-zero metrics — no full_metrics carrier, no runner involvement.
        assert_eq!(pr_env.metrics.tokens.total, 0);
        assert_eq!(pr_env.metrics.cost_usd_micros, 0);
        assert_eq!(pr_env.metrics.tool_calls, 0);
    }
}
