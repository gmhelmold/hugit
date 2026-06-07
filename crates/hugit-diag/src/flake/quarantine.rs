//! Policy-artifact quarantine list — annotation-only, no auto-act (contract ③).
//!
//! ## v0 auto-act prohibition (contract ③)
//!
//! "auto-act" is defined in the contract as ANY of:
//!   - reorder: changing the execution order of tests based on quarantine
//!   - skip:    omitting a test from execution
//!   - block:   preventing a check from running
//!   - annotation-that-gates: an annotation that causes a gate to fail
//!
//! ALL of these are PROHIBITED in v0.  The only permitted surface is
//! **annotation-only** (non-gating): a human-readable note that a test was
//! detected as flaky.
//!
//! ## Proof of absence
//!
//! [`AutoActMechanism`] is an uninhabited (zero-variant) enum.  There is no
//! value of this type that can ever exist at runtime — the type is the formal
//! proof that the mechanism is absent.  It has zero size.
//!
//! [`QuarantineList`] exposes only [`QuarantineList::annotation_for`] — a
//! read-only lookup.  There is no `reorder`, `skip`, `block`, or `gate` method.

/// Proof that no auto-act mechanism exists in v0.
///
/// This is a zero-variant (uninhabited) enum.  It can never be constructed.
/// Its zero size (asserted by the acceptance oracle) confirms the mechanism
/// is structurally absent.
///
/// Contract ③: "any reorder/skip/block/annotation-that-gates = prohibited in
/// v0 (annotation-only allowed)".
pub enum AutoActMechanism {}

/// A non-gating annotation for one quarantined test.
///
/// Carries exactly the information needed for a human to decide whether to
/// act (investigate, xfail, or suppress).  It does NOT gate, reorder, skip,
/// or block any execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineAnnotation {
    /// The `memo_key` of the test this annotation concerns.
    pub test_id: String,
    /// Human-readable note explaining the quarantine reason.
    pub note: String,
    /// Always `false` in v0 — this annotation NEVER gates an execution.
    ///
    /// The field exists to be asserted `== false` by the acceptance oracle,
    /// making the non-gating invariant an explicit, machine-checkable property.
    pub gates: bool,
}

/// The quarantine list: a **policy artifact** that carries annotations for
/// tests detected as flaky.
///
/// It is entirely read-only from the caller's perspective.  The only operation
/// is [`QuarantineList::annotation_for`].  No auto-act methods are present.
#[derive(Debug, Clone, Default)]
pub struct QuarantineList {
    annotations: Vec<QuarantineAnnotation>,
}

impl QuarantineList {
    /// Add one non-gating annotation (internal, called only by the collector).
    pub(crate) fn add(&mut self, test_id: String, note: String) {
        self.annotations.push(QuarantineAnnotation {
            test_id,
            note,
            gates: false, // invariant: NEVER gating in v0.
        });
    }

    /// Look up the annotation for a given `test_id` (read-only).
    ///
    /// Returns `None` if the test is not quarantined — i.e. it was not
    /// classified [`crate::flake::Classification::Flaky`] within this
    /// volume window.
    pub fn annotation_for(&self, test_id: &str) -> Option<&QuarantineAnnotation> {
        self.annotations.iter().find(|a| a.test_id == test_id)
    }

    /// Iterate over all annotations (read-only).
    pub fn iter(&self) -> impl Iterator<Item = &QuarantineAnnotation> {
        self.annotations.iter()
    }

    /// Number of annotations in this list.
    pub fn len(&self) -> usize {
        self.annotations.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.annotations.is_empty()
    }
}
