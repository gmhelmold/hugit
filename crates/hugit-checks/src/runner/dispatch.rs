//! The lease-scoped check ORCHESTRATION (WP-Wave-E-PR3).
//!
//! This is the glue that turns the lease client's individual verbs
//! ([`LeaseClient::acquire`] / `exec` / `poll_meta` / `close`) into a single
//! fail-closed run: acquire a runner lease, dispatch the check, collect the
//! result + the §13.1 per-job metrics, and **always** release the lease — even
//! when the exec or the collect fails. It is the orchestration half of the same
//! seam [`crate::runner::lease_exec::LiveBoxRunnerExecutor`] now wires.
//!
//! ## What it returns
//!
//! [`exec_and_collect`] assembles a frozen [`CheckResult`] from the runner's
//! result-envelope meta (stamping the supplied memo key + three axes onto the
//! record, identical to the local executor's contract) AND the §13.1
//! [`IntentMetrics`] (via [`crate::runner::metrics`]). [`dispatch_check`] wraps
//! that with the acquire/close lifecycle.
//!
//! ## Secret discipline
//!
//! The PAT never leaves the private [`RunnerConfig`] inside the [`LeaseClient`]:
//! this module only ever holds a `&LeaseClient`, mirrors the lease client's
//! fail-closed posture, and surfaces a lease-client failure as the
//! [`RunnerExecError::Lease`] wrapper (whose inner [`RunnerError`] is itself
//! secret-free by construction).
//!
//! ## Hermetic
//!
//! Every line here is proven against the fake transport (no socket is opened).
//! The only code that touches the network is `UreqRunnerTransport`, which lives
//! behind the same `RunnerTransport` seam the lease client already uses.

use hugit_contracts::check_result::Artifact;
use hugit_contracts::{CheckDef, CheckResult, IntentMetrics, RunnerLease, RunnerState};
use serde::Deserialize;

use crate::runner::lease_client::{AcquireLeaseRequest, LeaseClient, RunnerTransport};
use crate::runner::lease_exec::RunnerExecError;
use crate::runner::metrics::RunnerJobMetrics;

/// The runner's result-envelope META (`GET .../envelope/meta`), interpreted.
///
/// PR1 left `poll_meta` returning a raw JSON value on purpose; THIS is the typed
/// interpretation (PR3). A PERMISSIVE projection — deliberately NOT
/// `deny_unknown_fields` — so the runner may carry envelope-level extras
/// (status, attestation refs, …) that the forge does not consume here. The
/// frozen [`CheckResult`] is then BUILT from these fields plus the
/// caller-supplied memo key/axes, never decoded directly (the frozen type is
/// `deny_unknown_fields`).
#[derive(Debug, Clone, Deserialize)]
struct RunnerResultEnvelope {
    /// Process exit code (0 = success). The VERDICT — never defaulted.
    exit: i32,
    /// Output artifacts: (path, digest) pairs.
    #[serde(default)]
    artifacts: Vec<Artifact>,
    /// Content-addressed ref to captured stdout blob.
    #[serde(default)]
    stdout_ref: String,
    /// Content-addressed ref to captured stderr blob.
    #[serde(default)]
    stderr_ref: String,
    /// Wall-clock duration of the check, ms.
    #[serde(default)]
    duration_ms: u64,
    /// Reference to the runner that executed this check.
    #[serde(default)]
    runner_ref: String,
    /// Unix epoch ms when the result was produced.
    #[serde(default)]
    produced_at: u64,
    /// The §13.1 per-job metrics payload.
    metrics: RunnerJobMetrics,
}

/// The outcome of a lease-scoped check run: the frozen [`CheckResult`] and the
/// §13.1 [`IntentMetrics`] mapped from the runner's report.
#[derive(Debug, Clone, PartialEq)]
pub struct DispatchOutcome {
    /// The byte-identity-comparable check result (memo key + axes stamped on).
    pub result: CheckResult,
    /// The per-job metrics (ADR-0001 §2.3 inputs).
    pub metrics: IntentMetrics,
}

