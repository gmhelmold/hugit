//! Repo home — GitHub-faithful code home. Spec: ../githugr/design/repo-home.html
//!
//! ⚠️ WP-W3 STUB: replaced render-faithfully by the W3 agent (file rows with
//! intent links, README preview, About rail, synergy panel).

use crate::provider::RepoHomeVm;
use maud::{Markup, html};

/// Screen-specific CSS (W3 ports it from the mockup's `<style>` block).
pub const SCREEN_CSS: &str = "";

pub fn render(vm: &RepoHomeVm) -> Markup {
    html! {
        main style="padding:24px" {
            h1 { (vm.repo) " — " (vm.branch) }
            ul {
                @for f in &vm.files {
                    li { (f.name) @if f.is_dir { "/" } " — " (f.message) }
                }
            }
        }
    }
}
