//! Intent detail — the record of intent (hugit's killer concept: intent ≠
//! commit). Spec: ../githugr/design/intent.html — transcribed, not redesigned.
//!
//! Renders exactly one frozen [`IntentDetailVm`]. The honesty rule is
//! load-bearing: a `None` transcript/journal renders the explicit pt-BR
//! "não capturado" state (capture level / WP-F2 pending), never a silent blank
//! (whitepaper lock). The context.json ⤓/▸ buttons render as in the mockup but
//! disabled with honest titles — replay/signed-export land in a future wave.

use crate::provider::{DiffLineKind, IntentDetailVm, VerdictVm};
use maud::{Markup, PreEscaped, html};

/// Screen-specific CSS. Ports the mockup's own `<style>` block plus the shared
/// idiom classes the intent screen references (`.crumb` · `.sec` · `.pill` ·
/// the `.code`/diff family · `.kv` · `.btn`), all scoped under `.intent-screen`
/// — the kit shell (tokens/topbar/brand/cmdk/tabbar/footer/⌘K) is excluded.
pub const SCREEN_CSS: &str = r#"
.intent-screen{max-width:1080px;margin:0 auto;padding:18px 24px 60px}

/* breadcrumb + section labels (family idiom) */
.intent-screen .crumb{color:var(--faint);font-size:12.5px;margin-bottom:8px}
.intent-screen .crumb a{color:var(--muted)}
.intent-screen .crumb a:hover{color:var(--text)}
.intent-screen .crumb .mono{color:var(--faint)}
.intent-screen .sec{font-size:11px;letter-spacing:.05em;text-transform:uppercase;color:var(--faint);font-weight:600;margin:26px 0 11px;display:flex;align-items:center;gap:9px}
.intent-screen .sec .g{text-transform:none;letter-spacing:0;font-weight:400;color:var(--dim)}

/* status pill (family idiom; .green/.g add the landed coloring the names imply) */
.intent-screen .pill{display:inline-flex;align-items:center;gap:6px;font-size:12px;font-weight:500;padding:3px 11px;border-radius:20px}
.intent-screen .pill .dot{width:7px;height:7px;border-radius:50%;background:var(--faint)}
.intent-screen .pill.green{color:var(--green);background:var(--green-soft);border:1px solid rgba(90,158,120,.28)}
.intent-screen .pill.green .dot.g{background:var(--green)}

/* intent header (mirrors pr-detail) */
.intent-screen .ihdr .t1{display:flex;align-items:center;gap:11px;flex-wrap:wrap}
.intent-screen .ihdr .num{color:var(--faint);font-weight:400;font-size:21px;font-family:var(--mono)}
.intent-screen .ihdr h1{font-size:21px;font-weight:600;letter-spacing:-.02em}
.intent-screen .ihdr .t2{margin-top:9px;color:var(--muted);font-size:13px}
.intent-screen .ihdr .t2 b{color:var(--text-2)}
.intent-screen .ihdr .t2 a{color:var(--muted);border-bottom:1px solid var(--line-2)}
.intent-screen .ihdr .t2 a:hover{color:var(--text)}

.intent-screen .layout{display:grid;grid-template-columns:1fr 296px;gap:26px;margin-top:22px;align-items:start}
@media(max-width:880px){.intent-screen .layout{grid-template-columns:1fr}}

/* main column */
.intent-screen .lead-sum{font-size:14px;color:var(--text-2);line-height:1.6;max-width:72ch}
.intent-screen .charter-q{border-left:2px solid var(--line-3);padding-left:14px;color:var(--text-2);font-size:13.5px;line-height:1.6;max-width:72ch}
.intent-screen .charter-q .faint{color:var(--faint)}

