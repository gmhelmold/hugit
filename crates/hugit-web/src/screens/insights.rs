//! Insights + the Ledger view. Spec: ../githugr/design/insights.html
//!
//! ⚠️ WP-W6 STUB: replaced render-faithfully by the W6 agent (KPI cards,
//! landed bars, token-by-campaign, Cost X-ray, Ledger asked→done→proven).

use crate::provider::InsightsVm;
use maud::{Markup, html};

/// Screen-specific CSS (W6 ports it from the mockup's `<style>` block).
pub const SCREEN_CSS: &str = "";

pub fn render(vm: &InsightsVm) -> Markup {
    html! {
        main style="padding:24px" {
            h1 { "Insights — " (vm.repo) }
            h2 { "Ledger" }
            ul {
                @for c in &vm.ledger.campaigns {
                    li { (c.campaign.label) " — pedido " (c.asked) " · feito " (c.done) " · provado " (c.proven) }
                }
            }
        }
    }
}
