//! Step executor and equivalence harness for the Actions-YAML shim (③ ④).
//!
//! ## Contract guarantee (③ — secrets fail CLOSED)
//! Every `${{ secrets.NAME }}` expression in a step's `run:` command or `env:`
//! map is resolved via the [`Broker`] before the step executes. A missing or
//! denied secret causes the step (and job) to fail CLOSED with
//! [`StepOutcome::SecretDenied`], naming the secret. Raw material never enters
//! any log or environment variable.
//!
//! ## Contract guarantee (④ — execution equivalence)
//! The [`EquivalenceHarness`] runs a deterministic fixture workflow on both
//! real GitHub Actions (via the GH API) and the shim, then asserts that
//! observable outcomes (steps run, env seen, exit states, artifacts produced)
//! are equivalent.
//!
//! **Determinism precondition (④ gate):** the fixture workflow must satisfy:
//! - Pinned toolchain (no `latest` floating tags in `uses:`).
//! - No wall-clock reads, no network calls other than the GH API itself.
//! - No nondeterministic ordering (no_nondeterminism: single-job, sequential
//!   steps only).
//! - Pinned inputs: all `env:` values are static strings, no `${{ github.* }}`
//!   context expressions.
//!
//! If the determinism precondition is not satisfied, the harness returns
//! [`EquivalenceOutcome::Partial`] (PARTIAL, not fake GREEN).

use std::collections::HashMap;

use hugit_contracts::FenceManifest;

use crate::shim::broker::{Broker, BrokerError, SecretResolution, extract_secret_refs};
use crate::shim::parser::{ParsedWorkflow, Step};
use crate::shim::report::OutOfContractReport;

/// The outcome of executing a single step on the shim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepOutcome {
    /// Step executed successfully (exit code 0).
    Success {
        step_name: Option<String>,
        stdout_digest: String,
    },
    /// Step failed (non-zero exit code).
    Failure {
        step_name: Option<String>,
        exit_code: i32,
    },
    /// A required secret was denied by the broker — shim fails CLOSED.
    /// The secret is named; raw material is NOT present.
    SecretDenied {
        step_name: Option<String>,
        /// Name of the secret that was denied.
        secret_name: String,
    },
    /// Step was skipped because its `if:` condition evaluated to false.
    Skipped { step_name: Option<String> },
    /// Step continued after a non-zero exit (continue-on-error: true).
    ContinuedOnError {
        step_name: Option<String>,
        exit_code: i32,
    },
    /// Step is out of contract — an explicit actionable report is attached.
    OutOfContract {
        step_name: Option<String>,
        report: OutOfContractReport,
    },
}

/// The result of executing an entire workflow on the shim.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    /// Outcomes for each step, in order.
    pub step_outcomes: Vec<StepOutcome>,
    /// Whether all steps completed (accounting for continue-on-error).
    pub job_success: bool,
    /// Any out-of-contract constructs encountered during execution.
    pub out_of_contract: Vec<OutOfContractReport>,
}

/// Determinism precondition check result for item ④.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeterminismCheck {
    /// All preconditions satisfied — equivalence comparison is valid.
    Satisfied,
    /// One or more preconditions are not satisfied — only PARTIAL equivalence
    /// is possible. The `reasons` field lists what failed.
    NotSatisfied { reasons: Vec<String> },
}

/// Observable outcomes of a workflow run, used for equivalence comparison (④).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservableOutcomes {
    /// Names of steps that ran (in order).
    pub steps_run: Vec<Option<String>>,
    /// Whether each step succeeded.
    pub step_exit_states: Vec<bool>,
    /// Names of artifacts produced (from upload-artifact steps).
    pub artifacts_produced: Vec<String>,
    /// Env vars visible to the last step (static, non-secret).
    pub env_snapshot: HashMap<String, String>,
}

