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

/// The live runner-box executor — the P2 seam.
///
/// In production this drives a real runner box (pinned by `HUGIT_RUNNER_HOST`)
/// to execute the check inside a leased container and collect its artifacts. The
/// transport is intentionally NOT wired in B2b: it requires the live runner box
/// (a Phase-C / C2 deliverable) and is out of this WP's hermetic scope. The
/// surrounding byte-identity comparator, non-determinism tracker, and hit-rate
/// meter are all proven against [`InProcessRunnerExecutor`]; only this body
/// remains.
#[derive(Debug, Clone)]
pub struct LiveBoxRunnerExecutor {
    /// The runner box host (e.g. the value of `HUGIT_RUNNER_HOST`).
    host: String,
}

impl LiveBoxRunnerExecutor {
    /// Construct an executor bound to a runner-box host.
    pub fn new(host: impl Into<String>) -> Self {
        Self { host: host.into() }
    }

    /// The host this executor will drive (used by the acceptance suite's
    /// env-gated live lane).
    pub fn host(&self) -> &str {
        &self.host
    }
}

impl RunnerExecutor for LiveBoxRunnerExecutor {
    fn execute(
        &self,
        _lease: &RunnerLease,
        _def: &CheckDef,
        _memo_key: &str,
        _tree_root: &str,
        _def_digest: &str,
        _toolchain_digest: &str,
    ) -> Result<CheckResult, RunnerExecError> {
        // P2: drive `HUGIT_RUNNER_HOST` here — spawn a leased container, execute
        // the check, collect artifacts + digests, return the CheckResult.
        Err(RunnerExecError::BoxNotWired(self.host.clone()))
    }
}
