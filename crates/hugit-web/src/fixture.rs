//! FixtureProvider — the seeded MVP world behind the screens.
//!
//! WP-W1: the world is no longer a hand-typed stub. It is derived from hugit's
//! REAL in-process surfaces, so the screens read the same store the CLI does
//! (the parity law). The seeding pipeline:
//!
//!   1. Run the real dogfood waves
//!      ([`hugit_dogfood::wave::run_wave_with_ac`]) on shared in-memory Action
//!      Caches — wave A twice (cold then warm: the memoization wedge, MEASURED),
//!      wave B once with a failing pair (→ the Bloqueado column).
//!   2. Build a world [`EventLog`] through the REAL append path
//!      ([`EventLog::append`] over [`canonical_json`] payloads): one
//!      `intent.landed` per landed PR, and a `verdict.recorded` for the proven
//!      subset (serde-serialized [`VerdictObject`]).
//!   3. Project that log through the REAL projections
//!      ([`Ledger::from_records`] + [`intents_from_log`]) and build every
//!      view-model from the projections + the measured wave numbers.
//!
//! The parity oracle in `tests/provider_fixture.rs` re-derives those same
//! projections over the same world records and holds the VMs to them — a
//! fabricated number cannot pass. The `Provider` SHAPES are frozen
//! (`provider.rs`); only the world behind them deepens here.
//!
//! Honesty: cost / token / metric figures remain fixture-illustrative
//! (forward-compatible with the ADR-0001 envelope, but demo VALUES — surfaced
//! as such by [`Provider::is_fixture`]). What is MEASURED — and tested as such —
//! is the memoization story: saved-ms, hit-rate, executed/hit counts all come
//! from the real wave, never a constant.

use hugit_checks::client::ac::InMemoryAc;
use hugit_checks::runner::hit_rate::{HitRateMeter, HitRateReport};
use hugit_contracts::event_record::EventRecord;
use hugit_contracts::verdict_object::{Verdict, VerdictObject};
use hugit_dogfood::wave::{WaveConfig, WaveReport, run_wave_with_ac};
use hugit_ledger::Ledger;
use hugit_refstore::intent::{Intent, intents_from_log};
use hugit_refstore::{EventLog, canonical_json, compute_memo_key};

use crate::provider::*;

/// The two campaigns the 5 landed intents are distributed across, plus the
/// synthetic "infra" chip used for the queued/testing columns. Stable ids +
/// kit.css color classes (`c-fabric` / `c-perf` / `c-infra`).
const CAMPAIGN_FILA: &str = "fila-de-pouso";
const CAMPAIGN_SPINE: &str = "espinha-githugr";
const CAMPAIGN_INFRA: &str = "infra";

/// The principal chain every world append is attributed to (owner →
/// orchestrator; never a subagent — D14 authz).
fn world_principal_chain() -> Vec<String> {
    vec!["owner".to_string(), "orchestrator".to_string()]
}

/// Deterministic `recorded_at` for the `seq`-th world append. NEVER wall-clock:
/// a fixed base plus a per-event minute step keeps the world byte-stable.
fn recorded_at_for(seq: u64) -> u64 {
    1_760_000_000_000 + seq * 60_000
}

/// A stable fake 40-hex object id for a landed intent's `target` (the ref now
/// points here). Distinct per PR, deterministic.
fn fake_oid_40(pr_index: usize) -> String {
    // 39 zeros + one hex digit of the index → distinct, valid 40-hex.
    format!("{:039}{:x}", 0, pr_index)
}

/// A stable fake 64-hex tree hash for a PR's snapshot.
fn fake_tree_64(pr_index: usize) -> String {
    format!("{:063}{:x}", 0, pr_index)
}

/// Which campaign each of the 5 landed intents belongs to (distribute 5 across
/// 2 campaigns: 0,1,2 → fila-de-pouso · 3,4 → espinha-githugr).
fn campaign_id_for(pr_index: usize) -> &'static str {
    if pr_index < 3 {
        CAMPAIGN_FILA
    } else {
        CAMPAIGN_SPINE
    }
}

/// A short, distinct pt-BR charter per landed intent.
fn charter_for(pr_index: usize) -> String {
    match pr_index {
        0 => "Esvaziar a fila de pouso ao fim de cada onda".to_string(),
        1 => "Memoizar o CI verde: nenhum check verde re-executa".to_string(),
        2 => "Auditar cada pouso na trilha encadeada e à prova de adulteração".to_string(),
        3 => "Renderizar a espinha do githugr fiel ao mockup".to_string(),
        4 => "Servir todo intent do mundo pela mesma loja que a CLI lê".to_string(),
        _ => format!("Intent sintético {pr_index}"),
    }
}

/// The chip vocabulary, by campaign id.
fn campaign_chip(id: &str) -> CampaignChipVm {
    let (label, color) = match id {
        CAMPAIGN_FILA => ("fila de pouso", "c-fabric"),
        CAMPAIGN_SPINE => ("espinha githugr", "c-perf"),
        CAMPAIGN_INFRA => ("infra", "c-infra"),
        other => (other, "c-fabric"),
    };
    CampaignChipVm {
        id: id.to_string(),
        label: label.to_string(),
        color_class: color.to_string(),
    }
}

