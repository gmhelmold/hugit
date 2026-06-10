//! Checks — memoized CI. Spec: `../githugr/design/checks.html`
//!
//! Renders the cibar summary strip (hit-rate / cache-shape / hits / executed /
//! saved KPIs), the check list with expandable log panes, the optional bisect
//! walkthrough block, and the memo note.
//!
//! HONESTY RULE: `hit_rate_pct` and `shape` are displayed AS-IS from the VM
//! (measured, never promised). No rounding up, no "100%" hardcode.

use crate::provider::{BisectVm, CheckRowVm, ChecksKpisVm, ChecksVm};
use maud::{Markup, PreEscaped, html};

// ---------------------------------------------------------------------------
// Screen CSS (ported from checks.html <style>; scoped under .checks-screen;
// kit rules excluded: tokens/topbar/brand/cmdk/tabbar/rtab/foot/⌘K overlay)
// ---------------------------------------------------------------------------

pub const SCREEN_CSS: &str = r#"
/* checks screen root */
.checks-screen .body{overflow-y:auto}

/* summary bar */
.checks-screen .cibar{display:flex;align-items:center;padding:13px 18px;border-bottom:1px solid var(--line);background:var(--panel);flex-wrap:wrap;gap:10px}
.checks-screen .cibar .left{display:flex;flex-direction:column;gap:3px}
.checks-screen .cibar .ctx{font-size:13.5px;font-weight:600;color:var(--text)}
.checks-screen .cibar .ctx .mono{color:var(--muted);font-size:12.5px;font-weight:400}
.checks-screen .cibar .camp{font-size:12px;color:var(--faint)}
.checks-screen .cibar .camp span{color:var(--muted)}
.checks-screen .cibar .sp{flex:1}
.checks-screen .kpis{display:flex;align-items:center;gap:6px;flex-wrap:wrap}
.checks-screen .kpi{display:inline-flex;align-items:center;gap:6px;font-size:12px;padding:5px 11px;border-radius:7px;border:1px solid var(--line-2);background:rgba(255,255,255,.02);color:var(--muted);white-space:nowrap}
.checks-screen .kpi .v{font-family:var(--mono);font-weight:600;color:var(--text-2)}
.checks-screen .kpi.hit .v{color:var(--green)}
.checks-screen .kpi.save .v{color:var(--muted)}

/* dots cluster */
.checks-screen .dots{display:flex;gap:3px;align-items:center}
.checks-screen .dots .d{width:5px;height:5px;border-radius:50%}

/* checks list */
.checks-screen .chklist{border:1px solid var(--line);border-radius:var(--r);overflow:hidden}
.checks-screen .chkrow{display:flex;align-items:center;gap:12px;padding:10px 14px;border-bottom:1px solid var(--line);cursor:pointer}
.checks-screen .chkrow:last-child{border-bottom:0}
.checks-screen .chkrow:hover{background:var(--hover)}
.checks-screen .chkrow.on{background:rgba(255,255,255,.045)}
.checks-screen .chkrow .st{flex:0 0 auto;display:flex;align-items:center;gap:6px;font-size:11.5px;font-weight:500;width:130px}
.checks-screen .chkrow .st.hit{color:var(--green)}
.checks-screen .chkrow .st.ran{color:var(--text-2)}
.checks-screen .chkrow .st.aff{color:var(--amber)}
.checks-screen .chkrow .st.fail{color:var(--red)}
.checks-screen .chkrow .nm{font-family:var(--mono);font-size:12.5px;font-weight:500;color:var(--text-2);flex:0 0 210px}
.checks-screen .chkrow .desc{font-size:12px;color:var(--faint);flex:1}
.checks-screen .chkrow .tm{font-family:var(--mono);font-size:11.5px;color:var(--dim);width:52px;text-align:right}
/* re-run affordance */
.checks-screen .rerun-btn{opacity:0;font-size:11px;color:var(--faint);border:1px solid var(--line);border-radius:5px;padding:1px 7px;margin-left:6px;cursor:pointer;background:transparent;font-family:var(--font);transition:opacity .12s,border-color .12s,color .12s}
.checks-screen .chkrow:hover .rerun-btn{opacity:1}
.checks-screen .rerun-btn:hover{border-color:var(--line-3);color:var(--muted)}

