//! # Journey — a user's goal, executed, not asserted
//!
//! A journey is a high-level goal ("open a PR, run checks, land, and read the
//! attested cost") expressed as STEP ACTIONS over the real surfaces. The executor
//! runs each step the way a human/agent would: it ACTS, captures the outcome into
//! evidence, and ADAPTS (a recoverable failure is evidence + a retry/alternative
//! path — NOT a broken assertion). Nothing here hardcodes an expected status; the
//! DSL's `expect` hints are ANALYSIS inputs the [`crate::analyzer`] turns into
//! findings.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use serde::Deserialize;
use serde_json::Value;

use crate::evidence::{Evidence, StepEvidence, Surface, WorldSnapshot, bounded};
use crate::harness::{Harness, have_git, http_req};

/// The full journey definition (JSON file on disk).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Journey {
    pub name: String,
    pub goal: String,
    /// The tenant/user identity the journey runs AS.
    pub identity: Identity,
    /// World building that runs BEFORE the engine boots (seams are seeded here —
    /// a repo's git content, a pre-seeded log — so the served world is real).
    #[serde(default)]
    pub setup: Vec<SetupStep>,
    /// The goal's step sequence.
    pub steps: Vec<Step>,
    /// Quality rules the analyzer applies to the evidence AFTER the journey.
    #[serde(default)]
    pub analyze: Vec<AnalyzeRule>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub org: String,
    pub user: String,
    /// `fresh` step-up (reauthentication) — minted with `fresh_auth`, used by the
    /// GDPR/erase journeys.
    #[serde(default)]
    pub fresh_auth: bool,
}

/// One well-formed log record to seed (kind + payload object).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedRecord {
    pub kind: String,
    pub payload: Value,
}

/// A world-building action that runs BEFORE the engine serves (matches the
/// `ultimate_qa.rs` `spawn_with` seam — real files the served engine reads).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SetupStep {
    /// Seed `dir/<repo>.json` with a genesis `repo.meta` record.
    SeedMeta {
        repo: String,
        visibility: String,
        owner_tenant: String,
    },
    /// Wire a real (seeded) git object graph + refs onto `repo` for serving.
    WireGit { repo: String },
    /// Append well-formed records onto `repo`'s canonical log via the SAME
    /// `EventLog::append_for_test` the QA suites use — real hash chaining, real
    /// seq monotonicity (never hand-faked `this_hash` bytes). A prior `seed_meta`
    /// for the same repo composes (its records become the chain head).
    SeedRecords {
        repo: String,
        #[serde(default)]
        records: Vec<SeedRecord>,
    },
    /// Seed the full attested-cost world (campaign opened, intent landed, PR
    /// opened, and the `pr.envelope` + `intent.envelope` ContextEnvelope records
    /// the `/insights` cost X-ray + spend_proof rows read) in ONE step. Mirrors
    /// `parity_insights.rs::spend_proof_wired_on_cost_xray_and_ledger_row` — the
    /// envelope JSON shape is built by the harness, so the journey file stays
    /// compact and the shape cannot drift from the tested one.
    SeedEnvelopeWorld {
        repo: String,
        campaign: String,
        pr_id: String,
        intent_ids: Vec<String>,
        /// Attested spend for the PR-altitude envelope, in integer micro-USD.
        cost_usd_micros: u64,
    },
    /// Purge the world so upstream steps start from a clean slate.
    CleanWorld,
}