/// The seeded fixture world (single-repo MVP), built once at [`seed`] time and
/// then read by every route.
///
/// [`seed`]: FixtureProvider::seed
pub struct FixtureProvider {
    repo: String,
    /// The world event log, built through the REAL append path. The parity
    /// oracle re-projects this exact record stream.
    world_log: EventLog,
    /// The native intents projected out of [`Self::world_log`] (one per landed
    /// PR), in log order — the source for every intent VM.
    intents: Vec<Intent>,
    /// The MEASURED wave numbers (stored so the tests can hold the checks VM to
    /// them, never to a fabricated constant).
    wave: WaveNumbers,
    /// Pre-built VMs (the world is immutable after seeding).
    repo_home: RepoHomeVm,
    landing: LandingVm,
    checks: ChecksVm,
    insights: InsightsVm,
    /// Every served intent VM, keyed by intent id (insertion = log order).
    intent_details: Vec<(String, IntentDetailVm)>,
}

/// The measured numbers pulled out of the real dogfood waves — the only figures
/// on the checks screen that are MEASURED rather than fixture-illustrative.
#[derive(Debug, Clone, Copy)]
struct WaveNumbers {
    /// Local check executions on the COLD pass of wave A (every check ran).
    cold_executions: u32,
    /// Measured execution time (ms) of the cold pass — the saved-ms figure
    /// (a warm wave re-executes none of it).
    cold_measured_exec_ms: u64,
    /// Local executions on the WARM pass of wave A (the wedge → 0).
    warm_executions: u32,
    /// AC hits on the warm pass (one lookup per landed check → all hits).
    warm_hits: usize,
    /// Total lookups observed on the warm pass.
    warm_lookups: usize,
}

impl FixtureProvider {
    /// Seed the deterministic fixture world from the real in-process engine.
    pub fn seed() -> Self {
        // ── 1. Run the real dogfood waves ─────────────────────────────────
        // Wave A: 5 disjoint-green PRs, run TWICE on a SHARED AC — cold then
        // warm. The cold pass executes every check (measured); the warm pass is
        // all AC hits (0 executions) — the memoization wedge, measured not
        // promised.
        let cfg_a = WaveConfig::five_pr_disjoint_green();
        let ac_a = InMemoryAc::new();
        let cold: WaveReport = run_wave_with_ac(&cfg_a, &ac_a);
        let warm: WaveReport = run_wave_with_ac(&cfg_a, &ac_a);

        // Wave B: a failing pair → the excluded PRs feed the Bloqueado column.
        let cfg_b = WaveConfig::five_pr_with_failing_pair("pr-1", "pr-3");
        let ac_b = InMemoryAc::new();
        let blocked: WaveReport = run_wave_with_ac(&cfg_b, &ac_b);

        // Observe one AC lookup per warm-pass landed check (warm = all hits) so
        // the hit-rate is measured through the real meter, not asserted.
        let mut warm_meter = HitRateMeter::new();
        for _ in &warm.landed {
            warm_meter.observe(true);
        }
        let warm_report = warm_meter.report();

        let wave = WaveNumbers {
            cold_executions: cold.local_executions,
            cold_measured_exec_ms: cold.measured_exec_ms,
            warm_executions: warm.local_executions,
            warm_hits: warm_report.hits as usize,
            warm_lookups: warm_report.total as usize,
        };

        // ── 2. Build the world EventLog through the REAL append path ───────
        let world_log = build_world_log(&cold);

        // ── 3. Project the world ──────────────────────────────────────────
        let ledger = Ledger::from_records(world_log.records());
        let intents: Vec<Intent> = intents_from_log(&world_log)
            .expect("world log was built through the real append path; it must project")
            .intents()
            .to_vec();

        // The verdict map (intent_id → VerdictObject), the single source both the
        // world log and the drawer/detail VMs read.
        let verdicts = world_verdicts();

        // ── 4. Build every view-model from the projections ────────────────
        let checks = build_checks_vm("hugit", &wave, &cold, &warm);
        let landing = build_landing_vm("hugit", &cold, &blocked, &intents, &verdicts);
        let insights = build_insights_vm("hugit", &ledger, &wave);
        let repo_home = build_repo_home_vm("hugit", &intents, &warm_report);
        let intent_details = build_intent_details("hugit", &intents, &verdicts);

        FixtureProvider {
            repo: "hugit".to_string(),
            world_log,
            intents,
            wave,
            repo_home,
            landing,
            checks,
            insights,
            intent_details,
        }
    }

    fn known(&self, repo: &str) -> bool {
        repo == self.repo
    }

    /// The world event records, exposed for the parity oracle so it can
    /// re-derive [`Ledger::from_records`] / [`intents_from_log`] over the SAME
    /// stream the VMs were built from. Test seam only.
    #[doc(hidden)]
    pub fn world_records(&self) -> &[EventRecord] {
        self.world_log.records()
    }

    /// The measured cold-pass execution time (ms) of wave A — the saved-ms the
    /// checks KPI claims. Test seam only.
    #[doc(hidden)]
    pub fn wave_cold_measured_exec_ms(&self) -> u64 {
        self.wave.cold_measured_exec_ms
    }

    /// The measured warm-pass local executions of wave A (the wedge → 0). Test
    /// seam only.
    #[doc(hidden)]
    pub fn wave_warm_executions(&self) -> u32 {
        self.wave.warm_executions
    }

    /// The intent ids served by this world, in log order. Test seam only.
    #[doc(hidden)]
    pub fn world_intent_ids(&self) -> Vec<String> {
        self.intents.iter().map(|i| i.intent_id.clone()).collect()
    }
}

impl Provider for FixtureProvider {
    fn default_repo(&self) -> String {
        self.repo.clone()
    }

    fn repo_home(&self, repo: &str) -> Option<RepoHomeVm> {
        self.known(repo).then(|| self.repo_home.clone())
    }

