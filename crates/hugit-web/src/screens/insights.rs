//! Insights + the Ledger view. Spec: ../githugr/design/insights.html
//!
//! Render-faithful port of insights.html (WP-W6).
//! Layout: snav rail (Visão geral / Velocity / Tokens & custo / Contribuição /
//! Ledger) + two views (analytics + ledger) toggled client-side via goIns() /
//! showLedger() (ported from the mockup JS).
//!
//! CSS: ported from the <style> block of insights.html; excludes kit rules
//! (tokens / topbar / brand / cmdk / tabbar / rtab / foot / overlay).
//! All rules scoped under `.ins-wrap` root — no body overrides.

use crate::provider::InsightsVm;
use maud::{Markup, PreEscaped, html};

/// Screen-specific CSS ported from insights.html <style>.
/// Excludes kit rules (tokens/topbar/brand/cmdk/tabbar/rtab/foot/overlay).
pub const SCREEN_CSS: &str = r#"
.ins-wrap{display:flex;height:calc(100vh - 90px);overflow:hidden}
.snav{width:222px;flex:0 0 222px;border-right:1px solid var(--line);background:var(--rail);overflow-y:auto;padding:18px 12px 40px}
.snav-title{font-size:10.5px;letter-spacing:.07em;text-transform:uppercase;color:var(--faint);font-weight:600;padding:2px 10px 10px}
.snav-item{display:flex;align-items:center;gap:9px;padding:7px 10px;border-radius:6px;font-size:13px;color:var(--muted);cursor:pointer;margin-bottom:1px}
.snav-item:hover{background:var(--hover);color:var(--text-2)}
.snav-item.on{background:rgba(255,255,255,.05);color:var(--text);font-weight:500;box-shadow:inset 2px 0 0 var(--accent)}
.snav-item .ic{color:var(--faint);width:14px;text-align:center;font-size:11px}
.ins-content{flex:1;overflow-y:auto}
.ins-view{display:none;padding:24px 28px 40px}
.ins-view.vis{display:block}
.kpi-val{font-size:26px;font-weight:700;letter-spacing:-.03em;color:var(--text);line-height:1.1;margin:8px 0 4px}
.kpi-val .unit{font-size:14px;font-weight:500;color:var(--muted);letter-spacing:0}
.kpi-sub{font-size:11.5px;color:var(--faint);display:flex;align-items:center;gap:6px}
.kpi-streak{font-size:11.5px;color:var(--faint);display:flex;align-items:center;gap:7px;margin-top:4px}
.kpi-streak .dot{width:6px;height:6px;border-radius:50%;background:var(--green)}
.chart-wrap{border:1px solid var(--line);border-radius:var(--r);background:var(--panel);overflow:hidden}
.chart-head{display:flex;align-items:center;gap:10px;padding:13px 16px 11px;border-bottom:1px solid var(--line)}
.chart-head .ttl{font-size:13.5px;font-weight:600}
.chart-head .sub{font-size:11.5px;color:var(--faint);margin-left:2px}
.chart-head .sp{flex:1}
.chart-legend{display:flex;align-items:center;gap:14px;font-size:11px;color:var(--faint)}
.chart-legend span{display:inline-flex;align-items:center;gap:5px}
.chart-legend .swatch{width:8px;height:8px;border-radius:2px;background:var(--dim)}
.chart-legend .swatch.hi{background:var(--accent)}
.chart-body{padding:16px 16px 12px}
.bars{display:flex;align-items:flex-end;gap:5px;height:90px}
.bar-col{flex:1;display:flex;flex-direction:column;align-items:center;gap:4px}
.bar-col .b{width:100%;border-radius:3px 3px 0 0;background:rgba(255,255,255,.12);min-height:3px}
.bar-col .b.hi{background:var(--accent)}
.bar-col .lbl{font-family:var(--mono);font-size:9.5px;color:var(--faint);white-space:nowrap}
.bar-col .val{font-family:var(--mono);font-size:9.5px;color:var(--dim)}
.chart-xaxis{display:flex;gap:5px;padding:0 0 4px}
.chart-xaxis .xl{flex:1;text-align:center;font-size:9px;color:var(--dim);font-family:var(--mono)}
.hbar-row{display:flex;align-items:center;gap:11px;padding:9px 0;border-bottom:1px solid var(--line)}
.hbar-row:last-child{border-bottom:0}
.hbar-row .nm{width:120px;flex:0 0 120px;font-size:12.5px;color:var(--text-2);display:flex;align-items:center;gap:8px}
.hbar-row .nm .cd{width:8px;height:8px;border-radius:50%;flex:0 0 auto}
.hbar-row .track{flex:1;height:7px;background:rgba(255,255,255,.06);border-radius:4px;overflow:hidden}
.hbar-row .track i{display:block;height:100%;border-radius:4px}
.hbar-row .amt{width:120px;flex:0 0 120px;text-align:right;font-family:var(--mono);font-size:12px;color:var(--text-2)}
.cost-table{width:100%;border-collapse:collapse;font-size:12.5px}
.cost-table th{font-size:10.5px;letter-spacing:.04em;text-transform:uppercase;color:var(--faint);font-weight:600;text-align:left;padding:0 0 8px;border-bottom:1px solid var(--line)}
.cost-table th:not(:first-child){text-align:right}
.cost-table td{padding:9px 0;border-bottom:1px solid var(--line);color:var(--text-2);vertical-align:middle}
.cost-table td:not(:first-child){text-align:right;font-family:var(--mono);font-size:12px}
.cost-table tr:last-child td{border-bottom:0}
.cost-table .camp-dot{width:7px;height:7px;border-radius:50%;display:inline-block;margin-right:7px;vertical-align:middle}
.cost-table tfoot td{color:var(--muted);font-size:12px;padding-top:10px;border-top:1px solid var(--line-2)}
.tk-row{display:flex;align-items:center;gap:11px;padding:10px 0;border-bottom:1px solid var(--line)}
.tk-row:last-child{border-bottom:0}
.tk-row .nm{width:96px;flex:0 0 96px;font-size:12.5px;color:var(--text-2)}
.tk-bar{flex:1;display:flex;height:8px;border-radius:4px;overflow:hidden;background:rgba(255,255,255,.05)}
.tk-bar i{height:100%}
.tk-in{background:rgba(255,255,255,.5)}
.tk-out{background:rgba(255,255,255,.28)}
.tk-cache{background:rgba(255,255,255,.12)}
.tk-row .amt{width:120px;flex:0 0 120px;text-align:right;font-family:var(--mono);font-size:12px;color:var(--text-2)}
.fpill{user-select:none;font-size:12px;padding:3px 10px;border-radius:20px;border:1px solid var(--line-2);color:var(--muted);cursor:pointer}
.fpill.on{background:rgba(255,255,255,.07);color:var(--text);border-color:var(--line-3)}
.filters{display:flex;align-items:center;gap:7px}
.lhead{display:flex;align-items:flex-start;gap:14px;margin-bottom:6px}
.lhead .sp{flex:1}
.camp{margin:0 0 4px}
.camphd{display:flex;align-items:center;gap:9px;padding:10px 0 9px;border-bottom:1px solid var(--line);cursor:pointer}
.camphd .cd{width:9px;height:9px;border-radius:50%;flex:0 0 auto}
.camphd .nm{font-size:13px;font-weight:600;color:var(--text-2)}
.camphd .sp{flex:1}
.camphd .cnt{font-size:11.5px;color:var(--faint)}
.led-table{width:100%;border-collapse:collapse;font-size:12.5px;margin:4px 0 16px}
.led-table th{font-size:10.5px;letter-spacing:.04em;text-transform:uppercase;color:var(--faint);font-weight:600;text-align:left;padding:0 0 7px;border-bottom:1px solid var(--line)}
.led-table td{padding:8px 0;border-bottom:1px solid var(--line);color:var(--text-2);vertical-align:middle;font-size:12.5px}
.led-table tr:last-child td{border-bottom:0}
.led-table .iid{font-family:var(--mono);font-size:11.5px;color:var(--muted)}
.led-table .iid a{color:var(--muted);border-bottom:1px solid var(--line-2)}
.led-table .iid a:hover{color:var(--text)}
.vchip{display:inline-flex;align-items:center;gap:5px;font-size:11px;color:var(--muted);background:rgba(255,255,255,.04);border:1px solid var(--line-2);border-radius:20px;padding:2px 8px}
.vchip .dot{width:5px;height:5px;border-radius:50%;background:var(--green)}
.lead{font-size:13px;color:var(--muted);line-height:1.5;margin:0}
"#;

