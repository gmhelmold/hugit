//! hugit-app-ui — surface v0 (WP-B7).
//!
//! Provides: live status page (①), exactly-one-edited-comment-per-PR (②),
//! saved-minutes → CheckResult audit link (③), versioned auditable cost model (④).
//!
//! Claims: `crates/hugit-app/ui/` only.
//! Does NOT touch sidecar (B6) or the rest of hugit-app (B1).

pub mod comment;
pub mod cost_model;
pub mod status;

pub use comment::{COMMENT_MARKER, CommentRenderer};
pub use cost_model::{CostModel, CostModelVersion, SavedCost};
pub use status::{InstallState, PrCheckState, StatusPage};
