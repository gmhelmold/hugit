//! Intent detail — the record of intent. Spec: ../githugr/design/intent.html
//!
//! ⚠️ WP-W4 STUB: replaced render-faithfully by the W4 agent (header/meta,
//! resumo, charter, transcript/journal/context accordions, diff + why-blame,
//! rail: autoria · métricas · snapshot · verdicts).

use crate::provider::IntentDetailVm;
use maud::{Markup, html};

/// Screen-specific CSS (W4 ports it from the mockup's `<style>` block).
pub const SCREEN_CSS: &str = "";

pub fn render(vm: &IntentDetailVm) -> Markup {
    html! {
        main style="padding:24px" {
            h1 { "intent " (vm.id) }
            p { (vm.title) " · " (vm.status) }
            h2 { "Charter" }
            p { (vm.charter) }
        }
    }
}
