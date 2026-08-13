//! # hugit-qa — the real-user simulator harness
//!
//! NOT a test framework in the pytest sense. This crate drives multi-step
//! journeys the way a human or an agent would — through the REAL product surfaces
//! (the `/v1` HTTP API over a real socket, the real `git` smart-HTTP wire, and the
//! real `hugit` CLI binary) — and captures IMMUTABLE evidence of what the user
//! experienced. The analyzer turns that evidence into quality findings, and the
//! reporter renders an audit report. No step asserts; a deviation is evidence.
//!
//! Pipeline: [`journey::run_journey`] (execute) → [`analyzer::analyze`] (judge) →
//! [`reporter::render`] (report).

pub mod analyzer;
pub mod evidence;
pub mod harness;
pub mod journey;
pub mod reporter;

/// Run a journey end-to-end and return the analysis + evidence (the pipeline a
/// CLI or a test invokes).
pub fn run(journey: &journey::Journey) -> PipelineResult {
    let run = journey::run_journey(journey);
    let initial = run.initial_world.clone();
    let analysis = analyzer::analyze(journey, &run.evidence, &initial);
    let report = reporter::render(journey, &run.evidence, &analysis);
    PipelineResult {
        run,
        analysis,
        report,
    }
}

/// The full pipeline output for one journey.
pub struct PipelineResult {
    pub run: journey::JourneyRun,
    pub analysis: analyzer::Analysis,
    /// The rendered Markdown audit report.
    pub report: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every checked-in journey in `journeys/` must parse (the DSL contract the
    /// CLI and the gate both rely on).
    #[test]
    fn checked_in_journeys_parse() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("journeys");
        let mut found = 0;
        for entry in std::fs::read_dir(&dir).expect("journeys dir") {
            let entry = entry.expect("entry");
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let raw = std::fs::read_to_string(entry.path()).expect("read journey");
            let journey: journey::Journey =
                serde_json::from_str(&raw).expect("journey must parse from checked-in JSON");
            assert!(!journey.steps.is_empty(), "journey has steps");
            found += 1;
        }
        assert!(found >= 1, "at least one journey checked in");
    }

    /// THE GATE: every checked-in journey runs END-TO-END against the real engine
    /// (real smart-HTTP wire, real CLI binary, real seeded git objects). The gate
    /// asserts the HARD safety rails never break — the journey completes, and no
    /// Deviation-level finding appears (read≠write, no-oracle 404, idempotent
    /// replay, monotone head_seq). Warn/Info findings are evidence the agent
    /// reasons over in the report — they are NOT this gate. This is the boundary
    /// between the deterministic safety net and the reasoning layer.
    #[test]
    fn gate_all_checked_in_journeys_run_clean() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("journeys");
        let mut ran = 0;
        for entry in std::fs::read_dir(&dir).expect("journeys dir") {
            let entry = entry.expect("entry");
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let raw = std::fs::read_to_string(entry.path()).expect("read journey");
            let journey: journey::Journey = serde_json::from_str(&raw).expect("journey must parse");
            let result = run(&journey);

            assert!(
                result.run.aborted_at.is_none(),
                "journey {} ABORTED at step {} — a surface the journey needed was missing",
                journey.name,
                result.run.aborted_at.unwrap_or(0)
            );
            let worst = result.analysis.worst();
            assert!(
                worst != analyzer::Severity::Deviation,
                "journey {} violated a hard invariant:\n{}",
                journey.name,
                result.report
            );
            assert!(
                !result.analysis.claims.is_empty(),
                "journey {} produced no claims — the reasoning layer needs designed-vs-produced evidence",
                journey.name
            );
            ran += 1;
        }
        assert!(ran >= 1, "at least one journey ran the gate");
    }
}
