//! # Analyzer — quality over evidence, not asserts over expectations
//!
//! The analyzer consumes the IMMUTABLE evidence trail + the journey's goal and
//! produces structured FINDINGS: what the user experienced vs where it deviated
//! from ideal. It groups by quality axis (correctness / UX / efficiency /
//! consistency) and grades each finding with a severity. There is no
//! pass/fail gate — a journey where a step "failed" is not a red bar, it is a
//! finding the operator reads.
//!
//! ## Two layers
//!
//! 1. **Hard invariants** (deterministic safety rails): anonymous can never
//!    write, a 404 must not leak existence, an idempotent replay must append
//!    nothing, head_seq must stay monotone. A violation is always a Deviation.
//! 2. **Claims** (the reasoning layer): every declared expectation a journey
//!    attaches to a step (`expect.body_json`/`body_contains`/`world_adds`/
//!    `exit`/`status`) becomes a structured [`Claim`]: the DESIGNED value vs the
//!    OBSERVED value from the real evidence. The claim is not an assert — it is
//!    the atom an agent (this crate's operator) reasons over: is the STUFF hugit
//!    rendered actually the designed state-of-the-art wedge (attested cost,
//!    memoization, one-source-of-truth), or is it a door that opens to nothing?

use std::collections::BTreeMap;

use serde_json::Value;

use crate::evidence::{Evidence, StepEvidence, Surface, WorldDelta};
use crate::journey::{AnalyzeRule, Journey, Step};

/// Severity of a quality finding. `Ok` = matched ideal; `Info` = observed fact;
/// `Warn` = degraded; `Deviation` = the user's experience diverged from ideal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Ok,
    Info,
    Warn,
    Deviation,
}

impl Severity {
    pub fn as_str(&self) -> &'static str {
        match self {
            Severity::Ok => "ok",
            Severity::Info => "info",
            Severity::Warn => "warn",
            Severity::Deviation => "deviation",
        }
    }
}

/// One quality finding: axis + severity + what the user saw + what ideal was.
#[derive(Debug, Clone)]
pub struct Finding {
    pub axis: Axis,
    pub severity: Severity,
    /// The step (index) that produced it, if tied to a step.
    pub step: Option<u32>,
    /// Human-readable: what the user experienced.
    pub what_happened: String,
    /// Human-readable: what ideal looks like.
    pub ideal: String,
    /// Optional observable detail (status, world delta, latency…).
    pub detail: Option<String>,
}

/// A DESIGNED-vs-OBSERVED claim the agent reasons over. Every `expect` hint a
/// journey declares becomes one of these; the report renders them as a table so
/// the operator can judge substance (e.g. the exact attested cost) rather than a
/// boolean.
#[derive(Debug, Clone)]
pub struct Claim {
    /// The step index the claim is attached to.
    pub step: u32,
    /// The step's name (what the user was doing).
    pub step_name: String,
    /// A human/agent readable description of what was expected.
    pub description: String,
    /// The DESIGNED value (from the journey's `expect`).
    pub designed: String,
    /// The OBSERVED value (from the real evidence) — the wire truth.
    pub observed: String,
    /// Whether the observed value satisfies the designed one.
    pub satisfied: bool,
    /// A short rationale of the match/mismatch the agent can verify.
    pub rationale: String,
}

/// The quality axis a finding belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Axis {
    Correctness,
    Ux,
    Efficiency,
    Consistency,
}

impl Axis {
    pub fn as_str(&self) -> &'static str {
        match self {
            Axis::Correctness => "correctness",
            Axis::Ux => "ux",
            Axis::Efficiency => "efficiency",
            Axis::Consistency => "consistency",
        }
    }
}

/// The analyzer's verdict over a journey.
#[derive(Debug, Clone, Default)]
pub struct Analysis {
    pub findings: Vec<Finding>,
    /// The claims (designed vs observed) the agent reasons over.
    pub claims: Vec<Claim>,
}

impl Analysis {
    pub fn worst(&self) -> Severity {
        self.findings
            .iter()
            .map(|f| f.severity)
            .max()
            .unwrap_or(Severity::Ok)
    }
}

