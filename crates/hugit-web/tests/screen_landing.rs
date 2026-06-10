//! WP-W2 — Landing screen render tests (spec: ../githugr/design/landing.html).
//!
//! These build `LandingVm` literals BY HAND (not via the fixture provider) and
//! assert on `render(&vm).into_string()`, so they pin the render contract — the
//! four-column board, campaign bundles, PR-card field binding, and the per-card
//! drawer (charter · context.json · diff · verdicts · union/mirror · the lcard
//! with the disabled Land button) — independent of any seeded world.

use hugit_web::provider::*;
use hugit_web::screens::landing::render;

/// A drawer carrying one rich intent (charter + context.json + diff + verdicts),
/// a union batch, a cost figure, and a mirror detail — the drawer that the
/// drawer-content assertions read.
fn rich_drawer() -> PrDrawerVm {
    PrDrawerVm {
        union: UnionVm {
            batch: vec!["#128".to_string(), "#130".to_string()],
            verdict: "verde em união".to_string(),
            green: true,
        },
        cost: CostVm {
            tokens_total: 41_000,
            usd: 0.04,
            model_breakdown: vec![("opus-4.8".to_string(), 41_000)],
        },
        mirror: MirrorVm {
            synced: true,
            detail: "GitHub #128 sincronizado".to_string(),
        },
        intents: vec![IntentSummaryVm {
            id: "a31".to_string(),
            title: "fix: sessão expira cedo no refresh".to_string(),
            status: "atestado".to_string(),
            charter: "Refresh reusava o iat antigo — re-derivar do token emitido agora."
                .to_string(),
            context_json: "{ \"intent\":\"a31\", \"model\":\"opus-4.8\",\n  \"tool_calls\":14 }"
                .to_string(),
            diff: DiffVm {
                files: vec![FileRowVm {
                    path: "crates/auth/src/token.rs".to_string(),
                    added: 12,
                    removed: 4,
                }],
                hunks: vec![HunkVm {
                    file: "crates/auth/src/token.rs".to_string(),
                    header: "@@ -42,1 +42,1 @@".to_string(),
                    lines: vec![
                        DiffLineVm {
                            kind: DiffLineKind::Del,
                            text: "-  let iat = old.iat;".to_string(),
                        },
                        DiffLineVm {
                            kind: DiffLineKind::Add,
                            text: "+  let iat = now();".to_string(),
                        },
                    ],
                }],
            },
            verdicts: vec![VerdictVm {
                verdict: "APPROVE".to_string(),
                reviewer: "correctness".to_string(),
                summary: "dura o TTL completo".to_string(),
                adversarial: true,
            }],
        }],
        files: vec![FileRowVm {
            path: "crates/auth/src/token.rs".to_string(),
            added: 12,
            removed: 4,
        }],
    }
}

/// A minimal drawer for cards whose contents the tests don't inspect.
fn thin_drawer(verdict: &str, green: bool) -> PrDrawerVm {
    PrDrawerVm {
        union: UnionVm {
            batch: vec![],
            verdict: verdict.to_string(),
            green,
        },
        cost: CostVm {
            tokens_total: 0,
            usd: 0.0,
            model_breakdown: vec![],
        },
        mirror: MirrorVm {
            synced: green,
            detail: "espelho".to_string(),
        },
        intents: vec![],
        files: vec![],
    }
}

fn auth_chip() -> CampaignChipVm {
    CampaignChipVm {
        id: "auth".to_string(),
        label: "auth-hardening".to_string(),
        color_class: "c-auth".to_string(),
    }
}

/// The rich card #128: an auth-hardening PR with a full drawer.
fn card_128() -> PrCardVm {
    PrCardVm {
        number: 128,
        title: "fix: sessão expira cedo no refresh".to_string(),
        author: "opus-4.8".to_string(),
        campaign: Some(auth_chip()),
        intent_count: 3,
        file_count: 14,
        checks: ChecksBadgeVm {
            passed: 11,
            total: 11,
            cache_hits: 11,
        },
        state: PrState::Testing,
        drawer: rich_drawer(),
    }
}