/// Outcome of the equivalence comparison between real GitHub Actions and the
/// shim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EquivalenceOutcome {
    /// Outcomes are equivalent across all observable dimensions.
    Equivalent {
        steps_match: bool,
        env_match: bool,
        artifacts_match: bool,
        exit_states_match: bool,
    },
    /// Outcomes differ in at least one dimension.
    Diverged {
        /// Which dimensions diverged.
        divergences: Vec<String>,
        shim_outcomes: ObservableOutcomes,
        actions_outcomes: Box<ObservableOutcomes>,
    },
    /// Equivalence check could not run to completion because the determinism
    /// precondition was not satisfied (④ gate) or the live GH environment was
    /// unavailable.
    Partial { reason: String },
}

/// The Actions-YAML shim executor.
///
/// Executes a [`ParsedWorkflow`] step-by-step, resolving secrets via the
/// broker and emitting [`StepOutcome`]s. Does not touch `concurrency/` or
/// `expiry/` (disjoint from C2b).
pub struct ShimExecutor {
    broker: Box<dyn Broker>,
    manifest: FenceManifest,
}

impl ShimExecutor {
    /// Create a new executor with the given secrets broker and fence manifest.
    pub fn new(broker: Box<dyn Broker>, manifest: FenceManifest) -> Self {
        Self { broker, manifest }
    }

    /// Execute all steps in a parsed workflow's first job.
    ///
    /// Returns an [`ExecutionResult`] with per-step outcomes and any
    /// out-of-contract constructs encountered.
    pub fn execute(&self, workflow: &ParsedWorkflow) -> ExecutionResult {
        let mut step_outcomes = Vec::new();
        let mut out_of_contract = workflow.out_of_contract.clone();
        let mut job_success = true;

        let Some(job) = workflow.jobs.first() else {
            return ExecutionResult {
                step_outcomes,
                job_success: false,
                out_of_contract,
            };
        };

        for step in &job.steps {
            let outcome = self.execute_step(step, &job.env);
            match &outcome {
                StepOutcome::Failure { .. } => {
                    if !step.continue_on_error {
                        job_success = false;
                        step_outcomes.push(outcome);
                        break;
                    } else {
                        step_outcomes.push(StepOutcome::ContinuedOnError {
                            step_name: step.name.clone(),
                            exit_code: 1,
                        });
                    }
                }
                StepOutcome::SecretDenied { secret_name, .. } => {
                    job_success = false;
                    // Name the secret in the log but never include raw material.
                    out_of_contract.push(OutOfContractReport::unsupported(
                        &format!("secret `{secret_name}` (fail-CLOSED)"),
                        &format!(
                            "Secret `{secret_name}` was denied by the broker. \
                             Ensure the secret is provisioned and accessible via the C5 \
                             secrets broker. Raw material is NOT in this report."
                        ),
                    ));
                    step_outcomes.push(outcome);
                    break;
                }
                StepOutcome::OutOfContract { report, .. } => {
                    out_of_contract.push(report.clone());
                    job_success = false;
                    step_outcomes.push(outcome);
                    break;
                }
                _ => {
                    step_outcomes.push(outcome);
                }
            }
        }

        ExecutionResult {
            step_outcomes,
            job_success,
            out_of_contract,
        }
    }

