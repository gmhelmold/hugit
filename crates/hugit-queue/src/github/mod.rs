//! GitHub API surface of the union-testing landing queue (WP-B4b).
//!
//! This module is the *API-surface half* of B4 — it rides B4a's pure engine
//! (`crate::core`) and drives real GitHub side effects. It owns items ③④⑥:
//!
//! * ③ **force-push recompute** — on a force-push webhook to a batched PR's
//!   head, the prior union is invalidated and the batch is re-folded via B4a;
//!   no stale union may land. Recorded as an [`hugit_contracts::EventRecord`].
//!   See [`recompute`].
//! * ④ **crash idempotency (kill-test)** — the worker may die mid-land; on
//!   restart the durable state replays B4a's idempotent transitions, so there
//!   is no double-merge, no lost batch, no false green. See [`recovery`].
//! * ⑥ **branch protection** — a PR under branch protection / required review
//!   is HELD and reported (an `EventRecord` + surfaced status), NEVER
//!   force-merged; the configured merge method is honored on land. See
//!   [`protection`] and [`merge`].
//!
//! ## Design: a pure core behind a thin GitHub seam
//!
//! All decision logic (recompute trigger, hold predicate, merge-method
//! dispatch, idempotent replay) is GitHub-free and unit-testable. The only
//! impure part is [`MergeApi`] — the trait that performs the actual ordered
//! atomic merge against GitHub. The live acceptance test supplies a real
//! installation-token-backed implementation (App JWT → installations → merge);
//! the unit tests supply an in-process double. CoreLink is consumed as a
//! CLIENT and GitHub via B1's installation-token model — zero server changes,
//! credentials never logged (write-only secret model).

pub mod app_auth;
pub mod merge;
pub mod protection;
pub mod recompute;
pub mod recovery;

pub use app_auth::{AppCredentials, AppJwt, InstallationLookup, JwtError};
pub use merge::{MergeApi, MergeError, MergeMethod, MergeOutcome, MergeRecord, drive_atomic_merge};
pub use protection::{HoldReason, ProtectionStatus, evaluate_protection};
pub use recompute::{RecomputeTrigger, recompute_on_force_push};
pub use recovery::{LandLog, RecoveryOutcome, recover_and_replay};