fn card_130() -> PrCardVm {
    PrCardVm {
        number: 130,
        title: "rate-limit por tenant no edge".to_string(),
        author: "opus-4.8".to_string(),
        campaign: Some(auth_chip()),
        intent_count: 1,
        file_count: 2,
        checks: ChecksBadgeVm {
            passed: 4,
            total: 4,
            cache_hits: 3,
        },
        state: PrState::Testing,
        drawer: thin_drawer("verde", true),
    }
}

/// A blocked card whose union failed — exercises the red/union-fail affordance.
fn card_125_blocked() -> PrCardVm {
    PrCardVm {
        number: 125,
        title: "migração do schema de sessões".to_string(),
        author: "opus-4.8".to_string(),
        campaign: Some(CampaignChipVm {
            id: "perf".to_string(),
            label: "perf".to_string(),
            color_class: "c-perf".to_string(),
        }),
        intent_count: 2,
        file_count: 5,
        checks: ChecksBadgeVm {
            passed: 1,
            total: 2,
            cache_hits: 0,
        },
        state: PrState::Blocked,
        drawer: thin_drawer("conflitou com #133", false),
    }
}

fn card_124_landed() -> PrCardVm {
    PrCardVm {
        number: 124,
        title: "fix null deref no parser de manifest".to_string(),
        author: "opus-4.8".to_string(),
        campaign: Some(CampaignChipVm {
            id: "infra".to_string(),
            label: "infra".to_string(),
            color_class: "c-infra".to_string(),
        }),
        intent_count: 1,
        file_count: 1,
        checks: ChecksBadgeVm {
            passed: 1,
            total: 1,
            cache_hits: 1,
        },
        state: PrState::Landed,
        drawer: thin_drawer("verde", true),
    }
}

/// The full four-column world: lone card + bundle + blocked + landed.
fn vm_four_columns() -> LandingVm {
    LandingVm {
        repo: "corelink-server".to_string(),
        main_green: true,
        main_status: "main verde · 9 entraram hoje · 0 quebraram".to_string(),
        open_count: 24,
        columns: vec![
            LandingColumnVm {
                title: "Na fila".to_string(),
                items: vec![LandingItemVm::Card(Box::new(card_130()))],
            },
            LandingColumnVm {
                title: "Testando juntos".to_string(),
                items: vec![LandingItemVm::Bundle {
                    campaign: auth_chip(),
                    cards: vec![card_128(), card_130()],
                }],
            },
            LandingColumnVm {
                title: "Bloqueado".to_string(),
                items: vec![LandingItemVm::Card(Box::new(card_125_blocked()))],
            },
            LandingColumnVm {
                title: "Pousado hoje".to_string(),
                items: vec![LandingItemVm::Card(Box::new(card_124_landed()))],
            },
        ],
        campaigns: vec![auth_chip()],
    }
}

#[test]
fn all_four_column_titles_render_with_matching_counts() {
    let vm = vm_four_columns();
    let html = render(&vm).into_string();
    for title in ["Na fila", "Testando juntos", "Bloqueado", "Pousado hoje"] {
        assert!(html.contains(title), "column title `{title}` must render");
    }
    // Each column's `.n` badge shows its flattened card_count(): the bundle
    // column reports 2, the lone-card columns report 1.
    assert!(
        html.contains(">2</span>"),
        "the bundle column must show its 2-card count"
    );
    assert!(
        html.contains(">1</span>"),
        "lone-card columns must show their 1-card count"
    );
}

#[test]
fn bundle_header_shows_campaign_label_and_color_class() {
    let vm = vm_four_columns();
    let html = render(&vm).into_string();
    // The bundle header binds the campaign label...
    assert!(
        html.contains("class=\"nm\">auth-hardening"),
        "bundle header must show the campaign label"
    );
    // ...and color-codes via the chip's color_class → the kit's --c-* var.
    assert!(
        html.contains("--cc:var(--c-auth)"),
        "bundle must color-code through --c-auth"
    );
}

