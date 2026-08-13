//! hugit-qa — the real-user simulator harness CLI.
//!
//! Usage: `hugit-qa run <journey.json>` — boots a real engine + real surfaces,
//! executes the journey, captures evidence, analyzes quality, and writes an audit
//! report + evidence log.

use std::path::PathBuf;
use std::process::ExitCode;

use hugit_qa::journey::{AnalyzeRule, Journey};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.as_slice() {
        [cmd, path] if cmd == "run" => {
            let journey_path = PathBuf::from(path);
            let raw = match std::fs::read_to_string(&journey_path) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!(
                        "hugit-qa: cannot read journey {}: {e}",
                        journey_path.display()
                    );
                    return ExitCode::from(2);
                }
            };
            let journey: Journey = match serde_json::from_str(&raw) {
                Ok(j) => j,
                Err(e) => {
                    eprintln!(
                        "hugit-qa: journey parse error in {}: {e}",
                        journey_path.display()
                    );
                    return ExitCode::from(2);
                }
            };
            run(&journey);
            ExitCode::SUCCESS
        }
        [cmd] if cmd == "list-rules" => {
            println!("available analyze rules:");
            for r in [
                "expectation_fidelity",
                "correctness",
                "ux",
                "efficiency",
                "consistency",
            ] {
                println!("  {r}");
            }
            ExitCode::SUCCESS
        }
        [] => {
            eprintln!("usage: hugit-qa run <journey.json> | hugit-qa list-rules");
            ExitCode::from(2)
        }
        _ => {
            eprintln!("usage: hugit-qa run <journey.json> | hugit-qa list-rules");
            ExitCode::from(2)
        }
    }
}

/// Execute `journey`, analyze the evidence, print the report.
fn run(journey_orig: &Journey) {
    let mut rules = journey_orig.analyze.clone();
    // Always apply the invariant spine + expectation fidelity even if the journey
    // forgot to ask — quality is the point, not the checklist.
    for extra in [
        AnalyzeRule::Correctness,
        AnalyzeRule::ExpectationFidelity,
        AnalyzeRule::Ux,
    ] {
        if !rules.contains(&extra) {
            rules.push(extra);
        }
    }
    let mut j = journey_orig.clone();
    j.analyze = rules;

    eprintln!("hugit-qa: running journey `{}` — {}", j.name, j.goal);

    let result = hugit_qa::run(&j);
    if let Some(abort) = result.run.aborted_at {
        eprintln!(
            "hugit-qa: journey stopped at step {abort} (a surface was unavailable or the world could not proceed)"
        );
    }

    println!("{}", result.report);

    // Persist the audit artefacts: evidence JSONL + report + analysis JSON.
    let out_dir = std::env::temp_dir().join(format!("hugit-qa-run-{}", j.name));
    let _ = std::fs::create_dir_all(&out_dir);
    let _ = std::fs::write(
        out_dir.join("evidence.jsonl"),
        format!("{}\n", result.run.evidence.to_jsonl()),
    );
    let _ = std::fs::write(out_dir.join("report.md"), &result.report);
    let analysis_json =
        serde_json::to_string_pretty(&analysis_json(&result.analysis)).unwrap_or_default();
    let _ = std::fs::write(out_dir.join("analysis.json"), &analysis_json);
    eprintln!("hugit-qa: artefacts -> {}", out_dir.display());
}

fn analysis_json(a: &hugit_qa::analyzer::Analysis) -> serde_json::Value {
    serde_json::json!({
        "worst": a.worst().as_str(),
        "findings": a.findings.iter().map(|f| serde_json::json!({
            "axis": f.axis.as_str(),
            "severity": f.severity.as_str(),
            "step": f.step,
            "what_happened": f.what_happened,
            "ideal": f.ideal,
            "detail": f.detail,
        })).collect::<Vec<_>>(),
    })
}
