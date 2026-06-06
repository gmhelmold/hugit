//! hugit-diag — diagnostics + the experiment harness.
//!
//! Module layout (body lands per owning WP):
//!   - `experiment/` — the experiment harness + experiment gate (WP-D8):
//!     auto-collects claim-disjointness + regen-honesty datapoints from the
//!     fleet's real waves and BINDS the experiment gate (claims-as-oracle
//!     advisory/OFF + regen promotion blocked until the report shows PASS).

pub mod experiment;
