//! The Wave-1 read handlers — one pure `build_<screen>` mapping fn per `/v1` read.
//!
//! Each handler receives an ALREADY-verified [`EventLog`](hugit_refstore::EventLog)
//! and returns its frozen [`hugit_http_contracts`] view-model. Bodies are filled
//! with REAL engine data where it exists, the documented honest default
//! elsewhere — never faked. Signatures are FROZEN (the scaffold ⇄ handler seam).

pub mod admin;
pub mod attention;
pub mod blob;
pub mod branches;
pub mod campaign;
pub mod checks;
pub mod commit_detail;
pub mod commits;
pub mod compare;
pub mod dashboard;
pub mod diff;
pub mod edit;
pub mod events;
pub mod home;
pub mod insights;
pub mod intent_detail;
pub mod issues;
pub mod knowledge;
pub mod landing;
pub mod login;
pub mod new_pr;
pub mod org;
pub mod pr_detail;
pub mod releases;
pub mod repo_chrome;
pub mod repo_settings;
pub mod review;
pub mod review_qa;
pub mod search;
pub mod security;
pub mod usage_fold;
pub mod viewer_can;

pub use admin::{build_admin_overview, build_admin_tokens, build_audit, build_erasure};
pub use attention::{build_attention, build_me_attention};
pub use blob::build_blob;
pub use branches::build_branches;
pub use campaign::build_campaign;
pub use checks::build_checks;
pub use commit_detail::build_commit_detail;
pub use commits::build_commits;
pub use compare::build_compare;
pub use dashboard::{build_dashboard, build_me_dashboard};
pub use edit::build_edit;
pub use events::build_events;
pub use home::build_home;
pub use insights::build_insights;
pub use intent_detail::build_intent_detail;
pub use issues::build_issues;
pub use knowledge::build_knowledge;
pub use landing::build_landing;
pub use login::build_login;
pub use new_pr::build_new_pr;
pub use org::{build_me_org, build_org};
pub use pr_detail::build_pr_detail;
pub use releases::build_releases;
pub use repo_chrome::build_repo_chrome;
pub use repo_settings::build_repo_settings;
pub use review::build_review;
pub use review_qa::build_review_qa;
pub use search::build_search;
pub use security::build_security;
pub use viewer_can::build_viewer_can;
