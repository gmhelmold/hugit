//! Runner-side execution under a [`RunnerLease`].
//!
//! This is the *runner* half of the same pure check function the local executor
//! (B2a) runs. Where B2a's `CheckRunner` synthesizes a [`CheckResult`] for the
//! local/client path, [`RunnerExecutor`] executes the check inside a runner
//! granted a [`RunnerLease`] (the Phase-C fabric, consumed here only as an
//! executor surface — B2b does NOT build the runner; C2 does).
//!
//! The contract that makes byte-identity (item ③) provable: a runner execution
//! produces real OUTPUT ARTIFACTS, each carried in the [`CheckResult`] as a
//! `(path, content-digest)` pair. The content digest is the canonical SHA-256
//! lowercase-hex used everywhere in the codebase — so a local and a runner
//! execution of the same deterministic check can be compared digest-for-digest,
//! not merely result-for-result.
//!
//! ## The trait/seam split (PARTIAL-over-fake is law)
//!
//! [`InProcessRunnerExecutor`] is the in-process reference executor: it runs a
//! deterministic, content-defined check and digests its real outputs. It is NOT
//! a throwaway test stub — it is the reference execution semantics, so the same
//! code proves byte-identity, non-determinism flagging, and the hit-rate meter
//! NOW. The ONLY deferred piece is the live runner BOX: [`LiveBoxRunnerExecutor`]
//! is the explicitly-marked P2 seam over `HUGIT_RUNNER_HOST`, behind the same
//! trait. When the box is provisioned that seam is filled; the surrounding
//! comparator / tracker / meter do not change.

use hugit_contracts::check_result::Artifact;
use hugit_contracts::{CheckDef, CheckResult, RunnerLease, RunnerState};
use sha2::{Digest, Sha256};

use crate::runner::dispatch::exec_and_collect;
use crate::runner::lease_client::{
    ENV_RUNNER_HOST, LeaseClient, RunnerError, RunnerTransport, UreqRunnerTransport,
};

/// Errors from a runner-side execution.
#[derive(Debug, Clone, PartialEq)]
pub enum RunnerExecError {
    /// The lease was not in a state that permits execution (expired / released /
    /// crashed). Fail-closed: a runner must hold a live lease to execute.
    LeaseNotHeld(RunnerState),
    /// The underlying check failed to even run on the runner (distinct from a
    /// check that ran and reported a non-zero `CheckResult::exit`).
    Run(String),
    /// The live runner box (`HUGIT_RUNNER_HOST`) seam is not yet wired (P2).
    /// Carries the host it WILL drive so the deferral is self-documenting.
    BoxNotWired(String),
    /// A failure from the underlying lease client (acquire / exec / poll /
    /// close). Wraps the typed [`RunnerError`] so the caller can still see a
    /// retryable `Busy` apart from a terminal `Status`, and so the wire-level
    /// failure is not flattened into an opaque string. The inner error is
    /// secret-free by construction (the PAT never appears in any `RunnerError`).
    Lease(RunnerError),
}

impl std::fmt::Display for RunnerExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RunnerExecError::LeaseNotHeld(state) => {
                write!(f, "runner lease not held (state: {state:?})")
            }
            RunnerExecError::Run(e) => write!(f, "runner execution failed: {e}"),
            RunnerExecError::BoxNotWired(host) => {
                write!(f, "live runner box seam not wired (P2): {host}")
            }
            RunnerExecError::Lease(e) => write!(f, "runner lease client error: {e}"),
        }
    }
}

impl std::error::Error for RunnerExecError {}

/// Canonical content digest of a blob: lowercase-hex SHA-256. The SAME digest
/// the rest of the codebase uses for content addressing, so an artifact digest
/// produced here is directly comparable to a local execution's.
pub fn artifact_digest(content: &[u8]) -> String {
    hex::encode(Sha256::digest(content))
}

/// Executes a check inside a runner holding a [`RunnerLease`].
///
/// The runner produces real output artifacts; each artifact's content digest is
/// stamped into the returned [`CheckResult`]. The three memo axes
/// (`tree_root` / `def_digest` / `toolchain_digest`) and the `memo_key` are
/// passed in (derived once via B2a's frozen memo-key surface) and stamped onto
/// the result so the record is self-keyed — identical to B2a's `CheckRunner`
/// contract, on the runner side.
pub trait RunnerExecutor {
    /// Execute the check ONCE on the runner and return its result, including the
    /// produced artifacts (path + content digest). Implementors MUST stamp the
    /// supplied key/axes onto the returned record.
    fn execute(
        &self,
        lease: &RunnerLease,
        def: &CheckDef,
        memo_key: &str,
        tree_root: &str,
        def_digest: &str,
        toolchain_digest: &str,
    ) -> Result<CheckResult, RunnerExecError>;
}

