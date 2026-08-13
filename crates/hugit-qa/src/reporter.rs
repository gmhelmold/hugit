//! # Reporter — the audit report a user could actually read
//!
//! Turns the evidence trail + the analyzer's findings into a README-grade
//! Markdown report: the goal, what the user did on each real surface, what came
//! back, how it was judged, and the deviation list. This is the "here's what the
//! user experienced, here's where it deviated from ideal" deliverable.

use std::fmt::Write as _;

use crate::analyzer::{Analysis, Axis, Finding, Severity};
use crate::evidence::{Evidence, StepEvidence, Surface};
use crate::journey::Journey;

/// Render a Markdown audit report for a completed journey.
pub fn render(journey: &Journey, evidence: &Evidence, analysis: &Analysis) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Auditoria de jornada — `{}`", journey.name);
    let _ = writeln!(out);
    let _ = writeln!(out, "**Objetivo:** {}", journey.goal);
    let _ = writeln!(
        out,
        "**Identidade:** `{}` / `{}`{}",
        journey.identity.org,
        journey.identity.user,
        if journey.identity.fresh_auth {
            " (fresh_auth)"
        } else {
            ""
        }
    );
    let _ = writeln!(
        out,
        "**Veredito:** {}",
        match analysis.worst() {
            Severity::Ok => "aca-impecável (sem desvios)",
            Severity::Info => "observações",
            Severity::Warn => "atenção (degradado)",
            Severity::Deviation => "desvios do ideal",
        }
    );
    let _ = writeln!(
        out,
        "**Steps:** {} · **Finding:** {}",
        evidence.steps().len(),
        analysis.findings.len()
    );
    let _ = writeln!(out, "\n---\n");

    render_summary(&mut out, analysis);
    render_claims(&mut out, analysis);
    render_steps(&mut out, evidence);
    render_findings(&mut out, analysis);
    out
}

/// The reasoning layer: a DESIGNED-vs-OBSERVED table the operator reads to judge
/// whether hugit produced the designed state-of-the-art output — the actual
/// values the wire returned, cross-checked against what we declared.
fn render_claims(out: &mut String, analysis: &Analysis) {
    if analysis.claims.is_empty() {
        return;
    }
    let _ = writeln!(out, "## Claims — projetado vs produzido\n");
    let _ = writeln!(
        out,
        "| # | passo | expectativa | projetado | produzido | ok |"
    );
    let _ = writeln!(
        out,
        "|---|-------|-------------|-----------|-----------|----|"
    );
    for c in &analysis.claims {
        let _ = writeln!(
            out,
            "| s{} | {} | {} | {} | {} | {} |",
            c.step,
            truncate(&c.step_name, 14),
            truncate(&c.description, 34),
            truncate(&c.designed, 22),
            truncate(&c.observed, 30),
            if c.satisfied { "✓" } else { "✗" }
        );
    }
    let _ = writeln!(out);
}

fn render_summary(out: &mut String, analysis: &Analysis) {
    let _ = writeln!(out, "## Sumário por eixo\n");
    let mut by_axis: Vec<(Axis, Vec<&Finding>)> = vec![];
    for axis in [
        Axis::Correctness,
        Axis::Ux,
        Axis::Efficiency,
        Axis::Consistency,
    ] {
        let fs: Vec<&Finding> = analysis
            .findings
            .iter()
            .filter(|f| f.axis == axis)
            .collect();
        by_axis.push((axis, fs));
    }
    for (axis, fs) in by_axis {
        let worst = fs.iter().map(|f| f.severity).max().unwrap_or(Severity::Ok);
        let _ = writeln!(
            out,
            "- **{}**: {} ({})",
            axis.as_str(),
            count_label(fs.len()),
            worst.as_str()
        );
    }
    let _ = writeln!(out);
}

fn count_label(n: usize) -> &'static str {
    match n {
        0 => "nenhum finding",
        1 => "1 finding",
        _ => "vários findings",
    }
}

fn render_steps(out: &mut String, evidence: &Evidence) {
    let _ = writeln!(out, "## O que o usuário experienciou\n");
    let _ = writeln!(out, "```");
    for ev in evidence.steps() {
        let line = format_step(ev);
        let _ = writeln!(out, "{line}");
    }
    let _ = writeln!(out, "```\n");
}

fn format_step(ev: &StepEvidence) -> String {
    let surface = match ev.surface {
        Surface::Api => "api",
        Surface::Cli => "cli",
        Surface::Git => "git",
        Surface::Identity => "identity",
        Surface::Setup => "setup",
    };
    let result = if let Some(s) = ev.status {
        format!("HTTP {s}")
    } else if let Some(c) = ev.exit_code {
        format!("exit {c}")
    } else {
        "–".to_string()
    };
    let signal = if let Some(b) = &ev.body {
        format!(" · body={}", compact_json(b))
    } else {
        String::new()
    };
    format!(
        "[{:>2}] {surface:8} {:<52} {result} · {}ms{signal} · {}",
        ev.index,
        truncate(&ev.detail, 52),
        ev.duration_ms,
        ev.goal,
    )
}

fn render_findings(out: &mut String, analysis: &Analysis) {
    let _ = writeln!(out, "## Findings\n");
    if analysis.findings.is_empty() {
        let _ = writeln!(out, "Nenhum desvio observado. A jornada cumpriu o ideal.\n");
        return;
    }
    for (i, f) in analysis.findings.iter().enumerate() {
        let _ = writeln!(
            out,
            "_F{i}_ · **{}** · severity: {}",
            f.axis.as_str(),
            f.severity.as_str()
        );
        if let Some(step) = f.step {
            let _ = writeln!(out, "  - `step {step}`");
        }
        let _ = writeln!(out, "  - Experienciado: {}", f.what_happened);
        let _ = writeln!(out, "  - Ideal: {}", f.ideal);
        if let Some(d) = &f.detail {
            let _ = writeln!(out, "  - Detalhe: `{}`", truncate(d, 200));
        }
        let _ = writeln!(out);
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

fn compact_json(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "∅".into(),
        other => {
            let s = other.to_string();
            truncate(&s, 120)
        }
    }
}