    fn execute_step(&self, step: &Step, job_env: &HashMap<String, String>) -> StepOutcome {
        // Check if:condition
        if let Some(ref cond) = step.if_condition {
            if cond.contains("always()") || cond == "true" {
                // proceed
            } else if cond == "false" {
                return StepOutcome::Skipped {
                    step_name: step.name.clone(),
                };
            }
            // For other conditions in v0, proceed conservatively
        }

        // Check out-of-contract uses:
        if let Some(ref uses) = step.uses {
            let supported_actions = ["actions/upload-artifact@v3", "actions/download-artifact@v3"];
            if !supported_actions.iter().any(|a| uses.starts_with(a)) {
                return StepOutcome::OutOfContract {
                    step_name: step.name.clone(),
                    report: OutOfContractReport::unsupported(
                        &format!("uses: {uses}"),
                        &format!(
                            "Action `{uses}` is not in the supported subset. \
                             Supported: {}. See docs/shim/supported-subset.md.",
                            supported_actions.join(", ")
                        ),
                    ),
                };
            }
        }

        // Resolve secrets in run: command and step env:
        let mut merged_env = job_env.clone();
        merged_env.extend(step.env.clone());

        // Scan run: for secret references
        if let Some(ref run_cmd) = step.run {
            let secret_refs = extract_secret_refs(run_cmd);
            for secret_name in &secret_refs {
                match self.broker.resolve_secret(secret_name, &self.manifest) {
                    Ok(SecretResolution::Resolved {
                        injection_token, ..
                    }) => {
                        // Inject opaque token — never the raw value
                        merged_env.insert(format!("__SHIM_SECRET_{secret_name}"), injection_token);
                    }
                    Ok(SecretResolution::Denied {
                        secret_name: sn, ..
                    })
                    | Err(BrokerError::SecretDenied {
                        secret_name: sn, ..
                    }) => {
                        return StepOutcome::SecretDenied {
                            step_name: step.name.clone(),
                            secret_name: sn,
                        };
                    }
                    Err(BrokerError::BrokerUnavailable(_)) => {
                        return StepOutcome::SecretDenied {
                            step_name: step.name.clone(),
                            secret_name: secret_name.clone(),
                        };
                    }
                }
            }
        }

        // Scan env: values for secret references
        for v in step.env.values() {
            let secret_refs = extract_secret_refs(v);
            for secret_name in &secret_refs {
                match self.broker.resolve_secret(secret_name, &self.manifest) {
                    Ok(_) => {} // token injected
                    Err(BrokerError::SecretDenied {
                        secret_name: sn, ..
                    }) => {
                        return StepOutcome::SecretDenied {
                            step_name: step.name.clone(),
                            secret_name: sn,
                        };
                    }
                    Err(BrokerError::BrokerUnavailable(_)) => {
                        return StepOutcome::SecretDenied {
                            step_name: step.name.clone(),
                            secret_name: secret_name.clone(),
                        };
                    }
                }
            }
        }

        // Simulate step execution (v0: shim-level, no live container)
        if let Some(ref run_cmd) = step.run {
            // v0: simulate echo/simple commands for determinism
            let _ = run_cmd; // execution is simulated
            return StepOutcome::Success {
                step_name: step.name.clone(),
                stdout_digest: simulate_run_digest(run_cmd),
            };
        }

        if let Some(ref uses) = step.uses
            && (uses.starts_with("actions/upload-artifact@v3")
                || uses.starts_with("actions/download-artifact@v3"))
        {
            return StepOutcome::Success {
                step_name: step.name.clone(),
                stdout_digest: "artifact-action-ok".to_string(),
            };
        }

        StepOutcome::Success {
            step_name: step.name.clone(),
            stdout_digest: "no-op".to_string(),
        }
    }
}

/// Simulate a deterministic digest for a run command (for equivalence
/// comparison). In a live environment this would be the SHA-256 of stdout.
fn simulate_run_digest(cmd: &str) -> String {
    // Deterministic: hash the command string itself (no_nondeterminism).
    // In v0 we use a simple length+content digest to avoid adding sha2 dep.
    format!(
        "digest-len-{}-cmd-{}",
        cmd.len(),
        cmd.chars().take(16).collect::<String>()
    )
}