/// Drive a runner execution through a [`RunnerExecutor`], enforcing the lease is
/// live first (fail-closed). This is the single entry the runner-side path uses.
pub fn execute_on_lease<E: RunnerExecutor>(
    executor: &E,
    lease: &RunnerLease,
    def: &CheckDef,
    memo_key: &str,
    tree_root: &str,
    def_digest: &str,
    toolchain_digest: &str,
) -> Result<CheckResult, RunnerExecError> {
    if lease.state != RunnerState::Held {
        return Err(RunnerExecError::LeaseNotHeld(lease.state.clone()));
    }
    executor.execute(
        lease,
        def,
        memo_key,
        tree_root,
        def_digest,
        toolchain_digest,
    )
}

/// The in-process reference runner executor.
///
/// It executes a deterministic, content-defined check: the produced artifacts
/// are a pure function of the three memo axes (so the same inputs yield the same
/// artifact bytes → the same digests, which is exactly what byte-identity ③
/// requires). For the non-determinism proof (④) a caller can inject an entropy
/// source so the same key yields DIVERGENT outputs across runs — modelling a
/// non-hermetic check (e.g. one that stamps a timestamp into its output).
pub struct InProcessRunnerExecutor<F = NoEntropy> {
    /// Identifies which runner produced the result (stamped into `runner_ref`).
    runner_ref: String,
    /// Per-run entropy source. The default ([`NoEntropy`]) makes execution a
    /// pure function of the inputs (deterministic). A divergent source models a
    /// non-hermetic check whose output varies run-to-run for the SAME key.
    entropy: F,
}

/// A deterministic (empty) entropy source: every run for the same inputs
/// produces byte-identical artifacts. This is the hermetic, byte-identical case.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoEntropy;

/// An entropy source consulted once per execution, mixed into the produced
/// artifact bytes. Models a non-hermetic check (the source of divergence ④).
pub trait EntropySource {
    /// Bytes mixed into this run's artifact content. An empty slice ⇒
    /// deterministic.
    fn sample(&self) -> Vec<u8>;
}

impl EntropySource for NoEntropy {
    fn sample(&self) -> Vec<u8> {
        Vec::new()
    }
}

impl InProcessRunnerExecutor<NoEntropy> {
    /// A deterministic (hermetic) reference executor — the byte-identical case.
    pub fn new(runner_ref: impl Into<String>) -> Self {
        Self {
            runner_ref: runner_ref.into(),
            entropy: NoEntropy,
        }
    }
}

impl<F: EntropySource> InProcessRunnerExecutor<F> {
    /// A reference executor with an explicit entropy source — used to model a
    /// non-hermetic (non-deterministic) check for item ④.
    pub fn with_entropy(runner_ref: impl Into<String>, entropy: F) -> Self {
        Self {
            runner_ref: runner_ref.into(),
            entropy,
        }
    }
}

impl<F: EntropySource> RunnerExecutor for InProcessRunnerExecutor<F> {
    fn execute(
        &self,
        _lease: &RunnerLease,
        def: &CheckDef,
        memo_key: &str,
        tree_root: &str,
        def_digest: &str,
        toolchain_digest: &str,
    ) -> Result<CheckResult, RunnerExecError> {
        // The produced artifact's content is a pure function of the check's
        // identity (its command + the three axes), PLUS any injected entropy.
        // With NoEntropy this is fully deterministic → byte-identical across
        // runners and against the local executor (item ③). With a divergent
        // entropy source the same key yields different bytes → divergence (④).
        let mut content: Vec<u8> = Vec::new();
        content.extend_from_slice(def.command.as_bytes());
        content.push(b'\n');
        content.extend_from_slice(tree_root.as_bytes());
        content.push(b'\n');
        content.extend_from_slice(def_digest.as_bytes());
        content.push(b'\n');
        content.extend_from_slice(toolchain_digest.as_bytes());
        let entropy = self.entropy.sample();
        if !entropy.is_empty() {
            content.push(b'\n');
            content.extend_from_slice(&entropy);
        }

        let artifact = Artifact {
            path: "check.out".to_string(),
            digest: artifact_digest(&content),
        };

        Ok(CheckResult {
            memo_key: memo_key.to_string(),
            tree_hash: tree_root.to_string(),
            def_digest: def_digest.to_string(),
            toolchain_digest: toolchain_digest.to_string(),
            exit: 0,
            artifacts: vec![artifact],
            stdout_ref: format!("blob:{}", artifact_digest(b"stdout")),
            stderr_ref: format!("blob:{}", artifact_digest(b"stderr")),
            duration_ms: 0,
            runner_ref: self.runner_ref.clone(),
            produced_at: 0,
        })
    }
}

