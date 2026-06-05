//! hugit-app-sidecar — intent sidecar parser, validator, renderer, and
//! corpus writer (WP-B6).
//!
//! Consumes the frozen `IntentSidecar` and `AppWebhooks` contracts from
//! `hugit-contracts`. Non-authoritative by design: the sidecar NEVER gates
//! or blocks landing (B6④).
//!
//! Public surface:
//! - [`parse`]  — parse and validate raw JSON into `IntentSidecar`
//! - [`render`] — render a validated sidecar to PR comment + check summary
//! - [`corpus`] — write the validated sidecar to the CAS corpus keyed by
//!   `intent_id`
//! - [`guard`] — non-authoritative guard: assert the sidecar is never
//!   authoritative

pub mod corpus;
pub mod guard;
pub mod parse;
pub mod render;

pub use corpus::{CasRef, CorpusWriter};
pub use guard::assert_non_authoritative;
pub use parse::{ParseError, ValidationFailure, parse_sidecar};
pub use render::{RenderOutput, render_sidecar};
