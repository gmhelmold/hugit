//! `hugit why` — provenance resolver for lines, symbols, and derived bytes.
//!
//! WP-D10 item ①: resolve a line or symbol to the originating intent +
//! charter/author/model/cost, matching the event log.
//!
//! WP-D10 item ④ (R6): resolve regenerated/derived bytes to the
//! regen/derivation event — NEVER fabricate or mis-attribute a human author.

pub mod resolver;

pub use resolver::{AuthorKind, ProvenanceAnswer, WhyError, WhyQuery, resolve_why};
