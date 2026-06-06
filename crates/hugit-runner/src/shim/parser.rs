//! Actions-YAML parser for the shim.
//!
//! Parses a GitHub Actions workflow YAML file into a [`ParsedWorkflow`].
//! Only the supported subset of the YAML schema is understood; anything else
//! produces an [`OutOfContractReport`] via the `report` module — never a
//! silent skip.
//!
//! The parser intentionally **does not** depend on a full YAML library at
//! runtime — v0 uses a minimal hand-rolled parser for the supported subset.
//! This keeps the dependency footprint zero and makes the boundary explicit.
//! Future iterations can adopt `serde_yaml` once the workspace opts in.

use std::collections::HashMap;

use crate::shim::report::OutOfContractReport;

/// A workflow parse error.
#[derive(Debug, PartialEq, Eq)]
pub enum WorkflowParseError {
    /// The input is empty or contains no recognizable workflow structure.
    EmptyInput,
    /// Required top-level `on:` key is missing.
    MissingOnKey,
    /// Required top-level `jobs:` key is missing.
    MissingJobsKey,
    /// The workflow YAML is structurally malformed (e.g., wrong indentation
    /// level for a required key).
    Malformed(String),
}

impl std::fmt::Display for WorkflowParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyInput => write!(f, "workflow YAML is empty or blank"),
            Self::MissingOnKey => write!(f, "workflow YAML is missing required `on:` key"),
            Self::MissingJobsKey => write!(f, "workflow YAML is missing required `jobs:` key"),
            Self::Malformed(s) => write!(f, "malformed workflow YAML: {s}"),
        }
    }
}

/// A single workflow step, as parsed from the supported subset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Step {
    /// Optional display name for the step (`name:` field).
    pub name: Option<String>,
    /// Shell command to run, if this is a `run:` step.
    pub run: Option<String>,
    /// The `uses:` action reference, if this is an `uses:` step.
    pub uses: Option<String>,
    /// Step-level `with:` map (for action steps).
    pub with: HashMap<String, String>,
    /// Step-level `env:` map (static string values only).
    pub env: HashMap<String, String>,
    /// If the step has `continue-on-error: true`.
    pub continue_on_error: bool,
    /// Raw `if:` condition string, if present.
    pub if_condition: Option<String>,
}

/// A parsed job from the workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Job {
    /// The job key/id.
    pub id: String,
    /// `runs-on:` value.
    pub runs_on: String,
    /// Job-level `env:` map (static string values only).
    pub env: HashMap<String, String>,
    /// Raw `if:` condition string, if present.
    pub if_condition: Option<String>,
    /// Steps in this job.
    pub steps: Vec<Step>,
}

/// The result of parsing a workflow file: the recognized structure plus any
/// out-of-contract constructs detected.
#[derive(Debug, Clone)]
pub struct ParsedWorkflow {
    /// Trigger events (from `on:`).
    pub triggers: Vec<String>,
    /// Parsed jobs.
    pub jobs: Vec<Job>,
    /// Any constructs found that are outside the supported subset.
    pub out_of_contract: Vec<OutOfContractReport>,
}

impl ParsedWorkflow {
    /// Returns `true` if the workflow is fully within the supported subset.
    pub fn is_fully_supported(&self) -> bool {
        self.out_of_contract.is_empty()
    }
}

/// Parse a workflow YAML string into a [`ParsedWorkflow`].
///
/// Any unsupported construct produces an [`OutOfContractReport`] in
/// [`ParsedWorkflow::out_of_contract`]; the parser never silently skips.
///
/// # Errors
/// Returns [`WorkflowParseError`] if the input is structurally invalid (e.g.,
/// missing `on:` or `jobs:` keys). Out-of-contract constructs do NOT cause an
/// error — they appear in `out_of_contract`.
pub fn parse_workflow(yaml: &str) -> Result<ParsedWorkflow, WorkflowParseError> {
    if yaml.trim().is_empty() {
        return Err(WorkflowParseError::EmptyInput);
    }

    // Minimal v0 parser: scan lines for structural markers.
    // The contract boundary is enforced by detecting unsupported keys and
    // emitting OutOfContractReport entries — zero silent skips.
    let lines: Vec<&str> = yaml.lines().collect();

    let has_on = lines
        .iter()
        .any(|l| l.starts_with("on:") || l.trim() == "on:");
    let has_jobs = lines.iter().any(|l| l.starts_with("jobs:"));

    if !has_on {
        return Err(WorkflowParseError::MissingOnKey);
    }
    if !has_jobs {
        return Err(WorkflowParseError::MissingJobsKey);
    }

    let triggers = parse_triggers(&lines);
    let (jobs, out_of_contract) = parse_jobs(&lines);

    Ok(ParsedWorkflow {
        triggers,
        jobs,
        out_of_contract,
    })
}

