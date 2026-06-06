//! hugit-cli — command implementations.
//!
//! Module layout (body lands per owning WP):
//!   - `verdict/` — adversarial verdict panels (WP-D7)
//!   - `why/`     — `hugit why` provenance resolver (WP-D10)
//!   - `impact/`  — `hugit impact` blast-radius query + ground-truth export (WP-D10)

pub mod impact;
pub mod verdict;
pub mod why;
