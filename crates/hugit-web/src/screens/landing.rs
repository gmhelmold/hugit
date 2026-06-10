//! Landing (PRs) — the hero screen. Spec: ../githugr/design/landing.html
//!
//! Render-faithful WP-W2 port of the kanban Landing: filters + mainstat strip,
//! the four-column board (Na fila / Testando juntos / Bloqueado / Pousado hoje),
//! PR cards, campaign BUNDLES (nested cards under a color-coded header), and the
//! right-side PR DRAWER (breadcrumb · collapsible intents with charter /
//! context.json / diff / verdicts · the landing lcard · file list).
//!
//! Server-rendered reality vs. the mockup's single shared drawer: every card
//! embeds ITS OWN drawer body in a hidden `<template>`; `openPR(this)` clones
//! that template into the live drawer container. So clicking a card opens its
//! real data (intents/diff/verdicts/cost/mirror) — the mockup's `openPR(this)`
//! pattern, adapted, with no client-side data fetch.
//!
//! Read-only build (this wave): filters/search/list-toggle render faithfully but
//! are inert (the mockup itself is inert there); the Land button is disabled with
//! an honest title — the write-path lands in a future wave.

use crate::provider::{
    DiffLineKind, DiffVm, IntentSummaryVm, LandingColumnVm, LandingItemVm, LandingVm, PrCardVm,
    PrDrawerVm, PrState,
};
use maud::{Markup, PreEscaped, html};

/// Screen-specific CSS, ported from the mockup's `<style>` (everything NOT in
/// `static/kit.css` — the kit carries only :root tokens + the shared chrome).
///
/// `height:100vh;overflow:hidden` is scoped onto `.landing-app` (NOT body) since
/// the shared chrome wraps this screen; the app reserves the topbar+tabbar+footer
/// bands and gives the stage the remaining viewport (the mockup's `1fr` region).
pub const SCREEN_CSS: &str = r#"
.landing-app{display:flex;flex-direction:column;height:calc(100vh - 46px - 43px - 34px);overflow:hidden}

