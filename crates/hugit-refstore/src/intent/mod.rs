//! Native **Intent** objects over the D1 event log, and the one-directional
//! projection of the intent log down to git's two altitudes.
//!
//! WP-D4 makes Intents *first-class over* the append-only event log
//! ([`crate::log`]) — not a parallel store. The whole module rests on a single
//! invariant lifted from `docs/whitepaper/hugit-v1.md` §4:
//!
//! > Same store, two zooms; they can never disagree because one is derived from
//! > the other.
//!
//! Concretely:
//!
//! - [`model`] — native Intent objects read *out of* the event log. An intent
//!   is an event (`intent.landed`) carrying its `intent_id`; nothing is stored
//!   twice. A raw push (`ref.update` with no intent linkage) is **not** an
//!   intent and is never made into one.
//! - [`projection`] — the deterministic, one-directional intent→git projection.
//!   Every landed intent emits a generated git commit whose message **embeds the
//!   `intent_id`**; the commit set is reproducible from the log (①). The intent
//!   altitude (`hugit log`) and the machine altitude (`git log`) are both folds
//!   of the same log, so they are provably consistent (②). Raw pushes project as
//!   `external-change`, never as a synthesised intent (④).
//! - [`import`] — import of a B6 [`IntentSidecar`] corpus into the native intent
//!   model **by `intent_id`**: one lifecycle, one id (③).
//!
//! This module is purely additive over D1a's frozen public API
//! ([`crate::log`] / [`crate::replay`] / [`crate::tamper`]): it *consumes* the
//! event log, it never rewrites it.
//!
//! [`IntentSidecar`]: hugit_contracts::intent_sidecar::IntentSidecar

pub mod import;
pub mod model;
pub mod projection;

pub use import::{ImportError, import_sidecar};
pub use model::{
    INTENT_LANDED_KIND, Intent, IntentLog, IntentModelError, RAW_PUSH_KINDS, intents_from_log,
};
pub use projection::{
    Altitude, GitCommit, MachineHistory, ProjectionError, ProjectionRow, project, project_machine,
};
