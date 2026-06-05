//! Ported gate evaluators (WP-D6, item ①).
//!
//! Each gate is a pure `fn(&EvalContext) -> GateOutcome` with no I/O.
//! The 3 gates below mirror the house's `.github` enforcement exactly.

pub mod changelog;
pub mod dco;
pub mod secrets;