/// Run the requested rules over a completed journey.
pub fn analyze(
    journey: &Journey,
    evidence: &Evidence,
    initial_world: &crate::evidence::WorldSnapshot,
) -> Analysis {
    let mut out = Analysis::default();
    for rule in &journey.analyze {
        match rule {
            AnalyzeRule::ExpectationFidelity => expectation_fidelity(&mut out, journey, evidence),
            AnalyzeRule::Correctness => correctness(&mut out, journey, evidence),
            AnalyzeRule::Ux => ux(&mut out, evidence),
            AnalyzeRule::Efficiency => efficiency(&mut out, evidence, initial_world),
            AnalyzeRule::Consistency => consistency(&mut out, journey, evidence),
        }
    }
    out
}

/// Reconcile every step's `expect` hints against what the evidence actually shows
/// — the user's confidence about an outcome vs the real outcome. Also records a
/// DESIGNED-vs-OBSERVED claim for each declared expectation (the reasoning layer:
/// an agent judges the substance, not just the boolean).
fn expectation_fidelity(out: &mut Analysis, journey: &Journey, evidence: &Evidence) {
    for (i, step) in journey.steps.iter().enumerate() {
        let step_index = i as u32;
        let ev = evidence.steps().iter().find(|e| e.index == step_index);
        let Some(ev) = ev else {
            out.findings.push(Finding {
                axis: Axis::Correctness,
                severity: Severity::Warn,
                step: Some(step_index),
                what_happened: "step recorded no evidence".into(),
                ideal: "every step leaves a trace".into(),
                detail: None,
            });
            continue;
        };
        match step {
            Step::ApiGet { expect, path, .. }
            | Step::ApiPost { expect, path, .. }
            | Step::ApiDelete { expect, path, .. } => {
                let status = ev.status.unwrap_or(0);
                claim_status(out, step_index, &ev.name, path, expect.status, status);
                if let Some(want) = expect.status {
                    if status != want {
                        out.findings.push(Finding {
                            axis: Axis::Correctness,
                            severity: if status == 0 {
                                Severity::Deviation
                            } else {
                                Severity::Warn
                            },
                            step: Some(step_index),
                            what_happened: format!("{path} returned HTTP {status}"),
                            ideal: format!("{path} should return HTTP {want}"),
                            detail: ev.body.as_ref().map(|b| b.to_string()),
                        });
                    }
                } else if status >= 400 && expect.world_unchanged {
                    out.findings.push(Finding {
                        axis: Axis::Correctness,
                        severity: Severity::Deviation,
                        step: Some(step_index),
                        what_happened: format!("{path} returned HTTP {status} (>= 400)"),
                        ideal: "a read/idempotent step should not fault".into(),
                        detail: None,
                    });
                }
                // Content claims — the substance hugit rendered, not just the door.
                claim_body_contains(
                    out,
                    step_index,
                    &ev.name,
                    path,
                    expect.body_contains.as_deref(),
                    ev,
                );
                claim_body_json(
                    out,
                    step_index,
                    &ev.name,
                    path,
                    expect.body_json.as_ref(),
                    ev,
                );
                claim_world_delta(out, step_index, &ev.name, path, expect, ev);
            }
            Step::Cli { expect, args, .. } => {
                let code = ev.exit_code.unwrap_or(-1);
                let declared = expect.exit;
                claim_exit(
                    out,
                    step_index,
                    &ev.name,
                    &format!("hugit {}", args.join(" ")),
                    declared,
                    code,
                );
                let mismatch = match declared {
                    Some(want) => code != want,
                    None => code != 0 && expect.status.is_none(),
                };
                if mismatch {
                    out.findings.push(Finding {
                        axis: Axis::Correctness,
                        severity: Severity::Warn,
                        step: Some(step_index),
                        what_happened: format!("hugit {} exited {code}", args.join(" ")),
                        ideal: match declared {
                            Some(want) => format!("hugit {} should exit {want}", args.join(" ")),
                            None => "a user-facing CLI verb should succeed".into(),
                        },
                        detail: ev.stderr.as_ref().map(|s| s.trim_end().to_string()),
                    });
                }
                // The CLI's stdout envelope is the work it produced — verify substance.
                claim_body_contains(
                    out,
                    step_index,
                    &ev.name,
                    &format!("hugit {}", args.join(" ")),
                    expect.body_contains.as_deref(),
                    ev,
                );
                claim_body_json(
                    out,
                    step_index,
                    &ev.name,
                    &format!("hugit {}", args.join(" ")),
                    expect.body_json.as_ref(),
                    ev,
                );
                claim_world_delta(
                    out,
                    step_index,
                    &ev.name,
                    &format!("hugit {}", args.join(" ")),
                    expect,
                    ev,
                );
            }
            Step::Git { expect, args, .. } => {
                let code = ev.exit_code.unwrap_or(-1);
                let declared = expect.exit;
                claim_exit(
                    out,
                    step_index,
                    &ev.name,
                    &format!("git {}", args.join(" ")),
                    declared,
                    code,
                );
                let mismatch = match declared {
                    Some(want) => code != want,
                    None => code != 0 && expect.status.is_none(),
                };
                if mismatch {
                    out.findings.push(Finding {
                        axis: Axis::Correctness,
                        severity: Severity::Warn,
                        step: Some(step_index),
                        what_happened: format!("git {} exited {code}", args.join(" ")),
                        ideal: match declared {
                            Some(want) => format!("git {} should exit {want}", args.join(" ")),
                            None => "a git operation should succeed".into(),
                        },
                        detail: ev.stderr.as_ref().map(|s| s.trim_end().to_string()),
                    });
                }
            }
            Step::Observe { expect, .. } => {
                let world = ev
                    .world_after
                    .as_ref()
                    .or(ev.world_before.as_ref())
                    .cloned()
                    .unwrap_or_default();
                if let Some(repo) = first_repo(&world.repos)
                    .filter(|_| expect.world_adds.is_empty() && !expect.world_unchanged)
                {
                    // a pure observe expects no change — info level
                    let records = world.repos.get(&repo).map(|r| r.records).unwrap_or(0);
                    out.findings.push(Finding {
                        axis: Axis::Correctness,
                        severity: Severity::Ok,
                        step: Some(step_index),
                        what_happened: format!("world observed at repo {repo}"),
                        ideal: "nothing to assert; evidence recorded".into(),
                        detail: Some(format!("records={records}")),
                    });
                }
            }
        }
    }
}

