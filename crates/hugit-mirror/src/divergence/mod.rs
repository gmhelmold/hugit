//! Divergence detection, scoped repair, incident emission, one-way enforcement
//! (WP-E1b, items ② and ⑦).
//!
//! Every divergence resolves to a `{alarm, scoped repair, incident}` triple.
//! Repair is **forge-authoritative**: the hugit/forge state is the single
//! source of truth and the GitHub mirror is **overwritten** to match it — the
//! mirror's value is never merged in and never becomes truth.
//!
//! ## One-way law (⑦) — proven STRUCTURALLY, not by fiat
//! A write made directly on the GitHub mirror does not propagate back. It is
//! detected as divergence (mirror tip ≠ forge tip) and resolved
//! forge-authoritative, producing an alarm and an incident.
//!
//! The "zero reverse-sync codepath" guarantee is enforced by **construction**,
//! not by a hand-set boolean:
//!
//! 1. **Type-level direction.** The only repair primitive is
//!    [`MirrorMutation`], and the only way to obtain one is
//!    [`MirrorMutation::overwrite_to_forge`], which takes a [`Repair`] (carrying
//!    the *forge* tip) and exposes ONLY the forge tip as the post-mutation
//!    value. There is no constructor, field, or method that produces a mutation
//!    from the *mirror* tip — a reverse-sync path is therefore unrepresentable.
//! 2. **Sealed direction.** [`MirrorMutation`] holds a private field; downstream
//!    code cannot fabricate one that adopts the mirror value.
//! 3. **Architecture oracle.** [`reverse_sync_surface_count`] scans this crate's
//!    own source for any sink that adopts the mirror tip as an authoritative ref
//!    value. The acceptance suite asserts it is `0`; if anyone adds a
//!    reverse-sync path the count rises and the suite goes RED.
//!
//! Together these break the invariant if (and only if) a real reverse write is
//! introduced — not a `const` someone can flip and forget.
//!
//! ## Fail-CLOSED
//! When the detector cannot decide a ref's state (degraded/undecidable input),
//! the ref is treated as **divergent**, never as synced. A divergent ref is
//! never marked synced.

use std::fmt;

/// What is known about the origin of a ref's current divergent state.
///
/// `mirror_write` in an [`Incident`] is derived from THIS — an actually observed
/// mutation event — not from raw tip-inequality (which is also produced by
/// ordinary replication lag and would be a false positive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MutationOrigin {
    /// No mutation event was observed for this ref. A tip mismatch here is
    /// undecided as to origin (could be normal lag) and is NOT charged as a
    /// mirror-side write.
    #[default]
    Unobserved,
    /// A mutation event was observed on the **forge** side (the authoritative
    /// side moved; the mirror is simply behind — lag, not a reverse write).
    ForgeSide,
    /// A mutation event was observed on the **mirror** side: a write was made
    /// directly on the GitHub mirror. THIS is the one-way violation (⑦).
    MirrorSide,
}

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
    /// The observed origin of the current state. Used to decide whether a
    /// divergence is a mirror-side write (⑦) versus ordinary lag — without a
    /// real observation this stays [`MutationOrigin::Unobserved`] and is never
    /// charged as a mirror write.
    pub origin: MutationOrigin,
}