/* trajectory accordion */
.intent-screen .acc{border:1px solid var(--line);border-radius:var(--r);background:var(--panel);margin-bottom:9px;overflow:hidden}
.intent-screen .acc .ah{display:grid;grid-template-columns:1fr auto;align-items:center;gap:12px;padding:12px 14px;cursor:pointer}
.intent-screen .acc .ah:hover{background:var(--panel-2)}
.intent-screen .acc .ah .ti{font-size:13.5px;font-weight:500;color:var(--text)}
.intent-screen .acc .ah .mt{font-size:11.5px;color:var(--faint);margin-top:2px}
.intent-screen .acc .ah .chev{color:var(--faint);transition:.15s}
.intent-screen .acc.open .ah .chev{transform:rotate(90deg)}
.intent-screen .acc .ab{display:none;border-top:1px solid var(--line);padding:13px 14px}
.intent-screen .acc.open .ab{display:block}
.intent-screen .turn{border-left:2px solid var(--line-2);padding:0 0 11px 13px;margin-left:2px;font-size:12.5px;color:var(--text-2)}
.intent-screen .turn:last-child{padding-bottom:0}
.intent-screen .turn .role{font-size:10px;letter-spacing:.05em;text-transform:uppercase;color:var(--faint);font-weight:600;margin-bottom:3px}
.intent-screen .turn .tool{font-family:var(--mono);font-size:11.5px;color:var(--muted)}
.intent-screen .turn .res{color:var(--faint)}
.intent-screen .notcap{font-size:12.5px;color:var(--faint);line-height:1.6;font-style:italic}
.intent-screen .redbadge{font-size:10px;color:var(--amber);border:1px solid rgba(184,154,90,.3);background:rgba(184,154,90,.08);border-radius:5px;padding:1px 6px;font-weight:500;margin-left:7px}
.intent-screen .priv{font-size:11px;color:var(--faint);display:inline-flex;align-items:center;gap:7px;margin-top:11px}
.intent-screen .priv .dot{width:5px;height:5px;border-radius:50%;background:var(--faint)}

