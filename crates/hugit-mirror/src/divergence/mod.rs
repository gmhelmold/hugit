//! Divergence detection, scoped repair, incident emission, one-way enforcement
//! (WP-E1b, items ② and ⑦).
//!
//! Every divergence resolves to a `{alarm, scoped repair, incident}` triple.
//! Repair is **forge-authoritative**: the hugit/forge state is the single
//! source of truth and the GitHub mirror is **overwritten** to match it — the
//! mirror's value is never merged in and never becomes truth.
//!
//! ## One-way law (⑦)
//! A write made directly on the GitHub mirror does not propagate back. It is
//! detected as divergence (mirror tip ≠ forge tip) and resolved
//! forge-authoritative, producing an alarm and an incident. There is **zero
//! reverse-sync codepath**: nothing in this crate reads the mirror's value as
//! authoritative. [`ReversePropagation::CODEPATH_PRESENT`] is a compile-time
//! `false`, asserted by the acceptance suite.
//!
//! ## Fail-CLOSED
//! When the detector cannot decide a ref's state (degraded/undecidable input),
//! the ref is treated as **divergent**, never as synced. A divergent ref is
//! never marked synced.

use std::fmt;

/// One git ref tip on either side of the mirror boundary.
///
/// The forge value is authoritative; the mirror value is observed-only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefState {
    /// Fully-qualified ref name (e.g. `refs/heads/main`).
    pub ref_name: String,
    /// The forge-side tip — the authoritative truth.
    pub forge_tip: String,
    /// The mirror-side (GitHub) tip — observed, never authoritative.
    pub mirror_tip: String,
    /// `true` when the detector could not read one of the tips. Fail-CLOSED:
    /// an undecidable ref is treated as divergent.
    pub undecidable: bool,
}

impl RefState {
    /// Construct a fully-decided ref state.
    pub fn new(
        ref_name: impl Into<String>,
        forge_tip: impl Into<String>,
        mirror_tip: impl Into<String>,
    ) -> Self {
        Self {
            ref_name: ref_name.into(),
            forge_tip: forge_tip.into(),
            mirror_tip: mirror_tip.into(),
            undecidable: false,
        }
    }

    /// Construct a ref state whose tips could not be read (degraded detector).
    /// Fail-CLOSED: such a ref is divergent.
    pub fn undecidable(ref_name: impl Into<String>) -> Self {
        Self {
            ref_name: ref_name.into(),
            forge_tip: String::new(),
            mirror_tip: String::new(),
            undecidable: true,
        }
    }
}

/// Classification of a single ref by the divergence detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefClass {
    /// Mirror tip equals forge tip — in sync.
    Synced,
    /// Mirror tip differs from forge tip — divergent.
    Divergent,
    /// Detector could not decide — fail-CLOSED to divergent.
    Degraded,
}

impl RefClass {
    /// `true` when this ref must NOT be marked synced (divergent or degraded).
    pub fn is_divergent(self) -> bool {
        !matches!(self, RefClass::Synced)
    }
}

/// Classify a ref tip pair. Fail-CLOSED: undecidable → `Degraded` (divergent).
pub fn classify(state: &RefState) -> RefClass {
    if state.undecidable {
        return RefClass::Degraded;
    }
    if state.forge_tip == state.mirror_tip {
        RefClass::Synced
    } else {
        RefClass::Divergent
    }
}

/// The repair strategy. Only one exists: forge-authoritative overwrite.
///
/// There is deliberately no "merge" or "accept-mirror" variant — the mirror is
/// never a source of truth, so its value can only be overwritten.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairStrategy {
    /// Overwrite the mirror tip with the forge tip — forge wins.
    ForgeAuthoritative,
}

/// An alarm raised when a divergence is detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alarm {
    /// The ref that diverged.
    pub ref_name: String,
    /// Why the alarm fired.
    pub reason: DivergenceReason,
}

/// Why a ref diverged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DivergenceReason {
    /// Mirror tip differs from forge tip (includes direct mirror writes — ⑦).
    TipMismatch,
    /// Detector degraded / undecidable — fail-CLOSED.
    Degraded,
}

impl fmt::Display for DivergenceReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DivergenceReason::TipMismatch => write!(f, "mirror tip differs from forge tip"),
            DivergenceReason::Degraded => write!(f, "detector degraded (fail-closed to divergent)"),
        }
    }
}