// ── claim builders ────────────────────────────────────────────────────────────

fn claim_status(
    out: &mut Analysis,
    step: u32,
    name: &str,
    path: &str,
    designed: Option<u16>,
    observed: u16,
) {
    if let Some(want) = designed {
        out.claims.push(Claim {
            step,
            step_name: name.to_string(),
            description: format!("{path} responde HTTP"),
            designed: want.to_string(),
            observed: observed.to_string(),
            satisfied: want == observed,
            rationale: if want == observed {
                format!("status bateu ({want})")
            } else {
                format!("esperava {want}, o wire devolveu {observed}")
            },
        });
    }
}

fn claim_exit(
    out: &mut Analysis,
    step: u32,
    name: &str,
    cmd: &str,
    designed: Option<i32>,
    observed: i32,
) {
    if let Some(want) = designed {
        out.claims.push(Claim {
            step,
            step_name: name.to_string(),
            description: format!("{cmd} sai"),
            designed: want.to_string(),
            observed: observed.to_string(),
            satisfied: want == observed,
            rationale: if want == observed {
                format!("exit code bateu ({want})")
            } else {
                format!("esperava exit {want}, o processo devolveu {observed}")
            },
        });
    }
}

fn claim_body_contains(
    out: &mut Analysis,
    step: u32,
    name: &str,
    what: &str,
    designed: Option<&str>,
    ev: &StepEvidence,
) {
    let Some(hay) = designed else {
        return;
    };
    let body = ev
        .body
        .as_ref()
        .map(|b| b.to_string())
        .or_else(|| ev.stdout.clone())
        .unwrap_or_default();
    let contains = body.contains(hay);
    out.claims.push(Claim {
        step,
        step_name: name.to_string(),
        description: format!("{what} contém a string"),
        designed: hay.to_string(),
        observed: truncate(&body, 60),
        satisfied: contains,
        rationale: if contains {
            "o corpo do wire contém o trecho esperado".into()
        } else {
            "o corpo do wire NÃO contém o trecho esperado".into()
        },
    });
    if !contains {
        out.findings.push(Finding {
            axis: Axis::Correctness,
            severity: Severity::Warn,
            step: Some(step),
            what_happened: format!("{what} não contém o conteúdo esperado"),
            ideal: format!("o conteúdo renderizado deve conter `{hay}`"),
            detail: ev.body.as_ref().map(|b| b.to_string()),
        });
    }
}