/// Execute a check on an ALREADY-HELD lease and collect its outcome.
///
/// Fail-closed: refuses to exec unless the lease is [`RunnerState::Held`], and
/// refuses to fabricate a result if the runner did not accept the exec. Does NOT
/// acquire or close the lease — that lifecycle is the caller's
/// ([`dispatch_check`] for the full path; the live executor for the
/// already-acquired path). Stamps the supplied memo key + three axes onto the
/// returned [`CheckResult`].
pub fn exec_and_collect<T: RunnerTransport>(
    client: &LeaseClient<T>,
    lease: &RunnerLease,
    def: &CheckDef,
    memo_key: &str,
    tree_root: &str,
    def_digest: &str,
    toolchain_digest: &str,
) -> Result<DispatchOutcome, RunnerExecError> {
    if lease.state != RunnerState::Held {
        return Err(RunnerExecError::LeaseNotHeld(lease.state.clone()));
    }

    let ack = client
        .exec(&lease.lease_id, def)
        .map_err(RunnerExecError::Lease)?;
    if !ack.accepted {
        return Err(RunnerExecError::Run(
            "runner refused the exec (accepted=false)".to_string(),
        ));
    }

    let meta = client
        .poll_meta(&lease.lease_id)
        .map_err(RunnerExecError::Lease)?;
    let envelope: RunnerResultEnvelope = serde_json::from_value(meta.0)
        .map_err(|e| RunnerExecError::Run(format!("result-envelope meta decode failed: {e}")))?;

    let RunnerResultEnvelope {
        exit,
        artifacts,
        stdout_ref,
        stderr_ref,
        duration_ms,
        runner_ref,
        produced_at,
        metrics,
    } = envelope;

    let result = CheckResult {
        memo_key: memo_key.to_string(),
        tree_hash: tree_root.to_string(),
        def_digest: def_digest.to_string(),
        toolchain_digest: toolchain_digest.to_string(),
        exit,
        artifacts,
        stdout_ref,
        stderr_ref,
        duration_ms,
        runner_ref,
        produced_at,
    };

    Ok(DispatchOutcome {
        result,
        metrics: metrics.into_intent_metrics(),
    })
}