/// The `expect` hint attached to a step: what the USER expected to happen. This
/// is ANALYSIS input, never a hard assert — a mismatch becomes a finding.
#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Expect {
    /// Expected HTTP status class the user was confident about (e.g. 200).
    #[serde(rename = "status")]
    pub status: Option<u16>,
    /// Expected process exit code for a CLI/git step (e.g. the denial of an
    /// anonymous git clone to a private repo, or an anonymous `hugit` write). A
    /// declared exit turns the analyzer's "should succeed" Warn into a fidelity
    /// check (non-zero exit becomes CORRECT evidence, not a defect).
    #[serde(rename = "exit")]
    pub exit: Option<i32>,
    /// The body must CONTAIN this string (when the user eyeballs output).
    pub body_contains: Option<String>,
    /// A JSON SUBTREE the user expects the produced body to contain. This turns
    /// the analyzer from "did a door open" into "is the STUFF hugit rendered
    /// actually the designed values" — e.g. `{"cost_xray_totals":
    /// {"cost_micros": 8400000}}` asserts the exact attested-cost math landed in
    /// the wire body. A DEEP-SUBSET match: every declared key path must exist in
    /// the produced body and its leaf value must equal the declared one (extra
    /// keys in the body are fine; arrays must match by position for declared
    /// indices). Use `body_contains` for a loose substring check instead.
    #[serde(rename = "body_json")]
    pub body_json: Option<Value>,
    /// The user expects the world to have changed by these records (kind → count).
    #[serde(rename = "world_adds")]
    pub world_adds: BTreeMap<String, u64>,
    /// The user expects NO records to be added (idempotency/read expectations).
    #[serde(rename = "world_unchanged")]
    pub world_unchanged: bool,
}

/// One goal-oriented step. `action` discriminates the product surface verb; every
/// step carries a name + the goal it serves + optional analysis hints.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Step {
    /// `GET /v1/<path>` (auth = `owner` token / `dev` operator / `anon`).
    ApiGet {
        name: String,
        goal: String,
        path: String,
        auth: String,
        #[serde(default)]
        expect: Expect,
    },
    /// `POST /v1/<path>` with a JSON body.
    ApiPost {
        name: String,
        goal: String,
        path: String,
        auth: String,
        body: Value,
        #[serde(default)]
        idempotency_key: Option<String>,
        #[serde(default)]
        expect: Expect,
    },
    /// `DELETE /v1/<path>`.
    ApiDelete {
        name: String,
        goal: String,
        path: String,
        auth: String,
        #[serde(default)]
        expect: Expect,
    },
    /// Run the REAL `hugit` CLI binary (args after the verb, e.g. `["pr","list"]`).
    Cli {
        name: String,
        goal: String,
        repo: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        expect: Expect,
    },
    /// Run a real `git` command in a scratch worktree.
    Git {
        name: String,
        goal: String,
        repo: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        expect: Expect,
    },
    /// Raw world observation: snapshot the world (used at journey end).
    Observe {
        name: String,
        goal: String,
        #[serde(default)]
        expect: Expect,
    },
}

/// An analysis rule the analyzer applies to the evidence (quality, not asserts).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "rule", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnalyzeRule {
    /// Every step's expected world delta must match what the evidence shows.
    ExpectationFidelity,
    /// Correctness invariants: read≠write, no-oracle 404s, idempotent replay.
    Correctness,
    /// UX: actionable errors, meaningful output, latency budget respected.
    Ux,
    /// Efficiency: bounded world growth, cache/memoization working.
    Efficiency,
    /// Consistency: same journey through CLI and /v1 yields the same world.
    Consistency,
}

/// The result of running a journey to completion.
pub struct JourneyRun {
    pub journey_name: String,
    pub evidence: Evidence,
    /// Whether every step completed (a step that aborts the journey is a finding,
    /// not a panic — `aborted` marks the run stopped early).
    pub aborted_at: Option<u32>,
    /// World captured right before the journey started.
    pub initial_world: WorldSnapshot,
}