#[test]
fn pr_card_fields_bind_number_title_author_counts_and_checks() {
    let vm = vm_four_columns();
    let html = render(&vm).into_string();
    assert!(html.contains("#128"), "card number renders as #N");
    assert!(
        html.contains("fix: sessão expira cedo no refresh"),
        "card title binds"
    );
    assert!(html.contains("opus-4.8"), "card author binds");
    assert!(html.contains(">3</b> intents") || html.contains("3</b> intents"));
    // checks badge numbers surface in the drawer's lcard (11/11 · 11 cache-hit).
    assert!(
        html.contains("11/11"),
        "checks badge passed/total binds in the lcard"
    );
    assert!(
        html.contains("11 cache-hit"),
        "checks badge cache_hits binds in the lcard"
    );
}

#[test]
fn drawer_embeds_charter_context_diff_verdict_union_and_mirror() {
    let vm = vm_four_columns();
    let html = render(&vm).into_string();
    // charter text
    assert!(
        html.contains("Refresh reusava o iat antigo"),
        "drawer embeds the intent charter"
    );
    // the context.json block (rendered line-by-line)
    assert!(
        html.contains("a31.context.json"),
        "drawer embeds the context.json block header"
    );
    // maud HTML-escapes the JSON quotes (`"` → `&quot;`); assert on the escaped
    // body so the test pins the real rendered bytes.
    assert!(
        html.contains("tool_calls&quot;:14"),
        "drawer embeds context.json body lines (HTML-escaped)"
    );
    // a diff line
    assert!(
        html.contains("let iat = now();"),
        "drawer embeds a diff line"
    );
    // a verdict outcome + its lens
    assert!(html.contains("APPROVE"), "drawer embeds a verdict outcome");
    assert!(
        html.contains("correctness"),
        "drawer embeds the verdict lens/reviewer"
    );
    // union batch verdict text + mirror detail (in the lcard)
    assert!(
        html.contains("verde em união"),
        "drawer embeds the union verdict text"
    );
    assert!(
        html.contains("GitHub #128 sincronizado"),
        "drawer embeds the mirror detail"
    );
}

#[test]
fn intent_deep_link_points_at_the_real_intent_route() {
    let vm = vm_four_columns();
    let html = render(&vm).into_string();
    assert!(
        html.contains("href=\"/r/corelink-server/intent/a31\""),
        "`ver intent completo` must deep-link to /r/<repo>/intent/<id>"
    );
}

#[test]
fn land_button_renders_disabled_and_honest() {
    let vm = vm_four_columns();
    let html = render(&vm).into_string();
    // The Land button is present, disabled, and honest about the read-only build.
    let has_land = html.contains("Land (entra na main)");
    assert!(has_land, "the Land button must render");
    assert!(
        html.contains("read-only — write-path em wave futura"),
        "the Land button must carry the honest disabled title"
    );
    // assert the disabled attribute sits on the land button itself
    let idx = html.find("id=\"land-btn\"").expect("land button present");
    let around = &html[idx..(idx + 120).min(html.len())];
    assert!(
        around.contains("disabled"),
        "the Land button must be disabled"
    );
}

#[test]
fn blocked_card_renders_the_red_union_fail_affordance() {
    let vm = vm_four_columns();
    let html = render(&vm).into_string();
    // The blocked card carries the mockup's red `.blk` conflict note...
    assert!(
        html.contains("class=\"blk\""),
        "blocked card must render the red conflict affordance"
    );
    assert!(
        html.contains("voltou ao autor"),
        "blocked card shows the back-to-author conflict copy"
    );
    // ...and surfaces the failed union verdict text from its drawer.
    assert!(
        html.contains("conflitou com #133"),
        "blocked card surfaces its failed union verdict"
    );
}

/// Mirrors the standing app_smoke contract at the unit level: every `col.title`
/// renders with its `card_count()` so the smoke oracle stays green.
#[test]
fn keeps_rendering_column_titles_and_counts_for_app_smoke() {
    let vm = vm_four_columns();
    let html = render(&vm).into_string();
    for col in &vm.columns {
        assert!(
            html.contains(&col.title),
            "col.title `{}` renders",
            col.title
        );
        // the count appears somewhere in the column header badge
        assert!(
            html.contains(&format!(">{}</span>", col.card_count())),
            "card_count for `{}` renders",
            col.title
        );
    }
}
