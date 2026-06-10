//! The five spine screens. Each module owns exactly one
//! `pub fn render(vm: &…Vm) -> maud::Markup` (+ its `SCREEN_CSS`), faithful to
//! its mockup in `../githugr/design/`. Screens never touch the provider or the
//! router — they render a frozen VM, nothing else.

pub mod checks;
pub mod insights;
pub mod intent;
pub mod landing;
pub mod repo_home;