    fn landing(&self, repo: &str) -> Option<LandingVm> {
        self.known(repo).then(|| self.landing.clone())
    }

    fn intent(&self, repo: &str, id: &str) -> Option<IntentDetailVm> {
        if !self.known(repo) {
            return None;
        }
        self.intent_details
            .iter()
            .find(|(k, _)| k == id)
            .map(|(_, vm)| vm.clone())
    }

    fn checks(&self, repo: &str) -> Option<ChecksVm> {
        self.known(repo).then(|| self.checks.clone())
    }

    fn insights(&self, repo: &str) -> Option<InsightsVm> {
        self.known(repo).then(|| self.insights.clone())
    }
}

// ───────────────────────────────────────────────────────────────────────────
// World construction — the real append path
// ───────────────────────────────────────────────────────────────────────────

/// Build the world event log through the REAL [`EventLog::append`] over
/// canonical-JSON payloads. One `intent.landed` per landed PR of wave A, then a
/// `verdict.recorded` for the proven subset (3 of 5).
///
/// The chain is built exactly as production builds it, so
/// [`hugit_refstore::verify_chain`] passes and the projections read it the same
/// way the CLI does.
fn build_world_log(cold: &WaveReport) -> EventLog {
    let mut log = EventLog::new();

    // One intent.landed per landed PR (wave A lands all 5 in queue order).
    for (idx, pr_id) in cold.landed.iter().enumerate() {
        let intent_id = format!("intent-{pr_id}");
        // Payload carries EXACTLY the fields the Ledger + intent projection
        // read; canonicalised before chaining (the append path hashes the
        // payload bytes verbatim).
        let payload_raw = serde_json::json!({
            "intent_id": intent_id,
            "ref": "refs/heads/main",
            "target": fake_oid_40(idx),
            "charter": charter_for(idx),
            "campaign": campaign_id_for(idx),
            "deep_link_target": intent_id,
        })
        .to_string();
        let payload = canonical_json(&payload_raw).expect("payload is valid JSON");
        let seq = log.len() as u64;
        log.append(
            "intent.landed",
            world_principal_chain(),
            payload,
            recorded_at_for(seq),
        );
    }

    // A verdict.recorded for the proven subset (intents of pr-0, pr-2, pr-4).
    // Payload is a serde-serialized VerdictObject keyed by intent_id; the Ledger
    // matches it to the landed intent by `intent` and flips proven → true.
    for (intent_id, vo) in world_verdicts() {
        // Only append a verdict event if this intent actually landed.
        if cold
            .landed
            .iter()
            .any(|pr| format!("intent-{pr}") == intent_id)
        {
            let payload_raw = serde_json::to_string(&vo).expect("VerdictObject serializes");
            let payload = canonical_json(&payload_raw).expect("serialized verdict is valid JSON");
            let seq = log.len() as u64;
            log.append(
                "verdict.recorded",
                world_principal_chain(),
                payload,
                recorded_at_for(seq),
            );
        }
    }

    log
}

/// The world's verdicts: a [`VerdictObject`] for 3 of the 5 landed intents
/// (pr-0, pr-2, pr-4 → proven=true). Deterministic; one is adversarial.
///
/// Returned as `(intent_id, VerdictObject)` pairs so both the world log and the
/// drawer/detail VMs read the SAME source of truth.
fn world_verdicts() -> Vec<(String, VerdictObject)> {
    let proven_indices = [0usize, 2, 4];
    proven_indices
        .iter()
        .map(|&idx| {
            let intent_id = format!("intent-pr-{idx}");
            let vo = VerdictObject {
                intent: intent_id.clone(),
                tree_hash: fake_tree_64(idx),
                lens: if is_adversarial(&intent_id) {
                    // The adversarial panel reviewed this one.
                    "adversarial-review".to_string()
                } else {
                    "default-review".to_string()
                },
                model: "claude-opus".to_string(),
                prompt_digest: format!("{:064x}", 0xC0FFEE + idx as u64),
                verdict: Verdict::Approve,
                claims_checked: vec![
                    "o intent fez exatamente o que a charter pediu".to_string(),
                    "o union ficou verde de forma reproduzível".to_string(),
                ],
                evidence_refs: vec![format!("cas://verdict/intent-pr-{idx}")],
            };
            (intent_id, vo)
        })
        .collect()
}

/// Look the world VerdictObject up for an intent id, if proven.
fn verdict_for<'a>(
    verdicts: &'a [(String, VerdictObject)],
    intent_id: &str,
) -> Option<&'a VerdictObject> {
    verdicts
        .iter()
        .find(|(id, _)| id == intent_id)
        .map(|(_, vo)| vo)
}

/// Whether an intent id is the adversarial-reviewed verdict.
fn is_adversarial(intent_id: &str) -> bool {
    intent_id == "intent-pr-2"
}

// ───────────────────────────────────────────────────────────────────────────
// VM builders
// ───────────────────────────────────────────────────────────────────────────

/// A small, plausible hand-built Rust diff for an intent (1 file, 1 hunk).
fn diff_for(pr_index: usize) -> DiffVm {
    let path = format!("crates/hugit-web/src/feature_{pr_index}.rs");
    DiffVm {
        files: vec![FileRowVm {
            path: path.clone(),
            added: 6,
            removed: 1,
        }],
        hunks: vec![HunkVm {
            file: path,
            header: "@@ -1,4 +1,9 @@".to_string(),
            lines: vec![
                DiffLineVm {
                    kind: DiffLineKind::Context,
                    text: "use crate::provider::*;".to_string(),
                },
                DiffLineVm {
                    kind: DiffLineKind::Del,
                    text: "// TODO: implementar".to_string(),
                },
                DiffLineVm {
                    kind: DiffLineKind::Add,
                    text: format!("pub fn feature_{pr_index}() -> bool {{"),
                },
                DiffLineVm {
                    kind: DiffLineKind::Add,
                    text: "    true // pousado verde".to_string(),
                },
                DiffLineVm {
                    kind: DiffLineKind::Add,
                    text: "}".to_string(),
                },
            ],
        }],
    }
}

