//! The Wave-1 read handlers — one pure `build_<screen>` mapping fn per `/v1` read.
//!
//! Each handler receives an ALREADY-verified [`EventLog`](hugit_refstore::EventLog)
//! and returns its frozen [`hugit_http_contracts`] view-model. Bodies are filled
//! per the source map in `docs/plan/2026-06-13-hugit-serve-wave1-master-plan.md`
//! (§0/§5): REAL engine data where it exists, the documented honest default
//! elsewhere — never faked. Signatures are FROZEN (the scaffold ⇄ handler seam).

pub mod checks;
pub mod commits;
pub mod home;
pub mod landing;
pub mod pr_detail;

pub use checks::build_checks;
pub use commits::build_commits;
pub use home::build_home;
pub use landing::build_landing;
pub use pr_detail::build_pr_detail;