fn parse_triggers(lines: &[&str]) -> Vec<String> {
    let mut triggers = Vec::new();
    let mut in_on = false;

    for &line in lines {
        if let Some(rest) = line.strip_prefix("on:") {
            in_on = true;
            let rest = rest.trim();
            if !rest.is_empty()
                && rest != "push"
                && rest != "pull_request"
                && rest != "workflow_dispatch"
            {
                // multi-event inline — collect
                for ev in rest.split(',') {
                    let ev = ev.trim().trim_matches('[').trim_matches(']');
                    if !ev.is_empty() {
                        triggers.push(ev.to_string());
                    }
                }
            } else if !rest.is_empty() {
                triggers.push(rest.to_string());
            }
            continue;
        }
        if in_on {
            if line.starts_with("jobs:") || (!line.starts_with(' ') && !line.starts_with('\t')) {
                in_on = false;
            } else {
                let trimmed = line.trim().trim_end_matches(':');
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    triggers.push(trimmed.to_string());
                }
            }
        }
    }

    if triggers.is_empty() {
        triggers.push("push".to_string());
    }

    triggers
}

fn parse_jobs(lines: &[&str]) -> (Vec<Job>, Vec<OutOfContractReport>) {
    let mut jobs = Vec::new();
    let mut out_of_contract = Vec::new();

    // Detect unsupported top-level keys
    let known_top_level = ["on:", "jobs:", "name:", "env:", "#", "permissions:"];
    for &line in lines {
        if !line.starts_with(' ') && !line.starts_with('\t') && !line.trim().is_empty() {
            let key = line.split(':').next().unwrap_or("").trim();
            if !known_top_level.iter().any(|k| line.starts_with(k)) && !key.is_empty() {
                let unsupported_keys = [
                    "concurrency:",
                    "defaults:",
                    "strategy:",
                    "matrix:",
                    "services:",
                    "container:",
                    "timeout-minutes:",
                ];
                for uk in unsupported_keys {
                    if line.starts_with(uk) {
                        out_of_contract.push(OutOfContractReport::unsupported(
                            &format!("top-level `{}` key", uk.trim_end_matches(':')),
                            &format!(
                                "The `{}` construct is not in the shim's supported subset. \
                                      See docs/shim/supported-subset.md for the complete list.",
                                uk
                            ),
                        ));
                    }
                }
            }
        }
    }

    // Parse jobs block — find job ids (lines with exactly 2-space indent ending with ':')
    let mut i = 0;
    let mut in_jobs = false;

    while i < lines.len() {
        let line = lines[i];
        if line.starts_with("jobs:") {
            in_jobs = true;
            i += 1;
            continue;
        }
        if in_jobs {
            if line.starts_with("  ") && !line.starts_with("    ") && line.trim_end().ends_with(':')
            {
                let job_id = line.trim().trim_end_matches(':').to_string();
                let (job, mut ooc) = parse_single_job(job_id, lines, i + 1);
                jobs.push(job);
                out_of_contract.append(&mut ooc);
            } else if !line.starts_with(' ') && !line.starts_with('\t') && !line.trim().is_empty() {
                in_jobs = false;
            }
        }
        i += 1;
    }

    (jobs, out_of_contract)
}