.subhead{display:flex;align-items:center;gap:12px;padding:11px 18px;border-bottom:1px solid var(--line)}
.subhead .sp{flex:1}
.mainstat{display:inline-flex;align-items:center;gap:7px;font-size:12px;color:var(--green);background:rgba(90,158,120,.1);border:1px solid rgba(90,158,120,.28);border-radius:20px;padding:3px 11px}
.mainstat .dot{width:6px;height:6px;border-radius:50%;background:var(--green)}
.filters{display:flex;align-items:center;gap:6px;flex-wrap:wrap}
.fpill{font-size:12px;color:var(--muted);border:1px solid var(--line-2);border-radius:7px;padding:4px 10px;cursor:pointer}
.fpill:hover{border-color:var(--line-3);color:var(--text)}
.fpill.on{color:var(--text);background:rgba(255,255,255,.05);border-color:var(--line-3);font-weight:500}
.fpill b{color:var(--faint);font-weight:600;margin-left:4px} .fpill.on b{color:var(--muted)}
/* board|list toggle */
.vtoggle{display:inline-flex;border:1px solid var(--line-2);border-radius:7px;overflow:hidden;flex:0 0 auto}
.vtoggle span{padding:4px 11px;font-size:12px;color:var(--muted);cursor:pointer;border-right:1px solid var(--line-2);display:inline-flex;align-items:center;gap:6px}
.vtoggle span:last-child{border-right:0}
.vtoggle span.on{background:rgba(255,255,255,.06);color:var(--text);font-weight:500}
.btn{display:inline-flex;align-items:center;gap:7px;background:transparent;color:var(--text);border:1px solid var(--line-2);border-radius:7px;padding:6px 12px;font-size:12.5px;font-weight:500;cursor:pointer;font-family:var(--font)} .btn:hover{border-color:var(--line-3)}
.btn.primary{background:var(--accent);border-color:var(--accent);color:#15151a;font-weight:600} .btn.primary:hover{filter:brightness(.94)}
.btn.sm{padding:4px 10px;font-size:11.5px}
.btn:disabled{opacity:.45;cursor:not-allowed}

/* filter bar (GitHub-faithful: query + dropdowns + group/sort + save view) */
.lbar{display:flex;align-items:center;gap:8px;padding:9px 18px;border-bottom:1px solid var(--line);background:var(--rail)}
.lsearch{flex:1;max-width:540px;display:flex;align-items:center;gap:8px;border:1px solid var(--line-2);border-radius:7px;padding:6px 11px;color:var(--faint);font-size:12.5px}
.lsearch input{flex:1;background:transparent;border:0;outline:0;color:var(--text);font-family:var(--font);font-size:12.5px}
.lsearch input::placeholder{color:var(--faint)}
.lsearch .tok{font-family:var(--mono);font-size:11px;color:var(--muted);background:rgba(255,255,255,.05);border:1px solid var(--line-2);border-radius:5px;padding:1px 6px}
.ldrop{position:relative;display:inline-flex;align-items:center;gap:6px;border:1px solid var(--line-2);border-radius:7px;padding:6px 11px;color:var(--muted);font-size:12px;cursor:pointer;flex:0 0 auto}
.ldrop:hover{border-color:var(--line-3);color:var(--text)} .ldrop b{color:var(--text-2);font-weight:500}
.lbar .sp{flex:1}
.lsave{font-size:12px;color:var(--muted);border:1px dashed var(--line-3);border-radius:7px;padding:6px 11px;cursor:pointer;flex:0 0 auto}
.lsave:hover{color:var(--text)}
.lmenu{position:absolute;top:34px;left:0;width:182px;background:var(--panel-2);border:1px solid var(--line-2);border-radius:8px;box-shadow:0 16px 40px rgba(0,0,0,.5);padding:5px;z-index:30;display:none}
.lmenu.show{display:block}
.lmenu div{padding:7px 10px;font-size:12.5px;color:var(--text-2);border-radius:6px;cursor:pointer}
.lmenu div:hover{background:var(--accent-soft);color:var(--text)}

/* stage = board (flex) + drawer (right) */
.stage{display:flex;overflow:hidden;flex:1;min-height:0}
.board{flex:1;display:flex;gap:14px;padding:16px 18px;overflow-x:auto;align-items:flex-start}
.column{flex:0 0 300px;display:flex;flex-direction:column;background:var(--col);border:1px solid var(--line);border-radius:10px;max-height:100%}
.ch{display:flex;align-items:center;gap:9px;padding:12px 14px;border-bottom:1px solid var(--line)}
.ch .d{width:8px;height:8px;border-radius:50%} .ch .t{font-size:13px;font-weight:600} .ch .n{margin-left:auto;font-size:11px;color:var(--muted);background:rgba(255,255,255,.06);border-radius:20px;padding:0 7px;font-weight:600}
.ch.queue .d{background:var(--dim)} .ch.test .d{background:var(--accent)} .ch.block .d{background:var(--red)} .ch.done .d{background:var(--green)}
.cards{padding:10px;display:flex;flex-direction:column;gap:10px;overflow-y:auto}

.card{background:var(--card);border:1px solid var(--line-2);border-left:3px solid var(--cc,var(--dim));border-radius:var(--r);padding:11px 12px;cursor:pointer;transition:.12s}
.card:hover{background:var(--card-2);border-color:var(--line-3);transform:translateY(-1px)}
.card.sel{border-color:var(--accent);box-shadow:0 0 0 1px var(--accent)}
.card .c1{display:flex;align-items:center;gap:8px;margin-bottom:6px}
.card .num{font-family:var(--mono);font-size:11px;color:var(--faint)}
.card .camp{margin-left:auto;display:inline-flex;align-items:center;gap:5px;font-size:10.5px;color:var(--muted)} .card .camp .cd{width:7px;height:7px;border-radius:50%;background:var(--cc)}
.card .title{font-size:13.5px;font-weight:500;color:var(--text);line-height:1.4}
.card .c2{display:flex;align-items:center;gap:9px;margin-top:9px;font-size:11.5px;color:var(--faint)} .card .av{width:18px;height:18px;border-radius:50%;background:#2a2a31;border:1px solid var(--line-2)} .card .sp{flex:1} .card .ints b{color:var(--muted)}
.card .ok{color:var(--green)} .card .tm{color:var(--dim)}
.card .blk{margin-top:8px;font-size:11px;color:var(--red);background:rgba(207,112,112,.08);border:1px solid rgba(207,112,112,.2);border-radius:6px;padding:6px 9px;line-height:1.4}

.bundle{border:1px solid var(--line-2);border-radius:10px;overflow:hidden;box-shadow:inset 3px 0 0 0 var(--cc)}
.bh{display:flex;align-items:center;gap:8px;padding:10px 12px 9px 14px;background:rgba(255,255,255,.02);border-bottom:1px solid var(--line)}
.bh .cd{width:9px;height:9px;border-radius:50%;background:var(--cc)} .bh .nm{font-size:12.5px;font-weight:600;color:var(--text-2)} .bh .sp{flex:1} .bh .pct{font-family:var(--mono);font-size:11px;color:var(--accent);font-weight:600}
.bsub{font-size:10.5px;color:var(--faint);padding:0 14px 8px;background:rgba(255,255,255,.02);border-bottom:1px solid var(--line)}
.bcards{padding:9px;display:flex;flex-direction:column;gap:8px}

/* DRAWER — opens on the right, fills the free space */
.drawer{flex:0 0 0;width:0;overflow:hidden;border-left:1px solid var(--line);background:var(--rail);transition:width .2s ease,flex-basis .2s ease}
.landing-app.open .drawer{flex:0 0 560px;width:560px}
@media(max-width:1200px){.landing-app.open .drawer{flex:0 0 460px;width:460px}}
.din{width:560px;height:100%;overflow-y:auto;padding:18px 20px 40px}
.dx{position:sticky;top:-18px;background:var(--rail);padding:4px 0 12px;margin:-4px 0 0;z-index:3;display:flex;align-items:center;gap:10px;border-bottom:1px solid var(--line)}
.dx .num{font-family:var(--mono);font-size:12px;color:var(--faint)} .dx .sp{flex:1}
.dh1{font-size:18px;font-weight:600;letter-spacing:-.02em;margin:14px 0 8px}
.dpill{display:inline-flex;align-items:center;gap:6px;font-size:11.5px;font-weight:500;padding:2px 10px;border-radius:20px;color:var(--muted);background:var(--accent-soft);border:1px solid var(--line-2)} .dpill .dot{width:6px;height:6px;border-radius:50%;background:var(--faint)}
.dmeta{color:var(--muted);font-size:12.5px;margin:8px 0 0} .dmeta b{color:var(--text-2)} .dmeta .mono{color:var(--muted)}
.better{display:flex;flex-direction:column;gap:6px;margin:14px 0}
.chip{display:flex;align-items:center;gap:8px;font-size:12px;color:var(--text-2);background:var(--panel);border:1px solid var(--line);border-radius:7px;padding:7px 11px} .chip .i{color:var(--green)} .chip b{color:var(--text)} .chip .l{color:var(--faint);margin-left:auto}

.dsec{font-size:11px;letter-spacing:.05em;text-transform:uppercase;color:var(--faint);font-weight:600;margin:22px 0 9px}
.intent{border:1px solid var(--line);border-radius:var(--r);background:var(--panel);margin-bottom:9px;overflow:hidden}
.ih{display:grid;grid-template-columns:20px 1fr auto;align-items:center;gap:11px;padding:11px 13px;cursor:pointer} .ih:hover{background:var(--panel-2)}
.ih .ico{color:var(--green);text-align:center} .ih .ico.t{color:var(--accent)}
.ih .ti{font-size:13.5px;font-weight:500;color:var(--text)} .ih .mt{font-size:11.5px;color:var(--faint);margin-top:2px} .ih .mt .id{font-family:var(--mono);color:var(--muted)} .ih .mt b{color:var(--muted)}
.ih .chev{color:var(--faint);transition:.15s} .intent.open .ih .chev{transform:rotate(90deg)}
.ibody{display:none;border-top:1px solid var(--line);padding:14px 13px} .intent.open .ibody{display:block}
.blk{margin-bottom:15px} .blk:last-child{margin-bottom:0}
.bt{font-size:10.5px;letter-spacing:.04em;text-transform:uppercase;color:var(--faint);font-weight:600;margin-bottom:7px} .bt .g{text-transform:none;letter-spacing:0;font-weight:400;color:var(--dim)}
.charter{font-size:13px;color:var(--text-2)}
.code{border:1px solid var(--line);border-radius:var(--r);overflow:hidden;background:#0d0d10;font-family:var(--mono);font-size:11.5px}
.code .fn{padding:6px 12px;border-bottom:1px solid var(--line);background:var(--rail);color:var(--faint);font-size:10.5px}
.cl{display:grid;grid-template-columns:30px 1fr} .cl .ln{color:var(--dim);text-align:right;padding:1px 8px 1px 0} .cl .ct{padding:1px 12px;white-space:pre;color:var(--text-2)}
.cl.add{background:rgba(90,158,120,.08)} .cl.add .ct{color:#bfe6cf} .cl.del{background:rgba(207,112,112,.08)} .cl.del .ct{color:#e9b6b6} .cl.hl{background:rgba(255,255,255,.1);border-left:2px solid var(--accent)} .cl.hl .ln{padding-left:0} .cm{color:var(--faint)}
.acts{margin-top:8px;display:flex;gap:7px;flex-wrap:wrap} .bg{background:transparent;color:var(--text);border:1px solid var(--line-2);border-radius:6px;padding:5px 10px;font-size:11.5px;cursor:pointer;font-family:var(--font);text-decoration:none;display:inline-flex;align-items:center} .bg:hover{border-color:var(--line-3)}
.verd{display:flex;align-items:center;gap:9px;font-size:12px;color:var(--muted);padding:5px 0;border-bottom:1px solid var(--line)} .verd:last-child{border:0} .verd .lens{font-family:var(--mono);font-size:10.5px;color:var(--faint);width:78px} .verd .ok{margin-left:auto;color:var(--green)}
.lcard{border:1px solid var(--line);border-radius:var(--r);background:var(--panel);padding:13px;margin-top:10px}
.kv{display:flex;align-items:center;gap:9px;font-size:12.5px;color:var(--muted);padding:4px 0} .kv .v{margin-left:auto;color:var(--text-2);font-weight:500} .kv .v.g{color:var(--green)} .kv .v.r{color:var(--red)}
.bp{width:100%;background:var(--accent);color:#15151a;border:0;border-radius:7px;padding:9px;font-weight:600;font-size:13px;cursor:pointer;font-family:var(--font);margin-top:6px}
.bp:disabled{opacity:.45;cursor:not-allowed}
.bc{font-size:12.5px;color:var(--muted);display:inline-flex;align-items:center;gap:8px} .bc a{color:var(--muted);cursor:pointer} .bc a:hover{color:var(--text)} .bc .sep{color:var(--dim)} .bc .num{font-family:var(--mono);color:var(--text-2)}
.ib{width:28px;height:26px;border-radius:6px;border:1px solid var(--line-2);color:var(--muted);background:transparent;cursor:pointer;font-size:13px} .ib:hover{color:var(--text);border-color:var(--line-3)}
.filelist{border:1px solid var(--line);border-radius:var(--r);overflow:hidden}
.filelist .frow{display:flex;align-items:center;gap:10px;padding:8px 12px;border-bottom:1px solid var(--line);font-family:var(--mono);font-size:12px;color:var(--text-2);cursor:pointer} .filelist .frow:last-child{border:0} .filelist .frow:hover{background:var(--panel-2)} .filelist .frow .fc{margin-left:auto} .filelist .frow .a{color:var(--green)} .filelist .frow .d{color:var(--red)}

.legend{display:flex;gap:14px;align-items:center;padding:8px 18px;border-top:1px solid var(--line);font-size:11.5px;color:var(--faint);flex-wrap:wrap} .legend span{display:inline-flex;align-items:center;gap:6px} .legend .cd{width:8px;height:8px;border-radius:50%}
"#;

/// Drawer/accordion behaviour, ported from the mockup (openPR/closePR/openFull/
/// tg) and ADAPTED to server-rendered reality: each card carries its own drawer
/// body in a hidden `<template class="drawer-tpl">`; `openPR(card)` clones that
/// template into the live drawer container instead of copying a couple of fields.
pub const SCREEN_JS: &str = r#"
function openPR(card){
  document.querySelectorAll('.card.sel').forEach(function(c){c.classList.remove('sel')});
  card.classList.add('sel');
  var tpl=card.querySelector('template.drawer-tpl');
  var host=document.getElementById('drawer-body');
  if(tpl&&host){host.innerHTML=tpl.innerHTML}
  var num=card.querySelector('.num');
  var dnum=document.getElementById('d-num');
  if(num&&dnum){dnum.textContent=num.textContent}
  document.getElementById('landing-app').classList.add('open');
}
function closePR(){
  document.getElementById('landing-app').classList.remove('open');
  document.querySelectorAll('.card.sel').forEach(function(c){c.classList.remove('sel')});
}
function tg(el){el.closest('.intent').classList.toggle('open')}
function lmenu(e,id){
  e.stopPropagation();
  var m=document.getElementById(id), open=m.classList.contains('show');
  document.querySelectorAll('.lmenu.show').forEach(function(x){x.classList.remove('show')});
  if(!open) m.classList.add('show');
}
document.addEventListener('click',function(e){ if(!e.target.closest('.ldrop')) document.querySelectorAll('.lmenu.show').forEach(function(x){x.classList.remove('show')}); });
document.querySelectorAll('.fpill').forEach(function(p){ p.addEventListener('click',function(){ document.querySelectorAll('.fpill').forEach(function(x){x.classList.remove('on')}); this.classList.add('on'); }); });
document.querySelectorAll('.vtoggle span').forEach(function(s){ s.addEventListener('click',function(){ document.querySelectorAll('.vtoggle span').forEach(function(x){x.classList.remove('on')}); this.classList.add('on'); }); });
document.addEventListener('keydown',function(e){ if(e.key==='Escape') closePR(); });
"#;

/// The kanban dot/badge class for a column, keyed off its position in the
/// frozen four-column order (Na fila / Testando juntos / Bloqueado / Pousado).
fn column_kind(idx: usize) -> &'static str {
    match idx {
        0 => "queue",
        1 => "test",
        2 => "block",
        _ => "done",
    }
}

pub fn render(vm: &LandingVm) -> Markup {
    html! {
        div .landing-app #landing-app {
            // mainstat strip + filters + view toggle (mockup's .subhead)
            div .subhead {
                div .vtoggle {
                    span .on { "▦ Board" }
                    span { "≣ Lista" }
                }
                div .filters {
                    span .fpill .on { "Abertos " b { (vm.open_count) } }
                    @for col in &vm.columns {
                        span .fpill { (col.title) " " b { (col.card_count()) } }
                    }
                }
                span .sp {}
                span .mainstat { span .dot {} " " (vm.main_status) }
                button .btn .sm .primary style="margin-left:10px" disabled title="read-only — write-path em wave futura" { "Novo PR" }
            }

            // GitHub-faithful filter bar (inert this wave — systemic)
            div .lbar {
                span .lsearch { "⌕ " span .tok { "is:aberto" } input placeholder="filtrar — campanha:auth agente:opus label:segurança …"; }
                span .ldrop onclick="lmenu(event,'lm-camp')" {
                    "campanha: " b { "todas" } " ▾"
                    div .lmenu #lm-camp {
                        div { "Todas" }
                        @for c in &vm.campaigns { div { (c.label) } }
                    }
                }
                span .ldrop onclick="lmenu(event,'lm-grp')" {
                    "agrupar: " b { "campanha" } " ▾"
                    div .lmenu #lm-grp { div { "Campanha" } div { "Estado" } div { "Agente" } div { "Nenhum" } }
                }
                span .ldrop onclick="lmenu(event,'lm-sort')" {
                    "ordenar: " b { "recentes" } " ▾"
                    div .lmenu #lm-sort { div { "Mais recentes" } div { "Mais antigos" } div { "Mais intents" } }
                }
                span .sp {}
                span .lsave { "＋ salvar visão" }
            }

            // stage = board + drawer
            div .stage {
                div .board {
                    @for (idx, col) in vm.columns.iter().enumerate() {
                        (column(&vm.repo, idx, col))
                    }
                }
                (drawer_host())
            }

            // legend (decorative-faithful: static campaign swatches)
            div .legend {
                "campanhas:"
                @for c in &vm.campaigns {
                    span { span .cd style={ "background:var(--" (c.color_class) ")" } {} " " (c.label) }
                }
                span style="margin-left:auto;color:var(--dim)" { "mesma cor = mesmo bundle = testados juntos · clique num card pra abrir →" }
            }

            script { (PreEscaped(SCREEN_JS)) }
        }
    }
}

/// One kanban column: header dot/title/count + its cards and bundles.
fn column(repo: &str, idx: usize, col: &LandingColumnVm) -> Markup {
    let kind = column_kind(idx);
    html! {
        div .column {
            div .ch .(kind) {
                span .d {}
                span .t { (col.title) }
                span .n { (col.card_count()) }
            }
            div .cards {
                @for item in &col.items {
                    @match item {
                        LandingItemVm::Card(card) => (pr_card(repo, card)),
                        LandingItemVm::Bundle { campaign, cards } => {
                            div .bundle style={ "--cc:var(--" (campaign.color_class) ")" } {
                                div .bh {
                                    span .cd {}
                                    span .nm { (campaign.label) }
                                    span .sp {}
                                    span .pct { (cards.len()) " PRs" }
                                }
                                div .bsub { "PRs da mesma campanha → testados como um conjunto" }
                                div .bcards {
                                    @for card in cards { (pr_card(repo, card)) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// One PR card. Binds number/title/author/intent+file counts/checks; carries
/// its own drawer body in a hidden template that `openPR(this)` swaps in.
fn pr_card(repo: &str, card: &PrCardVm) -> Markup {
    let cc = card
        .campaign
        .as_ref()
        .map(|c| format!("--cc:var(--{})", c.color_class));
    let landed = matches!(card.state, PrState::Landed);
    let blocked = matches!(card.state, PrState::Blocked);
    html! {
        div .card style=[cc] onclick="openPR(this)" {
            div .c1 {
                span .num { "#" (card.number) }
                @if let Some(c) = &card.campaign {
                    span .camp { span .cd {} " " (c.label) }
                }
            }
            div .title { (card.title) }
            div .c2 {
                @if landed {
                    span .ok { "● entrou na main" }
                    span .sp {}
                    span .tm { (card.author) }
                } @else {
                    span .av {}
                    " " (card.author) " "
                    span .sp {}
                    span .ints { b { (card.intent_count) } " intents" }
                }
            }
            // red/union-fail affordance from the mockup (blocked state)
            @if blocked {
                div .blk {
                    "⚑ conflitou — voltou ao autor. Você decide."
                    @if !card.drawer.union.green { " (" (card.drawer.union.verdict) ")" }
                }
            }
            // server-rendered per-card drawer body (hidden until clicked)
            template .drawer-tpl {
                (drawer_content(repo, card))
            }
        }
    }
}

/// The live drawer shell: a fixed breadcrumb/close header + an empty body host
/// that `openPR` fills from the clicked card's template.
fn drawer_host() -> Markup {
    html! {
        aside .drawer {
            div .din {
                div .dx {
                    span .bc {
                        a onclick="closePR()" { "Landing" } " " span .sep { "›" } " "
                        span .num #d-num { "PR" }
                    }
                    span .sp {}
                    button .ib title="fechar (esc)" onclick="closePR()" { "✕" }
                }
                div #drawer-body {}
            }
        }
    }
}

/// The per-card drawer BODY (everything below the sticky header): title + state
/// pill + meta, the "better" chips (union/cost/atestados), the intents accordion
/// (charter · context.json · diff · verdicts), the file list, and the lcard
/// (union · checks · custo · espelho · the disabled Land button).
fn drawer_content(repo: &str, card: &PrCardVm) -> Markup {
    let d: &PrDrawerVm = &card.drawer;
    let camp_label = card.campaign.as_ref().map(|c| c.label.clone());
    let total_added: u32 = d.files.iter().map(|f| f.added).sum();
    let total_removed: u32 = d.files.iter().map(|f| f.removed).sum();
    html! {
        div .dh1 { (card.title) }
        span .dpill {
            span .dot {}
            " na fila de landing"
            @if let Some(l) = &camp_label { " · bundle " (l) }
        }
        div .dmeta {
            b { (card.author) } " quer mergear → " span .mono { "main" }
            " · " b { (card.intent_count) " intents" }
            " · " (card.file_count) " arquivos"
            " · " span style="color:var(--green)" { "+" (total_added) } " " span style="color:var(--red)" { "−" (total_removed) }
        }

        div .better {
            div .chip {
                span .i { "●" } " union-test "
                @if d.union.green { b { "verde" } } @else { b { (d.union.verdict) } }
                span .l { "testado junto com a campanha" }
            }
            div .chip {
                span .i style="color:var(--faint)" { "●" } " verificado por "
                b { "$" (format!("{:.2}", d.cost.usd)) }
                span .l { (d.cost.tokens_total) " tokens" }
            }
            div .chip {
                span .i { "●" } " "
                b { (card.intent_count) "/" (card.intent_count) " intents atestados" }
                span .l { "modelo+prompt+prova assinados" }
            }
        }

        div .dsec { "Intents (commits) · " (d.intents.len()) }
        @for (i, intent) in d.intents.iter().enumerate() {
            (intent_block(repo, card, intent, i == 0))
        }

        div .dsec {
            "Arquivos alterados · " (d.files.len())
            " · " span style="color:var(--green)" { "+" (total_added) } " " span style="color:var(--red)" { "−" (total_removed) }
        }
        div .filelist {
            @for f in &d.files {
                div .frow {
                    span { (f.path) }
                    span .fc { b .a { "+" (f.added) } " " b .d { "−" (f.removed) } }
                }
            }
        }

        div .dsec { "Landing" }
        div .lcard {
            div .kv { "union test " span class={ "v" @if d.union.green { " g" } @else { " r" } } { (d.union.verdict) } }
            div .kv { "checks " span .v .g { (card.checks.passed) "/" (card.checks.total) " · " (card.checks.cache_hits) " cache-hit" } }
            div .kv { "custo " span .v { "$" (format!("{:.2}", d.cost.usd)) } }
            div .kv { "espelho " span class={ "v" @if d.mirror.synced { " g" } } { (d.mirror.detail) } }
            button .bp #land-btn disabled title="read-only — write-path em wave futura" { "⇲ Land (entra na main)" }
        }
    }
}

/// One intent inside the drawer: header (id · author) + collapsible body with
/// charter, the context.json code block, the diff, and the verdict panel.
/// `ver intent completo →` is a REAL link to `/r/{repo}/intent/{id}`.
fn intent_block(repo: &str, card: &PrCardVm, intent: &IntentSummaryVm, open: bool) -> Markup {
    html! {
        div .intent .open[open] {
            div .ih onclick="tg(this)" {
                span .ico .t[open] { "◑" }
                div {
                    div .ti { (intent.title) }
                    div .mt { span .id { (intent.id) } " · " b { (card.author) } " · " span .mono { (intent.status) } }
                }
                span .chev { "›" }
            }
            div .ibody {
                div .blk {
                    div .bt { "Charter " span .g { "· o que pretende" } }
                    div .charter { (intent.charter) }
                }
                div .blk {
                    div .bt { "context.json " span .g { "· cada intent vem com o seu (arquivo, não prosa)" } }
                    div .code {
                        div .fn { (intent.id) ".context.json · tenant-private · nunca vira training data" }
                        @for line in intent.context_json.lines() {
                            div .cl { span .ln {} span .ct { (line) } }
                        }
                    }
                    div .acts {
                        a .bg href={ "/r/" (repo) "/intent/" (intent.id) } { "ver intent completo →" }
                        button .bg { "⤓ baixar json" }
                        button .bg { "▸ replay" }
                    }
                }
                (diff_block(&intent.diff))
                @if !intent.verdicts.is_empty() {
                    div .blk {
                        div .bt { "Verdicts " span .g { "· painel adversarial" } }
                        @for v in &intent.verdicts {
                            div .verd {
                                span .lens { (v.reviewer) }
                                " " (v.summary) " "
                                span .ok { (v.verdict) }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Render the diff hunks as the mockup's code block (file header + ±/context
/// lines). Skipped entirely when the intent carries no hunks.
fn diff_block(diff: &DiffVm) -> Markup {
    html! {
        @if !diff.hunks.is_empty() {
            div .blk {
                div .bt { "Diff + why-blame" }
                @for hunk in &diff.hunks {
                    div .code {
                        div .fn { (hunk.file) }
                        @for line in &hunk.lines {
                            @match line.kind {
                                DiffLineKind::Add => div .cl .add { span .ln {} span .ct { (line.text) } },
                                DiffLineKind::Del => div .cl .del { span .ln {} span .ct { (line.text) } },
                                DiffLineKind::Context => div .cl { span .ln {} span .ct { (line.text) } },
                            }
                        }
                    }
                }
            }
        }
    }
}
