//! The Provider contract — the FROZEN seam between data and screens.
//!
//! ⚠️ CONTRACT (wave githugr-spine-w1): every screen renders EXACTLY one of
//! these view-models; the fixture/live providers construct them. Screens never
//! reach past a VM into hugit crates; providers never emit HTML. Changing a VM
//! shape after dispatch is an integration break — additive changes only, and
//! only by the lead.
//!
//! Honesty notes (deliberate, documented — not debt):
//! - [`MetricsVm`] / cost figures are fixture-illustrative until WP-F2 lands
//!   (ADR-0001 §7 ratification pending — `docs/plan/2026-06-09-adr-0001-context-envelope-rework.md`).
//!   The FIELDS are forward-compatible with the envelope; the fixture VALUES
//!   are demo data and the fixture provider says so via [`Provider::is_fixture`].
//! - `readme_html` is provider-rendered: the spine ships fixture HTML; a real
//!   markdown pipeline slots into the provider later without touching screens
//!   (backlog §I "Markdown render" stays open).

/// The read-only data source every route reads through.
///
/// `None` = unknown repo (the router answers 404). The MVP world is
/// single-repo; [`Provider::default_repo`] names it for the `/` redirect.
pub trait Provider: Send + Sync + 'static {
    fn default_repo(&self) -> String;
    fn repo_home(&self, repo: &str) -> Option<RepoHomeVm>;
    fn landing(&self, repo: &str) -> Option<LandingVm>;
    fn intent(&self, repo: &str, id: &str) -> Option<IntentDetailVm>;
    fn checks(&self, repo: &str) -> Option<ChecksVm>;
    fn insights(&self, repo: &str) -> Option<InsightsVm>;
    /// True when the data behind the screens is the seeded fixture world
    /// (rendered as an honest badge in the chrome), false on live infra.
    fn is_fixture(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// Shared atoms
// ---------------------------------------------------------------------------

/// A campaign chip: stable id + human label + the kit color class
/// (`c-auth` / `c-perf` / `c-fabric` / `c-obs` / `c-infra` — kit.css tokens).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CampaignChipVm {
    pub id: String,
    pub label: String,
    pub color_class: String,
}

/// One file row in a diff / file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRowVm {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLineVm {
    pub kind: DiffLineKind,
    pub text: String,
}

/// One hunk of a rendered diff (`header` is the `@@ …` line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HunkVm {
    pub file: String,
    pub header: String,
    pub lines: Vec<DiffLineVm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffVm {
    pub files: Vec<FileRowVm>,
    pub hunks: Vec<HunkVm>,
}

/// A review verdict as displayed (APPROVE / FIX-FIRST / REJECT shape).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerdictVm {
    pub verdict: String,
    pub reviewer: String,
    pub summary: String,
    /// True when this verdict came from an adversarial / independent panel.
    pub adversarial: bool,
}