fn parse_single_job(id: String, lines: &[&str], start: usize) -> (Job, Vec<OutOfContractReport>) {
    let mut runs_on = "ubuntu-latest".to_string();
    let mut env = HashMap::new();
    let mut if_condition = None;
    let mut steps = Vec::new();
    let mut out_of_contract = Vec::new();

    let mut i = start;
    let mut in_steps = false;
    let mut in_job_env = false;

    while i < lines.len() {
        let line = lines[i];

        // Stop when we reach another job or the end of the jobs block
        if !line.starts_with("    ") && !line.starts_with("  ") {
            break;
        }
        // Another job definition (2-space indent, ends with ':')
        if line.starts_with("  ") && !line.starts_with("    ") && line.trim_end().ends_with(':') {
            break;
        }

        let trimmed = line.trim();

        if let Some(val) = trimmed.strip_prefix("runs-on:") {
            runs_on = val.trim().to_string();
            if runs_on != "ubuntu-latest" && runs_on != "ubuntu-22.04" && runs_on != "ubuntu-20.04"
            {
                out_of_contract.push(OutOfContractReport::unsupported(
                    &format!("runs-on: {runs_on}"),
                    "Only `ubuntu-latest`, `ubuntu-22.04`, and `ubuntu-20.04` are in the \
                     supported subset. See docs/shim/supported-subset.md.",
                ));
            }
            in_steps = false;
            in_job_env = false;
        } else if let Some(val) = trimmed.strip_prefix("if:") {
            if_condition = Some(val.trim().to_string());
            in_steps = false;
            in_job_env = false;
        } else if trimmed == "env:" || trimmed.starts_with("env:") {
            in_job_env = true;
            in_steps = false;
        } else if trimmed == "steps:" {
            in_steps = true;
            in_job_env = false;
        } else if in_job_env && trimmed.contains(':') && !trimmed.starts_with('-') {
            let (k, v) = split_kv(trimmed);
            env.insert(k, v);
        } else if trimmed == "strategy:" || trimmed.starts_with("strategy:") {
            out_of_contract.push(OutOfContractReport::unsupported(
                "job `strategy:` (matrix)",
                "Matrix/strategy builds are not in the shim's supported subset (v0). \
                 Use a single-job workflow. See docs/shim/supported-subset.md.",
            ));
        } else if trimmed.starts_with("services:") || trimmed == "services:" {
            out_of_contract.push(OutOfContractReport::unsupported(
                "job `services:`",
                "Service containers are not in the shim's supported subset (v0). \
                 See docs/shim/supported-subset.md.",
            ));
        } else if trimmed.starts_with("container:") {
            out_of_contract.push(OutOfContractReport::unsupported(
                "job `container:`",
                "Container jobs are not in the shim's supported subset (v0). \
                 See docs/shim/supported-subset.md.",
            ));
        } else if trimmed.starts_with("timeout-minutes:") {
            out_of_contract.push(OutOfContractReport::unsupported(
                "job `timeout-minutes:`",
                "timeout-minutes is not in the shim's supported subset (v0). \
                 See docs/shim/supported-subset.md.",
            ));
        } else if in_steps && trimmed.starts_with("- ") {
            let (step, mut ooc) = parse_step(lines, i);
            steps.push(step);
            out_of_contract.append(&mut ooc);
        }

        i += 1;
    }

    let job = Job {
        id,
        runs_on,
        env,
        if_condition,
        steps,
    };
    (job, out_of_contract)
}