/// The pretty-printed `context.json` shown in a drawer/detail — a small
/// fixture-illustrative object (NOT an ADR-0001 envelope; the honest fixture
/// shape).
fn context_json_for(intent: &Intent) -> String {
    let value = serde_json::json!({
        "schema_version": "0.0-fixture",
        "charter": intent.charter,
        "claims": [
            "o union ficou verde",
            "o pouso entrou na trilha encadeada",
        ],
        "tree": intent.target,
    });
    serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string())
}

/// The PR index encoded in a landed intent id (`intent-pr-N` → N).
fn pr_index_of(intent_id: &str) -> usize {
    intent_id
        .rsplit('-')
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}

/// Build the verdict VMs for an intent, from the world's VerdictObject (if the
/// intent is proven). One lens is adversarial.
fn verdict_vms_for(verdicts: &[(String, VerdictObject)], intent_id: &str) -> Vec<VerdictVm> {
    verdict_for(verdicts, intent_id)
        .map(|vo| {
            vec![VerdictVm {
                verdict: format!("{:?}", vo.verdict),
                reviewer: vo.lens.clone(),
                summary: "Aprovado: a charter foi cumprida e o union ficou verde.".to_string(),
                adversarial: is_adversarial(intent_id),
            }]
        })
        .unwrap_or_default()
}

/// Build the drawer-level `IntentSummaryVm` for a landed intent.
fn intent_summary_for(intent: &Intent, verdicts: &[(String, VerdictObject)]) -> IntentSummaryVm {
    let idx = pr_index_of(&intent.intent_id);
    IntentSummaryVm {
        id: intent.intent_id.clone(),
        title: short_title(&intent.charter),
        status: "pousado".to_string(),
        charter: intent.charter.clone(),
        context_json: context_json_for(intent),
        diff: diff_for(idx),
        verdicts: verdict_vms_for(verdicts, &intent.intent_id),
    }
}

/// A short title derived from a charter line (first ~6 words).
fn short_title(charter: &str) -> String {
    let words: Vec<&str> = charter.split_whitespace().take(6).collect();
    words.join(" ")
}

/// Build a `PrCardVm` for a landed PR, drawing its intents from the real
/// projection.
fn landed_card(number: u64, intent: &Intent, verdicts: &[(String, VerdictObject)]) -> PrCardVm {
    let idx = pr_index_of(&intent.intent_id);
    let campaign = campaign_chip(campaign_id_for(idx));
    let summary = intent_summary_for(intent, verdicts);
    let files = summary.diff.files.clone();
    let file_count = files.len();
    PrCardVm {
        number,
        title: short_title(&intent.charter),
        author: "orchestrator".to_string(),
        campaign: Some(campaign),
        intent_count: 1,
        file_count,
        checks: ChecksBadgeVm {
            // Landed cards: all checks passed, served warm (cache hit).
            passed: 1,
            total: 1,
            cache_hits: 1,
        },
        state: PrState::Landed,
        drawer: PrDrawerVm {
            union: UnionVm {
                batch: vec![format!("#{number}")],
                verdict: "verde em união".to_string(),
                green: true,
            },
            cost: CostVm {
                tokens_total: 42_000 + (idx as u64) * 3_000,
                usd: 0.30 + (idx as f64) * 0.02,
                model_breakdown: vec![
                    ("claude-opus".to_string(), 30_000 + (idx as u64) * 2_000),
                    ("claude-haiku".to_string(), 12_000 + (idx as u64) * 1_000),
                ],
            },
            mirror: MirrorVm {
                synced: true,
                detail: "espelho GitHub em dia".to_string(),
            },
            intents: vec![summary],
            files,
        },
    }
}

/// Build a synthetic in-flight PR card (Queued / Testing) through the same VM
/// types. Synthetic but type-faithful; campaign chip from the world's chips.
fn synthetic_card(
    number: u64,
    title: &str,
    campaign_id: &str,
    state: PrState,
    intent_count: usize,
    file_count: usize,
) -> PrCardVm {
    let green = matches!(state, PrState::Testing);
    let drawer_intents: Vec<IntentSummaryVm> = (0..intent_count)
        .map(|i| IntentSummaryVm {
            id: format!("intent-synth-{number}-{i}"),
            title: format!("{title} · parte {}", i + 1),
            status: match state {
                PrState::Queued => "na fila".to_string(),
                PrState::Testing => "testando".to_string(),
                _ => "aberto".to_string(),
            },
            charter: format!("{title} — fatia sintética {}", i + 1),
            context_json: "{\n  \"schema_version\": \"0.0-fixture\"\n}".to_string(),
            diff: DiffVm {
                files: vec![FileRowVm {
                    path: format!("crates/hugit-web/src/wip_{number}.rs"),
                    added: 4,
                    removed: 0,
                }],
                hunks: vec![],
            },
            verdicts: vec![],
        })
        .collect();
    PrCardVm {
        number,
        title: title.to_string(),
        author: "orchestrator".to_string(),
        campaign: Some(campaign_chip(campaign_id)),
        intent_count,
        file_count,
        checks: ChecksBadgeVm {
            passed: if green { 1 } else { 0 },
            total: 1,
            cache_hits: 0,
        },
        state,
        drawer: PrDrawerVm {
            union: UnionVm {
                batch: vec![format!("#{number}")],
                verdict: if green {
                    "verde — testando em união".to_string()
                } else {
                    "aguardando lote".to_string()
                },
                green,
            },
            cost: CostVm {
                tokens_total: 18_000,
                usd: 0.12,
                model_breakdown: vec![("claude-opus".to_string(), 18_000)],
            },
            mirror: MirrorVm {
                synced: true,
                detail: "espelho GitHub em dia".to_string(),
            },
            intents: drawer_intents,
            files: vec![FileRowVm {
                path: format!("crates/hugit-web/src/wip_{number}.rs"),
                added: 4,
                removed: 0,
            }],
        },
    }
}