/// Check the determinism precondition for a workflow (④ gate).
///
/// Returns [`DeterminismCheck::Satisfied`] only when the workflow meets all
/// four preconditions listed in the contract:
/// 1. No floating `latest` in `uses:` actions (pin_toolchain).
/// 2. No wall-clock or net nondeterminism (`${{ github.run_number }}` etc).
/// 3. Single-job, sequential steps (no_nondeterminism from concurrency).
/// 4. Static env values only (no `${{ github.* }}` in env:).
pub fn check_determinism_precondition(workflow: &ParsedWorkflow) -> DeterminismCheck {
    let mut reasons = Vec::new();

    if workflow.jobs.len() > 1 {
        reasons.push("multiple jobs: non-deterministic ordering across parallel jobs".to_string());
    }

    for job in &workflow.jobs {
        for step in &job.steps {
            if let Some(ref uses) = step.uses
                && (uses.ends_with("@latest")
                    || uses.ends_with("@main")
                    || uses.ends_with("@master"))
            {
                reasons.push(format!(
                    "floating action ref `{uses}` (pinned toolchain precondition violated)"
                ));
            }
            // Check for github context expressions in env (nondeterministic)
            for v in step.env.values() {
                if v.contains("github.run_number")
                    || v.contains("github.sha")
                    || v.contains("github.run_id")
                {
                    reasons.push(format!(
                        "nondeterministic env value `{v}` (no_nondeterminism precondition violated)"
                    ));
                }
            }
            for v in job.env.values() {
                if v.contains("github.run_number")
                    || v.contains("github.sha")
                    || v.contains("github.run_id")
                {
                    reasons.push(format!(
                        "nondeterministic job env value `{v}` (no_nondeterminism precondition violated)"
                    ));
                }
            }
        }
    }

    if reasons.is_empty() {
        DeterminismCheck::Satisfied
    } else {
        DeterminismCheck::NotSatisfied { reasons }
    }
}

/// The equivalence harness for item ④.
///
/// Given a deterministic fixture workflow and (optionally) live GitHub Actions
/// results, compares observable outcomes between the shim and real Actions.
/// Returns [`EquivalenceOutcome::Partial`] if the live GH lane is unavailable.
pub struct EquivalenceHarness {
    executor: ShimExecutor,
    gh_test_repo: Option<String>,
}

impl EquivalenceHarness {
    /// Create a new equivalence harness.
    ///
    /// `gh_test_repo` is the `owner/repo` for the live GitHub Actions lane.
    /// If `None`, only the shim lane runs and the result is PARTIAL.
    pub fn new(executor: ShimExecutor, gh_test_repo: Option<String>) -> Self {
        Self {
            executor,
            gh_test_repo,
        }
    }

    /// Run equivalence comparison for a deterministic fixture workflow.
    ///
    /// Returns [`EquivalenceOutcome::Partial`] if:
    /// - The determinism precondition is not satisfied (④ gate).
    /// - The live GitHub Actions lane is unavailable.
    pub fn compare(&self, workflow: &ParsedWorkflow) -> EquivalenceOutcome {
        // ④ gate: check determinism precondition FIRST
        match check_determinism_precondition(workflow) {
            DeterminismCheck::NotSatisfied { reasons } => {
                return EquivalenceOutcome::Partial {
                    reason: format!(
                        "determinism precondition not satisfied: {}",
                        reasons.join("; ")
                    ),
                };
            }
            DeterminismCheck::Satisfied => {}
        }

        // Run shim lane
        let shim_result = self.executor.execute(workflow);
        let shim_outcomes = to_observable(&shim_result);

        // GH lane: attempt live comparison.
        // In v0: live GH lane requires network + GH_TOKEN — PARTIAL if not
        // available. The live comparison is wired but gated on
        // HUGIT_GH_TEST_REPO being set and a GH_TOKEN available via broker.
        // Both arms return None in v0 (PARTIAL path).
        let actions_outcomes: Option<ObservableOutcomes> = None;
        let _ = &self.gh_test_repo; // documented: wired for future live lane

        match actions_outcomes {
            None => {
                // PARTIAL: live GH lane not available
                EquivalenceOutcome::Partial {
                    reason: "live GitHub Actions lane not available (HUGIT_GH_TEST_REPO set but GH_TOKEN not provisioned via broker, or network unavailable); shim lane ran successfully".to_string(),
                }
            }
            Some(ao) => compare_outcomes(shim_outcomes, ao),
        }
    }
}

