//! FixtureProvider — the seeded MVP world behind the screens.
//!
//! ⚠️ W1 SCAFFOLD STUB: this minimal world exists so the scaffold compiles and
//! every route serves. WP-W1 replaces the seeding internals with the REAL
//! in-process wave (`hugit_dogfood::wave::run_wave_with_ac` ×2 on a shared
//! `InMemoryAc` for measured memoization numbers) and projects the resulting
//! `EventLog` through `hugit_ledger::Ledger::from_records` /
//! `hugit_refstore::intent` — the parity oracle in `tests/provider_fixture.rs`
//! holds the VMs to those real projections. The `Provider` SHAPES are frozen;
//! only the world behind them deepens.

use crate::provider::*;

/// The seeded fixture world (single-repo MVP).
pub struct FixtureProvider {
    repo: String,
}

impl FixtureProvider {
    /// Seed the deterministic fixture world.
    pub fn seed() -> Self {
        FixtureProvider {
            repo: "hugit".to_string(),
        }
    }

    fn known(&self, repo: &str) -> bool {
        repo == self.repo
    }

    fn campaign(&self) -> CampaignChipVm {
        CampaignChipVm {
            id: "spine".to_string(),
            label: "githugr spine".to_string(),
            color_class: "c-fabric".to_string(),
        }
    }

    fn drawer(&self) -> PrDrawerVm {
        PrDrawerVm {
            union: UnionVm {
                batch: vec!["#1".to_string()],
                verdict: "verde em união".to_string(),
                green: true,
            },
            cost: CostVm {
                tokens_total: 0,
                usd: 0.0,
                model_breakdown: vec![],
            },
            mirror: MirrorVm {
                synced: true,
                detail: "espelho GitHub em dia".to_string(),
            },
            intents: vec![IntentSummaryVm {
                id: "intent-pr-0".to_string(),
                title: "stub intent".to_string(),
                status: "landed".to_string(),
                charter: "scaffold stub".to_string(),
                context_json: "{}".to_string(),
                diff: DiffVm {
                    files: vec![],
                    hunks: vec![],
                },
                verdicts: vec![],
            }],
            files: vec![],
        }
    }
}

impl Provider for FixtureProvider {
    fn default_repo(&self) -> String {
        self.repo.clone()
    }

    fn repo_home(&self, repo: &str) -> Option<RepoHomeVm> {
        self.known(repo).then(|| RepoHomeVm {
            repo: self.repo.clone(),
            branch: "main".to_string(),
            branch_count: 1,
            files: vec![TreeRowVm {
                name: "crates".to_string(),
                is_dir: true,
                intent_id: Some("intent-pr-0".to_string()),
                message: "scaffold".to_string(),
                age: "agora".to_string(),
            }],
            readme_html: "<p>hugit — o forge LLM-nativo.</p>".to_string(),
            about: AboutVm {
                description: "the git-compatible, LLM-native forge".to_string(),
                topics: vec!["hugit".to_string()],
                release: None,
                contributors: vec!["orchestrator".to_string()],
            },
            synergy: SynergyVm {
                lines: vec![("espelho GitHub".to_string(), "sincronizado".to_string())],
            },
        })
    }

    fn landing(&self, repo: &str) -> Option<LandingVm> {
        self.known(repo).then(|| LandingVm {
            repo: self.repo.clone(),
            main_green: true,
            main_status: "main verde".to_string(),
            open_count: 1,
            columns: vec![
                LandingColumnVm {
                    title: "Na fila".to_string(),
                    items: vec![],
                },
                LandingColumnVm {
                    title: "Testando juntos".to_string(),
                    items: vec![],
                },
                LandingColumnVm {
                    title: "Bloqueado".to_string(),
                    items: vec![],
                },
                LandingColumnVm {
                    title: "Pousado hoje".to_string(),
                    items: vec![LandingItemVm::Card(Box::new(PrCardVm {
                        number: 1,
                        title: "scaffold stub".to_string(),
                        author: "orchestrator".to_string(),
                        campaign: Some(self.campaign()),
                        intent_count: 1,
                        file_count: 0,
                        checks: ChecksBadgeVm {
                            passed: 1,
                            total: 1,
                            cache_hits: 1,
                        },
                        state: PrState::Landed,
                        drawer: self.drawer(),
                    }))],
                },
            ],
            campaigns: vec![self.campaign()],
        })
    }

    fn intent(&self, repo: &str, id: &str) -> Option<IntentDetailVm> {
        (self.known(repo) && id == "intent-pr-0").then(|| IntentDetailVm {
            repo: self.repo.clone(),
            id: id.to_string(),
            title: "stub intent".to_string(),
            status: "landed".to_string(),
            pr_number: Some(1),
            summary: "scaffold stub".to_string(),
            charter: "scaffold stub".to_string(),
            acceptance: vec!["compiles".to_string()],
            task_transcript: None,
            full_transcript: None,
            journal: None,
            context_json: "{}".to_string(),
            diff: DiffVm {
                files: vec![],
                hunks: vec![],
            },
            authorship: AuthorshipVm {
                model: "claude".to_string(),
                principal_chain: vec!["owner".to_string(), "orchestrator".to_string()],
                operator: "owner".to_string(),
            },
            metrics: MetricsVm {
                tokens: 0,
                wall_ms: 0,
                tool_calls: 0,
                cost_usd: 0.0,
            },
            snapshot: SnapshotVm {
                tree: "0".repeat(64),
                toolchain: "rust-1.96".to_string(),
                workspace: "ws-0".to_string(),
            },
            verdicts: vec![],
        })
    }

    fn checks(&self, repo: &str) -> Option<ChecksVm> {
        self.known(repo).then(|| ChecksVm {
            repo: self.repo.clone(),
            kpis: ChecksKpisVm {
                hit_rate_pct: 0.0,
                shape: "NO DATA".to_string(),
                hits: 0,
                executed: 0,
                saved_ms: 0,
            },
            checks: vec![],
            bisect: None,
            memo_note: "um hit no AC é byte-idêntico — zero execução".to_string(),
        })
    }

    fn insights(&self, repo: &str) -> Option<InsightsVm> {
        self.known(repo).then(|| InsightsVm {
            repo: self.repo.clone(),
            kpis: vec![],
            landed_by_day: vec![],
            tokens_by_campaign: vec![],
            cost_xray: vec![],
            tokens_by_model: vec![],
            ledger: LedgerViewVm {
                campaigns: vec![LedgerCampaignVm {
                    campaign: self.campaign(),
                    asked: 1,
                    done: 1,
                    proven: 1,
                    rows: vec![],
                }],
            },
        })
    }
}