pub fn render(vm: &InsightsVm) -> Markup {
    html! {
        div .ins-wrap {
            // ── left snav rail ────────────────────────────────────────────
            nav .snav {
                div .snav-title { "Análise · repo" }
                div .snav-item .on id="snav-top"
                    onclick="goIns('iv-top',this)" {
                    span .ic { "▤" } " Visão geral"
                }
                div .snav-item id="snav-velocity"
                    onclick="goIns('iv-velocity',this)" {
                    span .ic { "▦" } " Velocity"
                }
                div .snav-item id="snav-cost"
                    onclick="goIns('iv-cost',this)" {
                    span .ic { "◷" } " Tokens & custo"
                }
                div .snav-item id="snav-contrib"
                    onclick="goIns('iv-contrib',this)" {
                    span .ic { "◇" } " Contribuição"
                }
                div .snav-title style="margin-top:16px" { "Registro" }
                div .snav-item id="snav-ledger"
                    onclick="showLedger(this)" {
                    span .ic { "≣" } " Ledger"
                }
            }

            // ── scrollable content area ───────────────────────────────────
            div .ins-content {

                // ── analytics view ────────────────────────────────────────
                div .ins-view .vis id="analytics-view" {

                    // period selector + title row
                    div id="iv-top"
                        style="display:flex;align-items:center;gap:10px;margin-bottom:20px" {
                        h1 style="font-size:17px;margin:0" { "Insights" }
                        span style="color:var(--faint);font-size:13px" {
                            (vm.repo) " · últimos "
                            span id="ins-period-label" { "30d" }
                        }
                        span style="flex:1" {}
                        div .filters {
                            span .fpill id="ins-pill-7d"
                                onclick="insSetPeriod('7d',this)" { "7d" }
                            span .fpill .on id="ins-pill-30d"
                                onclick="insSetPeriod('30d',this)" { "30d" }
                            span .fpill id="ins-pill-90d"
                                onclick="insSetPeriod('90d',this)" { "90d" }
                        }
                    }

                    // KPI card grid (6 columns matching mockup)
                    (kpi_cards(&vm.kpis))

                    // Velocity section: bar chart + tokens by campaign
                    div id="iv-velocity" .grid2 style="margin-bottom:24px" {
                        (landed_bar_chart(&vm.landed_by_day))
                        (tokens_by_campaign_card(&vm.tokens_by_campaign))
                    }

                    // Cost X-ray section
                    div id="iv-cost" .card style="margin-bottom:24px" {
                        div .h {
                            "Cost X-ray por campanha "
                            span .g
                                style="font-size:10px;text-transform:none;letter-spacing:0;color:var(--dim);font-weight:400" {
                                "· tokens · custo decomposto · desperdício · economia · 30d"
                            }
                        }
                        (cost_xray_table(&vm.cost_xray))
                    }

                    // Tokens by model stacked bars
                    div id="iv-contrib" .grid2 style="margin-bottom:24px" {
                        (tokens_by_model_card(&vm.tokens_by_model))
                    }
                }

                // ── ledger view (pedido → feito → provado) ───────────────
                div .ins-view id="ledger-view" {
                    div .lhead {
                        div {
                            h1 .title style="font-size:20px;margin:0" { "Ledger" }
                            p .lead style="margin:4px 0 0" {
                                "A história em altitude de intent: "
                                b { "pedido → feito → provado" }
                                ", por campanha."
                            }
                        }
                        span .sp {}
                    }

                    @for camp in &vm.ledger.campaigns {
                        (ledger_campaign(camp, &vm.repo))
                    }
                }
            }
        }

        // ── screen JS (goIns / showLedger rail switch, ported from mockup) ──
        script { (PreEscaped(INSIGHTS_JS)) }
    }
}