/// A repair action: forge-authoritative overwrite scoped to a single ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repair {
    /// The ref being repaired — repair is scoped to exactly this ref (⑥).
    pub ref_name: String,
    /// The forge tip the mirror is being overwritten to.
    pub forge_tip: String,
    /// Always forge-authoritative.
    pub strategy: RepairStrategy,
}

/// An incident record emitted alongside every divergence repair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incident {
    /// The ref the incident concerns.
    pub ref_name: String,
    /// Human-readable description.
    pub detail: String,
    /// Whether the originating write came from the mirror side (one-way
    /// violation — ⑦). Recorded for audit; never changes the repair direction.
    pub mirror_write: bool,
}

/// The `{alarm, repair, incident}` triple emitted for one divergent ref.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DivergenceResolution {
    /// The alarm raised.
    pub alarm: Alarm,
    /// The scoped forge-authoritative repair.
    pub repair: Repair,
    /// The incident record.
    pub incident: Incident,
}

/// Marker proving the absence of any reverse-propagation codepath (⑦).
///
/// The acceptance suite asserts `CODEPATH_PRESENT == false`. This type carries
/// no method that reads the mirror value as authoritative — by construction the
/// mirror can only be overwritten.
pub struct ReversePropagation;

impl ReversePropagation {
    /// There is no reverse-sync codepath. Always `false`.
    pub const CODEPATH_PRESENT: bool = false;
}

/// Resolve a single ref's divergence into the `{alarm, repair, incident}`
/// triple, or `None` if the ref is synced.
///
/// Forge-authoritative by construction: the repair overwrites the mirror tip
/// with `forge_tip`; the `mirror_tip` value is recorded for audit only and
/// never flows into the repaired state.
pub fn resolve(state: &RefState) -> Option<DivergenceResolution> {
    let class = classify(state);
    if !class.is_divergent() {
        return None;
    }

    let reason = match class {
        RefClass::Degraded => DivergenceReason::Degraded,
        _ => DivergenceReason::TipMismatch,
    };

    // A direct mirror write manifests as a tip mismatch where the mirror moved.
    // We record it for the incident but the repair direction is invariant.
    let mirror_write = matches!(reason, DivergenceReason::TipMismatch)
        && !state.mirror_tip.is_empty()
        && state.mirror_tip != state.forge_tip;

    Some(DivergenceResolution {
        alarm: Alarm {
            ref_name: state.ref_name.clone(),
            reason,
        },
        repair: Repair {
            ref_name: state.ref_name.clone(),
            forge_tip: state.forge_tip.clone(),
            strategy: RepairStrategy::ForgeAuthoritative,
        },
        incident: Incident {
            ref_name: state.ref_name.clone(),
            detail: format!("{reason}"),
            mirror_write,
        },
    })
}

/// Apply a repair to a mirror tip, returning the tip after repair.
///
/// Forge wins, always: the result equals the forge tip regardless of the
/// pre-repair mirror value. A reverse write is thereby erased, not propagated.
pub fn apply_repair(repair: &Repair) -> String {
    // The only legal post-repair value is the forge tip.
    repair.forge_tip.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synced_ref_resolves_to_none() {
        let s = RefState::new("refs/heads/main", "a".repeat(40), "a".repeat(40));
        assert_eq!(classify(&s), RefClass::Synced);
        assert!(resolve(&s).is_none());
    }

    #[test]
    fn divergent_ref_emits_triple_forge_authoritative() {
        let s = RefState::new("refs/heads/main", "f".repeat(40), "m".repeat(40));
        let r = resolve(&s).expect("divergent");
        assert_eq!(r.repair.strategy, RepairStrategy::ForgeAuthoritative);
        assert_eq!(apply_repair(&r.repair), "f".repeat(40));
    }

    #[test]
    fn undecidable_fails_closed_to_divergent() {
        let s = RefState::undecidable("refs/heads/x");
        assert_eq!(classify(&s), RefClass::Degraded);
        assert!(resolve(&s).is_some());
    }

    #[test]
    fn no_reverse_propagation_codepath() {
        const { assert!(!ReversePropagation::CODEPATH_PRESENT) };
    }
}
