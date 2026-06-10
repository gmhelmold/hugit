//! Checks — memoized CI. Spec: ../githugr/design/checks.html
//!
//! ⚠️ WP-W5 STUB: replaced render-faithfully by the W5 agent (cibar KPIs,
//! check rows with expandable logs, bisect walkthrough, memo note).

use crate::provider::ChecksVm;
use maud::{Markup, html};

/// Screen-specific CSS (W5 ports it from the mockup's `<style>` block).
pub const SCREEN_CSS: &str = "";

pub fn render(vm: &ChecksVm) -> Markup {
    html! {
        main style="padding:24px" {
            h1 { "Checks — " (vm.repo) }
            p { "hit-rate " (format!("{:.1}", vm.kpis.hit_rate_pct)) "% · " (vm.kpis.shape) }
            ul {
                @for c in &vm.checks {
                    li { (c.name) " — " @if c.cache_hit { "cache" } @else { "exec" } }
                }
            }
        }
    }
}
