//! Session journals — tenant-private first-class objects bound to a
//! workspace/intent, and `ctx resume` within the supported horizon.
//!
//! # D11 charter
//!
//! Build session journals as tenant-private first-class objects bound to a
//! workspace/intent, and `ctx resume` for the crashed/replaced-agent case
//! within a supported horizon. Beyond the horizon, resume is
//! refused/degraded as documented — no false reconstruction.
//!
//! # Sub-modules
//!
//! - [`persist`] — journal object model, binding assertion, tenant-private
//!   scope, in-memory store.
//! - [`resume`] — `ctx resume` reconstruction logic + horizon enforcement.
//! - [`horizon`] — supported horizon constants + the pure horizon check.
//! - [`fixtures`] — test fixtures (within-horizon crash; beyond-horizon).

pub mod horizon;
pub mod persist;
pub mod resume;

pub mod fixtures;

pub use horizon::{DEFAULT_HORIZON_MS, HorizonResult, check_horizon};
pub use persist::{Journal, JournalEntry, JournalError, JournalKey, JournalStore};
pub use resume::{ReconstructedContext, ResumeError, ctx_resume, ctx_resume_from_journal};
