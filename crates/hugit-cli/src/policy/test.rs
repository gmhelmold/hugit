//! `hugit policy test --context <path>` — run the house gate set against a
//! context file and report each gate's outcome.
//!
//! Reads a JSON `EvalContext` description (the CLI input contract — the library
//! [`hugit_policy::EvalContext`] is not itself `Deserialize`, so we own this
//! seam, exactly as `hugit impact` owns its `GraphInput` seam), builds the real
//! [`EvalContext`], and calls [`hugit_policy::Engine::house().eval`] — the one
//! evaluator the forge landing path also uses. No log, no persistence: `test` is
//! a pure dry-run.

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::ExitCode;

use serde::Deserialize;
use serde_json::json;

use crate::campaign::CampaignError;

/// Arguments for `hugit policy test`.
#[derive(clap::Args, Debug)]
pub struct TestArgs {
    /// Path to a JSON file describing the evaluation context.
    #[arg(long)]
    pub context: PathBuf,
}

/// The on-disk JSON shape `hugit policy test` reads (the CLI input contract).
/// Mirrors [`hugit_policy::EvalContext`]'s fields; every field defaults so a
/// minimal context (e.g. just `commit_messages`) is valid.
#[derive(Deserialize, Default)]
struct ContextInput {
    #[serde(default)]
    commit_messages: Vec<String>,
    #[serde(default)]
    commit_parent_counts: Vec<usize>,
    #[serde(default)]
    changed_files: Vec<String>,
    #[serde(default)]
    file_contents: HashMap<String, String>,
    #[serde(default)]
    metadata: HashMap<String, String>,
}

/// Run `hugit policy test` — emit the per-gate outcomes as stable JSON, exit 0
/// (a failing gate is a real result, not a CLI error), or exit 2 on a structured
/// input error (missing/malformed context file).
pub fn run(args: TestArgs) -> ExitCode {
    match do_run(args) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            println!("{}", err.to_json());
            err.exit_code()
        }
    }
}

fn do_run(args: TestArgs) -> Result<String, CampaignError> {
    use hugit_policy::{Engine, EvalContext, GateOutcome};

    // ── Read the context under the one input-error law ───────────────────────
    // A missing FILE is explicit (`context_not_found`), never silently an empty
    // context; a malformed file is `parse_context`. Both exit 2.
    let bytes = match std::fs::read(&args.context) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(CampaignError::new(
                "context_not_found",
                format!("--context file does not exist: {}", args.context.display()),
                "point --context at an existing EvalContext JSON file \
                 {commit_messages[], commit_parent_counts[], changed_files[], file_contents{}, metadata{}}",
            ));
        }
        Err(e) => {
            return Err(CampaignError::new(
                "io",
                format!("could not read --context {}: {e}", args.context.display()),
                "ensure the --context path is readable",
            ));
        }
    };
    let input: ContextInput = serde_json::from_slice(&bytes).map_err(|e| {
        CampaignError::new(
            "parse_context",
            format!("--context file {} is not valid JSON: {e}", args.context.display()),
            "the --context file must be a JSON object \
             {commit_messages[], commit_parent_counts[], changed_files[], file_contents{}, metadata{}}",
        )
    })?;

    // ── Build the real EvalContext + run the house engine ────────────────────
    // This is the SAME Engine::house() the forge landing path evaluates, so the
    // local verdict is byte-identical to the forge verdict (WP-D6 ①).
    let ctx = EvalContext {
        commit_messages: input.commit_messages,
        commit_parent_counts: input.commit_parent_counts,
        changed_files: input.changed_files,
        file_contents: input.file_contents,
        metadata: input.metadata,
    };
    let outcomes = Engine::house().eval(&ctx);
    let all_pass = Engine::all_pass(&outcomes);

    // ── Stable JSON report ────────────────────────────────────────────────────
    // A gate `reason` is engine-authored prose, but a secret could surface in it
    // (e.g. a matched token echoed by the secrets gate); scrub through the
    // redaction engine at the read boundary (it leaves normal prose untouched).
    let gates: Vec<serde_json::Value> = outcomes
        .iter()
        .map(|(id, outcome)| match outcome {
            GateOutcome::Pass => json!({ "id": id, "outcome": "pass" }),
            GateOutcome::Fail { reason } => json!({
                "id": id,
                "outcome": "fail",
                "reason": crate::redaction::scrub(reason),
            }),
            GateOutcome::Blocked { reason } => json!({
                "id": id,
                "outcome": "blocked",
                "reason": crate::redaction::scrub(reason),
            }),
        })
        .collect();

    Ok(json!({ "gates": gates, "all_pass": all_pass }).to_string())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn write_context(tag: &str, v: serde_json::Value) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "hugit-policy-test-{}-{}-{:?}",
            tag,
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("context.json");
        std::fs::write(&path, v.to_string()).unwrap();
        path
    }

    /// A clean context (DCO-signed non-merge commit, no secrets, no feat/fix →
    /// changelog N/A) passes every gate.
    #[test]
    fn clean_context_passes_all_gates() {
        let ctx = write_context(
            "clean",
            json!({
                "commit_messages": ["docs: tidy README\n\nSigned-off-by: A <a@b.com>"],
                "commit_parent_counts": [1],
                "changed_files": ["README.md"],
                "file_contents": { "README.md": "hello" },
                "metadata": {}
            }),
        );

        let result = do_run(TestArgs { context: ctx }).expect("eval ok");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["all_pass"], true, "clean context must pass: {v}");
        assert_eq!(v["gates"].as_array().unwrap().len(), 3, "three house gates");
    }

    /// A commit missing the DCO trailer fails the `dco` gate (exit 0 — a failing
    /// gate is a real result, not a CLI error).
    #[test]
    fn missing_dco_fails_the_dco_gate() {
        let ctx = write_context(
            "no-dco",
            json!({
                "commit_messages": ["feat: add thing (no sign-off)"],
                "commit_parent_counts": [1],
                "changed_files": ["src/x.rs"],
                "file_contents": {}
            }),
        );

        let result = do_run(TestArgs { context: ctx }).expect("eval ok even when a gate fails");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert_eq!(v["all_pass"], false);
        let dco = v["gates"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["id"] == "dco")
            .expect("dco gate present");
        assert_eq!(
            dco["outcome"], "fail",
            "dco must fail without a sign-off: {v}"
        );
    }

    /// A missing `--context` file is `context_not_found`/exit-2.
    #[test]
    fn missing_context_is_context_not_found() {
        let dir = std::env::temp_dir().join(format!("hugit-policy-missing-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let absent = dir.join("no-such.json");

        let err = do_run(TestArgs { context: absent }).expect_err("missing context must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "context_not_found");
    }

    /// A malformed `--context` file is `parse_context`/exit-2.
    #[test]
    fn malformed_context_is_parse_context() {
        let dir = std::env::temp_dir().join(format!("hugit-policy-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bad.json");
        std::fs::write(&path, "{not json").unwrap();

        let err = do_run(TestArgs { context: path }).expect_err("malformed context must fail");
        let v: serde_json::Value = serde_json::from_str(&err.to_json()).unwrap();
        assert_eq!(v["error"]["kind"], "parse_context");
    }

    /// An empty/minimal context object is valid (all fields default).
    #[test]
    fn empty_context_object_is_valid() {
        let ctx = write_context("empty", json!({}));
        let result = do_run(TestArgs { context: ctx }).expect("empty context is valid");
        let v: serde_json::Value = serde_json::from_str(&result).unwrap();
        assert!(v["gates"].is_array());
    }
}
