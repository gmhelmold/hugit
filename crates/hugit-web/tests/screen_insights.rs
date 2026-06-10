//! screen_insights — render tests for the Insights + Ledger screen (WP-W6).
//!
//! Hand-built VMs; no fixture.rs dependency. Each test exercises one
//! structural invariant of the render output.

use hugit_web::provider::{
    CampaignChipVm, CostXrayRowVm, InsightsVm, KpiVm, LedgerCampaignVm, LedgerRowVm, LedgerViewVm,
};
use hugit_web::screens::insights::render;

// ── helpers ───────────────────────────────────────────────────────────────────

fn chip(id: &str, label: &str, color_class: &str) -> CampaignChipVm {
    CampaignChipVm {
        id: id.to_string(),
        label: label.to_string(),
        color_class: color_class.to_string(),
    }
}

fn minimal_vm() -> InsightsVm {
    InsightsVm {
        repo: "testrepo".to_string(),
        kpis: vec![],
        landed_by_day: vec![],
        tokens_by_campaign: vec![],
        cost_xray: vec![],
        tokens_by_model: vec![],
        ledger: LedgerViewVm { campaigns: vec![] },
    }
}

// ── 1. KPI cards bind label / value / delta ───────────────────────────────────

#[test]
fn kpi_cards_bind_label_value_delta() {
    let vm = InsightsVm {
        kpis: vec![
            KpiVm {
                label: "PRs / dia".to_string(),
                value: "4.3".to_string(),
                delta: Some("↑ 18%".to_string()),
            },
            KpiVm {
                label: "main verde".to_string(),
                value: "28d".to_string(),
                delta: None,
            },
        ],
        ..minimal_vm()
    };
    let html = render(&vm).into_string();

    assert!(html.contains("PRs / dia"), "kpi label must be present");
    assert!(html.contains("4.3"), "kpi value must be present");
    assert!(html.contains("↑ 18%"), "kpi delta must be present");
    assert!(
        html.contains("main verde"),
        "second kpi label must be present"
    );
    assert!(html.contains("28d"), "second kpi value must be present");
}

// ── 2. Bar chart renders one bar per landed_by_day; max-count day tallest ─────

#[test]
fn bar_chart_one_bar_per_day_max_gets_100pct() {
    let data = vec![
        ("7/6".to_string(), 3u32),
        ("8/6".to_string(), 9u32), // max
        ("9/6".to_string(), 5u32),
    ];
    let vm = InsightsVm {
        landed_by_day: data,
        ..minimal_vm()
    };
    let html = render(&vm).into_string();

    // One bar-col per entry
    assert_eq!(
        html.matches("bar-col").count(),
        3,
        "must render 3 bar-col divs"
    );

    // The max-count entry (9) gets height:100%
    assert!(
        html.contains("height:100%"),
        "max-count day must get height:100%"
    );

    // A non-max entry (3 out of 9 = 33%) must NOT be 100%
    // and must have a lower percentage: 3/9 * 100 = 33
    assert!(html.contains("height:33%"), "3/9 must render as height:33%");

    // x-axis labels are present
    assert!(html.contains("7/6"), "x-axis label 7/6 must be present");
    assert!(html.contains("8/6"), "x-axis label 8/6 must be present");
    assert!(html.contains("9/6"), "x-axis label 9/6 must be present");
}

// ── 3. Cost X-ray row binds all 6 numbers + campaign label + color_class ──────

#[test]
fn cost_xray_row_binds_numbers_and_campaign() {
    let vm = InsightsVm {
        cost_xray: vec![CostXrayRowVm {
            campaign: chip("auth", "auth-hardening", "c-auth"),
            work: 52,
            orchestration: 14,
            verification: 9,
            ci: 8,
            waste: 3,
            total: 86,
        }],
        ..minimal_vm()
    };
    let html = render(&vm).into_string();

    assert!(
        html.contains("auth-hardening"),
        "campaign label must be present"
    );
    assert!(html.contains("c-auth"), "color_class must appear in style");
    assert!(html.contains(">52<"), "work cost must be rendered");
    assert!(html.contains(">14<"), "orchestration cost must be rendered");
    assert!(html.contains(">9<"), "verification cost must be rendered");
    assert!(html.contains(">8<"), "ci cost must be rendered");
    assert!(html.contains(">3<"), "waste must be rendered");
    assert!(html.contains(">86<"), "total must be rendered");
}