// ---------------------------------------------------------------------------
// Landing (the hero) — landing.html
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrState {
    Open,
    Queued,
    Testing,
    Blocked,
    Landed,
    Draft,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChecksBadgeVm {
    pub passed: usize,
    pub total: usize,
    pub cache_hits: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnionVm {
    /// PR numbers tested together in this union batch (display order).
    pub batch: Vec<String>,
    pub verdict: String,
    pub green: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CostVm {
    pub tokens_total: u64,
    pub usd: f64,
    /// (model, tokens) pairs, largest first.
    pub model_breakdown: Vec<(String, u64)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorVm {
    pub synced: bool,
    pub detail: String,
}

/// One intent inside the PR drawer (collapsible body).
#[derive(Debug, Clone, PartialEq)]
pub struct IntentSummaryVm {
    pub id: String,
    pub title: String,
    pub status: String,
    pub charter: String,
    /// Pretty-printed `context.json` snapshot for the collapsible block.
    pub context_json: String,
    pub diff: DiffVm,
    pub verdicts: Vec<VerdictVm>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrDrawerVm {
    pub union: UnionVm,
    pub cost: CostVm,
    pub mirror: MirrorVm,
    pub intents: Vec<IntentSummaryVm>,
    pub files: Vec<FileRowVm>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PrCardVm {
    pub number: u64,
    pub title: String,
    /// PR author — orchestrator or human, never a subagent (D14 authz).
    pub author: String,
    pub campaign: Option<CampaignChipVm>,
    pub intent_count: usize,
    pub file_count: usize,
    pub checks: ChecksBadgeVm,
    pub state: PrState,
    pub drawer: PrDrawerVm,
}

/// A column item: a lone PR card or a campaign bundle of cards.
/// (`Card` is boxed: a `PrCardVm` is large and `Bundle` already indirects
/// through its `Vec` — keeps the enum small, per clippy::large_enum_variant.)
#[derive(Debug, Clone, PartialEq)]
pub enum LandingItemVm {
    Card(Box<PrCardVm>),
    Bundle {
        campaign: CampaignChipVm,
        cards: Vec<PrCardVm>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct LandingColumnVm {
    pub title: String,
    pub items: Vec<LandingItemVm>,
}

impl LandingColumnVm {
    /// Cards in this column, bundles flattened.
    pub fn card_count(&self) -> usize {
        self.items
            .iter()
            .map(|i| match i {
                LandingItemVm::Card(_) => 1,
                LandingItemVm::Bundle { cards, .. } => cards.len(),
            })
            .sum()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct LandingVm {
    pub repo: String,
    pub main_green: bool,
    /// The mainstat strip line, e.g. "main verde · 12 pousos hoje".
    pub main_status: String,
    /// Tab counter (open PRs).
    pub open_count: usize,
    /// Exactly the four mockup columns: Na fila / Testando juntos /
    /// Bloqueado / Pousado hoje.
    pub columns: Vec<LandingColumnVm>,
    /// Legend chips.
    pub campaigns: Vec<CampaignChipVm>,
}

// ---------------------------------------------------------------------------
// Repo home — repo-home.html
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeRowVm {
    pub name: String,
    pub is_dir: bool,
    /// Last-touch intent id (the `.ix` link), when fleet-authored.
    pub intent_id: Option<String>,
    pub message: String,
    pub age: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AboutVm {
    pub description: String,
    pub topics: Vec<String>,
    pub release: Option<String>,
    pub contributors: Vec<String>,
}

/// The synergy panel (the hugit layer at a glance on the code home).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SynergyVm {
    /// (label, value) lines, e.g. ("espelho GitHub", "sincronizado · 2 min").
    pub lines: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoHomeVm {
    pub repo: String,
    pub branch: String,
    pub branch_count: usize,
    pub files: Vec<TreeRowVm>,
    /// Provider-rendered README body (see module honesty notes).
    pub readme_html: String,
    pub about: AboutVm,
    pub synergy: SynergyVm,
}

// ---------------------------------------------------------------------------
// Intent detail — intent.html
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorshipVm {
    pub model: String,
    pub principal_chain: Vec<String>,
    pub operator: String,
}

/// Fixture-illustrative until WP-F2 (see module honesty notes).
#[derive(Debug, Clone, PartialEq)]
pub struct MetricsVm {
    pub tokens: u64,
    pub wall_ms: u64,
    pub tool_calls: u64,
    pub cost_usd: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotVm {
    pub tree: String,
    pub toolchain: String,
    pub workspace: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntentDetailVm {
    pub repo: String,
    pub id: String,
    pub title: String,
    pub status: String,
    pub pr_number: Option<u64>,
    pub summary: String,
    pub charter: String,
    pub acceptance: Vec<String>,
    /// Accordion bodies; `None` renders the honest "não capturado" state
    /// (capture level / F2 pending), never a silent blank.
    pub task_transcript: Option<String>,
    pub full_transcript: Option<String>,
    pub journal: Option<String>,
    pub context_json: String,
    pub diff: DiffVm,
    pub authorship: AuthorshipVm,
    pub metrics: MetricsVm,
    pub snapshot: SnapshotVm,
    pub verdicts: Vec<VerdictVm>,
}

// ---------------------------------------------------------------------------
// Checks — checks.html
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub struct ChecksKpisVm {
    /// Measured hit-rate percentage (0.0–100.0), displayed AS-IS.
    pub hit_rate_pct: f64,
    /// The honest shape label: FULL / PARTIAL / NONE / NO DATA
    /// (`hugit_checks::runner::hit_rate` semantics).
    pub shape: String,
    pub hits: usize,
    pub executed: usize,
    /// Execution time saved by cache hits (ms), measured not promised.
    pub saved_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckRowVm {
    pub name: String,
    pub ok: bool,
    pub duration_ms: u64,
    /// True = served from the AC (zero execution); false = executed.
    pub cache_hit: bool,
    pub log: String,
    pub memo_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BisectVm {
    pub culprit: String,
    pub probes: u32,
    pub max_probes: u32,
    pub steps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ChecksVm {
    pub repo: String,
    pub kpis: ChecksKpisVm,
    pub checks: Vec<CheckRowVm>,
    pub bisect: Option<BisectVm>,
    pub memo_note: String,
}

// ---------------------------------------------------------------------------
// Insights + Ledger — insights.html (Ledger is a view inside Insights)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KpiVm {
    pub label: String,
    pub value: String,
    pub delta: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CostXrayRowVm {
    pub campaign: CampaignChipVm,
    pub work: u64,
    pub orchestration: u64,
    pub verification: u64,
    pub ci: u64,
    pub waste: u64,
    pub total: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerRowVm {
    pub intent_id: String,
    /// What was asked (the charter line).
    pub asked: String,
    pub done_status: String,
    pub proven_status: String,
    pub verdict: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerCampaignVm {
    pub campaign: CampaignChipVm,
    pub asked: usize,
    pub done: usize,
    pub proven: usize,
    pub rows: Vec<LedgerRowVm>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerViewVm {
    pub campaigns: Vec<LedgerCampaignVm>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InsightsVm {
    pub repo: String,
    pub kpis: Vec<KpiVm>,
    /// 14-day landed bars: (day label, count).
    pub landed_by_day: Vec<(String, u32)>,
    pub tokens_by_campaign: Vec<(CampaignChipVm, u64)>,
    pub cost_xray: Vec<CostXrayRowVm>,
    pub tokens_by_model: Vec<(String, u64)>,
    pub ledger: LedgerViewVm,
}