/// Drive the full lease lifecycle for one check: acquire → exec → poll → close.
///
/// The lease is ALWAYS closed, regardless of how the exec/collect step fared
/// (fail-closed: a leaked lease holds runner capacity and a filesystem scope).
/// The run error takes precedence over a close error (the close failure is only
/// surfaced when the run itself succeeded, so a real failure is never masked by
/// a best-effort cleanup hiccup).
pub fn dispatch_check<T: RunnerTransport>(
    client: &LeaseClient<T>,
    acquire: &AcquireLeaseRequest,
    def: &CheckDef,
    memo_key: &str,
    tree_root: &str,
    def_digest: &str,
    toolchain_digest: &str,
) -> Result<DispatchOutcome, RunnerExecError> {
    let lease = client.acquire(acquire).map_err(RunnerExecError::Lease)?;
    let lease_id = lease.lease_id.clone();

    let outcome = exec_and_collect(
        client,
        &lease,
        def,
        memo_key,
        tree_root,
        def_digest,
        toolchain_digest,
    );

    // ALWAYS close — even when the run errored. Explicit (not a Drop guard) so
    // the close call is observable in the hermetic transport assertions.
    let close = client.close(&lease_id).map_err(RunnerExecError::Lease);

    match outcome {
        Ok(o) => {
            close?;
            Ok(o)
        }
        // The run error wins; the lease was still closed above.
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runner::lease_client::{ExecAck, RunnerConfig, RunnerError};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    const SENTINEL_PAT: &str = "pat-SUPER-SECRET-do-not-leak-7f3a";

    #[derive(Debug, Clone)]
    struct FakeCall {
        method: &'static str,
        url: String,
    }

    /// In-memory transport: FIFO queued `(status, body)` responses, records every
    /// call. NEVER opens a socket (mirrors the lease-client fake).
    #[derive(Debug, Default)]
    struct FakeTransport {
        responses: Mutex<VecDeque<(u16, Vec<u8>)>>,
        calls: Mutex<Vec<FakeCall>>,
    }

    impl FakeTransport {
        fn with_responses(responses: Vec<(u16, Vec<u8>)>) -> Self {
            Self {
                responses: Mutex::new(responses.into()),
                calls: Mutex::new(Vec::new()),
            }
        }
        fn next(&self) -> Result<(u16, Vec<u8>), RunnerError> {
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .ok_or_else(|| RunnerError::Transport("fake: no queued response".into()))
        }
        fn calls(&self) -> Vec<FakeCall> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl RunnerTransport for FakeTransport {
        fn post(
            &self,
            url: &str,
            _bearer: &str,
            _body: &[u8],
        ) -> Result<(u16, Vec<u8>), RunnerError> {
            self.calls.lock().unwrap().push(FakeCall {
                method: "POST",
                url: url.to_string(),
            });
            self.next()
        }
        fn get(&self, url: &str, _bearer: &str) -> Result<(u16, Vec<u8>), RunnerError> {
            self.calls.lock().unwrap().push(FakeCall {
                method: "GET",
                url: url.to_string(),
            });
            self.next()
        }
    }

    fn sample_lease(lease_id: &str, state: RunnerState) -> RunnerLease {
        RunnerLease {
            lease_id: lease_id.to_string(),
            principal_chain: vec!["agent:tester".to_string()],
            path_set: vec!["/work".to_string()],
            expiry: 1_000_000,
            net_policy: "deny-all".to_string(),
            tmp_root: "/tmp/runner".to_string(),
            state,
        }
    }

    fn sample_def() -> CheckDef {
        CheckDef {
            def_digest: "a".repeat(64),
            command: "cargo test".to_string(),
            inputs: vec!["src/**".to_string()],
            toolchain_ref: "rust-1.96".to_string(),
            env_manifest: "blob:abc".to_string(),
            glob_set: vec!["**/*.rs".to_string()],
        }
    }

    fn acquire_req() -> AcquireLeaseRequest {
        AcquireLeaseRequest {
            principal_chain: vec!["agent:tester".to_string()],
            path_set: vec!["/work".to_string()],
            net_policy: "deny-all".to_string(),
            ttl_ms: 60_000,
        }
    }

    /// A complete result-envelope meta body: the CheckResult fields + §13.1
    /// metrics (with a runner-specific extra to prove it is tolerated).
    fn envelope_body() -> Vec<u8> {
        br#"{
            "exit": 0,
            "artifacts": [{"path": "check.out", "digest": "deadbeef"}],
            "stdout_ref": "blob:out",
            "stderr_ref": "blob:err",
            "duration_ms": 4200,
            "runner_ref": "runner:box-7",
            "produced_at": 1718000000000,
            "status": "done",
            "metrics": {
                "tokens": {"input": 1000, "output": 200, "cache_read": 50, "cache_write": 25, "total": 1275},
                "wall_ms": 9000,
                "active_ms": 7500,
                "tool_calls": 3,
                "tool_breakdown": [{"tool": "Bash", "count": 3}],
                "model_turns": 2,
                "cost_usd_micros": 4281900,
                "cpu_ms": 1234
            }
        }"#
        .to_vec()
    }

    fn client(transport: FakeTransport) -> LeaseClient<FakeTransport> {
        let config = RunnerConfig::new("https://runner.example/", SENTINEL_PAT).unwrap();
        LeaseClient::with_transport(config, transport)
    }

    #[test]
    fn full_acquire_exec_poll_close_returns_result_and_metrics() {
        let lease = sample_lease("lease-abc123", RunnerState::Held);
        let ack = ExecAck {
            lease_id: "lease-abc123".to_string(),
            accepted: true,
        };
        let transport = FakeTransport::with_responses(vec![
            (201, serde_json::to_vec(&lease).unwrap()), // acquire
            (202, serde_json::to_vec(&ack).unwrap()),   // exec
            (200, envelope_body()),                     // poll_meta
            (204, Vec::new()),                          // close
        ]);
        let client = client(transport);

        let out = dispatch_check(
            &client,
            &acquire_req(),
            &sample_def(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
        )
        .expect("orchestration succeeds");

        // CheckResult: envelope fields + the stamped memo key/axes.
        assert_eq!(out.result.memo_key, "f".repeat(64));
        assert_eq!(out.result.tree_hash, "1".repeat(64));
        assert_eq!(out.result.def_digest, "2".repeat(64));
        assert_eq!(out.result.toolchain_digest, "3".repeat(64));
        assert_eq!(out.result.exit, 0);
        assert_eq!(out.result.artifacts.len(), 1);
        assert_eq!(out.result.artifacts[0].path, "check.out");
        assert_eq!(out.result.artifacts[0].digest, "deadbeef");
        assert_eq!(out.result.duration_ms, 4200);
        assert_eq!(out.result.runner_ref, "runner:box-7");

        // IntentMetrics mapped from §13.1.
        assert_eq!(out.metrics.tokens.total, 1275);
        assert_eq!(out.metrics.tokens.cache_read, 50);
        assert_eq!(out.metrics.cost_usd_micros, 4281900);
        assert_eq!(out.metrics.tool_calls, 3);
        assert_eq!(out.metrics.tool_breakdown.len(), 1);

        // The lifecycle hit all four endpoints in order, ending in close.
        let calls = client.transport().calls();
        assert_eq!(calls.len(), 4);
        assert_eq!(calls[0].url, "https://runner.example/v1/leases");
        assert_eq!(
            calls[1].url,
            "https://runner.example/v1/leases/lease-abc123/exec"
        );
        assert_eq!(
            calls[2].url,
            "https://runner.example/v1/leases/lease-abc123/envelope/meta"
        );
        assert_eq!(
            calls[3].url,
            "https://runner.example/v1/leases/lease-abc123/close"
        );
        assert_eq!(calls[3].method, "POST");
    }

    #[test]
    fn lease_is_always_closed_even_when_exec_errors() {
        let lease = sample_lease("lease-err", RunnerState::Held);
        let transport = FakeTransport::with_responses(vec![
            (201, serde_json::to_vec(&lease).unwrap()), // acquire
            (500, Vec::new()),                          // exec → terminal Status(500)
            (204, Vec::new()),                          // close MUST still be attempted
        ]);
        let client = client(transport);

        let err = dispatch_check(
            &client,
            &acquire_req(),
            &sample_def(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
        )
        .expect_err("exec failure surfaces");
        assert!(
            matches!(err, RunnerExecError::Lease(RunnerError::Status(500))),
            "the run error must win; got {err:?}"
        );

        // The lease was STILL closed despite the exec error.
        let calls = client.transport().calls();
        assert_eq!(calls.len(), 3);
        let closed = calls
            .iter()
            .any(|c| c.url.ends_with("/lease-err/close") && c.method == "POST");
        assert!(
            closed,
            "the lease must be closed on the error path: {calls:?}"
        );
    }

    #[test]
    fn exec_and_collect_refuses_a_non_held_lease() {
        let transport = FakeTransport::with_responses(vec![]);
        let client = client(transport);
        let lease = sample_lease("lease-x", RunnerState::Expired);
        let err = exec_and_collect(
            &client,
            &lease,
            &sample_def(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
        )
        .expect_err("must refuse an expired lease");
        assert_eq!(err, RunnerExecError::LeaseNotHeld(RunnerState::Expired));
        // Fail-closed: no transport call was made.
        assert!(client.transport().calls().is_empty());
    }

    #[test]
    fn refused_exec_is_not_fabricated() {
        // accepted=false MUST NOT be turned into a phantom CheckResult.
        let lease = sample_lease("lease-r", RunnerState::Held);
        let ack = ExecAck {
            lease_id: "lease-r".to_string(),
            accepted: false,
        };
        let transport = FakeTransport::with_responses(vec![
            (201, serde_json::to_vec(&lease).unwrap()),
            (200, serde_json::to_vec(&ack).unwrap()),
            (204, Vec::new()), // close still happens
        ]);
        let client = client(transport);
        let err = dispatch_check(
            &client,
            &acquire_req(),
            &sample_def(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
        )
        .expect_err("a refused exec is an error, not a fabricated pass");
        assert!(matches!(err, RunnerExecError::Run(_)));
        // Closed anyway.
        assert!(
            client
                .transport()
                .calls()
                .iter()
                .any(|c| c.url.ends_with("/lease-r/close"))
        );
    }

    #[test]
    fn no_pat_in_dispatch_error_rendering() {
        let lease = sample_lease("lease-s", RunnerState::Held);
        let transport = FakeTransport::with_responses(vec![
            (201, serde_json::to_vec(&lease).unwrap()),
            (401, Vec::new()), // exec rejected
            (204, Vec::new()),
        ]);
        let client = client(transport);
        let err = dispatch_check(
            &client,
            &acquire_req(),
            &sample_def(),
            "f".repeat(64).as_str(),
            "1".repeat(64).as_str(),
            "2".repeat(64).as_str(),
            "3".repeat(64).as_str(),
        )
        .unwrap_err();
        assert!(!format!("{err:?}").contains(SENTINEL_PAT));
        assert!(!format!("{err}").contains(SENTINEL_PAT));
    }
}