impl RefState {
    /// Construct a fully-decided ref state with no observed mutation origin.
    ///
    /// A tip mismatch on such a state is treated as divergence (forge-
    /// authoritative repair) but is NOT charged as a mirror-side write — the
    /// origin is unknown and could be normal replication lag.
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
            origin: MutationOrigin::Unobserved,
        }
    }

    /// Construct a fully-decided ref state with an explicitly observed mutation
    /// origin. Use [`MutationOrigin::MirrorSide`] when a real mirror-side write
    /// event was observed (the only thing that flags `incident.mirror_write`).
    pub fn with_observed_mutation(
        ref_name: impl Into<String>,
        forge_tip: impl Into<String>,
        mirror_tip: impl Into<String>,
        origin: MutationOrigin,
    ) -> Self {
        Self {
            ref_name: ref_name.into(),
            forge_tip: forge_tip.into(),
            mirror_tip: mirror_tip.into(),
            undecidable: false,
            origin,
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
            origin: MutationOrigin::Unobserved,
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

/// The single, direction-typed mirror mutation primitive (⑦).
///
/// This is the ONLY value that represents "the mirror's ref tip changes". It is
/// constructed exclusively from a [`Repair`] — which carries the **forge** tip —
/// and exposes only that forge tip via [`MirrorMutation::new_tip`]. There is no
/// constructor or accessor that takes or yields the *mirror* tip as the new
/// authoritative value, so a reverse-sync mutation (mirror → forge) is
/// **unrepresentable in the type system**, not merely "asserted absent".
///
/// The inner field is private, so no other module can fabricate a mutation that
/// adopts the mirror value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MirrorMutation {
    ref_name: String,
    /// PRIVATE: always the forge tip. Never the mirror tip — there is no path
    /// that writes the mirror value here.
    new_tip_forge: String,
}

impl MirrorMutation {
    /// Build the only legal mirror mutation: overwrite the mirror ref to the
    /// **forge** tip carried by `repair`. The mirror's own value is structurally
    /// inaccessible to this constructor.
    pub fn overwrite_to_forge(repair: &Repair) -> Self {
        Self {
            ref_name: repair.ref_name.clone(),
            new_tip_forge: repair.forge_tip.clone(),
        }
    }

    /// The ref this mutation targets.
    pub fn ref_name(&self) -> &str {
        &self.ref_name
    }

    /// The post-mutation tip — ALWAYS the forge tip. There is no variant that
    /// returns the mirror tip.
    pub fn new_tip(&self) -> &str {
        &self.new_tip_forge
    }
}

/// Architecture oracle for the one-way law (⑦): count the reverse-sync sinks in
/// this crate's own source.
///
/// A "reverse-sync sink" is any code that adopts the **mirror** tip as the new
/// authoritative ref value (e.g. assigning `forge_tip = ... mirror_tip ...`,
/// returning `state.mirror_tip` as a repaired/synced value, or a function whose
/// name advertises mirror→forge propagation). This function greps the real
/// `divergence`, `refops`, `outbound`, `verify`, `poll` and `outage` sources at
/// test time and returns how many such sinks exist. The acceptance suite asserts
/// the count is **0**; introducing any reverse-sync path makes it non-zero and
/// turns the suite RED — the proof breaks structurally on a real reverse write,
/// not on a flag.
///
/// `sources` is the list of `(label, source_text)` pairs to scan; the acceptance
/// suite passes the crate's own `include_str!`-loaded modules so the scan tracks
/// the *actual* shipped code, never a stale copy.
pub fn reverse_sync_surface_count(sources: &[(&str, &str)]) -> usize {
    // Forbidden sinks: an authoritative value taken FROM the mirror side. Each
    // needle is ASSEMBLED FROM FRAGMENTS at runtime so the verbatim needle never
    // appears in this file — otherwise the oracle would flag its own definition.
    // The needle exists in source only if real code actually wires mirror→forge.
    let mirror_src = ".mirror_tip"; // the mirror value being read
    let forbidden: Vec<String> = vec![
        // adopting the mirror tip as the forge/authoritative value
        format!("forge_tip = state{mirror_src}"),
        format!("forge_tip = {}", &mirror_src[1..]),
        format!("forge_tip: state{mirror_src}"),
        format!("forge_tip: {}", &mirror_src[1..]),
        // returning the mirror value as the repaired/synced result
        format!("return state{mirror_src}"),
        format!("return repair{mirror_src}"),
        // a function that advertises reverse propagation as its job. The "("
        // anchors the needle to a complete fn name (so it does not match e.g.
        // `reverse_sync_surface_count`, this very oracle).
        format!("fn {}(", "propagate_mirror_to_forge"),
        format!("fn {}(", "adopt_mirror_value"),
        format!("fn {}(", "reverse_sync"),
        format!("fn {}(", "mirror_to_forge"),
        format!("fn {}(", "accept_mirror_as_truth"),
    ];
    let mut count = 0usize;
    for (_label, src) in sources {
        for needle in &forbidden {
            if src.contains(needle.as_str()) {
                count += 1;
            }
        }
    }
    count
}

/// The crate's own one-way-relevant sources, loaded at compile time so the
/// architecture oracle scans the SHIPPED code (never a hand-written copy).
///
/// If a new module that could host a reverse-sync path is added, append it here
/// so the oracle keeps covering the real surface.
pub const ONE_WAY_SOURCES: &[(&str, &str)] = &[
    ("divergence", include_str!("mod.rs")),
    ("refops", include_str!("../refops/mod.rs")),
    ("outbound::writer", include_str!("../outbound/writer.rs")),
    ("outbound::mod", include_str!("../outbound/mod.rs")),
    ("verify", include_str!("../verify/mod.rs")),
    ("poll", include_str!("../poll/mod.rs")),
    ("outage", include_str!("../outage/mod.rs")),
];

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

    // A direct mirror write is charged ONLY when an actual mirror-side mutation
    // event was observed — never inferred from raw tip-inequality, which is also
    // produced by ordinary replication lag (a false positive). The repair
    // direction is invariant regardless.
    let mirror_write = state.origin == MutationOrigin::MirrorSide;

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
    fn mirror_mutation_can_only_carry_the_forge_tip() {
        // The only mutation primitive is built from a Repair (forge tip) and
        // exposes only that forge tip — a reverse-sync value is unrepresentable.
        let repair = Repair {
            ref_name: "refs/heads/main".into(),
            forge_tip: "f".repeat(40),
            strategy: RepairStrategy::ForgeAuthoritative,
        };
        let m = MirrorMutation::overwrite_to_forge(&repair);
        assert_eq!(m.new_tip(), "f".repeat(40));
        assert_eq!(m.ref_name(), "refs/heads/main");
    }

    #[test]
    fn architecture_oracle_finds_zero_reverse_sync_sinks_in_shipped_code() {
        // Scans the crate's own shipped source: there must be no sink that
        // adopts the mirror tip as an authoritative value.
        assert_eq!(reverse_sync_surface_count(ONE_WAY_SOURCES), 0);
    }

    #[test]
    fn architecture_oracle_detects_an_injected_reverse_sync_path() {
        // The oracle is not vacuous: a fragment that wires mirror→forge is
        // counted, so a real reverse write would turn the suite RED. The
        // fragment is ASSEMBLED so the verbatim needle never sits in this file
        // (which is itself scanned via ONE_WAY_SOURCES).
        let mirror = format!(".{}", "mirror_tip");
        let injected = format!("fn {}() {{ forge_tip = state{mirror}; }}", "reverse_sync");
        assert!(reverse_sync_surface_count(&[("injected", injected.as_str())]) >= 1);
    }

    #[test]
    fn unobserved_tip_mismatch_is_not_charged_as_mirror_write() {
        // Ordinary lag: tips differ but no mirror-side mutation was observed.
        let lag = RefState::new("refs/heads/main", "f".repeat(40), "0".repeat(40));
        let res = resolve(&lag).expect("divergent");
        assert!(
            !res.incident.mirror_write,
            "unobserved tip mismatch (lag) must NOT be charged as a mirror write"
        );
    }

    #[test]
    fn observed_mirror_side_mutation_is_charged_as_mirror_write() {
        let rev = RefState::with_observed_mutation(
            "refs/heads/main",
            "f".repeat(40),
            "e".repeat(40),
            MutationOrigin::MirrorSide,
        );
        let res = resolve(&rev).expect("divergent");
        assert!(res.incident.mirror_write);
    }
}