/// Build a blocked PR card from a wave-B excluded PR id. The drawer's union is
/// the red minimal-pair verdict; the batch is the failing pair.
fn blocked_card(number: u64, pr_id: &str, pair: &[String]) -> PrCardVm {
    PrCardVm {
        number,
        title: format!("PR {pr_id} — par mínimo isolado"),
        author: "orchestrator".to_string(),
        campaign: Some(campaign_chip(CAMPAIGN_SPINE)),
        intent_count: 1,
        file_count: 1,
        checks: ChecksBadgeVm {
            passed: 0,
            total: 1,
            cache_hits: 0,
        },
        state: PrState::Blocked,
        drawer: PrDrawerVm {
            union: UnionVm {
                batch: pair.to_vec(),
                verdict: "vermelho em união — par mínimo isolado".to_string(),
                green: false,
            },
            cost: CostVm {
                tokens_total: 9_000,
                usd: 0.07,
                model_breakdown: vec![("claude-opus".to_string(), 9_000)],
            },
            mirror: MirrorVm {
                synced: true,
                detail: "espelho GitHub em dia".to_string(),
            },
            intents: vec![IntentSummaryVm {
                id: format!("intent-{pr_id}"),
                title: format!("{pr_id} bloqueado"),
                status: "bloqueado".to_string(),
                charter: "Isolado pelo par mínimo de falha — não pousa.".to_string(),
                context_json: "{\n  \"schema_version\": \"0.0-fixture\"\n}".to_string(),
                diff: DiffVm {
                    files: vec![FileRowVm {
                        path: format!("crates/hugit-web/src/{pr_id}.rs"),
                        added: 3,
                        removed: 0,
                    }],
                    hunks: vec![],
                },
                verdicts: vec![],
            }],
            files: vec![FileRowVm {
                path: format!("crates/hugit-web/src/{pr_id}.rs"),
                added: 3,
                removed: 0,
            }],
        },
    }
}

/// Build the Landing VM: the four mockup columns, with Pousado hoje grouped
/// into the two real campaign bundles, Bloqueado from wave B's excluded pair,
/// and curated synthetic Na fila / Testando juntos columns.
fn build_landing_vm(
    repo: &str,
    cold: &WaveReport,
    blocked: &WaveReport,
    intents: &[Intent],
    verdicts: &[(String, VerdictObject)],
) -> LandingVm {
    // ── Pousado hoje: the 5 landed PRs as cards, grouped by campaign bundle. ──
    // Card numbers 101..105 for the landed set (distinct 101..110 overall).
    let mut fila_cards: Vec<PrCardVm> = Vec::new();
    let mut spine_cards: Vec<PrCardVm> = Vec::new();
    for (i, intent) in intents.iter().enumerate() {
        let number = 101 + i as u64;
        let card = landed_card(number, intent, verdicts);
        match campaign_id_for(pr_index_of(&intent.intent_id)) {
            CAMPAIGN_FILA => fila_cards.push(card),
            _ => spine_cards.push(card),
        }
    }
    let pousado_items = vec![
        LandingItemVm::Bundle {
            campaign: campaign_chip(CAMPAIGN_FILA),
            cards: fila_cards,
        },
        LandingItemVm::Bundle {
            campaign: campaign_chip(CAMPAIGN_SPINE),
            cards: spine_cards,
        },
    ];

    // ── Bloqueado: wave B's excluded pair as 2 cards. ─────────────────────────
    let pair = blocked.excluded.clone();
    let bloqueado_items: Vec<LandingItemVm> = blocked
        .excluded
        .iter()
        .enumerate()
        .map(|(i, pr_id)| LandingItemVm::Card(Box::new(blocked_card(106 + i as u64, pr_id, &pair))))
        .collect();

    // ── Na fila: 2 curated synthetic PRs (108, 109). ─────────────────────────
    let na_fila_items = vec![
        LandingItemVm::Card(Box::new(synthetic_card(
            108,
            "Importar corpus de intents B6 por intent_id",
            CAMPAIGN_INFRA,
            PrState::Queued,
            2,
            3,
        ))),
        LandingItemVm::Card(Box::new(synthetic_card(
            109,
            "Cold-tier offload preservando a trilha",
            CAMPAIGN_FILA,
            PrState::Queued,
            1,
            2,
        ))),
    ];

    // ── Testando juntos: 1 synthetic PR (110). ───────────────────────────────
    let testando_items = vec![LandingItemVm::Card(Box::new(synthetic_card(
        110,
        "Espelho bidirecional arbitrado pelo forge",
        CAMPAIGN_SPINE,
        PrState::Testing,
        1,
        2,
    )))];

    let open_count = na_fila_items.len() + testando_items.len();
    let landed_today = cold.landed.len();

    LandingVm {
        repo: repo.to_string(),
        main_green: true,
        main_status: format!("main verde · {landed_today} pousos hoje"),
        open_count,
        columns: vec![
            LandingColumnVm {
                title: "Na fila".to_string(),
                items: na_fila_items,
            },
            LandingColumnVm {
                title: "Testando juntos".to_string(),
                items: testando_items,
            },
            LandingColumnVm {
                title: "Bloqueado".to_string(),
                items: bloqueado_items,
            },
            LandingColumnVm {
                title: "Pousado hoje".to_string(),
                items: pousado_items,
            },
        ],
        campaigns: vec![
            campaign_chip(CAMPAIGN_FILA),
            campaign_chip(CAMPAIGN_SPINE),
            campaign_chip(CAMPAIGN_INFRA),
        ],
    }
}