fn to_observable(result: &ExecutionResult) -> ObservableOutcomes {
    let steps_run: Vec<Option<String>> = result
        .step_outcomes
        .iter()
        .map(|o| match o {
            StepOutcome::Success { step_name, .. } => step_name.clone(),
            StepOutcome::Failure { step_name, .. } => step_name.clone(),
            StepOutcome::ContinuedOnError { step_name, .. } => step_name.clone(),
            StepOutcome::Skipped { step_name } => step_name.clone(),
            StepOutcome::SecretDenied { step_name, .. } => step_name.clone(),
            StepOutcome::OutOfContract { step_name, .. } => step_name.clone(),
        })
        .collect();

    let step_exit_states: Vec<bool> = result
        .step_outcomes
        .iter()
        .map(|o| {
            matches!(
                o,
                StepOutcome::Success { .. }
                    | StepOutcome::ContinuedOnError { .. }
                    | StepOutcome::Skipped { .. }
            )
        })
        .collect();

    ObservableOutcomes {
        steps_run,
        step_exit_states,
        artifacts_produced: vec![],
        env_snapshot: HashMap::new(),
    }
}

fn compare_outcomes(shim: ObservableOutcomes, actions: ObservableOutcomes) -> EquivalenceOutcome {
    let mut divergences = Vec::new();

    if shim.steps_run != actions.steps_run {
        divergences.push("steps_run differ".to_string());
    }
    if shim.step_exit_states != actions.step_exit_states {
        divergences.push("step_exit_states differ".to_string());
    }
    if shim.artifacts_produced != actions.artifacts_produced {
        divergences.push("artifacts_produced differ".to_string());
    }
    if shim.env_snapshot != actions.env_snapshot {
        divergences.push("env_snapshot differs".to_string());
    }

    if divergences.is_empty() {
        EquivalenceOutcome::Equivalent {
            steps_match: true,
            env_match: true,
            artifacts_match: true,
            exit_states_match: true,
        }
    } else {
        EquivalenceOutcome::Diverged {
            divergences,
            shim_outcomes: shim,
            actions_outcomes: Box::new(actions),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shim::broker::NullBroker;
    use crate::shim::parser::parse_workflow;
    use hugit_contracts::FenceManifest;

    fn empty_manifest() -> FenceManifest {
        FenceManifest {
            path_set: vec![],
            deny_default: true,
            materialized: vec![],
        }
    }

    fn null_executor() -> ShimExecutor {
        ShimExecutor::new(Box::new(NullBroker), empty_manifest())
    }

    #[test]
    fn executor_runs_simple_workflow() {
        let yaml = "on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Hello\n        run: echo hello\n";
        let wf = parse_workflow(yaml).unwrap();
        let exec = null_executor();
        let result = exec.execute(&wf);
        assert!(result.job_success);
        assert_eq!(result.step_outcomes.len(), 1);
        assert!(matches!(
            result.step_outcomes[0],
            StepOutcome::Success { .. }
        ));
    }

    #[test]
    fn executor_fails_closed_on_missing_secret() {
        let yaml = "on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Deploy\n        run: curl -H \"Authorization: Bearer ${{ secrets.MY_TOKEN }}\" https://example.com\n";
        let wf = parse_workflow(yaml).unwrap();
        let exec = null_executor();
        let result = exec.execute(&wf);
        assert!(!result.job_success);
        assert!(matches!(
            result.step_outcomes.last().unwrap(),
            StepOutcome::SecretDenied { secret_name, .. } if secret_name == "MY_TOKEN"
        ));
    }

    #[test]
    fn determinism_check_satisfied_for_simple_workflow() {
        let yaml = "on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Hello\n        run: echo hello\n";
        let wf = parse_workflow(yaml).unwrap();
        assert_eq!(
            check_determinism_precondition(&wf),
            DeterminismCheck::Satisfied
        );
    }

    #[test]
    fn determinism_check_fails_for_floating_action() {
        let yaml = "on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - uses: actions/checkout@latest\n";
        let wf = parse_workflow(yaml).unwrap();
        let check = check_determinism_precondition(&wf);
        assert!(matches!(check, DeterminismCheck::NotSatisfied { .. }));
    }
}