// ── KPI card grid ─────────────────────────────────────────────────────────────

fn kpi_cards(kpis: &[crate::provider::KpiVm]) -> Markup {
    let count = kpis.len().max(1);
    let style = format!(
        "display:grid;grid-template-columns:repeat({count},1fr);gap:12px;margin-bottom:24px"
    );
    html! {
        div style=(style) {
            @for kpi in kpis {
                div .card {
                    div .h { (kpi.label) }
                    div .kpi-val { (kpi.value) }
                    @if let Some(delta) = &kpi.delta {
                        div .kpi-sub {
                            span style="color:var(--green)" { (delta) }
                        }
                    }
                }
            }
        }
    }
}

// ── Landed bar chart — server-side rendered ───────────────────────────────────
// Each bar is a div with height proportional to count/max (no JS chart lib).

fn landed_bar_chart(data: &[(String, u32)]) -> Markup {
    let max_count = data.iter().map(|(_, c)| *c).max().unwrap_or(1).max(1);
    let last_idx = data.len().saturating_sub(1);

    html! {
        div .chart-wrap {
            div .chart-head {
                span .ttl { "PRs landados" }
                span .sub { "últimos 14 dias" }
                span .sp {}
                div .chart-legend {
                    span { span .swatch {} " dia" }
                    span { span .swatch.hi {} " ontem" }
                }
            }
            div .chart-body {
                div .bars {
                    @for (idx, (day, count)) in data.iter().enumerate() {
                        @let pct = (*count as f64 / max_count as f64 * 100.0).round() as u32;
                        @let is_hi = idx == last_idx;
                        div .bar-col title={ (*count) " PRs · " (day) } {
                            div .val { (*count) }
                            @if is_hi {
                                div .b.hi style={ "height:" (pct) "%" } {}
                            } @else {
                                div .b style={ "height:" (pct) "%" } {}
                            }
                        }
                    }
                }
                div .chart-xaxis {
                    @for (day, _) in data {
                        div .xl { (day) }
                    }
                }
            }
        }
    }
}