/// The live runner-box executor — now WIRED (WP-Wave-E-PR3).
///
/// It drives a real runner box (pinned by `HUGIT_RUNNER_HOST`) through the
/// [`LeaseClient`]: under the already-acquired lease it dispatches the check
/// (`POST .../exec`), polls the result-envelope meta, and assembles the
/// [`CheckResult`] (artifacts + digests) with the supplied memo key/axes stamped
/// on — byte-identical to [`InProcessRunnerExecutor`] by construction. The
/// surrounding byte-identity comparator, non-determinism tracker, and hit-rate
/// meter are unchanged.
///
/// The transport is the same pluggable [`RunnerTransport`] seam the lease client
/// uses: production is the real `ureq` transport ([`LiveBoxRunnerExecutor::from_runtime`]),
/// and a fake transport proves the wiring hermetically. [`RunnerExecError::BoxNotWired`]
/// is retained for the genuinely-unconfigured case ONLY — an executor built with
/// [`LiveBoxRunnerExecutor::new`] (a host but no credentials), so the P2 deferral
/// never silently rots to a fabricated pass.
#[derive(Debug, Clone)]
pub struct LiveBoxRunnerExecutor<T: RunnerTransport = UreqRunnerTransport> {
    /// The runner box host (e.g. the value of `HUGIT_RUNNER_HOST`). Held for
    /// diagnostics / the `BoxNotWired` deferral message; the actual base URL the
    /// client dials lives privately inside [`LeaseClient`]'s config.
    host: String,
    /// The configured lease client, present once the box is wired (credentials
    /// loaded). `None` ⇒ genuinely unconfigured ⇒ `BoxNotWired`.
    client: Option<LeaseClient<T>>,
}

impl LiveBoxRunnerExecutor<UreqRunnerTransport> {
    /// Construct an executor bound to a runner-box host but WITHOUT credentials.
    /// Its [`RunnerExecutor::execute`] returns [`RunnerExecError::BoxNotWired`]
    /// — the self-documenting deferral for "I know the host but I am not wired".
    /// Use [`LiveBoxRunnerExecutor::from_runtime`] for the live, credentialled
    /// path.
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            client: None,
        }
    }

    /// Build a fully-wired live executor from the runtime environment: the
    /// runner host (`HUGIT_RUNNER_HOST`) + the PAT (via [`LeaseClient::from_runtime`],
    /// secret-file-preferred, env-fallback). Fail-closed: returns
    /// [`RunnerError::NotConfigured`] naming the missing piece when the host or
    /// PAT is unset, so an unconfigured deployment can never silently degrade.
    /// No network call is made here (only config is built).
    pub fn from_runtime() -> Result<Self, RunnerError> {
        let client = LeaseClient::from_runtime()?;
        // The host label for diagnostics; the client already validated it is set.
        let host = std::env::var(ENV_RUNNER_HOST).unwrap_or_default();
        Ok(Self {
            host,
            client: Some(client),
        })
    }
}

impl<T: RunnerTransport> LiveBoxRunnerExecutor<T> {
    /// Construct a wired executor over an explicit transport + client. Used to
    /// inject a fake transport in hermetic tests; in production `T =
    /// UreqRunnerTransport` (built via [`LiveBoxRunnerExecutor::from_runtime`]).
    pub fn with_client(host: impl Into<String>, client: LeaseClient<T>) -> Self {
        Self {
            host: host.into(),
            client: Some(client),
        }
    }

    /// The host this executor will drive (used by the acceptance suite's
    /// env-gated live lane).
    pub fn host(&self) -> &str {
        &self.host
    }
}

impl<T: RunnerTransport> RunnerExecutor for LiveBoxRunnerExecutor<T> {
    fn execute(
        &self,
        lease: &RunnerLease,
        def: &CheckDef,
        memo_key: &str,
        tree_root: &str,
        def_digest: &str,
        toolchain_digest: &str,
    ) -> Result<CheckResult, RunnerExecError> {
        match &self.client {
            // Wired: drive the box under the already-acquired lease and assemble
            // the CheckResult (the lease's acquire/close lifecycle is owned by
            // the caller — `execute_on_lease` / the dispatch orchestration).
            Some(client) => exec_and_collect(
                client,
                lease,
                def,
                memo_key,
                tree_root,
                def_digest,
                toolchain_digest,
            )
            .map(|outcome| outcome.result),
            // Genuinely unconfigured: the P2 deferral, surfaced loudly.
            None => Err(RunnerExecError::BoxNotWired(self.host.clone())),
        }
    }
}