/// Run one journey against a freshly-booted harness. The world is built from
/// `setup`, the engine boots, the steps execute against the REAL surfaces, and
/// every step becomes one immutable [`StepEvidence`]. No step asserts.
pub fn run_journey(journey: &Journey) -> JourneyRun {
    let mut abort_at: Option<u32> = None;
    let mut evidence = Evidence::new();

    // ── Setup (world building BEFORE the engine serves) ─────────────────────
    let harness;
    {
        let setup_clone = journey.setup.clone();
        harness = Harness::spawn_with(move |dir, state| {
            for setup in &setup_clone {
                match setup {
                    SetupStep::SeedMeta {
                        repo,
                        visibility,
                        owner_tenant,
                    } => crate::harness::seed_meta(dir, repo, visibility, owner_tenant),
                    SetupStep::WireGit { repo } => crate::harness::wire_git(state, repo),
                    SetupStep::SeedRecords { repo, records } => {
                        crate::harness::seed_records(dir, repo, records);
                    }
                    SetupStep::SeedEnvelopeWorld {
                        repo,
                        campaign,
                        pr_id,
                        intent_ids,
                        cost_usd_micros,
                    } => crate::harness::seed_env_world(
                        dir,
                        repo,
                        campaign,
                        pr_id,
                        intent_ids,
                        *cost_usd_micros,
                    ),
                    SetupStep::CleanWorld => {
                        // Nothing to do pre-boot; the world is fresh already.
                    }
                }
            }
        });
    }
    let log_dir = harness.log_dir.clone();
    let initial_world = harness.snapshot();
    let owner = harness.mint(
        &journey.identity.org,
        &journey.identity.user,
        journey.identity.fresh_auth,
    );
    let dev = crate::harness::DEV.to_string();

    // ── The steps (goal → action → evidence) ────────────────────────────────
    let mut seq: u32 = 0;
    let steps = journey.steps.clone();
    for step in &steps {
        if abort_at.is_some() {
            break;
        }
        let world_before = harness.snapshot();
        let ts_ms = millis();
        let start = Instant::now();
        let rec = execute_step(&harness, step, &owner, &dev, &log_dir, seq);
        let took = start.elapsed().as_millis() as u64;
        let mut rec = rec;
        rec.ts_ms = ts_ms;
        rec.duration_ms = took;
        rec.world_before = Some(world_before);
        rec.world_after = Some(harness.snapshot());

        // Adaptation posture: the executor does NOT hardcode outcomes, but a step
        // that literally cannot proceed (surface missing) is recorded as a skip.
        if is_surface_skip(&rec) {
            abort_at = Some(seq);
        }
        evidence.push(rec);
        seq += 1;
    }
    if let Some(s) = abort_at {
        // Final observe: record the world we stopped at.
        let world = harness.snapshot();
        let rec = StepEvidence {
            index: seq,
            name: "observe".into(),
            surface: Surface::Setup,
            goal: "capture the world where the journey stopped".into(),
            detail: "final snapshot".into(),
            command: None,
            stdout: None,
            stderr: None,
            status: None,
            exit_code: None,
            body: None,
            duration_ms: 0,
            world_before: Some(world.clone()),
            world_after: Some(world),
            ts_ms: millis(),
        };
        evidence.push(rec);
        evidence.final_world = harness.snapshot().into();
        return JourneyRun {
            journey_name: journey.name.clone(),
            evidence,
            aborted_at: Some(s),
            initial_world,
        };
    }
    evidence.final_world = harness.snapshot().into();
    JourneyRun {
        journey_name: journey.name.clone(),
        evidence,
        aborted_at: None,
        initial_world,
    }
}

fn is_surface_skip(rec: &StepEvidence) -> bool {
    matches!(rec.surface, Surface::Cli | Surface::Git) && rec.exit_code == Some(skip_exit_code())
}

/// Sentinel exit code a skipped CLI/git step reports (a skip, not a real failure).
fn skip_exit_code() -> i32 {
    i32::MIN + 7 // arbitrary, documented: means "surface unavailable, skipped"
}

fn millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn auth_header(auth: &str, owner: &str, dev: &str) -> Option<String> {
    match auth {
        "owner" => Some(format!("Bearer {owner}")),
        "dev" => Some(format!("Bearer {dev}")),
        "anon" => None,
        _ => None,
    }
}