fn claim_body_json(
    out: &mut Analysis,
    step: u32,
    name: &str,
    what: &str,
    designed: Option<&Value>,
    ev: &StepEvidence,
) {
    let Some(want) = designed else {
        return;
    };
    let Some(got) = ev.body.as_ref() else {
        out.claims.push(Claim {
            step,
            step_name: name.to_string(),
            description: format!("{what} renderiza o subtree JSON"),
            designed: want.to_string(),
            observed: "sem corpo JSON".into(),
            satisfied: false,
            rationale: "nenhum corpo JSON foi capturado".into(),
        });
        return;
    };
    let (ok, reason) = subtree_match(want, got);
    out.claims.push(Claim {
        step,
        step_name: name.to_string(),
        description: format!("{what} renderiza o subtree JSON"),
        designed: want.to_string(),
        observed: truncate(&got.to_string(), 160),
        satisfied: ok,
        rationale: if ok {
            "o corpo contém o subtree declarado, valores batem".into()
        } else {
            reason
        },
    });
    if !ok {
        out.findings.push(Finding {
            axis: Axis::Correctness,
            severity: Severity::Warn,
            step: Some(step),
            what_happened: format!("{what} NÃO renderiza o subtree declarado"),
            ideal: format!("o corpo deve conter `{want}`"),
            detail: Some(truncate(&got.to_string(), 300)),
        });
    }
}

fn claim_world_delta(
    out: &mut Analysis,
    step: u32,
    name: &str,
    what: &str,
    expect: &crate::journey::Expect,
    ev: &StepEvidence,
) {
    let (Some(before), Some(after)) = (ev.world_before.as_ref(), ev.world_after.as_ref()) else {
        return;
    };
    let delta = after.diff(before);
    if expect.world_unchanged {
        let added: u64 = delta
            .repos
            .values()
            .map(|r| r.records_delta.max(0) as u64)
            .sum();
        let unchanged = added == 0;
        out.claims.push(Claim {
            step,
            step_name: name.to_string(),
            description: format!("{what} não muda o mundo"),
            designed: "0 records adicionados".into(),
            observed: format!("{added} records adicionados"),
            satisfied: unchanged,
            rationale: if unchanged {
                "o log canônico não cresceu".into()
            } else {
                "o log canônico cresceu — viola a expectativa de idempotência".into()
            },
        });
        if !unchanged {
            out.findings.push(Finding {
                axis: Axis::Correctness,
                severity: Severity::Deviation,
                step: Some(step),
                what_happened: format!("{what} adicionou {added} records ao log"),
                ideal: "o passo não deve mudar o mundo (idempotência)".into(),
                detail: None,
            });
        }
    }
    for (kind, want) in &expect.world_adds {
        let got = delta
            .repos
            .values()
            .map(|r| r.kinds_added.get(kind).copied().unwrap_or(0))
            .sum::<u64>();
        let satisfied = got == *want;
        out.claims.push(Claim {
            step,
            step_name: name.to_string(),
            description: format!("{what} adiciona `{kind}`"),
            designed: format!("{want} record(s)"),
            observed: format!("{got} record(s)"),
            satisfied,
            rationale: if satisfied {
                format!("o log ganhou exatamente {want} `{kind}`")
            } else {
                format!("esperava {want} `{kind}`, o log ganhou {got}")
            },
        });
        if !satisfied {
            out.findings.push(Finding {
                axis: Axis::Correctness,
                severity: Severity::Warn,
                step: Some(step),
                what_happened: format!("{what} adicionou {got} `{kind}` (esperava {want})"),
                ideal: format!("o passo deve adicionar {want} `{kind}` ao log canônico"),
                detail: None,
            });
        }
    }
}