#[cfg(test)]
mod wired_tests {
    use super::*;
    use crate::runner::lease_client::{ENV_RUNNER_PAT, ENV_RUNNER_PAT_FILE, ExecAck, RunnerConfig};
    use std::collections::VecDeque;
    use std::sync::Mutex;

    /// Minimal in-memory transport: FIFO `(status, body)` responses, no socket.
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

    fn held_lease() -> RunnerLease {
        RunnerLease {
            lease_id: "lease-live-1".to_string(),
            principal_chain: vec!["agent:tester".to_string()],
            path_set: vec!["/work".to_string()],
            expiry: 1_000_000,
            net_policy: "deny-all".to_string(),
            tmp_root: "/tmp/runner".to_string(),
            state: RunnerState::Held,
        }
    }

    fn a_def() -> CheckDef {
        CheckDef {
            def_digest: "a".repeat(64),
            command: "cargo test".to_string(),
            inputs: vec![],
            toolchain_ref: "rust-1.96".to_string(),
            env_manifest: "blob:abc".to_string(),
            glob_set: vec![],
        }
    }

    /// The wired live executor drives exec + poll and assembles a CheckResult
    /// with the supplied memo key/axes stamped on — hermetically (fake transport).
    #[test]
    fn wired_executor_collects_check_result() {
        let ack = ExecAck {
            lease_id: "lease-live-1".to_string(),
            accepted: true,
        };
        let envelope = br#"{
            "exit": 0,
            "artifacts": [{"path": "check.out", "digest": "cafef00d"}],
            "stdout_ref": "blob:o",
            "stderr_ref": "blob:e",
            "duration_ms": 11,
            "runner_ref": "runner:box-live",
            "metrics": {
                "tokens": {"input": 1, "output": 2, "cache_read": 0, "cache_write": 0, "total": 3},
                "wall_ms": 10,
                "active_ms": 9,
                "tool_calls": 0,
                "tool_breakdown": [],
                "model_turns": 1,
                "cost_usd_micros": 7
            }
        }"#
        .to_vec();
        let transport = FakeTransport::with(vec![
            (202, serde_json::to_vec(&ack).unwrap()), // exec
            (200, envelope),                          // poll_meta
        ]);
        let config = RunnerConfig::new("https://runner.example", "pat-secret").unwrap();
        let client = LeaseClient::with_transport(config, transport);
        let live = LiveBoxRunnerExecutor::with_client("runner.example", client);

        let result = execute_on_lease(
            &live,
            &held_lease(),
            &a_def(),
            &"d".repeat(64),
            &"1".repeat(64),
            &"2".repeat(64),
            &"3".repeat(64),
        )
        .expect("wired execution succeeds");

        assert_eq!(result.memo_key, "d".repeat(64));
        assert_eq!(result.tree_hash, "1".repeat(64));
        assert_eq!(result.def_digest, "2".repeat(64));
        assert_eq!(result.toolchain_digest, "3".repeat(64));
        assert_eq!(result.exit, 0);
        assert_eq!(result.artifacts.len(), 1);
        assert_eq!(result.artifacts[0].digest, "cafef00d");
        assert_eq!(result.runner_ref, "runner:box-live");
    }

    /// An executor built with `new` (host but no credentials) is the genuinely
    /// unconfigured case → `BoxNotWired`.
    #[test]
    fn new_without_credentials_is_box_not_wired() {
        let live = LiveBoxRunnerExecutor::new("runner.example");
        let err = live
            .execute(
                &held_lease(),
                &a_def(),
                &"d".repeat(64),
                &"1".repeat(64),
                &"2".repeat(64),
                &"3".repeat(64),
            )
            .expect_err("unconfigured ⇒ BoxNotWired");
        assert!(
            matches!(err, RunnerExecError::BoxNotWired(_)),
            "got {err:?}"
        );
    }

    /// `from_runtime` is fail-closed: host/PAT unset ⇒ `NotConfigured` naming the
    /// missing piece. (Env access is serialized within this single test.)
    #[test]
    fn from_runtime_not_configured_when_unset() {
        unsafe {
            std::env::remove_var(ENV_RUNNER_HOST);
            std::env::remove_var(ENV_RUNNER_PAT);
            std::env::set_var(ENV_RUNNER_PAT_FILE, "/nonexistent/hugit/runner/pat");
        }
        match LiveBoxRunnerExecutor::from_runtime() {
            Err(RunnerError::NotConfigured(msg)) => {
                assert!(msg.contains(ENV_RUNNER_HOST), "should name the host: {msg}");
            }
            other => panic!("expected NotConfigured, got {other:?}"),
        }
        unsafe {
            std::env::remove_var(ENV_RUNNER_PAT_FILE);
        }
    }
}
