//! Landing (PRs) — the hero screen. Spec: ../githugr/design/landing.html
//!
//! ⚠️ WP-W2 STUB: replaced render-faithfully by the W2 agent (kanban columns,
//! campaign bundles, PR drawer with intents/charter/context/diff/verdicts).

use crate::provider::LandingVm;
use maud::{Markup, html};

/// Screen-specific CSS (W2 ports it from the mockup's `<style>` block).
pub const SCREEN_CSS: &str = "";

pub fn render(vm: &LandingVm) -> Markup {
    html! {
        main style="padding:24px" {
            h1 { "Landing (PRs) — " (vm.repo) }
            p { (vm.main_status) }
            ul {
                @for col in &vm.columns {
                    li { (col.title) " · " (col.card_count()) }
                }
            }
        }
    }
}