/* expanded log area */
.checks-screen .logwrap{border-bottom:1px solid var(--line);display:none}
.checks-screen .logwrap.open{display:block}
.checks-screen .logwrap .code{border-radius:0;border:0;border-top:1px solid var(--line)}
.checks-screen .logwrap .code .fn{font-size:11px;padding:6px 14px;display:flex;align-items:center;gap:10px}
.checks-screen .logwrap .code .fn .sp{flex:1}
.checks-screen .logwrap .code .fn .badge{font-size:10.5px;color:var(--green);background:rgba(76,183,130,.1);border:1px solid rgba(76,183,130,.25);border-radius:5px;padding:1px 7px}
.checks-screen .logwrap .code .fn .badge.fail{color:var(--red);background:rgba(239,68,68,.1);border-color:rgba(239,68,68,.25)}
.checks-screen .logwrap .code .fn .memo-key{font-family:var(--mono);font-size:10.5px;color:var(--dim)}
.checks-screen .cl{display:grid;grid-template-columns:38px 1fr}
.checks-screen .cl .ln{color:var(--dim);text-align:right;padding:2px 10px 2px 0;user-select:none;font-size:12px}
.checks-screen .cl .ct{padding:2px 14px;white-space:pre;color:var(--text-2);font-size:12px}
.checks-screen .cl.ok .ct{color:var(--green)}
.checks-screen .cl.hi .ct{color:var(--text)}

/* bisect block */
.checks-screen .bisect{border:1px solid var(--line);border-radius:var(--r);background:var(--panel);padding:14px 16px;margin-top:2px}
.checks-screen .bisect .bh{display:flex;align-items:center;gap:9px;margin-bottom:10px}
.checks-screen .bisect .bh .label{font-size:12px;font-weight:600;color:var(--text-2)}
.checks-screen .bisect .bh .ex{font-size:12px;color:var(--faint)}
.checks-screen .bisect .bh .sp{flex:1}
.checks-screen .bisect .bh .badge{font-size:11px;color:var(--muted);background:rgba(255,255,255,.04);border:1px solid var(--line);border-radius:5px;padding:2px 8px}
.checks-screen .bisect .bsteps{display:flex;flex-direction:column;gap:5px}
.checks-screen .bisect .bstep{display:flex;align-items:center;gap:10px;font-size:12px;padding:5px 0;border-bottom:1px solid var(--line)}
.checks-screen .bisect .bstep:last-child{border-bottom:0}
.checks-screen .bisect .bstep .n{font-family:var(--mono);font-size:11px;color:var(--dim);width:18px}
.checks-screen .bisect .bstep .commit{font-family:var(--mono);font-size:11.5px;color:var(--muted)}
.checks-screen .bisect .bstep .res{margin-left:auto;font-size:11.5px}
.checks-screen .bisect .bstep .res.ok{color:var(--green)}
.checks-screen .bisect .bstep .res.bad{color:var(--red)}
.checks-screen .bisect .bstep .res.culp{color:var(--amber);font-weight:600}
.checks-screen .bisect .note{font-size:11.5px;color:var(--faint);margin-top:10px;line-height:1.6}
.checks-screen .bisect .note b{color:var(--muted)}
.checks-screen .bisect .culprit-row{padding:10px 0 4px;border-top:1px solid var(--line);margin-top:8px}
.checks-screen .bisect .culprit-row .cv{font-family:var(--mono);font-size:12px;color:var(--amber);font-weight:600}
@keyframes bstep-in{from{opacity:0;transform:translateY(4px)}to{opacity:1;transform:none}}
.checks-screen .bisect .bstep{animation:bstep-in .25s ease forwards}