// ── 4. Ledger campaign header binds pedido/feito/provado; rows bind fields ────

#[test]
fn ledger_campaign_header_and_rows() {
    let vm = InsightsVm {
        ledger: LedgerViewVm {
            campaigns: vec![LedgerCampaignVm {
                campaign: chip("perf", "perf", "c-perf"),
                asked: 5,
                done: 4,
                proven: 3,
                rows: vec![
                    LedgerRowVm {
                        intent_id: "int-001".to_string(),
                        asked: "Cortar latência de lookup".to_string(),
                        done_status: "mergeado".to_string(),
                        proven_status: "correctness APPROVE".to_string(),
                        verdict: Some("APPROVE".to_string()),
                    },
                    LedgerRowVm {
                        intent_id: "int-002".to_string(),
                        asked: "Evitar serialização redundante".to_string(),
                        done_status: "em voo".to_string(),
                        proven_status: "—".to_string(),
                        verdict: None,
                    },
                ],
            }],
        },
        ..minimal_vm()
    };
    let html = render(&vm).into_string();

    // Campaign header counts
    assert!(html.contains(">5<"), "pedido count must be 5");
    assert!(html.contains(">4<"), "done count must be 4");
    assert!(html.contains(">3<"), "proven count must be 3");

    // Row fields
    assert!(html.contains("int-001"), "intent_id must be present");
    assert!(
        html.contains("Cortar latência de lookup"),
        "asked charter must be present"
    );
    assert!(html.contains("mergeado"), "done_status must be present");
    assert!(
        html.contains("correctness APPROVE"),
        "proven_status must be present"
    );

    // Verdict chip renders when Some
    assert!(
        html.contains("vchip"),
        "verdict chip must render when verdict is Some"
    );
    assert!(html.contains("APPROVE"), "verdict text must be rendered");

    // int-002: verdict is None — no second vchip (only one per the table)
    // Count: only one vchip in the whole output
    assert_eq!(
        html.matches("vchip").count(),
        1,
        "verdict chip must be absent when verdict is None"
    );
}

// ── 5. Intent links use /r/<repo>/intent/<id> ─────────────────────────────────

#[test]
fn intent_links_use_correct_href() {
    let vm = InsightsVm {
        repo: "corelink-server".to_string(),
        ledger: LedgerViewVm {
            campaigns: vec![LedgerCampaignVm {
                campaign: chip("obs", "obs", "c-obs"),
                asked: 1,
                done: 1,
                proven: 1,
                rows: vec![LedgerRowVm {
                    intent_id: "int-abc".to_string(),
                    asked: "Expor hit-rate".to_string(),
                    done_status: "mergeado".to_string(),
                    proven_status: "APPROVE".to_string(),
                    verdict: None,
                }],
            }],
        },
        ..minimal_vm()
    };
    let html = render(&vm).into_string();

    assert!(
        html.contains(r#"href="/r/corelink-server/intent/int-abc""#),
        "intent link must use /r/<repo>/intent/<id> format"
    );
}

// ── 6. Rail renders all five nav items including Ledger ──────────────────────

#[test]
fn rail_renders_all_nav_items_including_ledger() {
    let vm = minimal_vm();
    let html = render(&vm).into_string();

    assert!(
        html.contains("Visão geral"),
        "rail: Visão geral must be present"
    );
    assert!(html.contains("Velocity"), "rail: Velocity must be present");
    assert!(
        html.contains("Tokens &amp; custo") || html.contains("Tokens & custo"),
        "rail: Tokens & custo must be present"
    );
    assert!(
        html.contains("Contribuição"),
        "rail: Contribuição must be present"
    );
    assert!(html.contains("Ledger"), "rail: Ledger must be present");

    // The goIns JS function must be present for view switching
    assert!(html.contains("goIns"), "goIns JS function must be inlined");
    assert!(
        html.contains("showLedger"),
        "showLedger JS function must be inlined"
    );
}
