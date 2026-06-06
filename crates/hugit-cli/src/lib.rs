//! hugit-cli — command implementations.
//!
//! Module layout (body lands per owning WP):
//!   - `verdict/`    — adversarial verdict panels (WP-D7)
//!   - `why/`        — `hugit why` provenance resolver (WP-D10)
//!   - `impact/`     — `hugit impact` blast-radius query + ground-truth export (WP-D10)
//!   - `attention/`  — attention queue: composite ranking, fast-approve gating,
//!     honest degradation (WP-D9). Lives at the crate root per its WP-D9
//!     Claims, wired in via `#[path]`.
//!   - `tournament/` — `hugit tournament -n N` fan-out: N candidates, judge
//!     panel, budget-bounded (WP-D13)
//!   - `export/`     — `hugit export` + the exit proof (WP-E5)

pub mod export;
pub mod impact;
pub mod tournament;
pub mod verdict;
pub mod why;

#[path = "../attention/mod.rs"]
pub mod attention;
