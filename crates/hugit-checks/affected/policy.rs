//! Fail-open policy for unknown ecosystems (B3 ③).
//!
//! When the ecosystem is unrecognized, return the FULL check set — never
//! silently skip checks. "Fail-open" here means "over-run" (run everything),
//! the safe direction for an affected-set. Contrast with the fail-CLOSED
//! security/gate paths elsewhere in hugit.

use super::{AffectedSet, FullSetReason, PackageName};

/// Apply the fail-open policy for an unknown ecosystem.
///
/// Returns a full-set [`AffectedSet`] with [`FullSetReason::UnknownEcosystem`].
/// Every downstream consumer (B4a union batcher, B2a glob sensitivity) must
/// treat an `is_full_set` result as "run all checks" without further filtering.
pub fn apply_fail_open(
    ecosystem_tag: impl Into<String>,
    all_packages: impl IntoIterator<Item = PackageName>,
) -> AffectedSet {
    let hint = ecosystem_tag.into();
    AffectedSet::full(all_packages, FullSetReason::UnknownEcosystem { hint })
}

/// Check whether an ecosystem tag is recognized by this version of the engine.
///
/// Recognized ecosystems: `"cargo"`, `"pnpm"`, `"turbo"` (all lowercase).
/// Everything else triggers the fail_open full_set path.
pub fn is_recognized_ecosystem(tag: &str) -> bool {
    matches!(tag, "cargo" | "pnpm" | "turbo")
}
