//! Affected-set shape (consumed from B3) and disjointness.
//!
//! B3 produces, per landable change, the set of memoised-check keys it
//! touches (its blast radius). B4a only needs two facts from that shape:
//! the set membership and whether two changes' sets overlap. We model it
//! here as an ordered set of opaque keys — the engine never interprets the
//! keys, it only tests membership and intersection.

use std::collections::BTreeSet;

/// The set of affected check-keys for a single landable change (B3's shape).
///
/// Engine-internal: B4a consumes the *shape* (a set of opaque keys), not a
/// frozen `hugit-contracts` type. Disjointness of two changes is the empty
/// intersection of their affected sets (contract: "Disjointness =
/// affected-sets non-overlapping").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AffectedSet {
    keys: BTreeSet<String>,
}

impl AffectedSet {
    /// Build an affected set from any iterator of check-keys.
    pub fn new<I, S>(keys: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            keys: keys.into_iter().map(Into::into).collect(),
        }
    }

    /// The affected check-keys, in deterministic (sorted) order.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.keys.iter().map(String::as_str)
    }

    /// True when this set shares no check-key with `other` — i.e. the two
    /// changes are disjoint and may land in parallel lanes (contract ②).
    pub fn is_disjoint(&self, other: &AffectedSet) -> bool {
        self.keys.is_disjoint(&other.keys)
    }

    /// Number of affected keys.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// True when no key is affected.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disjoint_sets_report_disjoint() {
        let a = AffectedSet::new(["src/a.rs", "src/lib.rs"]);
        let b = AffectedSet::new(["src/b.rs", "src/main.rs"]);
        assert!(a.is_disjoint(&b));
    }

    #[test]
    fn overlapping_sets_report_not_disjoint() {
        let a = AffectedSet::new(["src/a.rs", "src/shared.rs"]);
        let b = AffectedSet::new(["src/b.rs", "src/shared.rs"]);
        assert!(!a.is_disjoint(&b));
    }

    #[test]
    fn keys_are_sorted_and_deduped() {
        let s = AffectedSet::new(["c", "a", "b", "a"]);
        assert_eq!(s.keys().collect::<Vec<_>>(), vec!["a", "b", "c"]);
        assert_eq!(s.len(), 3);
    }
}
