//! Admin area (operator control-plane) — `/admin`.
//!
//! The forge's own operator console: a one-call OVERVIEW snapshot, the RECENT
//! audit timeline (newest first), and the ERASURE governance history — all
//! served by the engine's pure log projections (`admin/overview`, `audit`,
//! `erasure`). Account-level chrome (`layout::page_account`). Render-faithful to
//! the kit.css design system: lists-not-cards for the timeline, KPI cards only
//! for the aggregate snapshot, dark tokens, no invented data.

use githugr_vm::AdminVm;
use maud::{Markup, html};

pub const SCREEN_CSS: &str = r#"
.adm .wrap{max-width:1000px;margin:0 auto;padding:26px 24px 60px}
.adm .ph{display:flex;align-items:baseline;gap:12px;margin-bottom:6px}
.adm .ph h1{font-size:17px;font-weight:600;letter-spacing:-.02em}
.adm .ph .sp{flex:1}
.adm .ph .meta{font-size:12px;color:var(--faint)}
.adm .lead{font-size:12.5px;color:var(--muted);margin:0 0 22px}

/* KPI snapshot cards */
.adm .kpis{display:grid;grid-template-columns:repeat(4,1fr);gap:12px;margin-bottom:28px}
.adm .kpi{border:1px solid var(--line);border-radius:var(--r);background:var(--panel);padding:13px 15px}
.adm .kpi .n{font-family:var(--mono);font-size:21px;font-weight:600;color:var(--text);line-height:1.1}
.adm .kpi .l{font-size:11px;letter-spacing:.04em;text-transform:uppercase;color:var(--faint);margin-top:5px}

/* section label */
.adm .sec{font-size:11px;letter-spacing:.06em;text-transform:uppercase;color:var(--faint);font-weight:600;margin:26px 0 10px;display:flex;align-items:center;gap:9px}
.adm .sec .sp{flex:1}
.adm .sec .badge{font-size:11px;color:var(--muted);font-weight:500;text-transform:none;letter-spacing:0}

/* dense list (timeline + governance) */
.adm .list{border:1px solid var(--line);border-radius:var(--r);overflow:hidden}
.adm .row{display:grid;grid-template-columns:62px 130px 1fr auto;align-items:center;gap:14px;padding:9px 14px;border-bottom:1px solid var(--line);font-size:12.5px}
.adm .row:last-child{border-bottom:0}
.adm .row:hover{background:var(--hover)}
.adm .row .seq{font-family:var(--mono);color:var(--faint);font-size:11.5px}
.adm .row .kind{font-family:var(--mono);color:var(--text-2);font-size:11.5px}
.adm .row .body{min-width:0;color:var(--text)}
.adm .row .body .who{color:var(--faint)}
.adm .row .right{display:flex;align-items:center;gap:12px;white-space:nowrap}
.adm .row .age{color:var(--faint);font-size:11.5px}
.adm .row .hash{font-family:var(--mono);color:var(--dim);font-size:11px}
.adm .erow{grid-template-columns:1fr auto auto}
.adm .pill{font-size:11px;border-radius:5px;padding:1px 7px;font-weight:500;border:1px solid var(--line-2);color:var(--muted)}
.adm .pill.green{color:var(--green);border-color:rgba(90,158,120,.4)}
.adm .pill.red{color:var(--red);border-color:rgba(207,112,112,.4)}
.adm .pill.amber{color:var(--amber);border-color:rgba(184,154,90,.4)}
.adm .note{font-size:12px;color:var(--faint);margin-top:9px;line-height:1.5}
.adm .zerobox{border:1px dashed var(--line-2);border-radius:var(--r);padding:22px;text-align:center;color:var(--faint);font-size:12.5px}
"#;

pub fn render(vm: &AdminVm) -> Markup {
    let o = &vm.overview;
    html! {
        div .adm {
            div .wrap {
                div .ph {
                    h1 { "Admin · operador" }
                    span .sp {}
                    span .meta { "última atividade: " (o.last_activity_age) }
                }
                p .lead { "Painel de controle do forge — estado operacional, trilha de auditoria e governança. Dados reais projetados do log verificado da engine." }

                // ── Overview snapshot ───────────────────────────────────────
                div .kpis {
                    (kpi(o.queue_depth, "na fila"))
                    (kpi(o.active_campaigns, "campanhas ativas"))
                    (kpi(o.attention_count, "precisam atenção"))
                    (kpi(o.total_prs, "PRs totais"))
                    (kpi(o.policy_rules_active, "regras ativas"))
                    (kpi(o.erasure_decisions, "decisões erasure"))
                    (kpi_u64(o.log_depth, "eventos no log"))
                }

                // ── Recent audit timeline ──────────────────────────────────
                div .sec {
                    "Trilha de auditoria"
                    span .sp {}
                    span .badge { (vm.recent_audit.len()) " eventos recentes" }
                }
                @if vm.recent_audit.is_empty() {
                    div .zerobox { "nenhum evento registrado ainda" }
                } @else {
                    div .list {
                        @for e in &vm.recent_audit {
                            div .row {
                                span .seq { "#" (e.seq) }
                                span .kind { (e.kind) }
                                span .body {
                                    @if !e.summary.is_empty() { (e.summary) " " }
                                    span .who { "· " (e.principal) }
                                }
                                span .right {
                                    span .age { (e.age) }
                                    span .hash { (e.hash_short) }
                                }
                            }
                        }
                    }
                }

                // ── Erasure governance ─────────────────────────────────────
                div .sec {
                    "Governança · erasure"
                    span .sp {}
                    span .badge { (vm.erasure.approved_count) " aprovadas · " (vm.erasure.denied_count) " negadas" }
                }
                @if vm.erasure.entries.is_empty() {
                    div .zerobox { "nenhuma decisão de erasure registrada" }
                } @else {
                    div .list {
                        @for er in &vm.erasure.entries {
                            div .row .erow {
                                span .body {
                                    (er.erasure_id)
                                    span .who { " · " (er.decided_by) " · " (er.age) }
                                }
                                span {
                                    @if er.state == "approved" {
                                        span .pill.green { "aprovada" }
                                    } @else {
                                        span .pill.red { "negada" }
                                    }
                                }
                                span { span .pill.amber { "exec: " (er.execution) } }
                            }
                        }
                    }
                    p .note { (vm.erasure.note) }
                }

                // ── Active sessions (engine tokens) ────────────────────────
                div .sec {
                    "Sessões ativas"
                    span .sp {}
                    span .badge { (vm.tokens.count) " sessões" }
                }
                @if vm.tokens.sessions.is_empty() {
                    div .zerobox { "nenhuma sessão de engine-token ativa" }
                } @else {
                    div .list {
                        @for s in &vm.tokens.sessions {
                            div .row .erow {
                                span .body {
                                    (s.user) span .who { " · " (s.org) }
                                }
                                span {
                                    @if s.fresh_auth { span .pill.green { "fresh" } } @else { span .pill { "—" } }
                                }
                                span .right { span .age { "expira em " (s.expires_in_secs) "s" } }
                            }
                        }
                    }
                    p .note { (vm.tokens.note) }
                }
            }
        }
    }
}

/// A single KPI snapshot card (usize value).
fn kpi(n: usize, label: &str) -> Markup {
    html! {
        div .kpi {
            div .n { (n) }
            div .l { (label) }
        }
    }
}

/// A KPI card for a `u64` value (the log depth).
fn kpi_u64(n: u64, label: &str) -> Markup {
    html! {
        div .kpi {
            div .n { (n) }
            div .l { (label) }
        }
    }
}