// ── Tokens by campaign horizontal bars ────────────────────────────────────────

fn tokens_by_campaign_card(data: &[(crate::provider::CampaignChipVm, u64)]) -> Markup {
    let max_tok = data.iter().map(|(_, t)| *t).max().unwrap_or(1).max(1);

    html! {
        div .card {
            div .h {
                "Tokens por campanha "
                span .g
                    style="font-size:10px;text-transform:none;letter-spacing:0;color:var(--dim);font-weight:400" {
                    "· 30d · tokens"
                }
            }
            @for (chip, tokens) in data {
                @let pct = (*tokens as f64 / max_tok as f64 * 100.0).round() as u32;
                div .hbar-row {
                    span .nm {
                        span .cd
                            style={ "background:var(--" (chip.color_class) ")" } {}
                        (chip.label)
                    }
                    span .track {
                        i style={
                            "width:" (pct) "%;"
                            "background:var(--" (chip.color_class) ")"
                        } {}
                    }
                    span .amt { (format_tokens(*tokens)) }
                }
            }
        }
    }
}

// ── Cost X-ray table ──────────────────────────────────────────────────────────

fn cost_xray_table(rows: &[crate::provider::CostXrayRowVm]) -> Markup {
    html! {
        table .cost-table {
            thead {
                tr {
                    th { "Campanha" }
                    th { "Work" }
                    th { "Orch" }
                    th { "Verif" }
                    th { "CI" }
                    th { "Desperdício" }
                    th { "Total" }
                }
            }
            tbody {
                @for row in rows {
                    tr {
                        td {
                            span .camp-dot
                                style={ "background:var(--" (row.campaign.color_class) ")" } {}
                            (row.campaign.label)
                        }
                        td { (row.work) }
                        td { (row.orchestration) }
                        td { (row.verification) }
                        td { (row.ci) }
                        td style="color:var(--amber)" { (row.waste) }
                        td style="color:var(--text);font-weight:600" { (row.total) }
                    }
                }
            }
        }
    }
}

// ── Tokens by model stacked bars ──────────────────────────────────────────────

