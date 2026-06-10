//! Shared page chrome: topbar · tabbar · footer · ⌘K overlay.
//!
//! ⚠️ CONTRACT (wave githugr-spine-w1): screens call [`page`] exactly once and
//! put screen-specific CSS in their own `screen_css` argument — never in
//! `static/kit.css` (the kit is the lead's file). Tabs NAVIGATE (this build
//! fixes the mockups' systemic gap #1); tabs whose screens are not in the
//! spine render dimmed (`rtab dis`) — honest, not dead-looking.

use maud::{DOCTYPE, Markup, PreEscaped, html};

/// Which tab is lit. Spine tabs route; the rest are explicitly disabled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Code,
    Landing,
    Checks,
    Insights,
}

impl Tab {
    fn href(self, repo: &str) -> String {
        match self {
            Tab::Code => format!("/r/{repo}"),
            Tab::Landing => format!("/r/{repo}/landing"),
            Tab::Checks => format!("/r/{repo}/checks"),
            Tab::Insights => format!("/r/{repo}/insights"),
        }
    }
}

/// Full HTML page: kit + chrome + screen body.
///
/// `landing_count` feeds the Landing tab counter (`None` hides it).
/// `fixture` renders the honest fixture-world badge in the topbar.
pub fn page(
    repo: &str,
    active: Tab,
    title: &str,
    landing_count: Option<usize>,
    fixture: bool,
    screen_css: &str,
    body: Markup,
) -> Markup {
    html! {
        (DOCTYPE)
        html lang="pt-BR" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (title) " — githugr" }
                link rel="preconnect" href="https://fonts.googleapis.com";
                link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;450;500;600;700&family=JetBrains+Mono:wght@400;500&display=swap" rel="stylesheet";
                link rel="stylesheet" href="/static/kit.css";
                @if !screen_css.is_empty() {
                    style { (PreEscaped(screen_css)) }
                }
            }
            body {
                (topbar(repo, fixture))
                (tabbar(repo, active, landing_count))
                (body)
                (footer())
                (cmdk_overlay(repo))
                script { (PreEscaped(CMDK_JS)) }
            }
        }
    }
}

fn topbar(repo: &str, fixture: bool) -> Markup {
    html! {
        div .topbar {
            a .brand href={ "/r/" (repo) } {
                span .logo {}
                span .nm { "githugr" }
                span .sep { "/" }
                span .repo { "hugr / " b { (repo) } }
            }
            div .cmdk onclick="ckOpen()" {
                span { "⌘" } " Buscar ou executar… " span .sp {} kbd { "⌘K" }
            }
            @if fixture {
                span .fixture-badge title="dados do mundo-fixture (P2 liga a infra viva)" { "fixture" }
            }
        }
    }
}

fn tabbar(repo: &str, active: Tab, landing_count: Option<usize>) -> Markup {
    let tab = |t: Tab, ic: &str, label: &str, count: Option<usize>| -> Markup {
        let on = t == active;
        html! {
            a .rtab .on[on] href=(t.href(repo)) {
                span .ic { (ic) } " " (label)
                @if let Some(n) = count { " " span .ct { (n) } }
            }
        }
    };
    html! {
        div .tabbar {
            (tab(Tab::Code, "<>", "Code", None))
            span .rtab .dis title="espelhado no GitHub" { span .ic { "○" } " Issues " span .ext { "↗" } }
            (tab(Tab::Landing, "⇲", "Landing (PRs)", landing_count))
            (tab(Tab::Checks, "✓", "Checks", None))
            span .rtab .dis title="em breve" { span .ic { "⛨" } " Security" }
            (tab(Tab::Insights, "▦", "Insights", None))
            span .rtab .dis title="em breve" { span .ic { "⚙" } " Settings" }
        }
    }
}

fn footer() -> Markup {
    html! {
        div .foot {
            span { kbd { "⌘K" } " buscar" }
            span { kbd { "g" } kbd { "l" } " landing" }
            span .sp {}
            span .mono { "githugr — o forge LLM-nativo, git-compatível" }
        }
    }
}

fn cmdk_overlay(repo: &str) -> Markup {
    let item = |href: String, ic: &str, label: &str| -> Markup {
        html! { a .pitem href=(href) { span .ic { (ic) } (label) span .sp {} } }
    };
    html! {
        div .kov #kov onclick="if(event.target===this)ckClose()" {
            div .pal {
                input #kq placeholder="Ir para…" autocomplete="off";
                (item(format!("/r/{repo}"), "<>", "Code — repo home"))
                (item(format!("/r/{repo}/landing"), "⇲", "Landing (PRs)"))
                (item(format!("/r/{repo}/checks"), "✓", "Checks — CI memoizado"))
                (item(format!("/r/{repo}/insights"), "▦", "Insights · Ledger"))
            }
        }
    }
}

const CMDK_JS: &str = r#"
function ckOpen(){var k=document.getElementById('kov');k.classList.add('open');document.getElementById('kq').focus()}
function ckClose(){document.getElementById('kov').classList.remove('open')}
document.addEventListener('keydown',function(e){
  if((e.metaKey||e.ctrlKey)&&e.key==='k'){e.preventDefault();ckOpen()}
  if(e.key==='Escape')ckClose()
});
"#;