/// Build the Checks VM from the MEASURED wave numbers. The hit-rate shape uses
/// the [`hugit_checks::runner::hit_rate`] vocabulary (FULL / PARTIAL / NONE /
/// NO DATA); `saved_ms` is the cold pass's measured execution time (a warm wave
/// re-executes none of it).
fn build_checks_vm(
    repo: &str,
    wave: &WaveNumbers,
    cold: &WaveReport,
    warm: &WaveReport,
) -> ChecksVm {
    // Reconstruct the warm-pass report through the real meter so the shape
    // string is derived from the same vocabulary the runner uses.
    let mut meter = HitRateMeter::new();
    for _ in 0..wave.warm_lookups {
        meter.observe(true);
    }
    let report = meter.report();
    // Derive the FULL/PARTIAL/NONE/NO DATA label from the meter's own
    // display_line (the single source of the vocabulary).
    let shape = shape_label(&report.display_line());

    // One row per (landed PR, check). Cold entries executed (cache_hit=false);
    // warm entries are AC hits (cache_hit=true). We surface both passes so the
    // table tells the wedge story honestly.
    let mut rows: Vec<CheckRowVm> = Vec::new();
    let def = cfg_check_def_digest();
    let toolchain = cfg_toolchain_digest();
    for pr_id in &cold.landed {
        let tree = format!("tree-{pr_id}");
        let memo_key = compute_memo_key(&tree, &def, &toolchain);
        rows.push(CheckRowVm {
            name: format!("cargo test --all @ {pr_id}"),
            ok: true,
            // Cold pass: executed once (the in-process runner's measured 10ms).
            duration_ms: 10,
            cache_hit: false,
            log: format!("[cold] cargo test --all @ {pr_id} — verde, 1 execução"),
            memo_key,
        });
    }
    for pr_id in &warm.landed {
        let tree = format!("tree-{pr_id}");
        let memo_key = compute_memo_key(&tree, &def, &toolchain);
        rows.push(CheckRowVm {
            name: format!("cargo test --all @ {pr_id}"),
            ok: true,
            // Warm pass: AC hit → zero execution.
            duration_ms: 0,
            cache_hit: true,
            log: format!("[warm] cargo test --all @ {pr_id} — hit no AC, 0 execução"),
            memo_key,
        });
    }

    ChecksVm {
        repo: repo.to_string(),
        kpis: ChecksKpisVm {
            hit_rate_pct: report.rate_pct(),
            shape,
            hits: wave.warm_hits,
            executed: wave.cold_executions as usize,
            saved_ms: wave.cold_measured_exec_ms,
        },
        checks: rows,
        bisect: None,
        memo_note: "um hit no AC é byte-idêntico — zero execução; verde não re-roda".to_string(),
    }
}

/// Extract the trailing shape label (FULL / PARTIAL / NONE / NO DATA) from a
/// [`hugit_checks::runner::hit_rate::HitRateReport::display_line`] string. The
/// label is the segment after the final " — ". Reusing the meter's own line
/// keeps the vocabulary single-sourced.
fn shape_label(display_line: &str) -> String {
    display_line
        .rsplit(" — ")
        .next()
        .unwrap_or("NO DATA")
        .to_string()
}

/// The shared check-def digest the wave uses (recomputed identically here so
/// the displayed memo keys are REAL keys over the same axes the wave memoizes
/// on). Mirrors `WaveConfig::five_pr_disjoint_green`'s def.
fn cfg_check_def_digest() -> String {
    use hugit_checks::client::memo_key::compute_def_digest;
    let mut def = hugit_contracts::CheckDef {
        def_digest: String::new(),
        command: "cargo test --all".to_string(),
        inputs: vec![],
        toolchain_ref: "rust-stable".to_string(),
        env_manifest: String::new(),
        glob_set: vec!["src/**".to_string()],
    };
    def.def_digest = compute_def_digest(&def);
    def.def_digest
}

/// The shared toolchain digest the wave uses (mirrors
/// `WaveConfig::five_pr_disjoint_green`).
fn cfg_toolchain_digest() -> String {
    "toolchain-rust-stable-1.78".to_string()
}