/* memo note */
.checks-screen .memonote{display:flex;align-items:flex-start;gap:10px;font-size:12px;color:var(--faint);padding:12px 16px;border:1px solid var(--line);border-radius:var(--r);background:var(--panel);line-height:1.6}
.checks-screen .memonote .ic{font-size:14px;flex:0 0 auto;margin-top:1px}
.checks-screen .memonote b{color:var(--muted)}
"#;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Humanize milliseconds as "Xs" or "Xm Ys" matching the mockup.
fn humanize_ms(ms: u64) -> String {
    if ms < 60_000 {
        format!("{}s", ms / 1000)
    } else {
        let secs = ms / 1000;
        format!("{}m {}s", secs / 60, secs % 60)
    }
}

/// Format a check row duration for display.
fn fmt_duration(ms: u64) -> String {
    if ms == 0 {
        "0ms".to_string()
    } else if ms < 1000 {
        format!("{}ms", ms)
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

// ---------------------------------------------------------------------------
// Sub-renderers
// ---------------------------------------------------------------------------

fn render_kpis(kpis: &ChecksKpisVm) -> Markup {
    // hit_rate_pct displayed AS-IS: one decimal, never rounded up, never hardcoded
    let hit_rate = format!("{:.1}", kpis.hit_rate_pct);
    let saved = humanize_ms(kpis.saved_ms);
    let total = kpis.hits + kpis.executed;
    let hits_of_total = format!("{} de {}", kpis.hits, total);
    let executed_label = format!("{} execuções", kpis.executed);

    html! {
        div .kpis {
            // shape label AS-IS from the VM
            div .kpi.hit {
                span { "hit-rate" }
                span .v { (hit_rate) "% " (kpis.shape) }
            }
            div .kpi {
                span { "do cache" }
                span .v { (hits_of_total) }
            }
            div .kpi {
                span { "executou" }
                span .v { (executed_label) }
            }
            div .kpi.save {
                span { "economizou" }
                span .v { (saved) }
            }
            @if total > 0 {
                div .dots style="margin-left:4px" {
                    @for i in 0..total {
                        @if i < kpis.hits {
                            span .d style="background:var(--green)" {}
                        } @else {
                            span .d style="background:var(--dim)" {}
                        }
                    }
                }
            }
        }
    }
}

fn render_check_row(row: &CheckRowVm, idx: usize) -> Markup {
    let log_id = format!("logwrap-{idx}");
    let row_id = format!("chkrow-{idx}");
    let duration = fmt_duration(row.duration_ms);

    // Status class and label — failure takes priority over cache_hit
    let (st_class, dot_style, st_label) = if !row.ok {
        ("fail", "background:var(--red)", "falhou")
    } else if row.cache_hit {
        ("hit", "background:var(--green)", "cache-hit")
    } else {
        ("ran", "background:var(--muted)", "executou")
    };

    // Memo key: truncated (first 16 chars + ellipsis) for display; full in title
    let memo_key_display = if row.memo_key.len() > 20 {
        format!("{}…", &row.memo_key[..16])
    } else {
        row.memo_key.clone()
    };
    let memo_key_full = row.memo_key.clone();

    // Log pane badge text and class
    let (badge_class, badge_text) = if !row.ok {
        ("badge fail", format!("falhou · {duration}"))
    } else if row.cache_hit {
        ("badge", format!("cache-hit · {duration}"))
    } else {
        ("badge", format!("passou · {duration}"))
    };

    html! {
        // Check row — click toggles log pane
        div .chkrow id=(row_id) onclick=(format!("toggleLog('{log_id}',this)")) {
            div .st.(st_class) {
                span .dot style=(dot_style) {}
                " " (st_label)
            }
            div .nm { (row.name) }
            div .desc {}
            button .rerun-btn onclick="event.stopPropagation()" { "⟳ re-run" }
            div .tm { (duration) }
        }
        // Expandable log pane (hidden by default, toggled by JS)
        div .logwrap id=(log_id) {
            div .code {
                div .fn {
                    span style="font-family:var(--mono);font-size:11px" { (row.name) }
                    span .sp {}
                    span class=(badge_class) { (badge_text) }
                    span .memo-key title=(memo_key_full) { (memo_key_display) }
                }
                @for (i, line) in row.log.lines().enumerate() {
                    div .cl {
                        span .ln { (i + 1) }
                        span .ct { (line) }
                    }
                }
            }
        }
    }
}

fn render_bisect(bisect: &BisectVm) -> Markup {
    html! {
        div .sec style="margin-top:22px" {
            "Auto-bisect "
            span .g { "· só entra em ação quando um check falha · log₂ execuções máximo" }
        }
        div .bisect {
            div .bh {
                div .label {
                    "Culpado isolado: "
                    span .mono style="font-size:11.5px;color:var(--amber)" { (bisect.culprit) }
                }
                div .ex { "— isolado em " (bisect.probes) " execuções" }
                span .sp {}
                div .badge { "≤ log₂(" (bisect.max_probes) ") execuções" }
            }
            div .bsteps {
                @for (i, step) in bisect.steps.iter().enumerate() {
                    div .bstep {
                        span .n { (i + 1) }
                        span style="font-size:12px;color:var(--faint);flex:1" { (step) }
                    }
                }
            }
            div .culprit-row {
                span style="font-size:12px;color:var(--faint)" { "culpado:" }
                span .cv { " " (bisect.culprit) }
            }
            div .note {
                "isolado em " b { (bisect.probes) }
                " execuções (≤ log₂ de " (bisect.max_probes) " sondagens máximas)."
            }
        }
    }
}

// ---------------------------------------------------------------------------
// toggleLog JS — ported from checks.html
// ---------------------------------------------------------------------------

const TOGGLE_LOG_JS: &str = r#"
function toggleLog(logId, row) {
  var wrap = document.getElementById(logId);
  var isOpen = wrap.classList.contains('open');
  document.querySelectorAll('.logwrap.open').forEach(function(el) { el.classList.remove('open'); });
  document.querySelectorAll('.chkrow.on').forEach(function(r) { r.classList.remove('on'); });
  if (!isOpen) {
    wrap.classList.add('open');
    row.classList.add('on');
  }
}
"#;

// ---------------------------------------------------------------------------
// Public render
// ---------------------------------------------------------------------------

pub fn render(vm: &ChecksVm) -> Markup {
    let total = vm.kpis.hits + vm.kpis.executed;
    let section_g = format!(
        "· {} total · {} cache-hit · {} executaram",
        total, vm.kpis.hits, vm.kpis.executed
    );

    html! {
        div .checks-screen {
            // SUMMARY BAR — cibar strip with KPIs
            div .cibar {
                div .left {
                    div .ctx {
                        "CI do repositório " span .mono { (vm.repo) }
                    }
                }
                span .sp {}
                (render_kpis(&vm.kpis))
            }

            // BODY
            div .body {
                div .wrap {

                    // Section header
                    div .sec {
                        "Checks do workspace "
                        span .g { (section_g) }
                        span .sp {}
                        span .g style="font-size:11.5px;color:var(--faint)" {
                            "byte-idêntico — mesmo input ⇒ mesmo resultado (a base da memoização)"
                        }
                    }

                    // CHECKS LIST
                    div .chklist {
                        @for (i, row) in vm.checks.iter().enumerate() {
                            (render_check_row(row, i))
                        }
                    }

                    // BISECT BLOCK — rendered only when vm.bisect is Some
                    @if let Some(bisect) = &vm.bisect {
                        (render_bisect(bisect))
                    }

                    // MEMO NOTE
                    div .memonote style="margin-top:14px" {
                        span .ic { "⊙" }
                        span { (vm.memo_note) }
                    }

                }
            }

            // Screen JS (toggleLog)
            script { (PreEscaped(TOGGLE_LOG_JS)) }
        }
    }
}
