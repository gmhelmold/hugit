//! Deterministic verdict-panel fixtures (WP-D7).
//!
//! These fixtures power the acceptance lane WITHOUT any live model call. Each
//! sub-module ships deterministic reviewers and the served corpus the
//! acceptance tests assert over:
//!
//! - [`lens_isolation`] — distinct, isolated lenses + the prompt-isolation
//!   corpus (①).
//! - [`planted_bug`] — a served corpus carrying a semantic/logic bug that no
//!   author test covers, and a reviewer lens that still catches it (③).
//! - [`persuasion_channel`] — an `IntentSidecar` whose author-controlled fields
//!   carry a persuasive false self-justification, used to prove the verdict is
//!   byte-identical with vs without it (⑥).

pub mod lens_isolation;
pub mod persuasion_channel;
pub mod planted_bug;