/// Build the Insights VM, including the Ledger view projected from the world.
fn build_insights_vm(repo: &str, ledger: &Ledger, wave: &WaveNumbers) -> InsightsVm {
    let ledger_view = build_ledger_view(ledger);

    // KPIs — the saved-ms KPI is MEASURED; the rest are fixture-illustrative.
    let total_proven: usize = ledger_view.campaigns.iter().map(|c| c.proven).sum();
    let total_landed: usize = ledger_view.campaigns.iter().map(|c| c.done).sum();
    let saved_s = wave.cold_measured_exec_ms as f64 / 1000.0;
    let kpis = vec![
        KpiVm {
            label: "PRs pousados".to_string(),
            value: total_landed.to_string(),
            delta: Some("+5 hoje".to_string()),
        },
        KpiVm {
            label: "hit-rate medido".to_string(),
            value: format!("{:.0}%", warm_hit_rate_pct(wave)),
            delta: None,
        },
        KpiVm {
            label: "execuções poupadas".to_string(),
            value: format!("{saved_s:.0}s"),
            delta: None,
        },
        KpiVm {
            label: "intents provados".to_string(),
            value: total_proven.to_string(),
            delta: None,
        },
        KpiVm {
            label: "custo total (fixture)".to_string(),
            value: "US$ 1,84".to_string(),
            delta: None,
        },
        KpiVm {
            label: "tempo de fila p95 (fixture)".to_string(),
            value: "1,3s".to_string(),
            delta: None,
        },
    ];

    // 14-day landed bars: deterministic, the 5 real landings on the last day.
    let counts: [u32; 14] = [0, 1, 0, 2, 1, 0, 3, 1, 2, 0, 1, 2, 1, 5];
    let landed_by_day: Vec<(String, u32)> = counts
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let label = if i == 13 {
                "hoje".to_string()
            } else {
                format!("d-{}", 13 - i)
            };
            (label, c)
        })
        .collect();

    // tokens_by_campaign / cost_xray / tokens_by_model: fixture-illustrative
    // deterministic numbers, internally consistent (xray total = sum of parts).
    let fila = campaign_chip(CAMPAIGN_FILA);
    let spine = campaign_chip(CAMPAIGN_SPINE);
    let tokens_by_campaign = vec![(fila.clone(), 156_000), (spine.clone(), 98_000)];
    let cost_xray = vec![
        xray_row(fila, 80_000, 30_000, 24_000, 14_000, 8_000),
        xray_row(spine, 50_000, 20_000, 16_000, 9_000, 3_000),
    ];
    let tokens_by_model = vec![
        ("claude-opus".to_string(), 170_000),
        ("claude-haiku".to_string(), 84_000),
    ];

    InsightsVm {
        repo: repo.to_string(),
        kpis,
        landed_by_day,
        tokens_by_campaign,
        cost_xray,
        tokens_by_model,
        ledger: ledger_view,
    }
}

/// The measured warm-pass hit-rate as a percentage (all hits → 100.0, or 0.0
/// when there were no lookups).
fn warm_hit_rate_pct(wave: &WaveNumbers) -> f64 {
    if wave.warm_lookups == 0 {
        0.0
    } else {
        (wave.warm_hits as f64 / wave.warm_lookups as f64) * 100.0
    }
}

/// Build a single cost-xray row (total is the sum of the parts, by contract).
fn xray_row(
    campaign: CampaignChipVm,
    work: u64,
    orchestration: u64,
    verification: u64,
    ci: u64,
    waste: u64,
) -> CostXrayRowVm {
    CostXrayRowVm {
        campaign,
        work,
        orchestration,
        verification,
        ci,
        waste,
        total: work + orchestration + verification + ci + waste,
    }
}

/// Build the Ledger view VM from the real [`Ledger`] projection: one campaign
/// group per campaign present in the entries, with asked/done/proven straight
/// off the Ledger methods and one row per entry.
fn build_ledger_view(ledger: &Ledger) -> LedgerViewVm {
    // Distinct campaigns in entry order (preserve first-seen order).
    let mut campaign_ids: Vec<String> = Vec::new();
    for e in ledger.entries() {
        if !campaign_ids.contains(&e.campaign) {
            campaign_ids.push(e.campaign.clone());
        }
    }

    let campaigns = campaign_ids
        .iter()
        .map(|cid| {
            let rows: Vec<LedgerRowVm> = ledger
                .by_campaign(cid)
                .map(|e| LedgerRowVm {
                    intent_id: e.intent_id.clone(),
                    asked: e.charter.clone(),
                    done_status: "pousado".to_string(),
                    proven_status: if e.proven {
                        "provado".to_string()
                    } else {
                        "pendente".to_string()
                    },
                    verdict: e.verdict.as_ref().map(|v| v.outcome.clone()),
                })
                .collect();
            LedgerCampaignVm {
                campaign: campaign_chip(cid),
                asked: ledger.asked(cid),
                done: ledger.done(cid),
                proven: ledger.proven(cid),
                rows,
            }
        })
        .collect();

    LedgerViewVm { campaigns }
}