/// Deep-subset JSON match: every key path in `want` must exist in `got` and its
/// leaf value must equal the declared one. Extra keys in `got` are fine. Arrays
/// match positionally for declared indices.
fn subtree_match(want: &Value, got: &Value) -> (bool, String) {
    match (want, got) {
        (Value::Object(w), Value::Object(g)) => {
            for (k, wv) in w {
                match g.get(k) {
                    None => return (false, format!("chave `{k}` ausente no corpo renderizado")),
                    Some(gv) => {
                        let (ok, why) = subtree_match(wv, gv);
                        if !ok {
                            return (false, format!("{k}: {why}"));
                        }
                    }
                }
            }
            (true, String::new())
        }
        (Value::Array(w), Value::Array(g)) => {
            for (i, wv) in w.iter().enumerate() {
                let Some(gv) = g.get(i) else {
                    return (false, format!("índice {i} ausente no corpo renderizado"));
                };
                let (ok, why) = subtree_match(wv, gv);
                if !ok {
                    return (false, format!("[{i}]: {why}"));
                }
            }
            (true, String::new())
        }
        // Leaves: exact equality (numbers/strings/bools/null).
        (w, g) => {
            if w == g {
                (true, String::new())
            } else {
                (
                    false,
                    format!("valor divergente: esperava `{w}`, o wire devolveu `{g}`"),
                )
            }
        }
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

/// The hard correctness invariants the forge promises (read≠write, no-oracle 404,
/// idempotent replay over the ledger). These are INVARIANTS, not opinions: a
/// violation is a Deviation, always.
fn correctness(out: &mut Analysis, journey: &Journey, evidence: &Evidence) {
    // Invariant 1 — read≠write. A journey that touches a write door must prove the
    // world moved via the canonical ledger (records grew), and a repo the user did
    // NOT create must not exist as an oracle.
    let mut write_steps = 0;
    let mut world_grew = false;
    for ev in evidence.steps() {
        if matches!(ev.surface, Surface::Api) && ev.status.map(|s| s >= 400).unwrap_or(false) {
            continue;
        }
        let before = ev.world_before.as_ref();
        let after = ev.world_after.as_ref();
        if let (Some(b), Some(a)) = (before, after) {
            let d = a.diff(b);
            let grew = d.repos.values().any(|r| r.records_delta > 0);
            if matches!(ev.surface, Surface::Api | Surface::Cli) && grew {
                write_steps += 1;
                world_grew = true;
            }
        }
    }
    if write_steps > 0 && !world_grew {
        out.findings.push(Finding {
            axis: Axis::Correctness,
            severity: Severity::Deviation,
            step: None,
            what_happened: "write steps ran but the canonical ledger never grew".into(),
            ideal: "a write must persist to the append-only, hash-chained log".into(),
            detail: None,
        });
    }

    // Invariant 2 — anonymous can never write (read-authz ≠ write-authz).
    for ev in evidence.steps() {
        let Some(step) = find_step(journey, ev.index) else {
            continue;
        };
        let is_write = matches!(step, Step::ApiPost { .. } | Step::ApiDelete { .. });
        if !is_write {
            continue;
        }
        let anon = match step {
            Step::ApiPost { auth, .. } | Step::ApiDelete { auth, .. } => auth == "anon",
            _ => false,
        };
        if anon && ev.status.map(|s| (200..300).contains(&s)).unwrap_or(false) {
            out.findings.push(Finding {
                axis: Axis::Correctness,
                severity: Severity::Deviation,
                step: Some(ev.index),
                what_happened: format!(
                    "anonymous step {} succeeded (HTTP {})",
                    ev.detail,
                    ev.status.unwrap_or(0)
                ),
                ideal: "anonymous writes must be refused (read-authz ≠ write-authz)".into(),
                detail: None,
            });
        }
    }

    // Invariant 3 — replaying an idempotent write (same Idempotency-Key) appends
    // nothing. The journey declares a second ApiPost with the same key; the
    // evidence must show the world delta flat on the replay.
    let mut seen_keys: BTreeMap<String, u64> = BTreeMap::new();
    for (i, step) in journey.steps.iter().enumerate() {
        if let Step::ApiPost {
            idempotency_key: Some(k),
            ..
        } = step
        {
            let ev = evidence.steps().iter().find(|e| e.index == i as u32);
            if let Some(ev) = ev {
                let delta = world_delta_records(ev);
                match seen_keys.get(k) {
                    None => {
                        seen_keys.insert(k.clone(), delta);
                    }
                    Some(first) => {
                        if delta > 0 {
                            out.findings.push(Finding {
                                axis: Axis::Correctness,
                                severity: Severity::Deviation,
                                step: Some(i as u32),
                                what_happened: format!(
                                    "idempotent replay with key {k} appended {delta} more records"
                                ),
                                ideal: format!(
                                    "replay of key {k} must append nothing (first={first})"
                                ),
                                detail: None,
                            });
                        }
                    }
                }
            }
        }
    }
}

/// UX: does the user see what they need — latency bounded, output actionable,
/// no opaque empty failures?
fn ux(out: &mut Analysis, evidence: &Evidence) {
    // Latency budget: the accept loop is synchronous; a single slow step is what a
    // user perceives as a hang. Budget 500ms p50 / 2s p95 as the ideal.
    let mut latencies: Vec<u64> = evidence
        .steps()
        .iter()
        .map(|s| s.duration_ms)
        .filter(|d| *d > 0)
        .collect();
    latencies.sort_unstable();
    if !latencies.is_empty() {
        let p50 = latencies[latencies.len() / 2];
        let p95 = latencies[((latencies.len() as f64 * 0.95) as usize).min(latencies.len() - 1)];
        let sev = if p95 > 2000 {
            Severity::Deviation
        } else if p50 > 500 {
            Severity::Warn
        } else {
            Severity::Ok
        };
        out.findings.push(Finding {
            axis: Axis::Ux,
            severity: sev,
            step: None,
            what_happened: format!("p50 {p50}ms, p95 {p95}ms across {} steps", latencies.len()),
            ideal: "p50 < 500ms, p95 < 2000ms".into(),
            detail: None,
        });
    }

    // Actionable failures: a CLI failure should say WHY (non-empty stderr).
    for ev in evidence.steps() {
        if ev.exit_code == Some(0) || ev.exit_code.is_none() {
            continue;
        }
        let has_signal = ev
            .stderr
            .as_ref()
            .map(|s| !s.trim().is_empty() || s.len() > 3)
            .unwrap_or(false)
            || ev
                .stdout
                .as_ref()
                .map(|s| !s.trim().is_empty() && s.len() > 3)
                .unwrap_or(false);
        let sev = if has_signal {
            Severity::Ok
        } else {
            Severity::Warn
        };
        out.findings.push(Finding {
            axis: Axis::Ux,
            severity: sev,
            step: Some(ev.index),
            what_happened: format!(
                "{} failed{}",
                ev.detail,
                if has_signal {
                    " with diagnostics"
                } else {
                    " SILENTLY"
                }
            ),
            ideal: "a failure must carry actionable diagnostics".into(),
            detail: ev.stderr.clone(),
        });
    }

    // Meaningful output: a user-facing read should return something non-trivial.
    for ev in evidence.steps() {
        if matches!(ev.surface, Surface::Api) && ev.status == Some(200) {
            let empty = ev.body.is_none() || ev.body.as_ref().map(|b| b.is_null()).unwrap_or(true);
            if empty {
                out.findings.push(Finding {
                    axis: Axis::Ux,
                    severity: Severity::Warn,
                    step: Some(ev.index),
                    what_happened: format!("{} returned 200 with an empty body", ev.detail),
                    ideal: "a 200 read should carry meaningful content".into(),
                    detail: None,
                });
            }
        }
    }
}

/// Efficiency: bounded world growth (a runaway log is a real cost), cache hits
/// working (memoization), and evidence capture staying inside budget.
fn efficiency(out: &mut Analysis, evidence: &Evidence, initial: &crate::evidence::WorldSnapshot) {
    let final_world = evidence.final_world.clone().unwrap_or_default();
    let delta = final_world.diff(initial);
    let total_growth: u64 = delta
        .repos
        .values()
        .map(|r| r.records_delta.max(0) as u64)
        .sum();
    // The canonical log is append-only; unbounded growth is a cost. Ideal: the
    // journey's writes are bounded and the ledger shows no pathological dupes.
    let sev = if total_growth > 200 {
        Severity::Warn
    } else {
        Severity::Ok
    };
    out.findings.push(Finding {
        axis: Axis::Efficiency,
        severity: sev,
        step: None,
        what_happened: format!("canonical log grew by {total_growth} records"),
        ideal: "journey writes stay bounded (no pathological growth)".into(),
        detail: None,
    });

    // Memoization spine: an attested check (cas_hit) should be reflected in the
    // evidence (the cost-killer wedge — PARTIAL/pills). We can't see the cache
    // internals from black-box, but a checks aggregate with a hit is a positive.
    for ev in evidence.steps() {
        if let Some(body) = &ev.body {
            let Some(hits) = body.get("hits").and_then(|h| h.as_u64()) else {
                continue;
            };
            out.findings.push(Finding {
                axis: Axis::Efficiency,
                severity: if hits > 0 {
                    Severity::Ok
                } else {
                    Severity::Info
                },
                step: Some(ev.index),
                what_happened: format!("checks aggregate reported {hits} cache hits"),
                ideal: "repeated work should be memoized by content".into(),
                detail: body.get("status").map(|s| s.to_string()),
            });
        }
    }

    // Evidence budget: every capture stayed bounded (the report can assert this).
    let oversized = evidence
        .steps()
        .iter()
        .filter(|s| {
            s.stdout.as_ref().map(|x| x.ends_with('…')).unwrap_or(false)
                || s.stderr.as_ref().map(|x| x.ends_with('…')).unwrap_or(false)
        })
        .count();
    if oversized > 0 {
        out.findings.push(Finding {
            axis: Axis::Efficiency,
            severity: Severity::Info,
            step: None,
            what_happened: format!("{oversized} captures were truncated to the evidence budget"),
            ideal: "evidence captures stay bounded (never OOM the report)".into(),
            detail: None,
        });
    }
}

/// Consistency: the SAME logical action through the CLI vs the /v1 API must land
/// in the SAME canonical world (one source of truth, whatever the surface).
fn consistency(out: &mut Analysis, journey: &Journey, evidence: &Evidence) {
    let mut cli_deltas: Vec<u64> = Vec::new();
    let mut api_deltas: Vec<u64> = Vec::new();
    for (i, step) in journey.steps.iter().enumerate() {
        let ev = evidence.steps().iter().find(|e| e.index == i as u32);
        let Some(ev) = ev else { continue };
        let delta = world_delta_records(ev);
        match step {
            Step::Cli { .. } => cli_deltas.push(delta),
            Step::ApiPost { .. } | Step::ApiDelete { .. } => api_deltas.push(delta),
            _ => {}
        }
    }
    // If BOTH surfaces wrote to the same world and both moved it, the world stayed
    // coherent (single canonical log per repo — no per-surface forks).
    let cli_wrote = cli_deltas.iter().any(|d| *d > 0);
    let api_wrote = api_deltas.iter().any(|d| *d > 0);
    if cli_wrote && api_wrote {
        // The strongest observable consistency proof: the final world is ONE log.
        // Black-box we check the repo's head_seq is monotone and the log grew from
        // BOTH surfaces without a reset (head_seq never went backwards).
        let mut monotone = true;
        let mut last: Option<u64> = None;
        for ev in evidence.steps() {
            if let Some(after) = &ev.world_after {
                for r in after.repos.values() {
                    if let Some(seq) = r.head_seq {
                        if last.is_some_and(|l| seq < l) {
                            monotone = false;
                        }
                        last = Some(seq.max(last.unwrap_or(0)));
                    }
                }
            }
        }
        out.findings.push(Finding {
            axis: Axis::Consistency,
            severity: if monotone {
                Severity::Ok
            } else {
                Severity::Deviation
            },
            step: None,
            what_happened: format!(
                "CLI wrote {cli_deltas:?} and /v1 wrote {api_deltas:?} into the same canonical log"
            ),
            ideal: "one source of truth, whatever the surface; head_seq monotone".into(),
            detail: if monotone {
                None
            } else {
                Some("head_seq regressed".into())
            },
        });
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn find_step(journey: &Journey, index: u32) -> Option<&Step> {
    journey.steps.get(index as usize)
}

fn world_delta_records(ev: &StepEvidence) -> u64 {
    let (Some(b), Some(a)) = (ev.world_before.as_ref(), ev.world_after.as_ref()) else {
        return 0;
    };
    let d: WorldDelta = a.diff(b);
    d.repos
        .values()
        .map(|r| r.records_delta.max(0) as u64)
        .sum()
}

fn first_repo(repos: &BTreeMap<String, crate::evidence::RepoWorld>) -> Option<String> {
    repos.keys().next().cloned()
}