fn execute_step(
    harness: &Harness,
    step: &Step,
    owner: &str,
    dev: &str,
    log_dir: &Path,
    index: u32,
) -> StepEvidence {
    match step {
        Step::ApiGet {
            name,
            goal,
            path,
            auth,
            expect: _expect,
        } => {
            let hdr = auth_header(auth, owner, dev);
            let resp = http_req(
                &harness.addr,
                "GET",
                path,
                &hdr.iter()
                    .map(|h| ("Authorization", h.as_str()))
                    .collect::<Vec<_>>(),
                &[],
            );
            StepEvidence {
                index,
                name: name.clone(),
                surface: Surface::Api,
                goal: goal.clone(),
                detail: format!("GET {path} (auth={auth})"),
                command: None,
                stdout: None,
                stderr: None,
                status: Some(resp.status),
                exit_code: None,
                body: resp.json(),
                duration_ms: 0,
                world_before: None,
                world_after: None,
                ts_ms: 0,
            }
        }
        Step::ApiPost {
            name,
            goal,
            path,
            auth,
            body,
            idempotency_key,
            expect: _expect,
        } => {
            let mut headers: Vec<(String, String)> =
                vec![("Content-Type".into(), "application/json".into())];
            if let Some(a) = auth_header(auth, owner, dev) {
                headers.push(("Authorization".into(), a));
            }
            if let Some(k) = idempotency_key {
                headers.push(("Idempotency-Key".into(), k.clone()));
            }
            let headers_ref: Vec<(&str, &str)> = headers
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();
            let body_bytes = body.to_string().into_bytes();
            let resp = http_req(&harness.addr, "POST", path, &headers_ref, &body_bytes);
            StepEvidence {
                index,
                name: name.clone(),
                surface: Surface::Api,
                goal: goal.clone(),
                detail: format!(
                    "POST {path} (auth={auth}){k}",
                    k = idempotency_key
                        .as_ref()
                        .map(|k| format!(" key={k}"))
                        .unwrap_or_default()
                ),
                command: None,
                stdout: None,
                stderr: None,
                status: Some(resp.status),
                exit_code: None,
                body: resp.json(),
                duration_ms: 0,
                world_before: None,
                world_after: None,
                ts_ms: 0,
            }
        }
        Step::ApiDelete {
            name,
            goal,
            path,
            auth,
            expect: _expect,
        } => {
            let hdr = auth_header(auth, owner, dev);
            let resp = http_req(
                &harness.addr,
                "DELETE",
                path,
                &hdr.iter()
                    .map(|h| ("Authorization", h.as_str()))
                    .collect::<Vec<_>>(),
                &[],
            );
            StepEvidence {
                index,
                name: name.clone(),
                surface: Surface::Api,
                goal: goal.clone(),
                detail: format!("DELETE {path} (auth={auth})"),
                command: None,
                stdout: None,
                stderr: None,
                status: Some(resp.status),
                exit_code: None,
                body: resp.json(),
                duration_ms: 0,
                world_before: None,
                world_after: None,
                ts_ms: 0,
            }
        }
        Step::Cli {
            name,
            goal,
            repo,
            args,
            expect: _expect,
        } => {
            let Some(bin) = crate::harness::find_hugit_bin() else {
                return StepEvidence {
                    index,
                    name: name.clone(),
                    surface: Surface::Cli,
                    goal: goal.clone(),
                    detail: format!("hugit {}", args.join(" ")),
                    command: None,
                    stdout: Some("hugit CLI binary not resolvable".into()),
                    stderr: None,
                    status: None,
                    exit_code: Some(skip_exit_code()),
                    body: None,
                    duration_ms: 0,
                    world_before: None,
                    world_after: None,
                    ts_ms: 0,
                };
            };
            let world = log_dir.join(format!("{repo}.json"));
            let out = std::process::Command::new(&bin)
                .env("HUGIT_LOG", &world)
                .args(args)
                .current_dir(log_dir)
                .output()
                .expect("spawn hugit");
            let stdout = bounded(&String::from_utf8_lossy(&out.stdout));
            let stderr = bounded(&String::from_utf8_lossy(&out.stderr));
            let body = serde_json::from_str(&stdout).ok();
            StepEvidence {
                index,
                name: name.clone(),
                surface: Surface::Cli,
                goal: goal.clone(),
                detail: format!("hugit {}", args.join(" ")),
                command: Some(bin.to_string_lossy().into_owned()),
                stdout: Some(stdout),
                stderr: Some(stderr),
                status: None,
                exit_code: Some(out.status.code().unwrap_or(-1)),
                body,
                duration_ms: 0,
                world_before: None,
                world_after: None,
                ts_ms: 0,
            }
        }
        Step::Git {
            name,
            goal,
            repo,
            args,
            expect: _expect,
        } => {
            if !have_git() {
                return StepEvidence {
                    index,
                    name: name.clone(),
                    surface: Surface::Git,
                    goal: goal.clone(),
                    detail: format!("git {}", args.join(" ")),
                    command: None,
                    stdout: Some("git not on PATH".into()),
                    stderr: None,
                    status: None,
                    exit_code: Some(skip_exit_code()),
                    body: None,
                    duration_ms: 0,
                    world_before: None,
                    world_after: None,
                    ts_ms: 0,
                };
            }
            let work = scratch_worktree(harness, repo);
            let url = format!("http://{}/{}", harness.addr, repo);
            let mut cmd = std::process::Command::new("git");
            crate::harness::git_cfg(&mut cmd);
            cmd.args(["-C", work.to_str().unwrap()])
                .args(args)
                .arg(&url);
            let out = cmd.output().expect("run git");
            let stdout = bounded(&String::from_utf8_lossy(&out.stdout));
            let stderr = bounded(&String::from_utf8_lossy(&out.stderr));
            let body = serde_json::from_str(&stdout).ok();
            StepEvidence {
                index,
                name: name.clone(),
                surface: Surface::Git,
                goal: goal.clone(),
                detail: format!("git {} <url>", args.join(" ")),
                command: Some(format!("git {} {}", args.join(" "), &url)),
                stdout: Some(stdout),
                stderr: Some(stderr),
                status: None,
                exit_code: Some(out.status.code().unwrap_or(-1)),
                body,
                duration_ms: 0,
                world_before: None,
                world_after: None,
                ts_ms: 0,
            }
        }
        Step::Observe {
            name,
            goal,
            expect: _expect,
        } => {
            let world = harness.snapshot();
            let mut kinds: BTreeMap<String, u64> = BTreeMap::new();
            for r in world.repos.values() {
                for (k, c) in &r.kinds {
                    *kinds.entry(k.clone()).or_insert(0) += c;
                }
            }
            StepEvidence {
                index,
                name: name.clone(),
                surface: Surface::Setup,
                goal: goal.clone(),
                detail: "observe world".into(),
                command: None,
                stdout: None,
                stderr: None,
                status: None,
                exit_code: None,
                body: Some(serde_json::json!({ "total_records":
                    world.repos.values().map(|r| r.records).sum::<u64>() })),
                duration_ms: 0,
                world_before: None,
                world_after: None,
                ts_ms: 0,
            }
        }
    }
}

/// A scratch worktree per repo (reuse across steps of the same journey). The
/// harness keeps ONE worktree per repo under its per-RUN `work_dir`, so re-runs
/// never collide with a prior run's clone destination.
fn scratch_worktree(harness: &Harness, repo: &str) -> PathBuf {
    let base = harness.work_dir.join(repo);
    std::fs::create_dir_all(&base).expect("worktree base");
    base
}