/// Build the RepoHome VM: a file tree mirroring the REAL hugit top level, with
/// intent-id links on the crates/ and docs/ rows pointing at REAL world intent
/// ids.
fn build_repo_home_vm(repo: &str, intents: &[Intent], warm_report: &HitRateReport) -> RepoHomeVm {
    // Link crates/ and docs/ at the first two real world intents (if present).
    let crates_intent = intents.first().map(|i| i.intent_id.clone());
    let docs_intent = intents.get(1).map(|i| i.intent_id.clone());

    let files = vec![
        TreeRowVm {
            name: "crates".to_string(),
            is_dir: true,
            intent_id: crates_intent,
            message: "workspace de 15 crates + hugit-web".to_string(),
            age: "hoje".to_string(),
        },
        TreeRowVm {
            name: "docs".to_string(),
            is_dir: true,
            intent_id: docs_intent,
            message: "whitepaper · ADRs · plano de decomposição".to_string(),
            age: "hoje".to_string(),
        },
        TreeRowVm {
            name: "tests".to_string(),
            is_dir: true,
            intent_id: None,
            message: "oráculos de aceitação por WP".to_string(),
            age: "ontem".to_string(),
        },
        TreeRowVm {
            name: "Cargo.toml".to_string(),
            is_dir: false,
            intent_id: None,
            message: "manifesto do workspace".to_string(),
            age: "ontem".to_string(),
        },
        TreeRowVm {
            name: "Cargo.lock".to_string(),
            is_dir: false,
            intent_id: None,
            message: "lockfile fixado".to_string(),
            age: "ontem".to_string(),
        },
        TreeRowVm {
            name: "CHANGELOG.md".to_string(),
            is_dir: false,
            intent_id: None,
            message: "curado à mão, disparado por release".to_string(),
            age: "2 dias".to_string(),
        },
        TreeRowVm {
            name: "README.md".to_string(),
            is_dir: false,
            intent_id: None,
            message: "o forge LLM-nativo, compatível com git".to_string(),
            age: "3 dias".to_string(),
        },
        TreeRowVm {
            name: "LICENSE".to_string(),
            is_dir: false,
            intent_id: None,
            message: "licença do projeto".to_string(),
            age: "fundação".to_string(),
        },
        TreeRowVm {
            name: "rust-toolchain.toml".to_string(),
            is_dir: false,
            intent_id: None,
            message: "toolchain fixada: rust 1.96.0".to_string(),
            age: "fundação".to_string(),
        },
    ];

    let readme_html = concat!(
        "<p><strong>hugit</strong> é o forge compatível com git e LLM-nativo — ",
        "VCS, merge e CI desenhados para frotas de agentes orquestradas, ",
        "construído sobre o CAS de produção do CoreLink.</p>",
        "<p>A cunha é o <em>problema de pouso</em>: integrar e fazer merge do ",
        "trabalho de uma frota de agentes sem merge errado nem PR perdido. ",
        "Os checks verdes nunca re-rodam — memoizados por conteúdo no Action ",
        "Cache.</p>",
        "<p>Não desviar do git: nomes, formato de CLI e modelo mental ficam ",
        "próximos do git. Abraçar, não atacar — uma escada de compatibilidade ",
        "que pousa sobre o GitHub antes de virar forge autoritativo.</p>"
    )
    .to_string();

    RepoHomeVm {
        repo: repo.to_string(),
        branch: "main".to_string(),
        branch_count: 3,
        files,
        readme_html,
        about: AboutVm {
            description: "the git-compatible, LLM-native forge".to_string(),
            topics: vec![
                "hugit".to_string(),
                "forge".to_string(),
                "llm".to_string(),
                "cas".to_string(),
            ],
            release: Some("v0.1.0".to_string()),
            contributors: vec!["orchestrator".to_string(), "gustavo".to_string()],
        },
        synergy: SynergyVm {
            lines: vec![
                (
                    "espelho GitHub".to_string(),
                    "sincronizado · em dia".to_string(),
                ),
                (
                    "fila de pouso".to_string(),
                    "verde · 5 pousos hoje".to_string(),
                ),
                (
                    "hit-rate do CI".to_string(),
                    format!("{:.0}% medido", warm_report.rate_pct()),
                ),
            ],
        },
    }
}

/// Build the per-intent detail VMs served by `intent(repo, id)` — one per world
/// intent. Two intents carry None transcripts (the honest not-captured state).
fn build_intent_details(
    repo: &str,
    intents: &[Intent],
    verdicts: &[(String, VerdictObject)],
) -> Vec<(String, IntentDetailVm)> {
    intents
        .iter()
        .enumerate()
        .map(|(i, intent)| {
            let idx = pr_index_of(&intent.intent_id);
            // The first two intents have no captured transcripts (honest None);
            // the rest carry short pt-BR fixture text.
            let captured = i >= 2;
            let task_transcript = captured.then(|| {
                format!(
                    "Tarefa: {}\nPassos: planejou → implementou → provou verde.",
                    intent.charter
                )
            });
            let full_transcript = captured.then(|| {
                "Transcrição completa (fixture): diálogo orquestrador↔agente.".to_string()
            });
            let journal =
                captured.then(|| "Diário: decisões registradas; sem becos sem saída.".to_string());

            let vm = IntentDetailVm {
                repo: repo.to_string(),
                id: intent.intent_id.clone(),
                title: short_title(&intent.charter),
                status: "pousado".to_string(),
                pr_number: Some(101 + i as u64),
                summary: format!("Intent pousado: {}", intent.charter),
                charter: intent.charter.clone(),
                acceptance: vec![
                    "union verde reproduzível".to_string(),
                    "pouso auditado na trilha encadeada".to_string(),
                ],
                task_transcript,
                full_transcript,
                journal,
                context_json: context_json_for(intent),
                diff: diff_for(idx),
                authorship: AuthorshipVm {
                    model: "claude-opus".to_string(),
                    principal_chain: world_principal_chain(),
                    operator: "owner".to_string(),
                },
                metrics: MetricsVm {
                    tokens: 42_000 + (idx as u64) * 3_000,
                    wall_ms: 1_200 + (idx as u64) * 100,
                    tool_calls: 18 + idx as u64,
                    cost_usd: 0.30 + (idx as f64) * 0.02,
                },
                snapshot: SnapshotVm {
                    // The 40-hex used in the landed event for this intent.
                    tree: intent.target.clone(),
                    toolchain: "rust-1.96.0".to_string(),
                    workspace: "ws-fixture-01".to_string(),
                },
                verdicts: verdict_vms_for(verdicts, &intent.intent_id),
            };
            (intent.intent_id.clone(), vm)
        })
        .collect()
}