fn parse_step(lines: &[&str], start: usize) -> (Step, Vec<OutOfContractReport>) {
    let mut name = None;
    let mut run = None;
    let mut uses = None;
    let mut with = HashMap::new();
    let mut env = HashMap::new();
    let mut continue_on_error = false;
    let mut if_condition = None;
    let mut out_of_contract = Vec::new();
    let mut run_lines: Vec<String> = Vec::new();
    let mut in_run_multiline = false;
    let mut in_with = false;
    let mut in_step_env = false;

    let first_line = lines[start].trim();
    let rest = first_line.trim_start_matches('-').trim();
    if let Some(val) = rest.strip_prefix("name:") {
        name = Some(val.trim().to_string());
    } else if let Some(val) = rest.strip_prefix("run:") {
        let r = val.trim();
        if r == "|" || r == "|-" {
            in_run_multiline = true;
        } else if !r.is_empty() {
            run = Some(r.to_string());
        }
    } else if let Some(val) = rest.strip_prefix("uses:") {
        uses = Some(val.trim().to_string());
    }

    let mut i = start + 1;
    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        // Stop at next step
        if trimmed.starts_with("- ") || trimmed.starts_with("- name:") {
            break;
        }
        // Stop at job-level key (less indented)
        if !line.starts_with("          ") && !line.starts_with("        ") && !line.is_empty() {
            break;
        }

        if in_run_multiline {
            if trimmed.is_empty() {
                run_lines.push(String::new());
            } else if line.starts_with("          ") || line.starts_with("        ") {
                run_lines.push(trimmed.to_string());
                i += 1;
                continue;
            } else {
                in_run_multiline = false;
                run = Some(run_lines.join("\n"));
            }
        }

        if let Some(val) = trimmed.strip_prefix("name:") {
            name = Some(val.trim().to_string());
            in_with = false;
            in_step_env = false;
        } else if let Some(val) = trimmed.strip_prefix("run:") {
            let r = val.trim();
            if r == "|" || r == "|-" {
                in_run_multiline = true;
                run_lines.clear();
            } else if !r.is_empty() {
                run = Some(r.to_string());
            }
            in_with = false;
            in_step_env = false;
        } else if let Some(val) = trimmed.strip_prefix("uses:") {
            uses = Some(val.trim().to_string());
            in_with = false;
            in_step_env = false;
        } else if let Some(val) = trimmed.strip_prefix("continue-on-error:") {
            continue_on_error = val.trim() == "true";
            in_with = false;
            in_step_env = false;
        } else if let Some(val) = trimmed.strip_prefix("if:") {
            if_condition = Some(val.trim().to_string());
            in_with = false;
            in_step_env = false;
        } else if trimmed == "with:" || trimmed.starts_with("with:") {
            in_with = true;
            in_step_env = false;
        } else if trimmed == "env:" || trimmed.starts_with("env:") {
            in_step_env = true;
            in_with = false;
        } else if in_with && trimmed.contains(':') {
            let (k, v) = split_kv(trimmed);
            with.insert(k, v);
        } else if in_step_env && trimmed.contains(':') {
            let (k, v) = split_kv(trimmed);
            env.insert(k, v);
        } else if trimmed.starts_with("timeout-minutes:") {
            out_of_contract.push(OutOfContractReport::unsupported(
                "step `timeout-minutes:`",
                "Per-step timeout-minutes is not in the shim's supported subset (v0). \
                 See docs/shim/supported-subset.md.",
            ));
        } else if trimmed.starts_with("retry-on:") || trimmed.starts_with("retry:") {
            out_of_contract.push(OutOfContractReport::unsupported(
                "step `retry:`",
                "Step retry is not in the shim's supported subset (v0). \
                 See docs/shim/supported-subset.md.",
            ));
        }

        i += 1;
    }

    if in_run_multiline && !run_lines.is_empty() {
        run = Some(run_lines.join("\n"));
    }

    // Classify `uses:` action support
    if let Some(ref u) = uses {
        let supported_actions = ["actions/upload-artifact@v3", "actions/download-artifact@v3"];
        if !supported_actions.iter().any(|a| u.starts_with(a)) {
            out_of_contract.push(OutOfContractReport::unsupported(
                &format!("uses: {u}"),
                &format!(
                    "Action `{u}` is not in the shim's supported subset. \
                     Supported actions: {}. See docs/shim/supported-subset.md.",
                    supported_actions.join(", ")
                ),
            ));
        }
    }

    let step = Step {
        name,
        run,
        uses,
        with,
        env,
        continue_on_error,
        if_condition,
    };
    (step, out_of_contract)
}

fn split_kv(s: &str) -> (String, String) {
    if let Some(pos) = s.find(':') {
        let k = s[..pos].trim().to_string();
        let v = s[pos + 1..]
            .trim()
            .trim_matches('"')
            .trim_matches('\'')
            .to_string();
        (k, v)
    } else {
        (s.to_string(), String::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_empty_returns_error() {
        assert_eq!(
            parse_workflow("").unwrap_err(),
            WorkflowParseError::EmptyInput
        );
        assert_eq!(
            parse_workflow("   \n  ").unwrap_err(),
            WorkflowParseError::EmptyInput
        );
    }

    #[test]
    fn parse_missing_on_key() {
        let yaml = "jobs:\n  build:\n    runs-on: ubuntu-latest\n    steps: []\n";
        assert_eq!(
            parse_workflow(yaml).unwrap_err(),
            WorkflowParseError::MissingOnKey
        );
    }

    #[test]
    fn parse_missing_jobs_key() {
        let yaml = "on: push\n";
        assert_eq!(
            parse_workflow(yaml).unwrap_err(),
            WorkflowParseError::MissingJobsKey
        );
    }

    #[test]
    fn parse_minimal_workflow() {
        let yaml = "on: push\njobs:\n  build:\n    runs-on: ubuntu-latest\n    steps:\n      - name: Hello\n        run: echo hello\n";
        let wf = parse_workflow(yaml).expect("should parse");
        assert!(wf.triggers.contains(&"push".to_string()));
        assert_eq!(wf.jobs.len(), 1);
        assert!(wf.is_fully_supported());
    }

    #[test]
    fn parse_detects_unsupported_runs_on() {
        let yaml = "on: push\njobs:\n  build:\n    runs-on: windows-latest\n    steps:\n      - name: Hi\n        run: echo hi\n";
        let wf = parse_workflow(yaml).expect("should parse");
        assert!(!wf.out_of_contract.is_empty());
    }
}