fn tokens_by_model_card(data: &[(String, u64)]) -> Markup {
    let max_tok = data.iter().map(|(_, t)| *t).max().unwrap_or(1).max(1);

    html! {
        div .card {
            div .h {
                "Tokens por modelo "
                span .g
                    style="font-size:10px;text-transform:none;letter-spacing:0;color:var(--dim);font-weight:400" {
                    "· input · output · cache · 30d"
                }
            }
            @for (model, tokens) in data {
                @let in_pct  = (*tokens as f64 / max_tok as f64 * 64.0).round() as u32;
                @let out_pct = (*tokens as f64 / max_tok as f64 * 17.0).round() as u32;
                @let ca_pct  = (*tokens as f64 / max_tok as f64 * 19.0).round() as u32;
                div .tk-row {
                    span .nm { (model) }
                    span .tk-bar {
                        i .tk-in    style={ "width:" (in_pct)  "%" } {}
                        i .tk-out   style={ "width:" (out_pct) "%" } {}
                        i .tk-cache style={ "width:" (ca_pct)  "%" } {}
                    }
                    span .amt { (format_tokens(*tokens)) }
                }
            }
        }
    }
}

// ── Ledger campaign block (pedido → feito → provado) ─────────────────────────

fn ledger_campaign(camp: &crate::provider::LedgerCampaignVm, repo: &str) -> Markup {
    html! {
        div .camp style={ "--cc:var(--" (camp.campaign.color_class) ")" } {
            div .camphd {
                span .cd
                    style={ "background:var(--" (camp.campaign.color_class) ")" } {}
                span .nm { (camp.campaign.label) }
                span .cnt {
                    " · pedido " b { (camp.asked) }
                    " · feito " b { (camp.done) }
                    " · provado " b { (camp.proven) }
                }
                span .sp {}
            }

            @if !camp.rows.is_empty() {
                table .led-table {
                    thead {
                        tr {
                            th { "Intent" }
                            th { "Pedido (charter)" }
                            th { "Feito" }
                            th { "Provado" }
                            th { "Verdict" }
                        }
                    }
                    tbody {
                        @for row in &camp.rows {
                            tr {
                                td .iid {
                                    a href={
                                        "/r/" (repo)
                                        "/intent/" (row.intent_id)
                                    } {
                                        (row.intent_id)
                                    }
                                }
                                td { (row.asked) }
                                td { (row.done_status) }
                                td { (row.proven_status) }
                                td {
                                    @if let Some(v) = &row.verdict {
                                        span .vchip {
                                            span .dot {}
                                            (v)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

// ── helpers ───────────────────────────────────────────────────────────────────

fn format_tokens(t: u64) -> String {
    if t >= 1_000_000 {
        format!("{:.1}M", t as f64 / 1_000_000.0)
    } else if t >= 1_000 {
        format!("{:.0}k", t as f64 / 1_000.0)
    } else {
        format!("{t}")
    }
}

// ── Client-side JS: rail switch ported from insights.html goIns/showLedger ───

const INSIGHTS_JS: &str = r#"
// Analytics sub-nav scroll — show analytics view, highlight item, scroll to anchor.
function goIns(id, el) {
  document.getElementById('analytics-view').classList.add('vis');
  document.getElementById('ledger-view').classList.remove('vis');
  document.querySelectorAll('.snav-item').forEach(function(x){ x.classList.remove('on'); });
  el.classList.add('on');
  var t = document.getElementById(id);
  if (t) t.scrollIntoView({ behavior: 'smooth', block: 'start' });
}

// Show ledger view, hide analytics.
function showLedger(el) {
  document.querySelectorAll('.snav-item').forEach(function(x){ x.classList.remove('on'); });
  if (el) el.classList.add('on');
  document.getElementById('analytics-view').classList.remove('vis');
  document.getElementById('ledger-view').classList.add('vis');
}

// Period pill visual toggle (data is server-rendered; pills update the label only).
function insSetPeriod(p, el) {
  document.querySelectorAll('.filters .fpill').forEach(function(x){ x.classList.remove('on'); });
  el.classList.add('on');
  var lbl = document.getElementById('ins-period-label');
  if (lbl) lbl.textContent = p;
}

// Deep-link: insights#ledger lands directly on the Ledger view.
if (location.hash === '#ledger') {
  showLedger(document.getElementById('snav-ledger'));
}
"#;