/* diff + why-blame (family code idiom) */
.intent-screen .code{border:1px solid var(--line);border-radius:var(--r);overflow:hidden;background:#0d0d10;font-family:var(--mono);font-size:12.5px}
.intent-screen .code .fn{padding:7px 14px;border-bottom:1px solid var(--line);background:var(--rail);color:var(--muted);font-size:11.5px}
.intent-screen .cl{display:grid;grid-template-columns:40px 1fr}
.intent-screen .cl .ln{color:var(--dim);text-align:right;padding:2px 10px 2px 0;user-select:none}
.intent-screen .cl .ct{padding:2px 14px;white-space:pre;color:var(--text-2)}
.intent-screen .cl.add{background:rgba(90,158,120,.08)}
.intent-screen .cl.add .ct{color:#bfe6cf}
.intent-screen .cl.del{background:rgba(207,112,112,.08)}
.intent-screen .cl.del .ct{color:#e9b6b6}
.intent-screen .cl.hl{background:rgba(255,255,255,.1);border-left:2px solid var(--accent)}
.intent-screen .cl.hl .ln{padding-left:0}
.intent-screen .cm{color:var(--faint)}

/* right rail */
.intent-screen .rail .card{border:1px solid var(--line);border-radius:var(--r);background:var(--panel);padding:13px;margin-bottom:12px}
.intent-screen .rail .card .h{font-size:11px;letter-spacing:.05em;text-transform:uppercase;color:var(--faint);font-weight:600;margin-bottom:9px}
.intent-screen .rail .acts{display:flex;flex-direction:column;gap:7px}
.intent-screen .rail .acts .btn{width:100%;justify-content:flex-start}
.intent-screen .kv{display:flex;align-items:center;gap:9px;font-size:12.5px;color:var(--muted);padding:4px 0}
.intent-screen .kv .v{margin-left:auto;color:var(--text-2);font-weight:500}
.intent-screen .kv .v.g{color:var(--green)}
.intent-screen .lifenote{font-size:11.5px;color:var(--faint);line-height:1.5;margin-bottom:11px;padding-bottom:11px;border-bottom:1px solid var(--line)}
.intent-screen .files-chips{display:flex;flex-wrap:wrap;gap:5px;margin:6px 0 4px}
.intent-screen .files-chips .f{font-family:var(--mono);font-size:11px;color:var(--text-2);background:rgba(255,255,255,.04);border:1px solid var(--line);border-radius:5px;padding:1px 7px}
.intent-screen .verd{display:flex;align-items:center;gap:9px;font-size:12px;color:var(--muted);padding:5px 0;border-bottom:1px solid var(--line)}
.intent-screen .verd:last-child{border:0}
.intent-screen .verd .lens{font-family:var(--mono);font-size:10px;color:var(--faint);width:72px}
.intent-screen .verd .sp{flex:1}
.intent-screen .verd .ok{color:var(--green);font-weight:500;font-size:11px}
.intent-screen .verd .advm{font-size:9px;letter-spacing:.04em;text-transform:uppercase;color:var(--faint);border:1px solid var(--line-2);border-radius:5px;padding:0 5px}
.intent-screen .cap{font-size:11px;color:var(--faint);margin-top:9px}
.intent-screen .cap b{color:var(--muted)}

/* buttons (family idiom; disabled = honest, not dead) */
.intent-screen .btn{display:inline-flex;align-items:center;gap:7px;background:transparent;color:var(--text);border:1px solid var(--line-2);border-radius:7px;padding:6px 12px;font-size:12.5px;font-weight:500;cursor:pointer;font-family:var(--font)}
.intent-screen .btn:hover{border-color:var(--line-3)}
.intent-screen .btn.sm{padding:4px 10px;font-size:11.5px}
.intent-screen .btn:disabled{color:var(--dim);cursor:not-allowed;opacity:.7}
.intent-screen .btn:disabled:hover{border-color:var(--line-2)}
"#;

/// The accordion toggle, ported verbatim from the mockup (`tg(this)`).
const SCREEN_JS: &str = r#"function tg(el){el.parentElement.classList.toggle('open')}"#;

/// Honest pt-BR state for an accordion whose body was not captured for this
/// intent (capture level / WP-F2 pending). Never a silent blank.
const NOT_CAPTURED: &str = "não capturado neste intent (nível de captura / WP-F2 pendente)";

pub fn render(vm: &IntentDetailVm) -> Markup {
    let landing = format!("/r/{}/landing", vm.repo);
    html! {
        div .body {
            div .intent-screen {

                // breadcrumb: Landing › PR #N › intent id (Landing+PR → landing).
                div .crumb {
                    a href=(landing) { "Landing" }
                    @if let Some(n) = vm.pr_number {
                        " › " a href=(landing) { "PR #" (n) }
                    }
                    " › " span .mono { "intent " (vm.id) }
                }

                // intent header: id · title · status badge · meta.
                div .ihdr {
                    div .t1 {
                        span .num { (vm.id) }
                        h1 { (vm.title) }
                        span .pill .green { span .dot .g {} " " (vm.status) }
                    }
                    div .t2 {
                        "intent (commit) " span .mono { (vm.id) }
                        @if let Some(n) = vm.pr_number {
                            " · dentro do " a href=(landing) { "PR #" (n) }
                        }
                    }
                }

                div .layout {

                    // MAIN — the narrative.
                    div .main {

                        div .sec { "Resumo" }
                        div .lead-sum { (vm.summary) }

                        div .sec { "Charter " span .g { "· o que pretendia + aceite" } }
                        div .charter-q {
                            (vm.charter)
                            @if !vm.acceptance.is_empty() {
                                br;
                                @for (i, item) in vm.acceptance.iter().enumerate() {
                                    @if i > 0 { " · " }
                                    span .faint { "aceite:" } " " (item)
                                }
                            }
                        }

                        div .sec {
                            "Trajetória " span .g { "· o transcript do agente, em 3 altitudes" }
                        }

                        // Transcript da task (task-level).
                        (accordion(
                            "Transcript da task",
                            "brief → passos → resultado · nível-tarefa",
                            false,
                            vm.task_transcript.as_deref(),
                        ))

                        // Transcript completo (the full loop, born→died · replayable).
                        (accordion(
                            "Transcript completo",
                            "o loop inteiro, nasce→morre: cada turn + tool call + resultado · replayável",
                            true,
                            vm.full_transcript.as_deref(),
                        ))

                        // Journal (human annotation trail).
                        (accordion(
                            "Journal",
                            "trilha de anotação humana",
                            false,
                            vm.journal.as_deref(),
                        ))

                        // context.json — the content-addressed file (always present).
                        div .acc {
                            div .ah onclick="tg(this)" {
                                div {
                                    div .ti { "context.json" }
                                    div .mt { "o arquivo content-addressed · v1.0.0 · não prosa" }
                                }
                                span .chev { "›" }
                            }
                            div .ab {
                                div .code {
                                    div .fn { (vm.id) ".context.json · content-addressed · tenant-private" }
                                    @for line in vm.context_json.lines() {
                                        div .cl { span .ln {} span .ct { (line) } }
                                    }
                                }
                                div style="margin-top:9px;display:flex;gap:7px" {
                                    button .btn .sm disabled
                                        title="disponível quando o envelope ADR-0001 estiver capturado" {
                                        "⤓ baixar context.json"
                                    }
                                    button .btn .sm disabled
                                        title="replay em wave futura" {
                                        "▸ replay"
                                    }
                                }
                            }
                        }

                        // Diff + why-blame.
                        div .sec { "Diff + why-blame" }
                        @for file in &vm.diff.files {
                            div .code {
                                div .fn {
                                    (file.path) " · +" (file.added) " −" (file.removed)
                                }
                                @for hunk in vm.diff.hunks.iter().filter(|h| h.file == file.path) {
                                    @if !hunk.header.is_empty() {
                                        div .cl { span .ln {} span .ct { (hunk.header) } }
                                    }
                                    @for line in &hunk.lines {
                                        (diff_line(line.kind, &line.text))
                                    }
                                }
                            }
                        }
                    }

                    // RAIL — the facts.
                    div .rail {

                        div .card {
                            div .acts {
                                @if let Some(n) = vm.pr_number {
                                    a .btn href=(landing) { "⤴ ver no PR #" (n) }
                                }
                                button .btn disabled
                                    title="disponível quando o envelope ADR-0001 estiver capturado" {
                                    "⤓ context.json"
                                }
                                button .btn disabled title="replay em wave futura" {
                                    "▸ replay da sessão"
                                }
                            }
                        }

                        div .card {
                            div .h { "Autoria" }
                            div .lifenote {
                                "Escrito por um agente que "
                                b style="color:var(--muted)" { "nasceu e morreu" }
                                " neste intent — este é o registro durável do que ele fez."
                            }
                            div .kv { "modelo " span .v { (vm.authorship.model) } }
                            div .kv {
                                "cadeia "
                                span .v .mono {
                                    @for (i, p) in vm.authorship.principal_chain.iter().enumerate() {
                                        @if i > 0 { " ← " }
                                        (p)
                                    }
                                }
                            }
                            div .kv { "operador " span .v { (vm.authorship.operator) } }
                        }

                        div .card {
                            div .h { "Métricas" }
                            div .kv { "tokens " span .v { (vm.metrics.tokens) } }
                            div .kv { "tempo de relógio " span .v { (vm.metrics.wall_ms) " ms" } }
                            div .kv { "tool calls " span .v { (vm.metrics.tool_calls) } }
                            div .kv { "custo (COGS) " span .v { "$" (format_usd(vm.metrics.cost_usd)) } }
                            div .cap {
                                "custo é "
                                b { "transparência, não fatura" }
                                " — você nunca é cobrado por isto"
                            }
                        }

                        div .card {
                            div .h { "Snapshot" }
                            div .kv {
                                "tree "
                                span .v .mono title=(vm.snapshot.tree) { (truncate(&vm.snapshot.tree)) }
                            }
                            div .kv {
                                "toolchain "
                                span .v .mono title=(vm.snapshot.toolchain) { (truncate(&vm.snapshot.toolchain)) }
                            }
                            div .kv {
                                "workspace "
                                span .v .mono title=(vm.snapshot.workspace) { (truncate(&vm.snapshot.workspace)) }
                            }
                        }

                        div .card {
                            div .h {
                                "Verdicts "
                                span style="color:var(--dim);text-transform:none;letter-spacing:0;font-weight:400" {
                                    "· painel adversarial"
                                }
                            }
                            @for v in &vm.verdicts {
                                (verdict_row(v))
                            }
                        }
                    }
                }
            }
        }
        script { (PreEscaped(SCREEN_JS)) }
    }
}

/// One trajectory accordion. `redacted` adds the mockup's amber badge; a `None`
/// body renders the honest "não capturado" state, never a blank.
fn accordion(title: &str, meta: &str, redacted: bool, body: Option<&str>) -> Markup {
    html! {
        div .acc {
            div .ah onclick="tg(this)" {
                div {
                    div .ti {
                        (title)
                        @if redacted { " " span .redbadge { "redactado" } }
                    }
                    div .mt { (meta) }
                }
                span .chev { "›" }
            }
            div .ab {
                @match body {
                    Some(text) => div .turn { (text) },
                    None => div .notcap { (NOT_CAPTURED) },
                }
            }
        }
    }
}

/// One diff line, carrying its add/del/context class (and `hl` on adds, as the
/// mockup highlights the introduced line).
fn diff_line(kind: DiffLineKind, text: &str) -> Markup {
    match kind {
        DiffLineKind::Add => html! {
            div .cl .add .hl { span .ln {} span .ct { (text) } }
        },
        DiffLineKind::Del => html! {
            div .cl .del { span .ln {} span .ct { (text) } }
        },
        DiffLineKind::Context => html! {
            div .cl { span .ln {} span .ct { (text) } }
        },
    }
}

/// One verdict row; adversarial verdicts are marked (as in the mockup).
fn verdict_row(v: &VerdictVm) -> Markup {
    html! {
        div .verd {
            span .lens { (v.reviewer) }
            span .sp {}
            @if v.adversarial { span .advm title=(v.summary) { "adversarial" } }
            span .ok { (v.verdict) }
        }
    }
}

/// Truncate a long hash for the snapshot rail (full value goes in `title=`).
fn truncate(s: &str) -> String {
    if s.len() > 12 {
        format!("{}…", &s[..11])
    } else {
        s.to_string()
    }
}

/// Render a COGS figure to cents, matching the mockup's `$0.04` form.
fn format_usd(usd: f64) -> String {
    format!("{usd:.2}")
}
