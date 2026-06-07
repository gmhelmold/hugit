//! The git wire-protocol **read path**: clone + delta-only fetch.
//!
//! The pipeline is three stages, each its own module:
//!
//! ```text
//!  D1 derived-view refs ──► negotiate ──► (wants, haves) ──► serve ──► pack ──► packfile
//!         (RefView)        (protocol v2)                   (closure)  (from CAS)
//! ```
//!
//! - [`negotiate`] frames protocol v2 (capabilities, ref advertisement,
//!   want/have).
//! - [`pack`] assembles a real git packfile from CAS-held objects.
//! - [`serve`] is the clone / fetch orchestration that walks reachability and
//!   drives pack assembly.

pub mod negotiate;
pub mod pack;
pub mod serve;

// WP-D2b — the read path at the edges (D2 items ③④⑤⑥⑦): client matrix + jj
// stacked changes, the per-request CPU budget + chunked fallback, and the scale
// ceilings + degradation invariant. These load and prove the D2a core above;
// they never modify negotiate/pack/serve.
pub mod clients;
pub mod fallback;
pub mod limits;
